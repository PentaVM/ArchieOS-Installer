//! Long bridge jobs, read from the workspace folders `archie-bridges` writes them to:
//! `<workspace>/.archie/jobs/<id>/status.json` (`archie_bridges::jobs::Status`).
//!
//! Read-only. Cancelling a job writes a `cancel` file, which is an action with an effect
//! (`archie_job_cancel` is `Effect::Acts` over MCP): it is not offered here, and the shell
//! shows no cancel button until there is an authorised path for it.
//!
//! `latest` (a job's own free-form result) is not forwarded: it can hold anything the job
//! printed. Only the fields below reach the shell, and the error text is truncated.

use serde::{Deserialize, Serialize};
use std::io::Read;
use std::path::{Path, PathBuf};

pub const MAX_STATUS_BYTES: u64 = 64 * 1024;
pub const MAX_WORKSPACES: usize = 8;
pub const MAX_ENTRIES_PER_WORKSPACE: usize = 256;
pub const MAX_JOBS: usize = 64;
const MAX_ERROR_CHARS: usize = 300;

/// A job's state, as `archie-bridges` names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Starting,
    Running,
    Done,
    Cancelled,
    Failed,
}

/// The subset of `archie_bridges::jobs::Status` the shell shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Job {
    /// Folder name of the workspace (never its full path).
    pub workspace: String,
    pub id: String,
    pub bridge: String,
    pub kind: String,
    pub state: State,
    pub done: u64,
    pub total: u64,
    pub started_ms: u64,
    pub updated_ms: u64,
    /// Milliseconds since the job last wrote its status. Not a verdict: a job with long
    /// iterations is quiet for a long time. The shell may say "no news for N", not "stuck".
    pub idle_ms: u64,
    pub error: Option<String>,
}

#[derive(Deserialize)]
struct Raw {
    id: String,
    bridge: String,
    kind: String,
    state: State,
    done: u64,
    total: u64,
    started_ms: u64,
    updated_ms: u64,
    #[serde(default)]
    error: Option<String>,
}

/// What the bridge can say about long jobs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Jobs {
    /// No workspace was given to the bridge.
    NotConfigured,
    Ok {
        /// Newest first, at most [`MAX_JOBS`].
        jobs: Vec<Job>,
        /// Jobs found beyond the cap, or whose `status.json` was unreadable or invalid.
        skipped: usize,
        /// Workspaces given that could not be listed (name, reason).
        unreadable: Vec<(String, String)>,
    },
}

fn label(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "workspace".to_string())
}

fn read_status(path: &Path) -> Option<Raw> {
    let file = std::fs::File::open(path).ok()?;
    let meta = file.metadata().ok()?;
    if !meta.is_file() || meta.len() > MAX_STATUS_BYTES {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(MAX_STATUS_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 > MAX_STATUS_BYTES {
        return None;
    }
    serde_json::from_slice(&bytes).ok()
}

/// Read the jobs of every given workspace.
pub fn read(workspaces: &[PathBuf], now_ms: u64) -> Jobs {
    if workspaces.is_empty() {
        return Jobs::NotConfigured;
    }
    let mut jobs: Vec<Job> = Vec::new();
    let mut skipped = 0usize;
    let mut unreadable = Vec::new();

    for workspace in workspaces.iter().take(MAX_WORKSPACES) {
        let name = label(workspace);
        let root = workspace.join(".archie").join("jobs");
        let entries = match std::fs::read_dir(&root) {
            Ok(entries) => entries,
            // No jobs folder just means no job has ever run in that workspace.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => {
                unreadable.push((name, e.to_string()));
                continue;
            }
        };
        for entry in entries.flatten().take(MAX_ENTRIES_PER_WORKSPACE) {
            // A symlink could point anywhere on the machine: never follow one.
            let Ok(kind) = entry.file_type() else {
                skipped += 1;
                continue;
            };
            if !kind.is_dir() || kind.is_symlink() {
                continue;
            }
            match read_status(&entry.path().join("status.json")) {
                Some(raw) => jobs.push(Job {
                    workspace: name.clone(),
                    id: raw.id,
                    bridge: raw.bridge,
                    kind: raw.kind,
                    state: raw.state,
                    done: raw.done,
                    total: raw.total,
                    started_ms: raw.started_ms,
                    updated_ms: raw.updated_ms,
                    idle_ms: now_ms.saturating_sub(raw.updated_ms),
                    error: raw.error.map(|e| e.chars().take(MAX_ERROR_CHARS).collect()),
                }),
                None => skipped += 1,
            }
        }
    }

    jobs.sort_by_key(|j| std::cmp::Reverse(j.started_ms));
    if jobs.len() > MAX_JOBS {
        skipped += jobs.len() - MAX_JOBS;
        jobs.truncate(MAX_JOBS);
    }
    Jobs::Ok {
        jobs,
        skipped,
        unreadable,
    }
}

/// The workspace folders directly under `root` (e.g. `/archie/workspaces`), sorted, at most
/// [`MAX_WORKSPACES`]. Symlinks are not followed. `Err` carries why `root` could not be listed.
pub fn children(root: &Path) -> Result<Vec<PathBuf>, String> {
    let entries = std::fs::read_dir(root).map_err(|e| e.to_string())?;
    let mut dirs: Vec<PathBuf> = entries
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir() && !t.is_symlink()))
        .map(|e| e.path())
        .collect();
    dirs.sort();
    dirs.truncate(MAX_WORKSPACES);
    Ok(dirs)
}

/// Jobs from explicit workspaces plus every workspace under `root`, re-listed on each call so
/// that a workspace created later appears without restarting the bridge.
pub fn read_sources(explicit: &[PathBuf], root: Option<&Path>, now_ms: u64) -> Jobs {
    let Some(root) = root else {
        return read(explicit, now_ms);
    };
    let mut workspaces = explicit.to_vec();
    let listed = children(root);
    if let Ok(found) = &listed {
        workspaces.extend(found.iter().cloned());
    }
    workspaces.truncate(MAX_WORKSPACES);
    let mut jobs = if workspaces.is_empty() {
        // A configured root with nothing in it is a real, empty answer, not "not configured".
        Jobs::Ok {
            jobs: Vec::new(),
            skipped: 0,
            unreadable: Vec::new(),
        }
    } else {
        read(&workspaces, now_ms)
    };
    if let (Err(reason), Jobs::Ok { unreadable, .. }) = (&listed, &mut jobs) {
        unreadable.push((label(root), reason.clone()));
    }
    jobs
}
