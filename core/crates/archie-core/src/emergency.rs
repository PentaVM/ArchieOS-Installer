//! Core's emergency pause, as recorded beside Core state.
//!
//! The live switch is the Policy engine the daemon shares with every AI dispatch
//! ([`crate::Runtime::live_policy`]). This record keeps it across a restart, a crash or
//! a standalone diagnostic, none of which may quietly resume what the owner stopped.
//! The file's presence is the whole record; nothing is read from inside it.

use archie_state::StatePaths;
use std::io;
use std::path::PathBuf;

/// Where the pause is recorded.
pub fn marker(paths: &StatePaths) -> PathBuf {
    paths.state_dir.join("policy").join("emergency-pause")
}

/// Whether a pause is recorded. Anything but a clean "not there" counts as paused: a
/// record that cannot be inspected is no proof that nobody stopped the node.
pub fn recorded(paths: &StatePaths) -> bool {
    match std::fs::symlink_metadata(marker(paths)) {
        Ok(_) => true,
        Err(error) => error.kind() != io::ErrorKind::NotFound,
    }
}

pub(crate) fn record(paths: &StatePaths) -> io::Result<()> {
    let path = marker(paths);
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path.parent().expect("marker has a parent"))?;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    use std::io::Write;
    let mut file = options.open(&path)?;
    file.write_all(b"paused\n")?;
    file.sync_all()
}

pub(crate) fn clear(paths: &StatePaths) -> io::Result<()> {
    match std::fs::remove_file(marker(paths)) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("archie-{name}-{}", archie_types::TraceId::new()))
    }

    #[test]
    fn a_recorded_pause_is_seen_until_it_is_cleared() {
        let root = root("emergency-record");
        let paths = StatePaths::rooted(&root);
        assert!(!recorded(&paths));
        record(&paths).unwrap();
        assert!(recorded(&paths));
        // Recording twice is still one pause, and one clear lifts it.
        record(&paths).unwrap();
        clear(&paths).unwrap();
        assert!(!recorded(&paths));
        clear(&paths).unwrap();
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_record_that_cannot_be_inspected_counts_as_paused() {
        let root = root("emergency-unreadable");
        let paths = StatePaths::rooted(&root);
        // The state directory is a file, so the record's directory cannot be walked:
        // the answer is "cannot tell", never "not paused".
        std::fs::create_dir_all(paths.state_dir.parent().unwrap()).unwrap();
        std::fs::write(&paths.state_dir, b"not a directory").unwrap();
        assert!(record(&paths).is_err());
        assert!(recorded(&paths));
        std::fs::remove_dir_all(&root).unwrap();
    }
}
