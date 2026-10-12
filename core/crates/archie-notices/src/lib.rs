//! Owner notices: the first response to what Sentry sees, and the only one taken
//! without asking.
//!
//! A notice tells the owner; it changes nothing on the network. Blocking a device or
//! quarantining it is a different kind of act and goes through an approved `Intent`
//! (`archie-sentry`), never through here. That is what makes this response reversible:
//! the worst a wrong notice does is interrupt someone, and acknowledging it ends that.
//!
//! The board is one small JSON file (`archie-sentry-notices-v1`) that the alert follower
//! writes and anyone may read: the CLI, `archie-displayd`'s `/api/notices` for the
//! Observatory, and the voice. Each notice carries two texts:
//!
//! - `detail`, what the audit saw, for a screen the owner is looking at;
//! - `spoken`, a fixed sentence per rule that names no device, address or DNS name, so
//!   it can be said aloud in a shared room. The voice reads `spoken` and nothing else.
//!
//! A notice's id is derived from its content, so the follower re-reading alerts after a
//! crash posts nothing twice.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::Path;

/// The format marker of the file.
pub const FORMAT: &str = "archie-sentry-notices-v1";
/// How many notices the board keeps; the oldest acknowledged ones go first.
pub const MAX_NOTICES: usize = 256;
/// A board larger than this is not one this crate wrote.
pub const MAX_FILE_BYTES: u64 = 4 * 1024 * 1024;
const MAX_FIELD_CHARS: usize = 1024;

/// How serious a notice is, in the netwatch audit's own three steps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Notice,
    Warning,
    High,
}

impl Severity {
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "notice" => Some(Self::Notice),
            "warning" => Some(Self::Warning),
            "high" => Some(Self::High),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Notice => "notice",
            Self::Warning => "warning",
            Self::High => "high",
        }
    }
}

/// Which alerts become notices. Below the floor an alert is still recorded by Cerebra;
/// it only does not interrupt anyone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Policy {
    pub min_severity: Severity,
}

impl Default for Policy {
    /// `warning`: a new device or an ordinary DHCP move is not worth an interruption.
    fn default() -> Self {
        Self {
            min_severity: Severity::Warning,
        }
    }
}

/// What the audit raised, as the follower read it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Raised<'a> {
    pub at_ms: u64,
    pub severity: Severity,
    pub rule: &'a str,
    pub device: &'a str,
    pub detail: &'a str,
}

/// One notice on the board.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Notice {
    /// Content-derived: the same alert is the same notice.
    pub id: String,
    pub at_ms: u64,
    pub severity: Severity,
    pub rule: String,
    pub device: String,
    /// What was seen, for a screen.
    pub detail: String,
    /// What may be said aloud: no device, address or name in it.
    pub spoken: String,
    /// When the voice said it, if it has.
    #[serde(default)]
    pub spoken_ms: Option<u64>,
    /// When the owner acknowledged it, if they have.
    #[serde(default)]
    pub acknowledged_ms: Option<u64>,
}

impl Notice {
    pub fn pending(&self) -> bool {
        self.acknowledged_ms.is_none()
    }
}

/// Why a board file was not used.
#[derive(Debug, thiserror::Error)]
pub enum NoticeError {
    #[error("{path}: {message}")]
    Io { path: String, message: String },
    #[error("the notice board is {bytes} bytes, over the {cap}-byte cap")]
    TooLarge { bytes: u64, cap: u64 },
    #[error("the notice board is not {FORMAT} JSON: {0}")]
    Malformed(String),
}

fn io(path: &Path, error: impl std::fmt::Display) -> NoticeError {
    NoticeError::Io {
        path: path.display().to_string(),
        message: error.to_string(),
    }
}

/// The board: every notice still kept, oldest first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Board {
    pub format: String,
    pub notices: Vec<Notice>,
}

