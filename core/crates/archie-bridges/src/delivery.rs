//! Delivery: results streamed from the node that works to the person's own computer.
//!
//! Archie can run for days on a server or in a VM — two hundred hours of FreeCAD
//! iterations — while the person works elsewhere. Each stage it delivers lands in a folder
//! on their workstation, where their own software opens it: a STEP in Fusion 360 or
//! COMSOL, a PSD in Photoshop, a DOCX in Word, an ODB++ in the board house's viewer.
//!
//! ```text
//! node (server or VM)                               workstation
//! bridge exports ──► archie_deliver ──► 127.0.0.1:7447 ══ssh -R══► archie-desk ──► ~/Archie/<project>/
//!                         │ queued in .archie/outbox when the desk is away      stages/<stage>/…
//!                         └ retried on the next delivery or archie_delivery_flush  latest/…  index.html
//! ```
//!
//! The node only ever talks to its own loopback. The workstation brings the connection,
//! with `ssh -R 7447:127.0.0.1:7447 <node>`, so nothing listens on a network and the
//! bytes travel inside ssh. Every request carries an HMAC-SHA256 over what it delivers,
//! with the key the two sides were paired with, and a timestamp the desk checks, so a
//! process on the node without the key cannot deliver and a recorded request cannot be
//! replayed.

use crate::workspace::{Workspace, WorkspaceError};
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// The desk's port, by default, on both ends of the tunnel.
pub const DEFAULT_PORT: u16 = 7447;
/// How far a request's timestamp may be from the desk's clock.
pub const MAX_SKEW_MS: u64 = 5 * 60 * 1000;
/// The largest file one delivery carries.
pub const MAX_FILE: u64 = 8 * 1024 * 1024 * 1024;
const DOMAIN: &str = "archie-deliver/1";

#[derive(Debug, thiserror::Error)]
pub enum DeliveryError {
    #[error("the delivery key: {0}")]
    Key(String),
    #[error(
        "{0:?} cannot be a name on the desk: letters, digits, '.', '-' and '_', 1 to 96 of them"
    )]
    Name(String),
    #[error("the desk at {0} is not reachable: {1}. Is `ssh -R {port}:127.0.0.1:{port}` open from the workstation? The delivery is queued.", port = DEFAULT_PORT)]
    Unreachable(SocketAddr, String),
    #[error("the desk refused {file}: {status}")]
    Refused { file: String, status: String },
    #[error(transparent)]
    Workspace(#[from] WorkspaceError),
    #[error("{0}")]
    Io(String),
}

/// The key a node and a desk share, 32 bytes, kept as 64 hex characters in a file only
/// its owner can read.
#[derive(Clone)]
pub struct DeliveryKey([u8; 32]);

impl std::fmt::Debug for DeliveryKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DeliveryKey(…)")
    }
}

impl DeliveryKey {
    pub fn generate() -> Self {
        let mut bytes = [0u8; 32];
        ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut bytes)
            .expect("the system's random source");
        Self(bytes)
    }

    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }

    pub fn from_hex(text: &str) -> Result<Self, DeliveryError> {
        let bytes = hex::decode(text.trim()).map_err(|_| DeliveryError::Key("not hex".into()))?;
        let bytes: [u8; 32] = bytes
            .try_into()
            .map_err(|_| DeliveryError::Key("a key is 64 hex characters".into()))?;
        Ok(Self(bytes))
    }

    pub fn load(path: &Path) -> Result<Self, DeliveryError> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(path)
                .map_err(|e| DeliveryError::Key(format!("{}: {e}", path.display())))?
                .permissions()
                .mode();
            if mode & 0o077 != 0 {
                return Err(DeliveryError::Key(format!(
                    "{} is readable by others; chmod 600 it",
                    path.display()
                )));
            }
        }
        let text = std::fs::read_to_string(path)
            .map_err(|e| DeliveryError::Key(format!("{}: {e}", path.display())))?;
        Self::from_hex(&text)
    }

    /// Writes a new key file that only its owner can read.
    pub fn save(&self, path: &Path) -> Result<(), DeliveryError> {
        let io = |e: std::io::Error| DeliveryError::Key(format!("{}: {e}", path.display()));
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(path).map_err(io)?;
        file.write_all(self.to_hex().as_bytes()).map_err(io)?;
        Ok(())
    }

    fn hmac(&self) -> ring::hmac::Key {
        ring::hmac::Key::new(ring::hmac::HMAC_SHA256, &self.0)
    }
}

