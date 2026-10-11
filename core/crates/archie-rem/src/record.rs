//! Keeping a run for the night, or dropping it.
//!
//! IRIS writes the windows of a run it was asked to record into a spool under `/run`:
//! what it saw, the inputs before, and the input it chose. Those windows are IRIS's own
//! decisions, and a model's own output cannot teach the same model (RFC-011, rule 4). What
//! makes one a label is a fact from outside the model: the task's judge, or the person,
//! said the run succeeded. ArchieOS is where that is known, so this is where the receipts
//! are written, one per window, each bound to the window's bytes and to the input it
//! records. A run nobody verified gets no receipts and is deleted here; it never reaches
//! the day's buffer.

use crate::buffer::{BufferError, DayBuffer};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use thiserror::Error;

/// The format the trainer's admission reads (`archie_iris_foundry.rem_sleep`).
pub const RECEIPT_FORMAT: &str = "archie-iris-rem-supervision-v1";

#[derive(Debug, Error)]
pub enum RecordError {
    #[error(transparent)]
    Buffer(#[from] BufferError),
    #[error("the spool at {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("{0:?} is not a name a run or a skill may have")]
    Name(String),
    #[error("IRIS recorded {recorded} window(s) of this run and {found} are in the spool")]
    Mismatch { recorded: usize, found: usize },
}

/// One recorded window, as IRIS reports it (`POST /act/recorded`).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Sample {
    pub sample_id: String,
    pub receipt_id: String,
    pub window_sha256: String,
    pub base_sha256: String,
    pub sensitivity: String,
    pub target_sha256: String,
    pub future_sha256: String,
}

/// How a run's outcome was established.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verified {
    /// The task's judge measured success.
    TaskOutcome,
    /// A person said so, or corrected it.
    OperatorCorrection,
}

fn named(value: &str) -> Result<&str, RecordError> {
    let valid = !value.is_empty()
        && value.len() <= 64
        && !value.starts_with('.')
        && value.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.');
    if valid {
        Ok(value)
    } else {
        Err(RecordError::Name(value.to_string()))
    }
}

/// The receipts for a verified run, one JSON line per window, with exactly the fields the
/// trainer checks.
pub fn receipts(app: &str, run: &str, how: Verified, samples: &[Sample]) -> String {
    let kind = match how {
        Verified::TaskOutcome => "task_outcome",
        Verified::OperatorCorrection => "operator_correction",
    };
    samples
        .iter()
        .map(|sample| {
            let line = serde_json::json!({
                "format": RECEIPT_FORMAT, "receipt_id": sample.receipt_id, "kind": kind, "app": app,
                "run_id": run, "sample_id": sample.sample_id, "base_sha256": sample.base_sha256,
                "window_sha256": sample.window_sha256, "target_sha256": sample.target_sha256,
                "future_sha256": sample.future_sha256, "machine_analysis_allowed": true,
                "training_allowed": true, "sensitivity": sample.sensitivity,
            });
            format!("{line}\n")
        })
        .collect()
}

fn run_dirs(spool: &Path, app: &str, run: &str) -> [PathBuf; 2] {
    [spool.join("windows").join(app).join(run), spool.join("records").join(app).join(run)]
}

fn files_of(dir: &Path) -> Result<Vec<PathBuf>, RecordError> {
    let io = |source| RecordError::Io { path: dir.display().to_string(), source };
    match std::fs::read_dir(dir) {
        Ok(entries) => {
            let mut files: Vec<PathBuf> = entries.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.is_file()).collect();
            files.sort();
            Ok(files)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(source) => Err(io(source)),
    }
}

/// Seals a verified run into the day's buffer with its receipts, then removes it from the
/// spool. Returns how many windows were kept.
///
/// The buffer's names carry the place each file goes back to when the night's cycle opens
/// them, with `+` between the folders: `windows+<app>+<run>+<sample>.npz`.
pub fn keep_run(
    buffer: &DayBuffer,
    spool: &Path,
    app: &str,
    run: &str,
    how: Verified,
    samples: &[Sample],
) -> Result<usize, RecordError> {
    let (app, run) = (named(app)?, named(run)?);
    let [windows, records] = run_dirs(spool, app, run);
    let window_files = files_of(&windows)?;
    if window_files.len() != samples.len() {
        return Err(RecordError::Mismatch { recorded: samples.len(), found: window_files.len() });
    }
    for (kind, files) in [("windows", window_files), ("records", files_of(&records)?)] {
        for file in files {
            let name = file.file_name().and_then(|n| n.to_str()).unwrap_or_default();
            let plain = std::fs::read(&file).map_err(|source| RecordError::Io { path: file.display().to_string(), source })?;
            buffer.seal(&format!("{kind}+{app}+{run}+{name}"), &plain)?;
        }
    }
    buffer.seal(&format!("receipts+{app}+{run}.jsonl"), receipts(app, run, how, samples).as_bytes())?;
    discard_run(spool, app, run)?;
    Ok(samples.len())
}

