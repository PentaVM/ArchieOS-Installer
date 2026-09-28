//! What the end-to-end tests share: whether a bridge's test is switched on, a fresh
//! workspace, and a call that fails loudly.

#![allow(dead_code)]

use archie_bridges::{open, Bridge, Outcome, Programs};
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};

/// Whether `ARCHIE_BRIDGE_E2E` names `bridge` (a comma-separated list, or `all`).
pub fn enabled(bridge: &str) -> bool {
    std::env::var("ARCHIE_BRIDGE_E2E")
        .map(|v| {
            v.split(',')
                .any(|b| b.trim() == bridge || b.trim() == "all")
        })
        .unwrap_or(false)
}

pub fn workspace(bridge: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("archie-{bridge}-e2e-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The bridge, with any program named in `ARCHIE_PROGRAM_<NAME>` (dashes as underscores).
pub fn bridge(id: &str, dir: &Path) -> Box<dyn Bridge> {
    let mut programs = Programs::default();
    for (key, value) in std::env::vars() {
        if let Some(name) = key.strip_prefix("ARCHIE_PROGRAM_") {
            programs.set(name.to_ascii_lowercase().replace('_', "-"), value);
        }
    }
    open(id, dir, &programs).unwrap()
}

pub fn outcome(bridge: &dyn Bridge, tool: &str, arguments: Value) -> Outcome {
    let arguments: Map<String, Value> = arguments.as_object().unwrap().clone();
    let outcome = bridge.call(tool, &arguments);
    println!(
        "{tool}: {}{}",
        outcome.text.chars().take(400).collect::<String>(),
        if outcome.images.is_empty() {
            ""
        } else {
            " [+image]"
        }
    );
    outcome
}

pub fn call(bridge: &dyn Bridge, tool: &str, arguments: Value) -> Value {
    let outcome = outcome(bridge, tool, arguments);
    assert!(!outcome.is_error, "{tool}: {}", outcome.text);
    serde_json::from_str(&outcome.text).unwrap_or(Value::String(outcome.text))
}

pub fn finish(dir: PathBuf) {
    if std::env::var_os("ARCHIE_KEEP_E2E").is_none() {
        let _ = std::fs::remove_dir_all(dir);
    } else {
        println!("kept {}", dir.display());
    }
}
