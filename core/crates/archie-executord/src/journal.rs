//! Task records that survive the executor.
//!
//! RFC-007 §7 step 8. Without these, startup reconciliation has nothing to reconcile
//! *against*: every cgroup it finds is untracked by definition, and "untracked" is the
//! finding that demands a human. A restart would raise an alarm about every task the
//! executor had been running perfectly well a second earlier.
//!
//! # One file per task, replaced atomically
//!
//! Not an append-only log. A log needs compaction, and compaction is a rewrite that can
//! be interrupted — which puts the crash-safety of the whole record store in the code
//! path that runs least often and is therefore tested least.
//!
//! A file per task has no such path. A write is a temporary file and a rename, and rename
//! is atomic on every filesystem this runs on: a reader sees the old record or the new
//! one, never half of either. Deleting a finished task is unlinking a file. Listing them
//! is reading a directory.
//!
//! # A record that cannot be read is kept, not skipped
//!
//! A corrupt record is a task the executor may still be running. Skipping it would make
//! reconciliation conclude the cgroup is untracked and hand an operator an alarm with no
//! task attached to it; the file itself is the only remaining evidence of what was there.
//! So it is reported and left on disk.

use std::path::{Path, PathBuf};

use archie_execution::lifecycle::ExecutionState;
use archie_types::{TaskId, Timestamp};
use serde::{Deserialize, Serialize};

/// What survives a restart about one task.
///
/// Deliberately small. This is written on every state change, so anything expensive to
/// serialize here becomes a cost paid on the hot path — and anything derivable from the
/// spec belongs with the spec, not duplicated into a file that can disagree with it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    pub task_id: TaskId,
    pub state: ExecutionState,
    /// The pid last seen. For the audit trail and for signalling; never used to decide
    /// whether the task is alive, because a pid can be reused.
    pub pid: Option<u32>,
    /// The task's cgroup, so a restarted executor can find it again without guessing.
    pub cgroup: Option<String>,
    pub updated_at: Timestamp,
}

/// Why the journal could not do something.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JournalError {
    /// The directory could not be created or read.
    Unusable { path: String, detail: String },
    /// One record could not be written.
    WriteFailed { task_id: TaskId, detail: String },
}

impl std::fmt::Display for JournalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unusable { path, detail } => write!(
                f,
                "the task journal at {path} is unusable ({detail}); without it a restart \
                 cannot tell its own tasks from someone else's"
            ),
            Self::WriteFailed { task_id, detail } => write!(
                f,
                "could not record the state of task {task_id} ({detail}); a restart will \
                 not know about it"
            ),
        }
    }
}

impl std::error::Error for JournalError {}

/// A record that could not be decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unreadable {
    pub path: String,
    pub detail: String,
}

/// What a startup read found.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Loaded {
    pub records: Vec<Record>,
    /// Files that are there and could not be read.
    ///
    /// Never silently dropped: a corrupt record is a task that may still be running, and
    /// the file is the only evidence left of what it was.
    pub unreadable: Vec<Unreadable>,
}

/// Durable task records.
#[derive(Debug, Clone)]
pub struct Journal {
    directory: PathBuf,
}

impl Journal {
    pub fn new(directory: impl Into<PathBuf>) -> Self {
        Self {
            directory: directory.into(),
        }
    }

    pub fn directory(&self) -> &Path {
        &self.directory
    }

    /// Create the directory if it is not there.
    pub fn prepare(&self) -> Result<(), JournalError> {
        std::fs::create_dir_all(&self.directory).map_err(|e| JournalError::Unusable {
            path: self.directory.display().to_string(),
            detail: e.to_string(),
        })
    }

    fn path_for(&self, task_id: &TaskId) -> PathBuf {
        self.directory.join(format!("{}.json", task_id.as_uuid()))
    }