/// What one request delivers, and what the signature covers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Envelope {
    pub project: String,
    pub stage: String,
    pub file: String,
    pub sha256: String,
    pub bytes: u64,
    pub stamp_ms: u64,
    /// A line about the stage, shown on the desk's page.
    pub note: String,
    /// Whether the desk should open the file with the person's application, if its
    /// rules allow that kind of file.
    pub open: bool,
}

impl Envelope {
    pub fn path(&self) -> String {
        format!("/v1/deliver/{}/{}/{}", self.project, self.stage, self.file)
    }

    fn signed_text(&self) -> String {
        format!(
            "{DOMAIN}\nPUT\n{}\n{}\n{}\n{}\n{}\n{}",
            self.path(),
            self.sha256,
            self.bytes,
            self.stamp_ms,
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(self.note.as_bytes()),
            u8::from(self.open)
        )
    }

    pub fn sign(&self, key: &DeliveryKey) -> String {
        hex::encode(ring::hmac::sign(&key.hmac(), self.signed_text().as_bytes()).as_ref())
    }

    pub fn verify(&self, key: &DeliveryKey, signature: &str) -> bool {
        let Ok(tag) = hex::decode(signature) else {
            return false;
        };
        ring::hmac::verify(&key.hmac(), self.signed_text().as_bytes(), &tag).is_ok()
    }

    /// The headers that carry the envelope beside the body.
    pub fn headers(&self, key: &DeliveryKey) -> Vec<(&'static str, String)> {
        vec![
            ("x-archie-sha256", self.sha256.clone()),
            ("x-archie-stamp", self.stamp_ms.to_string()),
            (
                "x-archie-note",
                base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(self.note.as_bytes()),
            ),
            ("x-archie-open", u8::from(self.open).to_string()),
            ("x-archie-signature", self.sign(key)),
        ]
    }

    /// The envelope a desk rebuilds from a request, to check its signature.
    pub fn from_request(
        project: &str,
        stage: &str,
        file: &str,
        bytes: u64,
        header: impl Fn(&str) -> Option<String>,
    ) -> Result<Self, String> {
        for name in [project, stage, file] {
            if !is_name(name) {
                return Err(format!("{name:?} is not a plain name"));
            }
        }
        let sha256 = header("x-archie-sha256").ok_or("no x-archie-sha256")?;
        if sha256.len() != 64 || !sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("x-archie-sha256 is 64 hex characters".into());
        }
        let stamp_ms = header("x-archie-stamp")
            .and_then(|s| s.parse().ok())
            .ok_or("no x-archie-stamp")?;
        let note = header("x-archie-note")
            .map(|n| {
                base64::engine::general_purpose::URL_SAFE_NO_PAD
                    .decode(n.as_bytes())
                    .ok()
                    .and_then(|b| String::from_utf8(b).ok())
                    .ok_or("x-archie-note is not base64 UTF-8")
            })
            .transpose()?
            .unwrap_or_default();
        Ok(Self {
            project: project.into(),
            stage: stage.into(),
            file: file.into(),
            sha256: sha256.to_ascii_lowercase(),
            bytes,
            stamp_ms,
            note,
            open: header("x-archie-open").as_deref() == Some("1"),
        })
    }
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// A name the desk accepts for a project, a stage or a file: nothing that could climb
/// out of its folder, and nothing a shell or a file system would read specially.
pub fn is_name(name: &str) -> bool {
    (1..=96).contains(&name.len())
        && !name.starts_with('.')
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
}

/// `text` made into a name the desk accepts: other characters become `_`.
pub fn to_name(text: &str) -> String {
    let mut name: String = text
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect();
    while name.starts_with('.') {
        name.remove(0);
    }
    name.truncate(96);
    if name.is_empty() {
        "unnamed".into()
    } else {
        name
    }
}

