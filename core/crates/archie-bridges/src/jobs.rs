//! Long jobs: work that outlives a conversation.
//!
//! Two hundred hours of iterations cannot run inside one MCP call, and should not die
//! when the client that asked for them closes. A bridge starts such work as a job: a
//! separate `archie-bridges run-job` process, in its own process group, that writes its
//! progress where any later call can read it and stops cleanly between iterations when
//! asked.
//!
//! ```text
//! <workspace>/.archie/jobs/<id>/spec.json     what to do, written once
//!                              status.json   state, progress, the latest result
//!                              results.jsonl one line per iteration
//!                              log.txt       what the process printed
//!                              cancel        present = stop after this iteration
//! ```

use crate::workspace::{Workspace, WorkspaceError};
use crate::Programs;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Debug, thiserror::Error)]
pub enum JobError {
    #[error(transparent)]
    Workspace(#[from] WorkspaceError),
    #[error("no job {0} in this workspace")]
    Unknown(String),
    #[error("{0}")]
    Io(String),
}

fn io(e: impl std::fmt::Display) -> JobError {
    JobError::Io(e.to_string())
}

/// What a job is: which bridge runs it, and its bridge-specific description.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Spec {
    pub bridge: String,
    pub kind: String,
    pub arguments: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Starting,
    Running,
    Done,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Status {
    pub id: String,
    pub bridge: String,
    pub kind: String,
    pub state: State,
    pub done: u64,
    pub total: u64,
    pub started_ms: u64,
    pub updated_ms: u64,
    #[serde(default)]
    pub latest: Option<Value>,
    #[serde(default)]
    pub error: Option<String>,
}

fn now_ms() -> u64 {
    crate::delivery::now_ms()
}

/// The folder of a job, which must exist.
pub fn folder(workspace: &Workspace, id: &str) -> Result<PathBuf, JobError> {
    if !crate::delivery::is_name(id) {
        return Err(JobError::Unknown(id.to_string()));
    }
    let dir = workspace.scratch("jobs")?.join(id);
    if dir.join("spec.json").is_file() {
        Ok(dir)
    } else {
        Err(JobError::Unknown(id.to_string()))
    }
}

pub fn read_status(workspace: &Workspace, id: &str) -> Result<Status, JobError> {
    let dir = folder(workspace, id)?;
    let text = std::fs::read_to_string(dir.join("status.json")).map_err(io)?;
    serde_json::from_str(&text).map_err(io)
}

pub fn list(workspace: &Workspace) -> Vec<Status> {
    let Ok(dir) = workspace.scratch("jobs") else {
        return Vec::new();
    };
    let mut all: Vec<Status> = std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .filter_map(|e| std::fs::read_to_string(e.path().join("status.json")).ok())
                .filter_map(|text| serde_json::from_str(&text).ok())
                .collect()
        })
        .unwrap_or_default();
    all.sort_by_key(|s: &Status| std::cmp::Reverse(s.started_ms));
    all
}

pub fn cancel(workspace: &Workspace, id: &str) -> Result<(), JobError> {
    let dir = folder(workspace, id)?;
    std::fs::write(dir.join("cancel"), b"").map_err(io)
}

/// Writes the job and starts `archie-bridges run-job` for it, detached. Returns its id.
pub fn start(
    workspace: &Workspace,
    programs: &Programs,
    spec: &Spec,
    total: u64,
) -> Result<String, JobError> {
    let jobs = workspace.scratch("jobs")?;
    let id = format!("{}-{}-{}", spec.bridge, spec.kind, now_ms());
    let dir = jobs.join(&id);
    std::fs::create_dir_all(&dir).map_err(io)?;
    std::fs::write(dir.join("spec.json"), serde_json::to_vec_pretty(spec).map_err(io)?).map_err(io)?;
    let status = Status {
        id: id.clone(),
        bridge: spec.bridge.clone(),
        kind: spec.kind.clone(),
        state: State::Starting,
        done: 0,
        total,
        started_ms: now_ms(),
        updated_ms: now_ms(),
        latest: None,
        error: None,
    };
    write_status(&dir, &status)?;
    let log = std::fs::File::create(dir.join("log.txt")).map_err(io)?;
    let mut command = Command::new(programs.get("archie-bridges"));
    command
        .args(["run-job", "--workspace"])
        .arg(workspace.root())
        .args(["--job", &id])
        .args(programs.as_args())
        .stdin(Stdio::null())
        .stdout(log.try_clone().map_err(io)?)
        .stderr(log);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // Its own group: a signal to the MCP client's group does not reach it.
        command.process_group(0);
    }
    command.spawn().map_err(|e| {
        JobError::Io(format!(
            "starting archie-bridges run-job: {e}. It is installed beside archie-mcp-server."
        ))
    })?;
    Ok(id)
}

