//! The Vault pillar's producer: documents the owner puts under Archie's care.
//!
//! Sentry gives Cerebra what happened on the network and Wanda what the owner's chats
//! are about. The Vault pillar is the durable evidence of a piece of work: the brief,
//! its restrictions, its versions. Nothing produced it; this module does, from plain
//! text and Markdown files the owner drops in a directory.
//!
//! Each document is sealed whole (up to [`MAX_DOCUMENT_BYTES`]) as
//! [`EntryClass::Document`] at `Restricted`, under a reference derived from its name
//! and content, and included in a [`PillarContext`] as [`Pillar::Vault`]. Its kind
//! (brief, restriction, version, note) comes from an explicit `kind:` line in a
//! front-matter block, else from words in the file name, else it is a note: never
//! from reading the prose and guessing.
//!
//! [`watch_documents_once`] is the live caller, in the shape of
//! [`crate::wanda_watcher`]: a journal of content hashes makes it idempotent across
//! restarts, and one bad file is a count on the report, never a stop.
//!
//! Domains are explicit. The same file watched into `personal/owner` and into
//! `work/acme/bridge` lands under two references; nothing here moves a document from
//! one domain to another.

use crate::pillars::{Pillar, PillarContext, PillarError};
use crate::vault_io::{self, VaultIoError};
use archie_types::PrivacyGrade;
use archie_vault::{ContextDomain, EntryClass, Vault, VaultError, VaultRef};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, VecDeque};
use std::path::{Path, PathBuf};

/// The largest document sealed. Larger files are reported, never truncated.
pub const MAX_DOCUMENT_BYTES: usize = 256 * 1024;
/// Format marker of a sealed document.
pub const DOCUMENT_FORMAT: &str = "archie-vault-document-v1";
const ACTOR: &str = "vault-bridge";
const MAX_NAME_CHARS: usize = 128;
const MAX_JOURNAL_ENTRIES: usize = 8192;
const MAX_FAILURE_NAMES: usize = 64;
const JOURNAL_FORMAT: &str = "archie-cerebra-vault-watcher-v1";

/// What a document is to the work it belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DocumentKind {
    /// What is asked: the brief, the commission, the scope.
    Brief,
    /// What must not be done or must hold: constraints, rules, norms.
    Restriction,
    /// A version or a record of changes.
    Version,
    /// Anything else the owner kept.
    Note,
}

impl DocumentKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Brief => "brief",
            Self::Restriction => "restriction",
            Self::Version => "version",
            Self::Note => "note",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "brief" | "encargo" => Some(Self::Brief),
            "restriction" | "restriccion" | "restricción" | "constraint" => {
                Some(Self::Restriction)
            }
            "version" | "versión" | "changelog" => Some(Self::Version),
            "note" | "nota" => Some(Self::Note),
            _ => None,
        }
    }

    /// How much a document of this kind weighs in a focus plan (0–100).
    pub fn weight(self) -> u8 {
        match self {
            Self::Brief => 90,
            Self::Restriction => 80,
            Self::Version => 60,
            Self::Note => 40,
        }
    }
}

/// The kind a document declares (`kind:` in a leading `---` block), else the one its
/// file name suggests, else [`DocumentKind::Note`].
pub fn kind_of(name: &str, text: &str) -> DocumentKind {
    let mut lines = text.lines();
    if lines.next().map(str::trim) == Some("---") {
        for line in lines.take(32) {
            if line.trim() == "---" {
                break;
            }
            if let Some(kind) = line
                .split_once(':')
                .filter(|(key, _)| key.trim().eq_ignore_ascii_case("kind"))
                .and_then(|(_, value)| DocumentKind::parse(value))
            {
                return kind;
            }
        }
    }
    let lower = name.to_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| lower.contains(w));
    if has(&["brief", "encargo", "alcance", "scope"]) {
        DocumentKind::Brief
    } else if has(&["restric", "constraint", "norma", "rules", "reglas"]) {
        DocumentKind::Restriction
    } else if has(&["version", "versión", "changelog", "cambios"]) {
        DocumentKind::Version
    } else {
        DocumentKind::Note
    }
}

/// One document as read from disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentInput {
    /// The file name, without directories.
    pub name: String,
    pub text: String,
    /// The file's own modification time, when it has one.
    pub modified_ms: Option<u64>,
}

/// A document as it is sealed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DocumentContent {
    pub format: String,
    pub name: String,
    pub kind: DocumentKind,
    pub modified_ms: Option<u64>,
    pub bytes: usize,
    pub text: String,
}

/// What one recorded document became.
#[derive(Debug)]
pub struct DocumentRecord {
    pub reference: VaultRef,
    pub kind: DocumentKind,
    pub context: PillarContext,
}