/// The SHA-256 of a file and its size, read once.
pub fn digest_file(path: &Path) -> std::io::Result<(String, u64)> {
    let mut file = std::fs::File::open(path)?;
    let mut context = ring::digest::Context::new(&ring::digest::SHA256);
    let mut buffer = vec![0u8; 256 * 1024];
    let mut total = 0u64;
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        context.update(&buffer[..n]);
        total += n as u64;
    }
    Ok((hex::encode(context.finish().as_ref()), total))
}

/// One delivery waiting in the outbox.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Pending {
    /// The file, relative to the workspace it came from.
    pub file: String,
    pub project: String,
    pub stage: String,
    pub note: String,
    pub open: bool,
}

/// What a delivery did with each file.
#[derive(Debug, Clone, Serialize)]
pub struct Delivered {
    pub delivered: Vec<String>,
    pub queued: Vec<String>,
    pub desk: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub why_queued: Option<String>,
}

/// The node's side: where the desk is, and the key.
#[derive(Debug, Clone)]
pub struct Deliverer {
    desk: SocketAddr,
    key: DeliveryKey,
    timeout: Duration,
}

impl Deliverer {
    /// `desk` must be a loopback address: the workstation's tunnel ends there.
    pub fn new(desk: SocketAddr, key: DeliveryKey) -> Result<Self, DeliveryError> {
        if !desk.ip().is_loopback() {
            return Err(DeliveryError::Key(format!(
                "{desk} is not a loopback address; the desk is reached through an ssh tunnel that ends on this node's loopback"
            )));
        }
        Ok(Self {
            desk,
            key,
            timeout: Duration::from_secs(60),
        })
    }

    pub fn desk(&self) -> SocketAddr {
        self.desk
    }

    /// Whether the desk answers, and its version.
    pub fn health(&self) -> Result<String, DeliveryError> {
        let mut stream = TcpStream::connect_timeout(&self.desk, Duration::from_secs(3))
            .map_err(|e| DeliveryError::Unreachable(self.desk, e.to_string()))?;
        stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
        write!(
            stream,
            "GET /v1/health HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
            self.desk
        )
        .map_err(|e| DeliveryError::Unreachable(self.desk, e.to_string()))?;
        let mut answer = String::new();
        stream
            .take(4096)
            .read_to_string(&mut answer)
            .map_err(|e| DeliveryError::Unreachable(self.desk, e.to_string()))?;
        if !answer.starts_with("HTTP/1.1 200") {
            return Err(DeliveryError::Unreachable(
                self.desk,
                "not an archie-desk".into(),
            ));
        }
        Ok(answer
            .split("\r\n\r\n")
            .nth(1)
            .unwrap_or("")
            .trim()
            .to_string())
    }

    /// Sends one file.
    fn put(&self, path: &Path, envelope: &Envelope) -> Result<(), DeliveryError> {
        let unreachable = |e: std::io::Error| DeliveryError::Unreachable(self.desk, e.to_string());
        let mut stream =
            TcpStream::connect_timeout(&self.desk, Duration::from_secs(3)).map_err(unreachable)?;
        stream.set_write_timeout(Some(self.timeout)).ok();
        stream.set_read_timeout(Some(self.timeout)).ok();
        let mut head = format!(
            "PUT {} HTTP/1.1\r\nHost: {}\r\nContent-Length: {}\r\nContent-Type: application/octet-stream\r\nConnection: close\r\n",
            envelope.path(),
            self.desk,
            envelope.bytes
        );
        for (name, value) in envelope.headers(&self.key) {
            head.push_str(&format!("{name}: {value}\r\n"));
        }
        head.push_str("\r\n");
        stream.write_all(head.as_bytes()).map_err(unreachable)?;
        let mut file = std::fs::File::open(path).map_err(|e| DeliveryError::Io(e.to_string()))?;
        std::io::copy(&mut (&mut file).take(envelope.bytes), &mut stream).map_err(unreachable)?;
        stream.flush().map_err(unreachable)?;
        let mut status = String::new();
        BufReader::new(stream)
            .read_line(&mut status)
            .map_err(unreachable)?;
        if status.starts_with("HTTP/1.1 2") {
            Ok(())
        } else {
            Err(DeliveryError::Refused {
                file: envelope.file.clone(),
                status: status.trim().to_string(),
            })
        }
    }