fn write_status(dir: &Path, status: &Status) -> Result<(), JobError> {
    let staged = dir.join("status.json.new");
    std::fs::write(&staged, serde_json::to_vec_pretty(status).map_err(io)?).map_err(io)?;
    std::fs::rename(&staged, dir.join("status.json")).map_err(io)
}

/// The running side of a job, inside `archie-bridges run-job`.
pub struct Runner {
    dir: PathBuf,
    status: Status,
}

impl Runner {
    pub fn open(workspace: &Workspace, id: &str) -> Result<(Self, Spec), JobError> {
        let dir = folder(workspace, id)?;
        let spec: Spec =
            serde_json::from_str(&std::fs::read_to_string(dir.join("spec.json")).map_err(io)?).map_err(io)?;
        let mut status = read_status(workspace, id)?;
        status.state = State::Running;
        status.updated_ms = now_ms();
        write_status(&dir, &status)?;
        Ok((Self { dir, status }, spec))
    }

    pub fn cancelled(&self) -> bool {
        self.dir.join("cancel").exists()
    }

    /// Records one finished iteration.
    pub fn record(&mut self, result: Value) -> Result<(), JobError> {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.dir.join("results.jsonl"))
            .map_err(io)?;
        writeln!(file, "{}", serde_json::to_string(&result).map_err(io)?).map_err(io)?;
        self.status.done += 1;
        self.status.latest = Some(result);
        self.status.updated_ms = now_ms();
        write_status(&self.dir, &self.status)
    }

    pub fn finish(mut self, state: State, error: Option<String>) -> Result<(), JobError> {
        self.status.state = state;
        self.status.error = error;
        self.status.updated_ms = now_ms();
        write_status(&self.dir, &self.status)
    }

    pub fn results_file(&self) -> PathBuf {
        self.dir.join("results.jsonl")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_runner_records_progress_that_status_reads_back() {
        let dir = std::env::temp_dir().join(format!("archie-jobs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let workspace = Workspace::open(&dir).unwrap();
        // Written by hand as start() would, without spawning the binary.
        let job_dir = workspace.scratch("jobs").unwrap().join("freecad-sweep-1");
        std::fs::create_dir_all(&job_dir).unwrap();
        let spec = Spec { bridge: "freecad".into(), kind: "sweep".into(), arguments: serde_json::json!({}) };
        std::fs::write(job_dir.join("spec.json"), serde_json::to_vec(&spec).unwrap()).unwrap();
        write_status(
            &job_dir,
            &Status {
                id: "freecad-sweep-1".into(),
                bridge: "freecad".into(),
                kind: "sweep".into(),
                state: State::Starting,
                done: 0,
                total: 3,
                started_ms: 1,
                updated_ms: 1,
                latest: None,
                error: None,
            },
        )
        .unwrap();

        let (mut runner, read) = Runner::open(&workspace, "freecad-sweep-1").unwrap();
        assert_eq!(read, spec);
        runner.record(serde_json::json!({"volume": 12.5})).unwrap();
        assert!(!runner.cancelled());
        cancel(&workspace, "freecad-sweep-1").unwrap();
        assert!(runner.cancelled());
        runner.finish(State::Cancelled, None).unwrap();

        let status = read_status(&workspace, "freecad-sweep-1").unwrap();
        assert_eq!((status.done, status.state), (1, State::Cancelled));
        assert_eq!(status.latest.unwrap()["volume"], 12.5);
        assert_eq!(list(&workspace).len(), 1);
        assert!(read_status(&workspace, "../../etc").is_err());
        let _ = std::fs::remove_dir_all(dir);
    }
}
