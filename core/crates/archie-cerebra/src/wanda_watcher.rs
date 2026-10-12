//! The live caller of [`crate::wanda_bridge`]: it watches a directory of chat
//! exports and feeds every new one through the bridge.
//!
//! The bridge parses WhatsApp `.txt` exports, Instagram `messages_*.json` files
//! and notification-dump `.jsonl` files, and nothing fed it. This watcher is that
//! caller: it scans a configured directory, classifies each file by its extension
//! (the three shapes the bridge parses, nothing else), parses it with the bridge's
//! own parsers, and records the messages with [`record_chat_context`] — validated,
//! deduplicated by content-derived reference, sealed at `Restricted`.
//!
//! # Idempotency
//!
//! A journal remembers the content hash of every file already ingested, so a
//! restart does not re-ingest, and re-exporting the same chat under a new name
//! ingests nothing twice. A file whose content changed is ingested again; the
//! messages it shares with the earlier export land on the same vault references
//! and are stored once — the bridge's dedup is the second layer, this journal the
//! first. The journal is bounded and atomic, in the spirit of the netwatch
//! `told.json` it sits next to conceptually.
//!
//! # Privacy stance
//!
//! Everything the bridge guarantees holds here: sender phone numbers are hashed
//! with the operator-configured salt at parse time, raw text below the salience
//! threshold never reaches the vault, and this module logs counts, never content.
//! The salt comes from local configuration (`[sentry.chat_exports]`); it is a
//! pseudonym key, not a defence — see the bridge's module note.

use crate::vault_io::{self, VaultIoError};
use crate::wanda_bridge::{
    parse_instagram_export, parse_notification_dump, parse_whatsapp_export, record_chat_context,
    ChatMessage, ParseError, WandaError, MAX_MESSAGES_PER_BATCH,
};
use archie_vault::ContextDomain;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, VecDeque};
use std::path::{Path, PathBuf};

/// The largest export file the watcher reads. WhatsApp exports of long chats fit
/// comfortably; anything bigger is reported, not truncated mid-message.
pub const MAX_EXPORT_BYTES: u64 = 16 * 1024 * 1024;
/// The most ingested files the journal remembers.
const MAX_JOURNAL_ENTRIES: usize = 8192;
/// The most failed file names one report carries.
const MAX_FAILURE_NAMES: usize = 64;
const JOURNAL_FORMAT: &str = "archie-cerebra-wanda-watcher-v1";

/// Why a watch pass failed. A file that does not parse is a count on
/// [`WatchReport`], never an error here — one bad export must not stop the rest.
#[derive(Debug, thiserror::Error)]
pub enum WatcherError {
    #[error("the watch directory could not be read: {0}")]
    Io(String),
    #[error("an export could not be parsed: {0}")]
    Parse(String),
    #[error("the vault directory failed: {0}")]
    VaultIo(#[from] VaultIoError),
    #[error("a message batch was refused: {0}")]
    Bridge(#[from] WandaError),
}

/// Which bridge parser a file's extension selects. Anything else is ignored:
/// the watch directory is a drop folder, and unrelated files in it are not data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportKind {
    Whatsapp,
    Instagram,
    Notifications,
}

impl ExportKind {
    fn of(path: &Path) -> Option<Self> {
        match path.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()) {
            Some(ext) if ext == "txt" => Some(Self::Whatsapp),
            Some(ext) if ext == "json" => Some(Self::Instagram),
            Some(ext) if ext == "jsonl" => Some(Self::Notifications),
            _ => None,
        }
    }
}

/// What one watch pass (or one manual ingest) did. Counts are exact; failed
/// file names are carried so the operator can look, bounded so a broken drop
/// folder cannot grow the report without limit.
#[derive(Debug, Default)]
pub struct WatchReport {
    /// Export files seen with an extension the bridge parses.
    pub files_seen: u64,
    /// Files parsed and recorded this pass.
    pub files_ingested: u64,
    /// Files skipped because the journal already holds their content hash.
    pub files_already_known: u64,
    /// Messages recorded across all ingested files.
    pub messages_recorded: u64,
    /// Chat aggregates recorded across all ingested files.
    pub chats_recorded: u64,
    /// Files that failed to parse, as `name: reason`, bounded.
    pub failures: Vec<String>,
}

/// The journal: content hashes of ingested files, newest last, bounded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct WatchJournal {
    format: String,
    seen: VecDeque<String>,
}

impl Default for WatchJournal {
    fn default() -> Self {
        Self {
            format: JOURNAL_FORMAT.to_string(),
            seen: VecDeque::new(),
        }
    }
}