impl Default for Board {
    fn default() -> Self {
        Self {
            format: FORMAT.into(),
            notices: Vec::new(),
        }
    }
}

/// The sentence the voice may say for a rule. Fixed text: nothing the network chose
/// (a host name, a DNS question) is ever read aloud.
pub fn spoken_for(rule: &str, severity: Severity) -> &'static str {
    match (rule, severity) {
        ("address-moved", Severity::High) => {
            "Sentry: otro equipo está reclamando la dirección del router; el tráfico podría estar pasando por él."
        }
        ("address-moved", _) => "Sentry: una dirección de la red cambió de equipo.",
        ("new-device", _) => "Sentry: hay un dispositivo nuevo en la red.",
        ("port-scan", _) => "Sentry: un equipo de la red está recorriendo los puertos de otro.",
        ("host-sweep", _) => {
            "Sentry: un equipo de la red está probando el mismo puerto en muchos otros."
        }
        ("cleartext-login", _) => "Sentry: un equipo abrió una sesión sin cifrar.",
        ("traffic-surge", _) => {
            "Sentry: un equipo está enviando mucho más tráfico de lo habitual."
        }
        ("dns-tunnel-suspect", _) => {
            "Sentry: un equipo hace consultas DNS que parecen llevar datos."
        }
        _ => "Sentry: hay un aviso de la red; los detalles están en el Observatory.",
    }
}

/// The id of a raised alert: the first 16 bytes of SHA-256 over its fields.
pub fn notice_id(raised: &Raised<'_>) -> String {
    let mut hash = Sha256::new();
    for part in [
        raised.at_ms.to_string().as_str(),
        raised.severity.as_str(),
        raised.rule,
        raised.device,
        raised.detail,
    ] {
        hash.update((part.len() as u64).to_le_bytes());
        hash.update(part.as_bytes());
    }
    hex::encode(&hash.finalize()[..16])
}

fn clip(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_control())
        .take(MAX_FIELD_CHARS)
        .collect()
}

