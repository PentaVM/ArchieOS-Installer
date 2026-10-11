//! The long jobs REM pauses, and getting every one of them back.
//!
//! A job is registered as a file: how to checkpoint and pause it, how to resume it, and
//! how to tell that it is making progress. On 2026-09-28 a planned pause left a GPU idle
//! for 2 h 20 min because the resume lived in a chat session that ended and the wait
//! judged a process by a pipeline's exit status. So the list of what was paused is written
//! to disk before anything is paused, and "resumed" means the heartbeat moved.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum JobError {
    #[error("cannot read {path}: {source}")]
    Read {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("{path} is not a valid job: {reason}")]
    Invalid { path: String, reason: String },
    #[error("cannot write the resume list {path}: {source}")]
    Write {
        path: String,
        #[source]
        source: std::io::Error,
    },
}

/// One long job, as the file that registers it says.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Job {
    pub id: String,
    /// Saves the job's state and stops it. Run as a program and its arguments, no shell.
    pub checkpoint: Vec<String>,
    /// Starts it again from that state.
    pub resume: Vec<String>,
    /// A file the job rewrites while it makes progress.
    pub heartbeat: PathBuf,
    /// How old the heartbeat may be before the job counts as stuck.
    pub max_age_seconds: u64,
}

impl Job {
    fn validate(&self) -> Result<(), String> {
        let named = !self.id.is_empty()
            && self.id.len() <= 64
            && self.id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.');
        if !named {
            return Err("id is 1 to 64 letters, digits, '-', '_' or '.'".into());
        }
        if self.checkpoint.is_empty() || self.resume.is_empty() {
            return Err("checkpoint and resume each name a program".into());
        }
        if self.max_age_seconds == 0 {
            return Err("max_age_seconds is at least 1".into());
        }
        Ok(())
    }

    /// Whether the job has made progress since `since`: its heartbeat was rewritten after
    /// it. A process that exists and writes nothing has not resumed.
    pub fn moved_since(&self, since: SystemTime) -> bool {
        std::fs::metadata(&self.heartbeat)
            .and_then(|meta| meta.modified())
            .is_ok_and(|modified| modified > since)
    }

    /// Whether the job is alive at all: its heartbeat is younger than its maximum age.
    pub fn is_live(&self, now: SystemTime) -> bool {
        std::fs::metadata(&self.heartbeat)
            .and_then(|meta| meta.modified())
            .ok()
            .and_then(|modified| now.duration_since(modified).ok())
            .is_some_and(|age| age.as_secs() <= self.max_age_seconds)
    }
}

/// Every `*.toml` job in `dir`, sorted by file name. A missing directory is no jobs; a
/// file that does not parse is an error, since a job REM does not know about is a job it
/// will train on top of.
pub fn load_jobs(dir: &Path) -> Result<Vec<Job>, JobError> {
    let read = |path: &Path, source| JobError::Read { path: path.display().to_string(), source };
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| read(dir, e))?
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| path.extension().is_some_and(|e| e == "toml"))
        .collect();
    files.sort();
    let mut jobs: Vec<Job> = Vec::new();
    for path in files {
        let invalid = |reason: String| JobError::Invalid { path: path.display().to_string(), reason };
        let text = std::fs::read_to_string(&path).map_err(|e| read(&path, e))?;
        let job: Job = toml::from_str(&text).map_err(|e| invalid(e.to_string()))?;
        job.validate().map_err(invalid)?;
        if jobs.iter().any(|other| other.id == job.id) {
            return Err(invalid(format!("the id {} is registered twice", job.id)));
        }
        jobs.push(job);
    }
    Ok(jobs)
}

/// What was paused and must be resumed, on disk.
#[derive(Debug, Clone)]
pub struct ResumeList {
    path: PathBuf,
}

