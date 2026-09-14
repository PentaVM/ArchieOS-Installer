//! Unpacking an archive without letting it choose where its files go.
//!
//! A tarball is a list of paths and contents that its author chose. Unpacked naively, an
//! entry named `../../etc/profile.d/x.sh`, an absolute path, or a symlink followed by a file
//! "inside" it writes wherever the author liked. So every entry here must be a plain
//! relative path of normal components, only regular files and directories are created,
//! nothing is written over an existing file, permissions lose setuid, setgid and sticky
//! bits, and the number of entries and bytes are capped before they are written.

use flate2::read::GzDecoder;
use std::fs::{self, File, OpenOptions};
use std::io::{BufReader, Read};
use std::path::{Component, Path, PathBuf};
use tar::{Archive, EntryType};

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub max_entries: usize,
    pub max_bytes: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_entries: 20_000,
            max_bytes: 4 * 1024 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Unpacked {
    pub files: usize,
    pub bytes: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum UnpackError {
    #[error("reading the archive: {0}")]
    Io(#[from] std::io::Error),
    #[error("entry {0:?} is not a relative path inside the tool")]
    UnsafePath(String),
    #[error("entry {path:?} is a {kind}; only files and directories are unpacked")]
    UnsupportedEntry { path: String, kind: String },
    #[error("the archive has more than {0} entries")]
    TooManyEntries(usize),
    #[error("the archive unpacks to more than {0} bytes")]
    TooLarge(u64),
    #[error("entry {0:?} ended before its declared size")]
    Truncated(String),
}

/// Unpacks a `.tar.gz` into `dest`, which should be a fresh, empty directory.
pub fn unpack_tar_gz(archive: &Path, dest: &Path, limits: Limits) -> Result<Unpacked, UnpackError> {
    let mut tar = Archive::new(GzDecoder::new(BufReader::new(File::open(archive)?)));
    let mut entries = 0usize;
    let mut unpacked = Unpacked { files: 0, bytes: 0 };
    fs::create_dir_all(dest)?;
    for entry in tar.entries()? {
        let mut entry = entry?;
        entries += 1;
        if entries > limits.max_entries {
            return Err(UnpackError::TooManyEntries(limits.max_entries));
        }
        let raw = entry.path()?.into_owned();
        let shown = raw.to_string_lossy().into_owned();
        let relative = relative_inside(&raw).ok_or_else(|| UnpackError::UnsafePath(shown.clone()))?;
        let kind = entry.header().entry_type();
        match kind {
            // Metadata records carry no file of their own.
            EntryType::XGlobalHeader | EntryType::XHeader => continue,
            EntryType::Directory => {
                if relative.as_os_str().is_empty() {
                    continue;
                }
                fs::create_dir_all(dest.join(&relative))?;
            }
            EntryType::Regular | EntryType::Continuous => {
                if relative.as_os_str().is_empty() {
                    return Err(UnpackError::UnsafePath(shown));
                }
                let size = entry.header().size()?;
                unpacked.bytes = unpacked.bytes.saturating_add(size);
                if unpacked.bytes > limits.max_bytes {
                    return Err(UnpackError::TooLarge(limits.max_bytes));
                }
                let _executable = entry.header().mode().unwrap_or(0) & 0o111 != 0;
                let target = dest.join(&relative);
                if let Some(parent) = target.parent() {
                    fs::create_dir_all(parent)?;
                }
                // create_new: never over an existing file, and never through a link.
                let mut out = OpenOptions::new().write(true).create_new(true).open(&target)?;
                let copied = std::io::copy(&mut (&mut entry).take(size), &mut out)?;
                if copied != size {
                    return Err(UnpackError::Truncated(shown));
                }
                // Only the executable bit survives: no setuid, setgid or sticky bit, and no
                // file writable by anyone but its owner.
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let mode = if _executable { 0o755 } else { 0o644 };
                    fs::set_permissions(&target, fs::Permissions::from_mode(mode))?;
                }
                unpacked.files += 1;
            }
            other => {
                return Err(UnpackError::UnsupportedEntry {
                    path: shown,
                    kind: format!("{other:?}").to_lowercase(),
                })
            }
        }
    }
    Ok(unpacked)
}

/// `path` as plain normal components, `.` dropped. `None` for a root, a prefix or `..`.
fn relative_inside(path: &Path) -> Option<PathBuf> {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => {
                let text = part.to_str()?;
                if text.contains('\\') || text.contains('\0') {
                    return None;
                }
                out.push(part);
            }
            Component::CurDir => {}
            Component::RootDir | Component::Prefix(_) | Component::ParentDir => return None,
        }
    }
    Some(out)
}