#[derive(Debug, thiserror::Error)]
pub enum VaultBridgeError {
    #[error("a document name must be 1..={MAX_NAME_CHARS} printable characters")]
    Name,
    #[error("a document is {bytes} bytes, over the {MAX_DOCUMENT_BYTES}-byte cap")]
    TooLarge { bytes: usize },
    #[error("a document must be text without NUL bytes")]
    NotText,
    #[error("the domain cannot hold this document's reference")]
    Domain,
    #[error("another record already sits at {reference:?}")]
    ReferenceCollision { reference: VaultRef },
    #[error(transparent)]
    Pillar(#[from] PillarError),
    #[error(transparent)]
    Vault(#[from] VaultError),
}

/// The same dual-FNV content id the other bridges use: a dedup key, never trusted
/// against a collision, which the vault compares in full.
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

/// Seal one document in `domain` and include it in a context as [`Pillar::Vault`].
///
/// The reference is derived from the name and the text: the same file recorded again
/// is the same entry, stored once; a renamed or edited one is a new entry, and the
/// older version stays as evidence. The entry is sealed at the file's own time, or at
/// `0`, the explicit unknown: this bridge has no clock.
pub fn record_document(
    vault: &mut Vault,
    domain: &ContextDomain,
    task_id: &str,
    document: &DocumentInput,
) -> Result<DocumentRecord, VaultBridgeError> {
    let name = document.name.trim();
    if name.is_empty()
        || name.chars().count() > MAX_NAME_CHARS
        || name
            .chars()
            .any(|c| c.is_control() || c == '/' || c == '\\')
    {
        return Err(VaultBridgeError::Name);
    }
    if document.text.len() > MAX_DOCUMENT_BYTES {
        return Err(VaultBridgeError::TooLarge {
            bytes: document.text.len(),
        });
    }
    if document.text.contains('\0') {
        return Err(VaultBridgeError::NotText);
    }
    let kind = kind_of(name, &document.text);
    let content = DocumentContent {
        format: DOCUMENT_FORMAT.into(),
        name: name.into(),
        kind,
        modified_ms: document.modified_ms,
        bytes: document.text.len(),
        text: document.text.clone(),
    };
    let json = serde_json::to_string(&content).map_err(|_| VaultBridgeError::NotText)?;
    let id = content_id(format!("{name}\0{}", document.text).as_bytes());
    let reference = domain
        .reference(&format!("vault/documents/{id}"))
        .map_err(|_| VaultBridgeError::Domain)?;
    let at_ms = document
        .modified_ms
        .and_then(|ms| i64::try_from(ms).ok())
        .unwrap_or(0);
    if vault.contains(&reference) {
        let existing = vault
            .disclose(ACTOR, &reference, PrivacyGrade::Restricted, at_ms)
            .ok()
            .and_then(|d| d.text().map(str::to_owned));
        if existing.as_deref() != Some(json.as_str()) {
            return Err(VaultBridgeError::ReferenceCollision { reference });
        }
    } else {
        vault.store(
            ACTOR,
            reference.clone(),
            EntryClass::Document,
            PrivacyGrade::Restricted,
            &json,
            at_ms,
            None,
        )?;
    }
    let mut context = PillarContext::new(domain.clone(), task_id)?;
    context.include(Pillar::Vault, reference.clone())?;
    Ok(DocumentRecord {
        reference,
        kind,
        context,
    })
}

/// Whether a path is a document the watcher reads: `.md`, `.markdown` or `.txt`, not
/// hidden.
pub fn is_document(path: &Path) -> bool {
    let hidden = path
        .file_name()
        .and_then(|n| n.to_str())
        .is_none_or(|n| n.starts_with('.'));
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    !hidden && matches!(extension.as_deref(), Some("md" | "markdown" | "txt"))
}

/// Read one document file, within the caps, as the bridge takes it.
pub fn read_document(path: &Path) -> Result<DocumentInput, String> {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| format!("{}: the name is not UTF-8", path.display()))?
        .to_string();
    let meta = std::fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if meta.len() > MAX_DOCUMENT_BYTES as u64 {
        return Err(format!(
            "{name}: {} bytes, over the {MAX_DOCUMENT_BYTES}-byte cap",
            meta.len()
        ));
    }
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let text = String::from_utf8(bytes).map_err(|_| format!("{name}: not UTF-8 text"))?;
    let modified_ms = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .and_then(|d| u64::try_from(d.as_millis()).ok());
    Ok(DocumentInput {
        name,
        text,
        modified_ms,
    })
}

/// What one watch pass did. Counts are exact; failures are named, bounded.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize)]
pub struct DocumentWatchReport {
    pub files_seen: u64,
    pub files_already_known: u64,
    pub documents_recorded: u64,
    pub briefs: u64,
    pub restrictions: u64,
    pub versions: u64,
    pub notes: u64,
    pub failures: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum DocumentWatchError {
    #[error("the document directory could not be read: {0}")]
    Io(String),
    #[error("the vault directory failed: {0}")]
    VaultIo(#[from] VaultIoError),
}

#[derive(Debug, Serialize, Deserialize)]
struct Journal {
    format: String,
    seen: VecDeque<String>,
}

impl Journal {
    fn load(path: &Path) -> Self {
        let fresh = || Self {
            format: JOURNAL_FORMAT.into(),
            seen: VecDeque::new(),
        };
        match std::fs::read(path) {
            Ok(bytes) if bytes.len() as u64 <= 4 * 1024 * 1024 => {
                serde_json::from_slice::<Self>(&bytes)
                    .ok()
                    .filter(|j| j.format == JOURNAL_FORMAT)
                    .map(|mut j| {
                        j.seen.truncate(MAX_JOURNAL_ENTRIES);
                        j
                    })
                    .unwrap_or_else(fresh)
            }
            _ => fresh(),
        }
    }

    fn save(&self, path: &Path) -> Result<(), DocumentWatchError> {
        let partial = path.with_extension("partial");
        let bytes = serde_json::to_vec(self).map_err(|e| DocumentWatchError::Io(e.to_string()))?;
        std::fs::write(&partial, &bytes)
            .and_then(|()| std::fs::rename(&partial, path))
            .map_err(|e| DocumentWatchError::Io(format!("{}: {e}", path.display())))
    }
}

/// One pass over a document directory: every document not seen before (by name and
/// content) is sealed into the vault in `domain`. The vault is persisted before the
/// journal, so a crash between them re-reads files the vault then dedupes.
pub fn watch_documents_once(
    vault_dir: &Path,
    journal_path: &Path,
    watch_dir: &Path,
    domain: &ContextDomain,
    task_id: &str,
) -> Result<DocumentWatchReport, DocumentWatchError> {
    let mut report = DocumentWatchReport::default();
    let mut paths: Vec<PathBuf> = std::fs::read_dir(watch_dir)
        .map_err(|e| DocumentWatchError::Io(format!("{}: {e}", watch_dir.display())))?
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| path.is_file() && is_document(path))
        .collect();
    paths.sort();
    let mut journal = Journal::load(journal_path);
    let known: BTreeSet<String> = journal.seen.iter().cloned().collect();
    let mut vault: Option<vault_io::VaultGuard> = None;
    let fail = |report: &mut DocumentWatchReport, message: String| {
        if report.failures.len() < MAX_FAILURE_NAMES {
            report.failures.push(message);
        }
    };
    for path in paths {
        report.files_seen += 1;
        let document = match read_document(&path) {
            Ok(document) => document,
            Err(message) => {
                fail(&mut report, message);
                continue;
            }
        };
        let key = content_id(
            format!("{}\0{}\0{}", domain.path(), document.name, document.text).as_bytes(),
        );
        if known.contains(&key) {
            report.files_already_known += 1;
            continue;
        }
        let open = match vault.as_mut() {
            Some(open) => open,
            None => vault.insert(vault_io::open_vault(vault_dir)?),
        };
        match record_document(open, domain, task_id, &document) {
            Ok(record) => {
                report.documents_recorded += 1;
                match record.kind {
                    DocumentKind::Brief => report.briefs += 1,
                    DocumentKind::Restriction => report.restrictions += 1,
                    DocumentKind::Version => report.versions += 1,
                    DocumentKind::Note => report.notes += 1,
                }
                journal.seen.push_back(key);
                while journal.seen.len() > MAX_JOURNAL_ENTRIES {
                    journal.seen.pop_front();
                }
            }
            Err(error) => fail(&mut report, format!("{}: {error}", document.name)),
        }
    }
    if report.documents_recorded > 0 {
        if let Some(vault) = &vault {
            vault_io::persist_vault(vault_dir, vault)?;
        }
        journal.save(journal_path)?;
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("archie-vault-bridge-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn document(name: &str, text: &str) -> DocumentInput {
        DocumentInput {
            name: name.into(),
            text: text.into(),
            modified_ms: Some(1_791_749_339_000),
        }
    }

    #[test]
    fn the_kind_is_declared_or_named_never_guessed_from_prose() {
        let declared = "---\ntitle: Puente\nkind: restricción\n---\nEl brief dice otra cosa.";
        assert_eq!(kind_of("notas.md", declared), DocumentKind::Restriction);
        assert_eq!(kind_of("Encargo-puente.md", "texto"), DocumentKind::Brief);
        assert_eq!(kind_of("CHANGELOG.md", "texto"), DocumentKind::Version);
        assert_eq!(
            kind_of("normas-obra.txt", "texto"),
            DocumentKind::Restriction
        );
        // Prose that mentions a brief does not make the file one.
        assert_eq!(kind_of("ideas.md", "this is the brief"), DocumentKind::Note);
        // An unknown declared kind falls back to the name.
        assert_eq!(
            kind_of("brief.md", "---\nkind: poem\n---\n"),
            DocumentKind::Brief
        );
    }

    #[test]
    fn a_document_is_sealed_once_as_a_restricted_vault_pillar_reference() {
        let mut vault = Vault::open(&archie_vault::VaultKey::from_bytes([7; 32])).unwrap();
        let domain = ContextDomain::work("acme", "bridge").unwrap();
        let brief = document("brief.md", "Diseñar el puente peatonal.");
        let first = record_document(&mut vault, &domain, "work-1", &brief).unwrap();
        assert_eq!(first.kind, DocumentKind::Brief);
        assert!(domain.contains(&first.reference));
        let entries = vault.sealed_entries();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].class, EntryClass::Document);
        assert_eq!(entries[0].grade, PrivacyGrade::Restricted);
        assert_eq!(entries[0].created_at_ms, 1_791_749_339_000);
        // Recorded again: the same entry, not a second one.
        let again = record_document(&mut vault, &domain, "work-1", &brief).unwrap();
        assert_eq!(again.reference, first.reference);
        assert_eq!(vault.sealed_entries().len(), 1);
        // An edit is a new entry; the old version stays as evidence.
        let edited = document("brief.md", "Diseñar el puente peatonal y la rampa.");
        let second = record_document(&mut vault, &domain, "work-1", &edited).unwrap();
        assert_ne!(second.reference, first.reference);
        assert_eq!(vault.sealed_entries().len(), 2);
        // The same document in the personal domain is another reference entirely.
        let personal = ContextDomain::personal("owner").unwrap();
        let mine = record_document(&mut vault, &personal, "home", &brief).unwrap();
        assert!(personal.contains(&mine.reference) && !domain.contains(&mine.reference));
    }