/// Removes a run from the spool: what happens to one nobody verified.
pub fn discard_run(spool: &Path, app: &str, run: &str) -> Result<(), RecordError> {
    let (app, run) = (named(app)?, named(run)?);
    for dir in run_dirs(spool, app, run) {
        match std::fs::remove_dir_all(&dir) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => return Err(RecordError::Io { path: dir.display().to_string(), source }),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("archie-rem-record-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn sample(id: &str) -> Sample {
        Sample {
            sample_id: id.into(),
            receipt_id: format!("run-7.{id}"),
            window_sha256: "a".repeat(64),
            base_sha256: "b".repeat(64),
            sensitivity: "public".into(),
            target_sha256: "c".repeat(64),
            future_sha256: "d".repeat(64),
        }
    }

    fn spooled(root: &Path, samples: &[&str]) -> PathBuf {
        let spool = root.join("spool");
        for kind in ["windows", "records"] {
            std::fs::create_dir_all(spool.join(kind).join("photopea/run-7")).unwrap();
        }
        for id in samples {
            std::fs::write(spool.join(format!("windows/photopea/run-7/{id}.npz")), format!("window {id}")).unwrap();
            std::fs::write(spool.join(format!("records/photopea/run-7/{id}.json")), "{}").unwrap();
        }
        spool
    }

    /// The trainer rejects a receipt with a field missing or one too many.
    #[test]
    fn a_receipt_has_exactly_the_fields_the_trainer_checks() {
        let lines = receipts("photopea", "run-7", Verified::TaskOutcome, &[sample("000000"), sample("000001")]);
        let lines: Vec<serde_json::Value> = lines.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        assert_eq!(lines.len(), 2);
        let mut names: Vec<&str> = lines[0].as_object().unwrap().keys().map(String::as_str).collect();
        names.sort_unstable();
        assert_eq!(
            names,
            [
                "app", "base_sha256", "format", "future_sha256", "kind", "machine_analysis_allowed", "receipt_id",
                "run_id", "sample_id", "sensitivity", "target_sha256", "training_allowed", "window_sha256"
            ]
        );
        assert_eq!(lines[0]["kind"], "task_outcome");
        assert_eq!(lines[1]["receipt_id"], "run-7.000001");
        assert_eq!(lines[0]["training_allowed"], true);
    }

    #[test]
    fn a_verified_run_moves_into_the_buffer_with_its_receipts() {
        let root = scratch("keep");
        let spool = spooled(&root, &["000000", "000001"]);
        let buffer = DayBuffer::open(root.join("day")).unwrap();
        let kept = keep_run(&buffer, &spool, "photopea", "run-7", Verified::TaskOutcome, &[sample("000000"), sample("000001")]).unwrap();
        assert_eq!(kept, 2);
        assert_eq!(
            buffer.names().unwrap(),
            [
                "receipts+photopea+run-7.jsonl",
                "records+photopea+run-7+000000.json",
                "records+photopea+run-7+000001.json",
                "windows+photopea+run-7+000000.npz",
                "windows+photopea+run-7+000001.npz",
            ]
        );
        assert_eq!(buffer.unseal("windows+photopea+run-7+000001.npz").unwrap(), b"window 000001");
        assert!(!spool.join("windows/photopea/run-7").exists(), "nothing of the run stays in the clear");
    }

    #[test]
    fn a_run_nobody_verified_is_deleted_and_never_sealed() {
        let root = scratch("discard");
        let spool = spooled(&root, &["000000"]);
        discard_run(&spool, "photopea", "run-7").unwrap();
        assert!(!spool.join("windows/photopea/run-7").exists() && !spool.join("records/photopea/run-7").exists());
        discard_run(&spool, "photopea", "run-7").unwrap();
        assert!(!DayBuffer::exists(&root.join("day")));
    }

    #[test]
    fn windows_iris_did_not_report_are_not_vouched_for() {
        let root = scratch("mismatch");
        let spool = spooled(&root, &["000000", "000001"]);
        let buffer = DayBuffer::open(root.join("day")).unwrap();
        let kept = keep_run(&buffer, &spool, "photopea", "run-7", Verified::TaskOutcome, &[sample("000000")]);
        assert!(matches!(kept, Err(RecordError::Mismatch { recorded: 1, found: 2 })));
        assert!(buffer.names().unwrap().is_empty());
        assert!(matches!(discard_run(&spool, "../etc", "run-7"), Err(RecordError::Name(_))));
    }
}