    /// Delivers `files` (relative to `workspace`) as one stage of `project`. Files the
    /// desk cannot take now wait in the workspace's outbox; files waiting from before go
    /// first.
    pub fn deliver(
        &self,
        workspace: &Workspace,
        files: &[String],
        project: &str,
        stage: &str,
        note: &str,
        open: bool,
    ) -> Result<Delivered, DeliveryError> {
        let project = to_name(project);
        let stage = to_name(stage);
        let mut batch = Vec::new();
        for file in files {
            // Checked now, so a planner learns of a bad path at once and not from the queue.
            workspace.file(file)?;
            batch.push(Pending {
                file: file.clone(),
                project: project.clone(),
                stage: stage.clone(),
                note: note.to_string(),
                open,
            });
        }
        let mut waiting = Outbox::new(workspace)?.take()?;
        waiting.extend(batch);
        self.send(workspace, waiting)
    }

    /// Sends what waits in the outbox.
    pub fn flush(&self, workspace: &Workspace) -> Result<Delivered, DeliveryError> {
        let waiting = Outbox::new(workspace)?.take()?;
        self.send(workspace, waiting)
    }

    fn send(&self, workspace: &Workspace, items: Vec<Pending>) -> Result<Delivered, DeliveryError> {
        let mut report = Delivered {
            delivered: Vec::new(),
            queued: Vec::new(),
            desk: self.desk.to_string(),
            why_queued: None,
        };
        let mut left = Vec::new();
        for item in items {
            if report.why_queued.is_some() {
                left.push(item);
                continue;
            }
            let Ok(path) = workspace.file(&item.file) else {
                // The file is gone since it was queued: nothing left to deliver.
                continue;
            };
            let (sha256, bytes) =
                digest_file(&path).map_err(|e| DeliveryError::Io(e.to_string()))?;
            if bytes > MAX_FILE {
                return Err(DeliveryError::Io(format!(
                    "{} is larger than {MAX_FILE} bytes",
                    item.file
                )));
            }
            let name = to_name(path.file_name().and_then(|n| n.to_str()).unwrap_or("file"));
            let envelope = Envelope {
                project: item.project.clone(),
                stage: item.stage.clone(),
                file: name,
                sha256,
                bytes,
                stamp_ms: now_ms(),
                note: item.note.clone(),
                open: item.open,
            };
            match self.put(&path, &envelope) {
                Ok(()) => report.delivered.push(format!(
                    "{}/{}/{}",
                    envelope.project, envelope.stage, envelope.file
                )),
                Err(DeliveryError::Unreachable(_, why)) => {
                    report.why_queued = Some(why);
                    left.push(item);
                }
                Err(other) => return Err(other),
            }
        }
        report.queued = left.iter().map(|p| p.file.clone()).collect();
        Outbox::new(workspace)?.put_back(&left)?;
        Ok(report)
    }
}

/// Deliveries waiting for the desk, in `.archie/outbox/pending.jsonl`.
pub struct Outbox {
    file: PathBuf,
}

impl Outbox {
    pub fn new(workspace: &Workspace) -> Result<Self, DeliveryError> {
        Ok(Self {
            file: workspace.scratch("outbox")?.join("pending.jsonl"),
        })
    }

    pub fn pending(&self) -> Vec<Pending> {
        std::fs::read_to_string(&self.file)
            .unwrap_or_default()
            .lines()
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect()
    }

