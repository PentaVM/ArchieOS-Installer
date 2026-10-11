//! The day's windows, encrypted under a key that lives one day.
//!
//! Nelson's retention decision (docs/IRIS-PRIVACY.md): what a person's sessions leave for
//! learning stays on the node, encrypted, and is deleted after each REM cycle. Deleting
//! here means destroying the key. A file removed from a directory can sit in a disk's
//! spare blocks for years; ciphertext whose key was overwritten cannot be read from them.
//!
//! Each window is sealed with AES-256-GCM under the day's key, with its own name as
//! authenticated data, so a window renamed to pass for another does not open.

use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM, NONCE_LEN};
use ring::rand::{SecureRandom, SystemRandom};
use std::io::Write;
use std::path::{Path, PathBuf};
use thiserror::Error;

const KEY_FILE: &str = "day.key";
const SEALED: &str = "sealed";

#[derive(Debug, Error)]
pub enum BufferError {
    #[error("the day buffer at {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("a name is 1 to 200 letters, digits, '-', '_', '.' or '+' with no part starting with '.', and {0:?} is not")]
    Name(String),
    #[error("the window {0} does not open under the day's key")]
    Unreadable(String),
    #[error("the day's key is gone: the buffer was shredded")]
    Shredded,
    #[error("the system's random source failed")]
    Random,
}

/// A directory of sealed windows and the key they are sealed under.
pub struct DayBuffer {
    dir: PathBuf,
    key: LessSafeKey,
    rng: SystemRandom,
}

impl DayBuffer {
    /// Opens the buffer in `dir`, making the directory and a new key when there is none.
    pub fn open(dir: impl Into<PathBuf>) -> Result<Self, BufferError> {
        let dir = dir.into();
        let io = |source| BufferError::Io { path: dir.display().to_string(), source };
        std::fs::create_dir_all(&dir).map_err(io)?;
        let rng = SystemRandom::new();
        let key_path = dir.join(KEY_FILE);
        let bytes = match std::fs::read(&key_path) {
            Ok(bytes) if bytes.len() == 32 => bytes,
            Ok(_) => return Err(BufferError::Shredded),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let mut bytes = vec![0u8; 32];
                rng.fill(&mut bytes).map_err(|_| BufferError::Random)?;
                write_private(&key_path, &bytes).map_err(io)?;
                bytes
            }
            Err(source) => return Err(io(source)),
        };
        let unbound = UnboundKey::new(&AES_256_GCM, &bytes).map_err(|_| BufferError::Random)?;
        Ok(Self { dir, key: LessSafeKey::new(unbound), rng })
    }

    /// Whether `dir` holds a day's windows at all.
    pub fn exists(dir: &Path) -> bool {
        dir.join(KEY_FILE).exists()
    }

    /// When the day in `dir` started: when its key was made.
    pub fn started(dir: &Path) -> Option<std::time::SystemTime> {
        std::fs::metadata(dir.join(KEY_FILE)).and_then(|meta| meta.modified()).ok()
    }

    fn path_of(&self, name: &str) -> Result<PathBuf, BufferError> {
        // `+` stands between the folders a file goes back into when the buffer is opened
        // for the night, so no part may be empty or start with a dot: `a+..+b` would be a
        // path out of the work directory.
        let valid = !name.is_empty()
            && name.len() <= 200
            && name.split('+').all(|part| !part.is_empty() && !part.starts_with('.'))
            && name.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'+'));
        if !valid {
            return Err(BufferError::Name(name.to_string()));
        }
        Ok(self.dir.join(format!("{name}.{SEALED}")))
    }

    pub fn seal(&self, name: &str, plain: &[u8]) -> Result<(), BufferError> {
        let path = self.path_of(name)?;
        let mut nonce = [0u8; NONCE_LEN];
        self.rng.fill(&mut nonce).map_err(|_| BufferError::Random)?;
        let mut sealed = plain.to_vec();
        self.key
            .seal_in_place_append_tag(Nonce::assume_unique_for_key(nonce), Aad::from(name.as_bytes()), &mut sealed)
            .map_err(|_| BufferError::Random)?;
        let mut file = nonce.to_vec();
        file.extend_from_slice(&sealed);
        write_private(&path, &file).map_err(|source| BufferError::Io { path: path.display().to_string(), source })
    }

    pub fn unseal(&self, name: &str) -> Result<Vec<u8>, BufferError> {
        let path = self.path_of(name)?;
        let mut file = std::fs::read(&path).map_err(|source| BufferError::Io { path: path.display().to_string(), source })?;
        if file.len() < NONCE_LEN {
            return Err(BufferError::Unreadable(name.to_string()));
        }
        let mut nonce = [0u8; NONCE_LEN];
        nonce.copy_from_slice(&file[..NONCE_LEN]);
        let plain = self
            .key
            .open_in_place(Nonce::assume_unique_for_key(nonce), Aad::from(name.as_bytes()), &mut file[NONCE_LEN..])
            .map_err(|_| BufferError::Unreadable(name.to_string()))?;
        Ok(plain.to_vec())
    }

    /// The names of the windows held, sorted.
    pub fn names(&self) -> Result<Vec<String>, BufferError> {
        let io = |source| BufferError::Io { path: self.dir.display().to_string(), source };
        let mut names: Vec<String> = std::fs::read_dir(&self.dir)
            .map_err(io)?
            .filter_map(|entry| entry.ok().map(|e| e.path()))
            .filter(|path| path.extension().is_some_and(|e| e == SEALED))
            .filter_map(|path| path.file_stem().and_then(|s| s.to_str()).map(str::to_string))
            .collect();
        names.sort();
        Ok(names)
    }

    /// Destroys the key, then removes what it sealed. The key is overwritten and synced
    /// before it is unlinked; after this, [`DayBuffer::open`] on the same directory starts
    /// a new day with a new key.
    pub fn shred(self) -> Result<(), BufferError> {
        shred_dir(&self.dir)
    }
}

