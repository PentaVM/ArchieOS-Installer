//! Bounded on-disk persistence for the vault the live followers keep.
//!
//! `archie-vault` deliberately does not persist: [`Vault::sealed_entries`] hands
//! ciphertext to whichever store the caller already trusts. The live callers in this
//! crate — [`crate::alert_follower`] and [`crate::wanda_watcher`] — trust one local
//! directory, and this module is the whole contract with it: a 32-byte key file
//! (mode 0600, created once from OS randomness) and a JSON
//! array of sealed entries, written atomically and held to hard caps, so a tampered
//! or runaway file is a typed error and never an unbounded read.
//!
//! The key never leaves the directory and never appears in a log; `VaultKey`'s own
//! `Debug` is redacted and this module prints nothing.

use archie_vault::{SealedEntry, Vault, VaultError, VaultKey};
use fs2::FileExt;
use ring::rand::{SecureRandom, SystemRandom};
use std::fs::File;
use std::io::{Read, Write};
use std::ops::{Deref, DerefMut};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// The most sealed entries one follower vault may hold. A vault this full is an
/// operational fact to surface, not a file to keep growing.
pub const MAX_SEALED_ENTRIES: usize = 65_536;
/// Byte cap on the entries file, checked before deserializing.
pub const MAX_ENTRIES_BYTES: u64 = 64 * 1024 * 1024;

const KEY_FILE: &str = "vault.key";
const ENTRIES_FILE: &str = "entries.json";

/// Why the on-disk vault could not be opened or persisted.
#[derive(Debug, thiserror::Error)]
pub enum VaultIoError {
    #[error("the vault directory is not usable: {0}")]
    Io(String),
    #[error("the vault key file is not one this component wrote")]
    BadKey,
    #[error("the sealed entries file is too large ({bytes} bytes; the cap is {cap})")]
    TooLarge { bytes: u64, cap: u64 },
    #[error("the sealed entries file is not one this component wrote")]
    MalformedEntries,
    #[error("the vault is busy with another transaction")]
    LockTimeout,
    #[error("the vault transaction belongs to another directory")]
    DirectoryMismatch,
    #[error("the vault refused its own sealed entries: {0}")]
    Vault(#[from] VaultError),
}

/// A transaction owns the OS lock from loading through persistence. A process
/// exit releases it; the lock file is never removed or replaced while in use.
pub struct VaultGuard {
    vault: Vault,
    directory: PathBuf,
    _lock: File,
}
impl Deref for VaultGuard {
    type Target = Vault;
    fn deref(&self) -> &Vault {
        &self.vault
    }
}
impl DerefMut for VaultGuard {
    fn deref_mut(&mut self) -> &mut Vault {
        &mut self.vault
    }
}
impl std::fmt::Debug for VaultGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("VaultGuard(redacted)")
    }
}