impl WatchJournal {
    fn load(path: &Path) -> Self {
        match std::fs::read(path) {
            Ok(bytes) if bytes.len() as u64 <= 4 * 1024 * 1024 => {
                serde_json::from_slice::<Self>(&bytes)
                    .ok()
                    .filter(|journal| journal.format == JOURNAL_FORMAT)
                    .map(|mut journal| {
                        journal.seen.truncate(MAX_JOURNAL_ENTRIES);
                        journal
                    })
                    .unwrap_or_default()
            }
            _ => Self::default(),
        }
    }

    fn save(&self, path: &Path) -> Result<(), WatcherError> {
        let partial = path.with_extension("partial");
        let bytes = serde_json::to_vec(self).expect("a journal serializes");
        std::fs::write(&partial, &bytes)
            .and_then(|()| std::fs::rename(&partial, path))
            .map_err(|e| WatcherError::Io(format!("{}: {e}", path.display())))
    }
}

/// The same dual-FNV content id both bridges use: a dedup key, stable across
/// compiler versions, never trusted against a collision the vault compares in
/// full.
fn content_id(bytes: &[u8]) -> String {
    const fn fnv1a(bytes: &[u8], basis: u64) -> u64 {
        let mut hash = basis;
        let mut index = 0;
        while index < bytes.len() {
            hash ^= bytes[index] as u64;
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
            index += 1;
        }
        hash
    }
    let first = fnv1a(bytes, 0xcbf2_9ce4_8422_2325);
    let second = fnv1a(bytes, 0x9e37_79b9_7f4a_7c15);
    format!("{first:016x}{second:016x}")
}

/// The chat id a WhatsApp export does not carry: the file's own name, bounded
/// and free of control characters so the bridge's validation accepts it.
fn whatsapp_chat_id(path: &Path) -> String {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("whatsapp-chat");
    let cleaned: String = stem
        .chars()
        .filter(|c| !c.is_control())
        .take(64)
        .collect();
    let cleaned = cleaned.trim();
    if cleaned.is_empty() {
        "whatsapp-chat".to_string()
    } else {
        cleaned.to_string()
    }
}

/// Parse one export file into messages with the parser its extension selects.
fn parse_export(kind: ExportKind, path: &Path, text: &str, salt: &str) -> Result<Vec<ChatMessage>, ParseError> {
    match kind {
        ExportKind::Whatsapp => parse_whatsapp_export(&whatsapp_chat_id(path), text, salt),
        ExportKind::Instagram => parse_instagram_export(text, salt),
        ExportKind::Notifications => parse_notification_dump(text, salt),
    }
    .map(|outcome| outcome.messages)
}

/// Ingest one export file: parse, then record in batches of the bridge's own
/// batch cap. Returns the messages and chats recorded. Vault writes persist
/// after the whole file records, so a failure mid-file leaves the journal
/// untouched and the file is retried whole next time — vault references dedupe
/// whatever the first attempt did store.
fn ingest(
    vault: &mut archie_vault::Vault,
    domain: &ContextDomain,
    task_id: &str,
    kind: ExportKind,
    path: &Path,
    salt: &str,
) -> Result<(u64, u64), WatcherError> {
    let bytes = std::fs::read(path)
        .map_err(|e| WatcherError::Io(format!("{}: {e}", path.display())))?;
    if bytes.is_empty() || bytes.len() as u64 > MAX_EXPORT_BYTES {
        return Err(WatcherError::Io(format!(
            "{}: empty or over the {}-byte export cap",
            path.display(),
            MAX_EXPORT_BYTES
        )));
    }
    let text = String::from_utf8_lossy(&bytes);
    let messages = parse_export(kind, path, &text, salt)
        .map_err(|e| WatcherError::Parse(format!("{}: {e}", path.display())))?;
    let mut recorded = (0u64, 0u64);
    for batch in messages.chunks(MAX_MESSAGES_PER_BATCH) {
        if let Some(batch) = record_chat_context(vault, domain, task_id, batch)?.batch() {
            recorded.0 += batch.messages().len() as u64;
            recorded.1 += batch.chats().len() as u64;
        }
    }
    Ok(recorded)
}