/// Shreds the buffer in `dir` without opening it: the path taken after a crash, when the
/// key may already be half gone. Nothing there is not an error.
pub fn shred_dir(dir: &Path) -> Result<(), BufferError> {
    let io = |source| BufferError::Io { path: dir.display().to_string(), source };
    let key_path = dir.join(KEY_FILE);
    match std::fs::OpenOptions::new().write(true).open(&key_path) {
        Ok(mut key) => {
            key.write_all(&[0u8; 32]).map_err(io)?;
            key.sync_all().map_err(io)?;
            drop(key);
            std::fs::remove_file(&key_path).map_err(io)?;
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(source) => return Err(io(source)),
    }
    match std::fs::remove_dir_all(dir) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(io(source)),
    }
}

/// Writes a file only its owner can read.
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("archie-rem-buffer-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn a_window_is_unreadable_on_disk_and_opens_under_the_days_key() {
        let dir = scratch("seal");
        let buffer = DayBuffer::open(&dir).unwrap();
        let window = b"a frame of someone's screen".repeat(40);
        buffer.seal("photopea.run1.s0", &window).unwrap();
        let on_disk = std::fs::read(dir.join("photopea.run1.s0.sealed")).unwrap();
        assert!(!on_disk.windows(12).any(|w| w == b"someone's sc"), "the plain bytes are not on disk");
        assert_eq!(buffer.unseal("photopea.run1.s0").unwrap(), window);
        assert_eq!(buffer.names().unwrap(), vec!["photopea.run1.s0"]);

        // The same key reopens it for the night's cycle.
        assert_eq!(DayBuffer::open(&dir).unwrap().unseal("photopea.run1.s0").unwrap(), window);
    }

    #[test]
    fn a_window_renamed_to_pass_for_another_does_not_open() {
        let dir = scratch("rename");
        let buffer = DayBuffer::open(&dir).unwrap();
        buffer.seal("bank.s0", b"excluded").unwrap();
        std::fs::rename(dir.join("bank.s0.sealed"), dir.join("photopea.s0.sealed")).unwrap();
        assert!(matches!(buffer.unseal("photopea.s0"), Err(BufferError::Unreadable(_))));
        for escape in ["../escape", "windows+..+escape", "windows++x", ".hidden"] {
            assert!(matches!(buffer.seal(escape, b"x"), Err(BufferError::Name(_))), "{escape}");
        }
    }

    #[test]
    fn shredding_destroys_the_key_and_a_kept_copy_of_a_window_stays_closed() {
        let dir = scratch("shred");
        let buffer = DayBuffer::open(&dir).unwrap();
        buffer.seal("s0", b"today").unwrap();
        // What a failing disk, a backup or a snapshot might keep.
        let kept = std::fs::read(dir.join("s0.sealed")).unwrap();
        buffer.shred().unwrap();
        assert!(!dir.exists());
        assert!(!DayBuffer::exists(&dir));

        // A new day starts with a new key, and yesterday's ciphertext does not open under it.
        let tomorrow = DayBuffer::open(&dir).unwrap();
        std::fs::write(dir.join("s0.sealed"), &kept).unwrap();
        assert!(matches!(tomorrow.unseal("s0"), Err(BufferError::Unreadable(_))));
        shred_dir(&dir).unwrap();
        shred_dir(&dir).unwrap();
    }
}