    #[test]
    fn oversize_binary_and_badly_named_documents_are_refused() {
        let mut vault = Vault::open(&archie_vault::VaultKey::from_bytes([7; 32])).unwrap();
        let domain = ContextDomain::personal("owner").unwrap();
        let big = document("big.md", &"x".repeat(MAX_DOCUMENT_BYTES + 1));
        assert!(matches!(
            record_document(&mut vault, &domain, "t", &big),
            Err(VaultBridgeError::TooLarge { .. })
        ));
        assert!(matches!(
            record_document(&mut vault, &domain, "t", &document("a.txt", "a\0b")),
            Err(VaultBridgeError::NotText)
        ));
        for name in ["", "../escape.md", "a\nb.md"] {
            assert!(matches!(
                record_document(&mut vault, &domain, "t", &document(name, "x")),
                Err(VaultBridgeError::Name)
            ));
        }
        assert!(vault.sealed_entries().is_empty());
    }

    #[test]
    fn the_watcher_seals_new_documents_once_and_reports_the_rest() {
        let root = dir("watch");
        let docs = root.join("docs");
        std::fs::create_dir_all(&docs).unwrap();
        std::fs::write(docs.join("brief.md"), "Encargo: puente.").unwrap();
        std::fs::write(docs.join("normas.txt"), "No usar acero corten.").unwrap();
        std::fs::write(docs.join("notas.md"), "Reunión el martes.").unwrap();
        std::fs::write(docs.join("foto.jpg"), [0xff, 0xd8, 0xff]).unwrap();
        std::fs::write(docs.join(".oculto.md"), "no").unwrap();
        std::fs::write(docs.join("roto.md"), [0xff, 0xfe, 0x00]).unwrap();
        let vault_dir = root.join("vault");
        let journal = root.join("vault-docs-journal.json");
        let domain = ContextDomain::work("acme", "bridge").unwrap();

        let first = watch_documents_once(&vault_dir, &journal, &docs, &domain, "w").unwrap();
        assert_eq!(first.files_seen, 4, "md and txt only, hidden skipped");
        assert_eq!(first.documents_recorded, 3);
        assert_eq!((first.briefs, first.restrictions, first.notes), (1, 1, 1));
        assert_eq!(first.failures.len(), 1, "{:?}", first.failures);
        assert!(first.failures[0].contains("roto.md"));

        let second = watch_documents_once(&vault_dir, &journal, &docs, &domain, "w").unwrap();
        assert_eq!(second.documents_recorded, 0);
        assert_eq!(second.files_already_known, 3);

        let vault = vault_io::open_vault(&vault_dir).unwrap();
        assert_eq!(vault.sealed_entries().len(), 3);
        let _ = std::fs::remove_dir_all(&root);
    }
}