/// Ingest one export file by path, journal entry included. This is the manual
/// path (`archie wanda-ingest`) and the unit the watch loop iterates over.
pub fn ingest_export_file(
    vault_dir: &Path,
    journal_path: &Path,
    file: &Path,
    salt: &str,
    task_id: &str,
) -> Result<WatchReport, WatcherError> {
    let mut report = WatchReport::default();
    let Some(kind) = ExportKind::of(file) else {
        report
            .failures
            .push(format!("{}: not a .txt, .json or .jsonl export", file.display()));
        return Ok(report);
    };
    report.files_seen = 1;
    let bytes = std::fs::read(file)
        .map_err(|e| WatcherError::Io(format!("{}: {e}", file.display())))?;
    let hash = content_id(&bytes);
    let mut journal = WatchJournal::load(journal_path);
    if journal.seen.contains(&hash) {
        report.files_already_known = 1;
        return Ok(report);
    }
    let mut vault = vault_io::open_vault(vault_dir)?;
    let domain = ContextDomain::personal("owner")
        .map_err(|e| WatcherError::Io(format!("the wanda domain is fixed and must parse: {e}")))?;
    match ingest(&mut vault, &domain, task_id, kind, file, salt) {
        Ok((messages, chats)) => {
            report.files_ingested = 1;
            report.messages_recorded = messages;
            report.chats_recorded = chats;
            vault_io::persist_vault(vault_dir, &vault)?;
            journal.seen.push_back(hash);
            while journal.seen.len() > MAX_JOURNAL_ENTRIES {
                journal.seen.pop_front();
            }
            journal.save(journal_path)?;
        }
        Err(error) => {
            if report.failures.len() < MAX_FAILURE_NAMES {
                report.failures.push(error.to_string());
            }
        }
    }
    Ok(report)
}