impl ResumeList {
    pub fn at(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// Replaces the list. Written beside itself and renamed, so a crash leaves the old
    /// list or the new one, never half of either.
    pub fn write(&self, jobs: &[Job]) -> Result<(), JobError> {
        let failed = |source| JobError::Write { path: self.path.display().to_string(), source };
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(failed)?;
        }
        let partial = self.path.with_extension("partial");
        let text = serde_json::to_string_pretty(jobs).expect("jobs serialize");
        std::fs::write(&partial, text).map_err(failed)?;
        std::fs::rename(&partial, &self.path).map_err(failed)
    }

    /// The jobs still to resume. No file is nothing to resume.
    pub fn read(&self) -> Result<Vec<Job>, JobError> {
        match std::fs::read_to_string(&self.path) {
            Ok(text) => serde_json::from_str(&text).map_err(|e| JobError::Invalid {
                path: self.path.display().to_string(),
                reason: e.to_string(),
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(source) => Err(JobError::Read { path: self.path.display().to_string(), source }),
        }
    }

    pub fn clear(&self) -> Result<(), JobError> {
        match std::fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(source) => Err(JobError::Write { path: self.path.display().to_string(), source }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("archie-rem-jobs-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn job(id: &str, heartbeat: &Path) -> Job {
        Job {
            id: id.into(),
            checkpoint: vec!["supervisorctl".into(), "stop".into(), id.into()],
            resume: vec!["supervisorctl".into(), "start".into(), id.into()],
            heartbeat: heartbeat.into(),
            max_age_seconds: 600,
        }
    }

    #[test]
    fn jobs_are_read_from_their_files_and_a_bad_one_is_an_error() {
        let dir = scratch("load");
        assert!(load_jobs(&dir.join("absent")).unwrap().is_empty());
        std::fs::write(
            dir.join("trainer.toml"),
            "id = \"trainer\"\ncheckpoint = [\"stop-trainer\"]\nresume = [\"start-trainer\"]\n\
             heartbeat = \"/run/trainer/heartbeat.json\"\nmax_age_seconds = 900\n",
        )
        .unwrap();
        let jobs = load_jobs(&dir).unwrap();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].resume, vec!["start-trainer"]);

        std::fs::write(dir.join("zz-typo.toml"), "id = \"x\"\ncheckpoint = []\nresume = [\"r\"]\nheartbeat = \"h\"\nmax_age_seconds = 5\n").unwrap();
        assert!(matches!(load_jobs(&dir), Err(JobError::Invalid { .. })));
        std::fs::write(dir.join("zz-typo.toml"), "id = \"trainer\"\ncheckpoint = [\"c\"]\nresume = [\"r\"]\nheartbeat = \"h\"\nmax_age_seconds = 5\n").unwrap();
        assert!(matches!(load_jobs(&dir), Err(JobError::Invalid { .. })), "the same id twice");
    }

    #[test]
    fn the_resume_list_survives_on_disk_and_clears() {
        let dir = scratch("list");
        let list = ResumeList::at(dir.join("state/resume.json"));
        assert!(list.read().unwrap().is_empty());
        let jobs = vec![job("trainer", &dir.join("hb")), job("render", &dir.join("hb2"))];
        list.write(&jobs).unwrap();
        assert_eq!(ResumeList::at(dir.join("state/resume.json")).read().unwrap(), jobs);
        list.write(&jobs[1..]).unwrap();
        assert_eq!(list.read().unwrap().len(), 1);
        list.clear().unwrap();
        list.clear().unwrap();
        assert!(list.read().unwrap().is_empty());
    }

    #[test]
    fn a_job_has_resumed_when_its_heartbeat_moves_not_when_it_exists() {
        let dir = scratch("heartbeat");
        let heartbeat = dir.join("heartbeat.json");
        let job = job("trainer", &heartbeat);
        let resumed_at = SystemTime::now() + Duration::from_secs(5);
        assert!(!job.moved_since(resumed_at), "no heartbeat at all");
        std::fs::write(&heartbeat, "{}").unwrap();
        assert!(!job.moved_since(resumed_at), "written before the resume");
        assert!(job.moved_since(SystemTime::now() - Duration::from_secs(60)));
        assert!(job.is_live(SystemTime::now()));
        assert!(!job.is_live(SystemTime::now() + Duration::from_secs(601)), "older than its maximum age");
    }
}