    /// Write a record, replacing any previous one atomically.
    ///
    /// Temporary file plus rename. A reader concurrent with this sees the old record or
    /// the new one and never half of either, which matters because the reader is a
    /// restarted executor deciding whether something is still running.
    pub fn write(&self, record: &Record) -> Result<(), JournalError> {
        let bytes = serde_json::to_vec(record).map_err(|e| JournalError::WriteFailed {
            task_id: record.task_id,
            detail: e.to_string(),
        })?;

        let final_path = self.path_for(&record.task_id);
        // The temporary lives in the same directory so the rename stays within one
        // filesystem; across a mount boundary it would be a copy, and a copy is not
        // atomic.
        let temp_path = final_path.with_extension("json.tmp");

        std::fs::write(&temp_path, &bytes).map_err(|e| JournalError::WriteFailed {
            task_id: record.task_id,
            detail: format!("writing {}: {e}", temp_path.display()),
        })?;

        std::fs::rename(&temp_path, &final_path).map_err(|e| {
            // Leaving a stray temporary would make the next listing report an unreadable
            // record for a task that is fine.
            let _ = std::fs::remove_file(&temp_path);
            JournalError::WriteFailed {
                task_id: record.task_id,
                detail: format!("replacing {}: {e}", final_path.display()),
            }
        })
    }