fn transaction_lock(dir: &Path) -> Result<File, VaultIoError> {
    let mut options = std::fs::OpenOptions::new();
    options.create(true).read(true).write(true).truncate(false);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let file = options
        .open(dir.join("vault.lock"))
        .map_err(|e| VaultIoError::Io(format!("vault lock: {e}")))?;
    let started = Instant::now();
    loop {
        match FileExt::try_lock_exclusive(&file) {
            Ok(()) => return Ok(file),
            Err(e) if e.raw_os_error() == fs2::lock_contended_error().raw_os_error() => {
                if started.elapsed() >= Duration::from_secs(10) {
                    return Err(VaultIoError::LockTimeout);
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(e) => return Err(VaultIoError::Io(format!("vault lock: {e}"))),
        }
    }
}

fn bounded_read(file: File, cap: u64) -> Result<Vec<u8>, VaultIoError> {
    let metadata = file
        .metadata()
        .map_err(|e| VaultIoError::Io(e.to_string()))?;
    if !metadata.is_file() {
        return Err(VaultIoError::MalformedEntries);
    }
    if metadata.len() > cap {
        return Err(VaultIoError::TooLarge {
            bytes: metadata.len(),
            cap,
        });
    }
    let mut bytes = Vec::new();
    file.take(cap + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| VaultIoError::Io(e.to_string()))?;
    if bytes.len() as u64 > cap {
        return Err(VaultIoError::TooLarge {
            bytes: bytes.len() as u64,
            cap,
        });
    }
    Ok(bytes)
}

fn write_private(path: &Path, bytes: &[u8]) -> Result<(), VaultIoError> {
    let mut options = std::fs::OpenOptions::new();
    options.create(true).write(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut file = options
        .open(path)
        .map_err(|e| VaultIoError::Io(format!("{}: {e}", path.display())))?;
    file.write_all(bytes)
        .map_err(|e| VaultIoError::Io(format!("{}: {e}", path.display())))
}

/// Write a file in one move: a partial write is never mistaken for the whole.
fn write_atomic(dir: &Path, name: &str, bytes: &[u8]) -> Result<(), VaultIoError> {
    let partial = dir.join(format!(".{name}.partial"));
    write_private(&partial, bytes)?;
    std::fs::rename(&partial, dir.join(name))
        .map_err(|e| VaultIoError::Io(format!("{}: {e}", dir.join(name).display())))
}

/// The directory's key, created under the transaction lock from OS randomness.
fn load_or_create_key(dir: &Path) -> Result<VaultKey, VaultIoError> {
    let path = dir.join(KEY_FILE);
    match File::open(&path) {
        Ok(file) => {
            let bytes = bounded_read(file, 32).map_err(|_| VaultIoError::BadKey)?;
            let raw: [u8; 32] = bytes
                .as_slice()
                .try_into()
                .map_err(|_| VaultIoError::BadKey)?;
            Ok(VaultKey::from_bytes(raw))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let mut raw = [0u8; 32];
            SystemRandom::new()
                .fill(&mut raw)
                .map_err(|_| VaultIoError::BadKey)?;
            write_private(&path, &raw)?;
            Ok(VaultKey::from_bytes(raw))
        }
        Err(e) => Err(VaultIoError::Io(format!("{}: {e}", path.display()))),
    }
}

/// Open the vault one directory holds, creating key and directory on first use.
/// A missing entries file is an empty vault; a malformed or oversized one is an
/// error — ciphertext is never "set aside", because losing it silently would lose
/// the owner's records without a trace.
pub fn open_vault(dir: &Path) -> Result<VaultGuard, VaultIoError> {
    std::fs::create_dir_all(dir)
        .map_err(|e| VaultIoError::Io(format!("{}: {e}", dir.display())))?;
    let directory = std::fs::canonicalize(dir).map_err(|e| VaultIoError::Io(e.to_string()))?;
    let lock = transaction_lock(&directory)?;
    let key = load_or_create_key(&directory)?;
    let path = dir.join(ENTRIES_FILE);
    let entries: Vec<SealedEntry> = match File::open(&path) {
        Ok(file) => {
            let bytes = bounded_read(file, MAX_ENTRIES_BYTES)?;
            let entries: Vec<SealedEntry> =
                serde_json::from_slice(&bytes).map_err(|_| VaultIoError::MalformedEntries)?;
            if entries.len() > MAX_SEALED_ENTRIES {
                return Err(VaultIoError::TooLarge {
                    bytes: entries.len() as u64,
                    cap: MAX_SEALED_ENTRIES as u64,
                });
            }
            entries
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(e) => return Err(VaultIoError::Io(format!("{}: {e}", path.display()))),
    };
    Ok(VaultGuard {
        vault: Vault::load(&key, entries)?,
        directory,
        _lock: lock,
    })
}

/// Persist every sealed entry, atomically. The caps are re-checked on the way out:
/// a vault that outgrew them refuses to write rather than writing a file the next
/// open would reject.
pub fn persist_vault(dir: &Path, vault: &VaultGuard) -> Result<(), VaultIoError> {
    let directory = std::fs::canonicalize(dir).map_err(|e| VaultIoError::Io(e.to_string()))?;
    if directory != vault.directory {
        return Err(VaultIoError::DirectoryMismatch);
    }
    let entries = vault.sealed_entries();
    if entries.len() > MAX_SEALED_ENTRIES {
        return Err(VaultIoError::TooLarge {
            bytes: entries.len() as u64,
            cap: MAX_SEALED_ENTRIES as u64,
        });
    }
    let bytes = serde_json::to_vec(&entries).map_err(|_| VaultIoError::MalformedEntries)?;
    if bytes.len() as u64 > MAX_ENTRIES_BYTES {
        return Err(VaultIoError::TooLarge {
            bytes: bytes.len() as u64,
            cap: MAX_ENTRIES_BYTES,
        });
    }
    write_atomic(dir, ENTRIES_FILE, &bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "archie-cerebra-vault-io-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_fresh_directory_opens_empty_and_reopens_with_its_entries() {
        let dir = temp_dir("roundtrip");
        let reference = archie_vault::VaultRef::parse("vault:test/one").unwrap();
        let mut vault = open_vault(&dir).unwrap();
        vault
            .store(
                "test",
                reference.clone(),
                archie_vault::EntryClass::PrivateFact,
                archie_types::PrivacyGrade::Restricted,
                "a sealed fixture record",
                1_000,
                None,
            )
            .unwrap();
        persist_vault(&dir, &vault).unwrap();
        drop(vault);
        let reopened = open_vault(&dir).unwrap();
        assert!(reopened.contains(&reference));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_malformed_entries_file_is_a_typed_error_not_an_empty_vault() {
        let dir = temp_dir("malformed");
        open_vault(&dir).unwrap();
        std::fs::write(dir.join(ENTRIES_FILE), b"not json").unwrap();
        assert!(matches!(
            open_vault(&dir),
            Err(VaultIoError::MalformedEntries)
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
