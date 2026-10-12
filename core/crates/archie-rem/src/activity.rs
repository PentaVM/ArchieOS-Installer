//! The activity file: what the machine is doing, for the trainer to read at every step.
//!
//! The IRIS Foundry trainer (`archie_iris_foundry.rem_sleep`) will not train without a
//! fresh one, and stops within a step when it says a person is back, inference is
//! waiting, or the machine is busy. Its fields and their types are that reader's, checked
//! exactly: a missing or extra field, or a float where it wants an integer, is a stop.

use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Activity {
    /// When this was written, in seconds since the epoch. The reader refuses one older
    /// than 15 seconds.
    pub updated_unix: u64,
    pub cpu_percent: u8,
    pub ram_bytes: u64,
    pub vram_bytes: u64,
    pub gpu_percent: u8,
    /// How long since a person last touched the machine.
    pub idle_seconds: u64,
    /// Whether something is waiting on IRIS to answer. Training yields to it.
    pub inference_pending: bool,
}

impl Activity {
    /// Writes beside the file and renames, so the reader never sees half a file.
    pub fn write(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let partial = path.with_extension("partial");
        std::fs::write(
            &partial,
            serde_json::to_vec(self).expect("an activity serializes"),
        )?;
        std::fs::rename(&partial, path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The names `rem_sleep.py` requires, no more and no fewer.
    const READER_FIELDS: [&str; 7] = [
        "updated_unix",
        "cpu_percent",
        "ram_bytes",
        "vram_bytes",
        "gpu_percent",
        "idle_seconds",
        "inference_pending",
    ];

    #[test]
    fn the_file_has_exactly_the_fields_and_types_the_trainer_checks() {
        let dir = std::env::temp_dir().join(format!("archie-rem-activity-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("run/activity.json");
        let activity = Activity {
            updated_unix: 1_790_000_000,
            cpu_percent: 12,
            ram_bytes: 6 << 30,
            vram_bytes: 3 << 30,
            gpu_percent: 40,
            idle_seconds: 2400,
            inference_pending: false,
        };
        activity.write(&path).unwrap();
        let written: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        let object = written.as_object().unwrap();
        let mut names: Vec<&str> = object.keys().map(String::as_str).collect();
        names.sort_unstable();
        let mut expected = READER_FIELDS.to_vec();
        expected.sort_unstable();
        assert_eq!(names, expected);
        for (name, value) in object {
            if name == "inference_pending" {
                assert!(value.is_boolean());
            } else {
                assert!(value.is_u64(), "{name} must be an integer, not {value}");
            }
        }
        assert!(!path.with_extension("partial").exists());
    }
}