    fn take(&self) -> Result<Vec<Pending>, DeliveryError> {
        let pending = self.pending();
        match std::fs::remove_file(&self.file) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(DeliveryError::Io(e.to_string())),
        }
        Ok(pending)
    }

    fn put_back(&self, items: &[Pending]) -> Result<(), DeliveryError> {
        if items.is_empty() {
            return Ok(());
        }
        let mut text = String::new();
        for item in items {
            text.push_str(
                &serde_json::to_string(item).map_err(|e| DeliveryError::Io(e.to_string()))?,
            );
            text.push('\n');
        }
        std::fs::write(&self.file, text).map_err(|e| DeliveryError::Io(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn envelope() -> Envelope {
        Envelope {
            project: "bracket".into(),
            stage: "iter-0042".into(),
            file: "bracket.step".into(),
            sha256: "ab".repeat(32),
            bytes: 1234,
            stamp_ms: 1_790_000_000_000,
            note: "mass 41.2 g, ñ ok".into(),
            open: true,
        }
    }

    #[test]
    fn a_signature_covers_every_field() {
        let key = DeliveryKey::generate();
        let e = envelope();
        let signature = e.sign(&key);
        assert!(e.verify(&key, &signature));
        assert!(
            !e.verify(&DeliveryKey::generate(), &signature),
            "another key"
        );
        for changed in [
            Envelope {
                stage: "final".into(),
                ..e.clone()
            },
            Envelope {
                file: "other.step".into(),
                ..e.clone()
            },
            Envelope {
                sha256: "cd".repeat(32),
                ..e.clone()
            },
            Envelope {
                bytes: 1,
                ..e.clone()
            },
            Envelope {
                stamp_ms: e.stamp_ms + 1,
                ..e.clone()
            },
            Envelope {
                note: "x".into(),
                ..e.clone()
            },
            Envelope {
                open: false,
                ..e.clone()
            },
        ] {
            assert!(!changed.verify(&key, &signature), "{changed:?}");
        }
    }

    #[test]
    fn a_request_rebuilds_the_envelope_it_was_signed_from() {
        let key = DeliveryKey::generate();
        let e = envelope();
        let headers = e.headers(&key);
        let header = |name: &str| {
            headers
                .iter()
                .find(|(n, _)| *n == name)
                .map(|(_, v)| v.clone())
        };
        let rebuilt =
            Envelope::from_request(&e.project, &e.stage, &e.file, e.bytes, header).unwrap();
        assert_eq!(rebuilt, e);
        assert!(rebuilt.verify(&key, &header("x-archie-signature").unwrap()));
        assert!(Envelope::from_request("..", "s", "f", 1, header).is_err());
    }

    #[test]
    fn names_stay_inside_their_folder() {
        for bad in [
            "",
            "..",
            ".hidden",
            "a/b",
            "a\\b",
            "C:x",
            "a b",
            &"x".repeat(97),
        ] {
            assert!(!is_name(bad), "{bad:?}");
        }
        assert!(is_name("bracket-v2.step"));
        assert_eq!(to_name("../Mi pieza (v2).step"), "_Mi_pieza__v2_.step");
        assert_eq!(to_name("..."), "unnamed");
    }

    #[test]
    fn the_key_round_trips_and_the_desk_is_only_loopback() {
        let key = DeliveryKey::generate();
        assert_eq!(DeliveryKey::from_hex(&key.to_hex()).unwrap().0, key.0);
        assert!(DeliveryKey::from_hex("abcd").is_err());
        assert!(Deliverer::new("10.0.0.5:7447".parse().unwrap(), key.clone()).is_err());
        assert!(Deliverer::new("127.0.0.1:7447".parse().unwrap(), key).is_ok());
    }

    #[test]
    fn with_no_desk_listening_the_files_wait_in_the_outbox() {
        let dir = std::env::temp_dir().join(format!("archie-deliver-queue-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("exports")).unwrap();
        std::fs::write(dir.join("exports/a.step"), b"ISO-10303-21;").unwrap();
        let workspace = Workspace::open(&dir).unwrap();
        // A port nothing listens on.
        let closed = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap();
        let deliverer = Deliverer::new(closed, DeliveryKey::generate()).unwrap();
        let report = deliverer
            .deliver(
                &workspace,
                &["exports/a.step".into()],
                "bracket",
                "iter 1",
                "first",
                false,
            )
            .unwrap();
        assert!(report.delivered.is_empty());
        assert_eq!(report.queued, ["exports/a.step"]);
        let pending = Outbox::new(&workspace).unwrap().pending();
        assert_eq!(pending[0].stage, "iter_1");
        assert!(deliverer
            .deliver(&workspace, &["../x".into()], "p", "s", "", false)
            .is_err());
        let _ = std::fs::remove_dir_all(dir);
    }
}