impl Board {
    /// Reads a board. A missing file is an empty board; a file that is too large, of
    /// another format or not JSON is an error the caller decides about.
    pub fn load(path: &Path) -> Result<Self, NoticeError> {
        let bytes = match std::fs::metadata(path) {
            Ok(meta) => meta.len(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => return Err(io(path, e)),
        };
        if bytes > MAX_FILE_BYTES {
            return Err(NoticeError::TooLarge {
                bytes,
                cap: MAX_FILE_BYTES,
            });
        }
        let text = std::fs::read_to_string(path).map_err(|e| io(path, e))?;
        Self::parse(&text)
    }

    /// Parses a board with the same caps [`Board::load`] applies.
    pub fn parse(text: &str) -> Result<Self, NoticeError> {
        if text.len() as u64 > MAX_FILE_BYTES {
            return Err(NoticeError::TooLarge {
                bytes: text.len() as u64,
                cap: MAX_FILE_BYTES,
            });
        }
        let board: Self =
            serde_json::from_str(text).map_err(|e| NoticeError::Malformed(e.to_string()))?;
        if board.format != FORMAT {
            return Err(NoticeError::Malformed(format!("format {:?}", board.format)));
        }
        if board.notices.len() > MAX_NOTICES {
            return Err(NoticeError::Malformed(format!(
                "{} notices, over the {MAX_NOTICES} kept",
                board.notices.len()
            )));
        }
        Ok(board)
    }

    /// Reads a board for writing: one that cannot be used is set aside as
    /// `<name>.bad` and replaced by an empty one, so a damaged file never stops new
    /// notices. Returns whether that happened.
    pub fn load_or_reset(path: &Path) -> Result<(Self, bool), NoticeError> {
        match Self::load(path) {
            Ok(board) => Ok((board, false)),
            Err(error @ NoticeError::Io { .. }) => Err(error),
            Err(_) => {
                let aside = path.with_extension("bad");
                std::fs::rename(path, &aside).map_err(|e| io(path, e))?;
                Ok((Self::default(), true))
            }
        }
    }

    /// Reads, changes and writes the board under an exclusive lock on
    /// `<board>.lock`, so the follower posting, the owner acknowledging and the voice
    /// marking a notice said never undo one another. `change` returns whether it
    /// changed anything; the file is written only then (or when a damaged board was
    /// set aside). Returns what `change` returned and whether the board was reset.
    pub fn update(
        path: &Path,
        change: impl FnOnce(&mut Board) -> bool,
    ) -> Result<(bool, bool), NoticeError> {
        use fs2::FileExt;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| io(parent, e))?;
        }
        let lock_path = path.with_extension("lock");
        let mut options = std::fs::OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let lock = options.open(&lock_path).map_err(|e| io(&lock_path, e))?;
        lock.lock_exclusive().map_err(|e| io(&lock_path, e))?;
        let outcome = (|| {
            let (mut board, reset) = Self::load_or_reset(path)?;
            let changed = change(&mut board);
            if changed || reset {
                board.save(path)?;
            }
            Ok((changed, reset))
        })();
        let _ = FileExt::unlock(&lock);
        outcome
    }

    /// Writes the board atomically, readable by its owner only. Concurrent writers
    /// go through [`Board::update`].
    pub fn save(&self, path: &Path) -> Result<(), NoticeError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| io(parent, e))?;
        }
        let text = serde_json::to_string_pretty(self)
            .map_err(|e| NoticeError::Malformed(e.to_string()))?;
        static WRITES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = WRITES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let temporary = path.with_extension(format!("tmp.{}.{n}", std::process::id()));
        {
            use std::io::Write;
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create(true).truncate(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(&temporary).map_err(|e| io(&temporary, e))?;
            file.write_all(text.as_bytes())
                .and_then(|()| file.sync_all())
                .map_err(|e| io(&temporary, e))?;
        }
        std::fs::rename(&temporary, path).map_err(|e| io(path, e))
    }

    /// Posts a raised alert if it clears the policy and is not on the board already.
    /// Returns whether a notice was added.
    pub fn post(&mut self, raised: &Raised<'_>, policy: Policy) -> bool {
        if raised.severity < policy.min_severity {
            return false;
        }
        let id = notice_id(raised);
        if self.notices.iter().any(|n| n.id == id) {
            return false;
        }
        self.notices.push(Notice {
            id,
            at_ms: raised.at_ms,
            severity: raised.severity,
            rule: clip(raised.rule),
            device: clip(raised.device),
            detail: clip(raised.detail),
            spoken: spoken_for(raised.rule, raised.severity).into(),
            spoken_ms: None,
            acknowledged_ms: None,
        });
        self.evict();
        true
    }

    /// Keeps the board within [`MAX_NOTICES`]: acknowledged notices leave first, oldest
    /// first; pending ones leave only when there is nothing else to drop.
    fn evict(&mut self) {
        while self.notices.len() > MAX_NOTICES {
            let index = self.notices.iter().position(|n| !n.pending()).unwrap_or(0);
            self.notices.remove(index);
        }
    }

    /// Notices the owner has not acknowledged, oldest first.
    pub fn pending(&self) -> impl Iterator<Item = &Notice> {
        self.notices.iter().filter(|n| n.pending())
    }

    /// Pending notices the voice has not said yet, most serious first and then oldest.
    pub fn unspoken(&self) -> Vec<&Notice> {
        let mut out: Vec<&Notice> = self.pending().filter(|n| n.spoken_ms.is_none()).collect();
        out.sort_by_key(|n| (std::cmp::Reverse(n.severity), n.at_ms));
        out
    }

    /// Acknowledges one notice by id (or a unique prefix of at least 6 characters).
    /// Returns how many changed: 0 when none matched, was ambiguous, or was already
    /// acknowledged.
    pub fn acknowledge(&mut self, id: &str, now_ms: u64) -> usize {
        if id.len() < 6 {
            return 0;
        }
        let matches: Vec<usize> = self
            .notices
            .iter()
            .enumerate()
            .filter(|(_, n)| n.id.starts_with(id))
            .map(|(i, _)| i)
            .collect();
        match matches.as_slice() {
            [only] if self.notices[*only].pending() => {
                self.notices[*only].acknowledged_ms = Some(now_ms);
                1
            }
            _ => 0,
        }
    }

    /// Acknowledges every pending notice. Returns how many.
    pub fn acknowledge_all(&mut self, now_ms: u64) -> usize {
        let mut count = 0;
        for notice in self.notices.iter_mut().filter(|n| n.pending()) {
            notice.acknowledged_ms = Some(now_ms);
            count += 1;
        }
        count
    }

    /// Records that the voice said a notice. Returns whether it was found unspoken.
    pub fn mark_spoken(&mut self, id: &str, now_ms: u64) -> bool {
        match self
            .notices
            .iter_mut()
            .find(|n| n.id == id && n.spoken_ms.is_none())
        {
            Some(notice) => {
                notice.spoken_ms = Some(now_ms);
                true
            }
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raised<'a>(at_ms: u64, severity: Severity, rule: &'a str, detail: &'a str) -> Raised<'a> {
        Raised {
            at_ms,
            severity,
            rule,
            device: "de:ad:be:ef:00:02",
            detail,
        }
    }

    fn dir(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("archie-notices-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn below_the_floor_nothing_interrupts_and_the_same_alert_posts_once() {
        let mut board = Board::default();
        let policy = Policy::default();
        assert!(!board.post(
            &raised(1, Severity::Notice, "new-device", "first seen"),
            policy
        ));
        let spoof = raised(
            2,
            Severity::High,
            "address-moved",
            "192.168.1.1: claimed by another device",
        );
        assert!(board.post(&spoof, policy));
        assert!(!board.post(&spoof, policy), "a replay is the same notice");
        assert_eq!(board.pending().count(), 1);
    }

    #[test]
    fn what_is_said_aloud_names_nothing_the_network_chose() {
        let mut board = Board::default();
        let tunnel = raised(
            3,
            Severity::Warning,
            "dns-tunnel-suspect",
            "asked for aGVsbG8.secret-clinic.example under secret-clinic.example",
        );
        board.post(&tunnel, Policy::default());
        let notice = &board.notices[0];
        assert!(notice.detail.contains("secret-clinic"));
        assert!(!notice.spoken.contains("secret-clinic"));
        assert!(!notice.spoken.contains("de:ad"));
        assert_eq!(
            spoken_for("something-new", Severity::High),
            "Sentry: hay un aviso de la red; los detalles están en el Observatory."
        );
    }

    #[test]
    fn the_voice_takes_the_most_serious_first_and_each_once() {
        let mut board = Board::default();
        let policy = Policy {
            min_severity: Severity::Notice,
        };
        board.post(&raised(1, Severity::Notice, "new-device", "a"), policy);
        board.post(&raised(2, Severity::High, "address-moved", "b"), policy);
        board.post(&raised(3, Severity::Warning, "port-scan", "c"), policy);
        let order: Vec<&str> = board.unspoken().iter().map(|n| n.rule.as_str()).collect();
        assert_eq!(order, ["address-moved", "port-scan", "new-device"]);
        let first = board.unspoken()[0].id.clone();
        assert!(board.mark_spoken(&first, 10));
        assert!(!board.mark_spoken(&first, 11), "said once");
        assert_eq!(board.unspoken().len(), 2);
    }

    #[test]
    fn acknowledging_takes_an_unambiguous_prefix_and_ends_the_notice() {
        let mut board = Board::default();
        board.post(
            &raised(1, Severity::High, "address-moved", "x"),
            Policy::default(),
        );
        let id = board.notices[0].id.clone();
        assert_eq!(board.acknowledge(&id[..3], 5), 0, "too short to be sure");
        assert_eq!(board.acknowledge(&id[..8], 5), 1);
        assert_eq!(board.acknowledge(&id, 6), 0, "already acknowledged");
        assert_eq!(board.notices[0].acknowledged_ms, Some(5));
        assert!(
            board.unspoken().is_empty(),
            "an acknowledged notice is not said"
        );
    }

    #[test]
    fn a_full_board_lets_acknowledged_notices_go_before_pending_ones() {
        let mut board = Board::default();
        let policy = Policy::default();
        let details: Vec<String> = (0..=MAX_NOTICES).map(|i| format!("move {i}")).collect();
        board.post(
            &raised(0, Severity::High, "address-moved", &details[0]),
            policy,
        );
        board.acknowledge_all(1);
        for (i, detail) in details.iter().enumerate().skip(1) {
            board.post(
                &raised(i as u64, Severity::High, "address-moved", detail),
                policy,
            );
        }
        assert_eq!(board.notices.len(), MAX_NOTICES);
        assert!(board.notices.iter().all(Notice::pending));
        // Over the cap with only pending notices left: the oldest pending goes.
        board.post(
            &raised(999, Severity::High, "address-moved", "one more"),
            policy,
        );
        assert_eq!(board.notices.len(), MAX_NOTICES);
        assert_eq!(board.notices[0].detail, "move 2");
    }

    #[test]
    fn the_board_survives_a_restart_and_a_damaged_one_is_set_aside() {
        let dir = dir("restart");
        let path = dir.join("notices.json");
        let mut board = Board::default();
        board.post(
            &raised(7, Severity::High, "port-scan", "100 ports"),
            Policy::default(),
        );
        board.save(&path).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        assert_eq!(Board::load(&path).unwrap(), board);

        std::fs::write(&path, b"{not json").unwrap();
        assert!(matches!(Board::load(&path), Err(NoticeError::Malformed(_))));
        let (fresh, reset) = Board::load_or_reset(&path).unwrap();
        assert!(reset && fresh.notices.is_empty());
        assert!(dir.join("notices.bad").exists());

        let missing = dir.join("absent.json");
        assert_eq!(Board::load(&missing).unwrap(), Board::default());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn concurrent_updates_lose_nothing() {
        let dir = dir("concurrent");
        let path = dir.join("notices.json");
        let threads: Vec<_> = (0..8u64)
            .map(|t| {
                let path = path.clone();
                std::thread::spawn(move || {
                    for i in 0..10u64 {
                        let detail = format!("thread {t} alert {i}");
                        Board::update(&path, |board| {
                            board.post(
                                &raised(t * 100 + i, Severity::High, "port-scan", &detail),
                                Policy::default(),
                            )
                        })
                        .unwrap();
                    }
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }
        assert_eq!(Board::load(&path).unwrap().notices.len(), 80);
        let (changed, _) = Board::update(&path, |board| board.acknowledge_all(1) > 0).unwrap();
        assert!(changed);
        assert_eq!(Board::load(&path).unwrap().pending().count(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_board_of_another_format_or_too_many_notices_is_refused() {
        let other = r#"{"format":"something-else","notices":[]}"#;
        assert!(matches!(
            Board::parse(other),
            Err(NoticeError::Malformed(_))
        ));
        let mut board = Board::default();
        let policy = Policy::default();
        for i in 0..MAX_NOTICES {
            board.post(&raised(i as u64, Severity::High, "port-scan", "x"), policy);
        }
        let mut text: serde_json::Value = serde_json::to_value(&board).unwrap();
        let extra = text["notices"][0].clone();
        text["notices"].as_array_mut().unwrap().push(extra);
        assert!(Board::parse(&text.to_string()).is_err());
    }
}