    /// Forget a task.
    ///
    /// Called once a terminal state has been reported to the control plane — not when the
    /// task finishes. A record removed before archied knows about it is a completion
    /// nobody ever hears, and the slot stays occupied in the scheduler's view forever.
    pub fn forget(&self, task_id: &TaskId) -> Result<(), JournalError> {
        let path = self.path_for(task_id);
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            // Already gone. The goal was that it not be there.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(JournalError::WriteFailed {
                task_id: *task_id,
                detail: format!("removing {}: {e}", path.display()),
            }),
        }
    }

    /// Read everything back.
    pub fn load(&self) -> Result<Loaded, JournalError> {
        let entries = match std::fs::read_dir(&self.directory) {
            Ok(entries) => entries,
            // No directory means no tasks were ever recorded, which is the truthful
            // answer for a node that has just been installed.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Loaded::default()),
            Err(e) => {
                return Err(JournalError::Unusable {
                    path: self.directory.display().to_string(),
                    detail: e.to_string(),
                })
            }
        };

        let mut loaded = Loaded::default();
        for entry in entries.flatten() {
            let path = entry.path();
            // Temporaries are a write that was interrupted. The rename never happened, so
            // the previous record — if any — is still the current one, and this file is
            // not evidence of anything.
            if path.extension().map_or(true, |ext| ext != "json") {
                continue;
            }

            match std::fs::read(&path) {
                Ok(bytes) => match serde_json::from_slice::<Record>(&bytes) {
                    Ok(record) => loaded.records.push(record),
                    Err(e) => loaded.unreadable.push(Unreadable {
                        path: path.display().to_string(),
                        detail: e.to_string(),
                    }),
                },
                Err(e) => loaded.unreadable.push(Unreadable {
                    path: path.display().to_string(),
                    detail: e.to_string(),
                }),
            }
        }

        // Sorted so a startup report reads the same twice, and so an operator comparing
        // two restarts is comparing content rather than directory order.
        loaded.records.sort_by_key(|r| r.task_id);
        loaded.unreadable.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(loaded)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn journal(tag: &str) -> (PathBuf, Journal) {
        let dir = std::env::temp_dir().join(format!(
            "archie-journal-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let journal = Journal::new(&dir);
        journal.prepare().unwrap();
        (dir, journal)
    }

    fn record(state: ExecutionState) -> Record {
        Record {
            task_id: TaskId::new(),
            state,
            pid: Some(4242),
            cgroup: Some("system.slice/archie-executord.service/task-x".into()),
            updated_at: Timestamp::now(),
        }
    }

    #[test]
    fn a_record_survives_a_restart() {
        // The whole point: without this, every cgroup found at startup is untracked, and
        // a restart raises an alarm about every task that was running perfectly well.
        let (dir, journal) = journal("survives");
        let written = record(ExecutionState::Running);
        journal.write(&written).unwrap();

        let loaded = Journal::new(&dir).load().unwrap();
        assert_eq!(loaded.records, vec![written]);
        assert!(loaded.unreadable.is_empty());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn writing_twice_replaces_rather_than_accumulates() {
        // A task has one current state. Two records for it would make reconciliation
        // depend on which was read last.
        let (dir, journal) = journal("replace");
        let mut r = record(ExecutionState::Starting);
        journal.write(&r).unwrap();

        r.state = ExecutionState::Running;
        journal.write(&r).unwrap();

        let loaded = journal.load().unwrap();
        assert_eq!(loaded.records.len(), 1);
        assert_eq!(loaded.records[0].state, ExecutionState::Running);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn an_interrupted_write_leaves_the_previous_record_intact() {
        // Temporary plus rename: a reader sees the old record or the new one, never half
        // of either. The reader here is a restarted executor deciding whether something
        // is still running.
        let (dir, journal) = journal("interrupted");
        let r = record(ExecutionState::Running);
        journal.write(&r).unwrap();

        // Stand in for a write that died before its rename.
        std::fs::write(dir.join("half-written.json.tmp"), b"{ truncated").unwrap();

        let loaded = journal.load().unwrap();
        assert_eq!(loaded.records, vec![r]);
        assert!(
            loaded.unreadable.is_empty(),
            "a temporary is an interrupted write, not evidence of anything"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_corrupt_record_is_reported_and_kept_rather_than_skipped() {
        // A corrupt record is a task that may still be running. Skipping it would have
        // reconciliation call the cgroup untracked and hand an operator an alarm with no
        // task attached — and the file is the only evidence left of what it was.
        let (dir, journal) = journal("corrupt");
        let good = record(ExecutionState::Running);
        journal.write(&good).unwrap();
        std::fs::write(dir.join("garbage.json"), b"{ not a record").unwrap();

        let loaded = journal.load().unwrap();
        assert_eq!(loaded.records, vec![good]);
        assert_eq!(loaded.unreadable.len(), 1);
        assert!(loaded.unreadable[0].path.contains("garbage.json"));
        assert!(
            dir.join("garbage.json").exists(),
            "the file must survive the read that could not use it"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn forgetting_a_task_removes_it_and_is_idempotent() {
        let (dir, journal) = journal("forget");
        let r = record(ExecutionState::Completed);
        journal.write(&r).unwrap();

        journal.forget(&r.task_id).unwrap();
        assert!(journal.load().unwrap().records.is_empty());

        // Removing something already gone is not an error: the goal was that it not be
        // there, and it is not.
        journal.forget(&r.task_id).unwrap();

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_journal_that_was_never_written_reports_nothing_rather_than_failing() {
        // The truthful answer for a node that has just been installed.
        let dir =
            std::env::temp_dir().join(format!("archie-journal-absent-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(Journal::new(&dir).load().unwrap(), Loaded::default());
    }

    #[test]
    fn the_cgroup_is_recorded_so_a_restart_does_not_have_to_guess() {
        // A restarted executor that knew a task was frozen but not where would have to
        // search the subtree for it, and a wrong guess thaws someone else's task.
        let (dir, journal) = journal("cgroup");
        let mut r = record(ExecutionState::Frozen);
        r.cgroup = Some("system.slice/archie-executord.service/task-7".into());
        journal.write(&r).unwrap();

        let loaded = journal.load().unwrap();
        assert_eq!(
            loaded.records[0].cgroup.as_deref(),
            Some("system.slice/archie-executord.service/task-7")
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_listing_is_stable_across_reads() {
        // An operator comparing two restarts should be comparing content, not directory
        // order.
        let (dir, journal) = journal("stable");
        for _ in 0..5 {
            journal.write(&record(ExecutionState::Running)).unwrap();
        }

        let first = journal.load().unwrap();
        assert_eq!(first, journal.load().unwrap());
        assert!(first
            .records
            .windows(2)
            .all(|w| w[0].task_id <= w[1].task_id));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn an_unusable_directory_says_what_it_costs() {
        // Not being able to record is not fatal, but an operator needs to know the
        // consequence rather than just the errno.
        let error = JournalError::Unusable {
            path: "/var/lib/archie-executor/tasks".into(),
            detail: "permission denied".into(),
        };
        assert!(error
            .to_string()
            .contains("cannot tell its own tasks from someone else's"));
    }
}