/// One pass over the watch directory: every export whose content the journal
/// does not know is ingested. Safe in a poll loop; an unchanged directory is a
/// no-op.
pub fn watch_exports_once(
    vault_dir: &Path,
    journal_path: &Path,
    watch_dir: &Path,
    salt: &str,
    task_id: &str,
) -> Result<WatchReport, WatcherError> {
    let mut report = WatchReport::default();
    let mut entries: Vec<PathBuf> = Vec::new();
    let read_dir = std::fs::read_dir(watch_dir)
        .map_err(|e| WatcherError::Io(format!("{}: {e}", watch_dir.display())))?;
    for entry in read_dir {
        let entry = entry.map_err(|e| WatcherError::Io(format!("{}: {e}", watch_dir.display())))?;
        let path = entry.path();
        if path.is_file() && ExportKind::of(&path).is_some() {
            entries.push(path);
        }
    }
    entries.sort();
    let mut journal = WatchJournal::load(journal_path);
    let known: BTreeSet<String> = journal.seen.iter().cloned().collect();
    let mut vault = None;
    let domain = ContextDomain::personal("owner")
        .map_err(|e| WatcherError::Io(format!("the wanda domain is fixed and must parse: {e}")))?;
    let mut dirty = false;
    for path in entries {
        report.files_seen += 1;
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(e) => {
                if report.failures.len() < MAX_FAILURE_NAMES {
                    report.failures.push(format!("{}: {e}", path.display()));
                }
                continue;
            }
        };
        let hash = content_id(&bytes);
        if known.contains(&hash) {
            report.files_already_known += 1;
            continue;
        }
        if vault.is_none() {
            vault = Some(vault_io::open_vault(vault_dir)?);
        }
        let open = vault.as_mut().expect("opened above");
        let kind = ExportKind::of(&path).expect("filtered above");
        match ingest(open, &domain, task_id, kind, &path, salt) {
            Ok((messages, chats)) => {
                report.files_ingested += 1;
                report.messages_recorded += messages;
                report.chats_recorded += chats;
                journal.seen.push_back(hash);
                while journal.seen.len() > MAX_JOURNAL_ENTRIES {
                    journal.seen.pop_front();
                }
                dirty = true;
            }
            Err(error) => {
                if report.failures.len() < MAX_FAILURE_NAMES {
                    report.failures.push(error.to_string());
                }
            }
        }
    }
    if let Some(vault) = vault.filter(|_| dirty) {
        vault_io::persist_vault(vault_dir, &vault)?;
    }
    if dirty {
        journal.save(journal_path)?;
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every export in these tests is a fixture written for the test: fixture
    /// names, a fixture phone number, fixture text. No real chat took part.
    const SALT: &str = "wanda-watcher-fixture-salt";

    struct Fixture {
        dir: PathBuf,
        vault_dir: PathBuf,
        journal: PathBuf,
        watch: PathBuf,
    }

    impl Fixture {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "archie-cerebra-watcher-{name}-{}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            let watch = dir.join("exports");
            std::fs::create_dir_all(&watch).unwrap();
            Self {
                vault_dir: dir.join("vault"),
                journal: dir.join("wanda-journal.json"),
                watch,
                dir,
            }
        }

        fn vault_entries(&self) -> usize {
            vault_io::open_vault(&self.vault_dir)
                .unwrap()
                .sealed_entries()
                .len()
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn whatsapp_export() -> &'static str {
        "[10/05/2026, 14:32:01] Alice: hola, ¿quedamos mañana?\n[10/05/2026, 14:32:40] Bob: sí, nos vemos a las 8\n"
    }

    fn instagram_export() -> &'static str {
        "{\"participants\":[{\"name\":\"Alice\"},{\"name\":\"Bob\"}],\"messages\":[{\"sender_name\":\"Alice\",\"timestamp_ms\":1746950000000,\"content\":\"viste el vídeo?\"}]}"
    }

    fn notification_dump() -> &'static str {
        "{\"package\":\"com.whatsapp\",\"title\":\"Alice\",\"text\":\"urgente: llámame\",\"posted_at_ms\":1746950000000}\n"
    }

    #[test]
    fn every_export_kind_is_ingested_once_across_restarts() {
        let fixture = Fixture::new("idempotent");
        std::fs::write(fixture.watch.join("chat.txt"), whatsapp_export()).unwrap();
        std::fs::write(fixture.watch.join("messages_1.json"), instagram_export()).unwrap();
        std::fs::write(fixture.watch.join("notifications.jsonl"), notification_dump()).unwrap();

        let first = watch_exports_once(
            &fixture.vault_dir,
            &fixture.journal,
            &fixture.watch,
            SALT,
            "wanda-watch",
        )
        .unwrap();
        assert_eq!(first.files_seen, 3);
        assert_eq!(first.files_ingested, 3);
        assert_eq!(first.failures.len(), 0, "failures: {:?}", first.failures);
        assert_eq!(first.messages_recorded, 4);
        let entries = fixture.vault_entries();
        assert!(entries >= 4, "messages plus chat aggregates are sealed: {entries}");

        // A "restart": the journal remembers; nothing is ingested twice.
        let second = watch_exports_once(
            &fixture.vault_dir,
            &fixture.journal,
            &fixture.watch,
            SALT,
            "wanda-watch",
        )
        .unwrap();
        assert_eq!(second.files_seen, 3);
        assert_eq!(second.files_ingested, 0);
        assert_eq!(second.files_already_known, 3);
        assert_eq!(fixture.vault_entries(), entries);
    }

    #[test]
    fn a_changed_file_reingests_and_the_vault_dedupes_what_it_shares() {
        let fixture = Fixture::new("changed");
        std::fs::write(fixture.watch.join("chat.txt"), whatsapp_export()).unwrap();
        watch_exports_once(&fixture.vault_dir, &fixture.journal, &fixture.watch, SALT, "wanda-watch").unwrap();
        let entries = fixture.vault_entries();

        // The export grows by one message; its hash changes, so it re-ingests.
        let grown = format!("{}[10/05/2026, 14:33:02] Alice: trae dinero para la cena\n", whatsapp_export());
        std::fs::write(fixture.watch.join("chat.txt"), grown).unwrap();
        let report = watch_exports_once(&fixture.vault_dir, &fixture.journal, &fixture.watch, SALT, "wanda-watch").unwrap();
        assert_eq!(report.files_ingested, 1);
        // The two replayed messages land on their existing references; exactly
        // one new message record (plus one new chat aggregate) appears.
        assert_eq!(fixture.vault_entries(), entries + 2);
    }

    #[test]
    fn a_manual_ingest_is_idempotent_and_reports_unsupported_files() {
        let fixture = Fixture::new("manual");
        let file = fixture.watch.join("chat.txt");
        std::fs::write(&file, whatsapp_export()).unwrap();

        let first = ingest_export_file(&fixture.vault_dir, &fixture.journal, &file, SALT, "wanda-manual").unwrap();
        assert_eq!(first.files_ingested, 1);
        assert_eq!(first.messages_recorded, 2);

        let second = ingest_export_file(&fixture.vault_dir, &fixture.journal, &file, SALT, "wanda-manual").unwrap();
        assert_eq!(second.files_ingested, 0);
        assert_eq!(second.files_already_known, 1);

        let odd = fixture.watch.join("photo.png");
        std::fs::write(&odd, b"not an export").unwrap();
        let third = ingest_export_file(&fixture.vault_dir, &fixture.journal, &odd, SALT, "wanda-manual").unwrap();
        assert_eq!(third.files_ingested, 0);
        assert_eq!(third.failures.len(), 1);
    }

    #[test]
    fn a_broken_export_is_counted_and_does_not_stop_the_rest() {
        let fixture = Fixture::new("broken");
        std::fs::write(fixture.watch.join("empty.txt"), "not a chat export at all\n").unwrap();
        std::fs::write(fixture.watch.join("good.txt"), whatsapp_export()).unwrap();
        let report = watch_exports_once(&fixture.vault_dir, &fixture.journal, &fixture.watch, SALT, "wanda-watch").unwrap();
        assert_eq!(report.files_seen, 2);
        assert_eq!(report.files_ingested, 1);
        assert_eq!(report.failures.len(), 1);
        assert!(fixture.vault_entries() > 0);
    }
}
