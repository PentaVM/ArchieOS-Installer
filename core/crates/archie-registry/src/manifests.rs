//! Reading a directory of TOML manifests.
//!
//! Modules, data providers and models are all installed the same way: one `*.toml` per
//! thing, in family directories, read-only and root-owned. The three registries read
//! them through here so that the rules are the same for each: a missing directory is an
//! empty registry, the files come back sorted so a duplicate-id error names the same
//! file every run whatever order the filesystem returned, and an unreadable file is an
//! error rather than a manifest silently skipped.

use std::path::{Path, PathBuf};

/// A directory or file that could not be read.
pub(crate) struct Unreadable {
    pub path: String,
    pub source: std::io::Error,
}

/// How deep family directories may nest before the walk stops descending. The cap is
/// deliberate: a symlink loop under `/etc/archie/modules` would otherwise hang the boot
/// sequence rather than failing it.
const MAX_DEPTH: usize = 8;

/// The path and text of every `*.toml` under `dir`, sorted by path.
pub(crate) fn read_all(dir: &Path) -> Result<Vec<(String, String)>, Unreadable> {
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut paths = Vec::new();
    collect(dir, &mut paths, 0)?;
    paths.sort();
    paths
        .into_iter()
        .map(|path| {
            let name = path.display().to_string();
            match std::fs::read_to_string(&path) {
                Ok(text) => Ok((name, text)),
                Err(source) => Err(Unreadable { path: name, source }),
            }
        })
        .collect()
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>, depth: usize) -> Result<(), Unreadable> {
    if depth > MAX_DEPTH {
        return Ok(());
    }
    let unreadable = |path: &Path, source| Unreadable {
        path: path.display().to_string(),
        source,
    };
    for entry in std::fs::read_dir(dir).map_err(|e| unreadable(dir, e))? {
        let entry = entry.map_err(|e| unreadable(dir, e))?;
        let path = entry.path();
        let file_type = entry.file_type().map_err(|e| unreadable(&path, e))?;
        if file_type.is_dir() {
            collect(&path, out, depth + 1)?;
        } else if path.extension().and_then(|e| e.to_str()) == Some("toml") {
            out.push(path);
        }
    }
    Ok(())
}
