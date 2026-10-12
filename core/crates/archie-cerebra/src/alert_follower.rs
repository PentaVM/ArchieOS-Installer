//! The live caller of [`crate::sentry_bridge`]: it tails the netwatch
//! `alerts.jsonl` and feeds every new alert through the bridge.
//!
//! `archie-netwatch` appends one JSON alert per line as its audit raises them,
//! rotates the file to `alerts.1.jsonl` past its size cap, and records what it
//! already raised in `told.json` next to it (`docs/SENTRY-NETWATCH.md`). Nothing
//! consumed that file. This follower is the integrator the bridge's module note
//! describes: it decodes each line into the wire shape the netwatch binary writes
//! (this crate deliberately has no `archie-lan` dependency, so the decode is a
//! bounded serde struct, not a crate import), converts it to
//! [`AlertInput`] exactly as the bridge documents — the audit's severity maps onto
//! [`AlertSeverity`] in order, the origin is always [`AlertOrigin::Measurement`],
//! and an `address-moved` alert's dedup count is recovered from `told.json` — and
//! runs one [`Cerebra`] pass per batch with [`consider_alerts`].
//!
//! # Restart and rotation safety
//!
//! A journal file remembers the byte offset already consumed. A restart resumes
//! after it, so alerts are not re-fed; if the journal is lost or the file was
//! rotated underneath it, the follower may re-read alerts — which is safe, because
//! the bridge stores each alert under a content-derived reference and a replay
//! lands on the same record. Rotation is drained before the new file: when
//! `alerts.jsonl` shrinks below the journal offset, the unread tail is first read
//! from `alerts.1.jsonl` at that same offset, so nothing is silently skipped
//! either. A trailing partial line (netwatch appends atomically per line, but a
//! crash mid-write is possible) is held until its newline arrives.
//!
//! # What the outcome is
//!
//! Every batch produces a [`PassSummary`]: reasoning, or a typed refusal. On a
//! node with no local model the restricted pass is refused with
//! [`PassRefusal::NoLocalModel`] — the designed outcome of `docs/PENDIENTES.md`
//! §3–4 — and that refusal is both in the pass's trace and in the returned
//! [`FollowReport`], never swallowed and never treated as a failure of the
//! follower.

use crate::pass::Cerebra;
use crate::sentry_bridge::{
    consider_alerts, AlertInput, AlertPassOutcome, AlertSeverity, BridgeError, PassRefusal,
    MAX_ALERTS_PER_BATCH,
};
use crate::vault_io::{self, VaultIoError};
use archie_registry::ModelRegistry;
use archie_types::TraceId;
use archie_vault::ContextDomain;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// The most bytes read from the alerts file in one pass. Netwatch rotates at
/// 4 MiB; anything still unread beyond twice that waits for the next pass — the
/// offset journal makes waiting lossless.
pub const MAX_READ_BYTES: u64 = 8 * 1024 * 1024;
/// One alert line is bounded; a longer line is malformed, not partially parsed
/// (the bridge's own detail cap is far below this, so nothing valid is lost).
const MAX_LINE_BYTES: usize = 16 * 1024;
/// `told.json` is read only to recover `address-moved` counts; same bound the
/// audit itself holds it to.
const MAX_TOLD_BYTES: u64 = 4 * 1024 * 1024;
const JOURNAL_FORMAT: &str = "archie-cerebra-alert-follower-v1";

/// The files the follower reads and writes. `told` is optional: without it,
/// `address-moved` alerts simply carry no occurrence count.
#[derive(Debug, Clone)]
pub struct FollowerPaths {
    /// The netwatch `alerts.jsonl`.
    pub alerts: PathBuf,
    /// The netwatch `told.json`, when it exists.
    pub told: Option<PathBuf>,
    /// The follower's own offset journal, kept next to the vault.
    pub journal: PathBuf,
    /// The directory [`crate::vault_io`] keeps the follower's vault in.
    pub vault_dir: PathBuf,
}

/// Why a follow pass failed. A malformed journal is not an error: it is set
/// aside and the file is re-read from the start, which the vault's
/// content-derived references dedupe.
#[derive(Debug, thiserror::Error)]
pub enum FollowerError {
    #[error("the alerts file could not be read: {0}")]
    Io(String),
    #[error("the vault directory failed: {0}")]
    VaultIo(#[from] VaultIoError),
    #[error("the alert batch was refused: {0}")]
    Bridge(#[from] BridgeError),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct FollowerJournal {
    format: String,
    offset: u64,
    /// Fingerprint of the alerts file's first bytes at save time; a mismatch on
    /// load means the file was rotated or recreated regardless of size.
    #[serde(default)]
    prefix_hash: Option<String>,
}

impl Default for FollowerJournal {
    fn default() -> Self {
        Self {
            format: JOURNAL_FORMAT.to_string(),
            offset: 0,
            prefix_hash: None,
        }
    }
}

/// The same dual-FNV content id the bridges use: a stable fingerprint, not
/// cryptography. The journal keeps it over the file's first bytes so a rotated
/// or recreated alerts file is recognized even when its size alone would not
/// say so (a fresh file can be exactly as long as the old offset).
fn prefix_hash(path: &Path, offset: u64) -> Option<String> {
    if offset == 0 {
        return None;
    }
    let file = std::fs::File::open(path).ok()?;
    let mut bytes = Vec::new();
    file.take(offset.min(4096)).read_to_end(&mut bytes).ok()?;
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
    let first = fnv1a(&bytes, 0xcbf2_9ce4_8422_2325);
    let second = fnv1a(&bytes, 0x9e37_79b9_7f4a_7c15);
    Some(format!("{first:016x}{second:016x}"))
}

impl FollowerJournal {
    fn load(path: &Path) -> (Self, bool) {
        match std::fs::read(path) {
            Ok(bytes) if bytes.len() <= 4096 => match serde_json::from_slice::<Self>(&bytes) {
                Ok(journal) if journal.format == JOURNAL_FORMAT => (journal, false),
                _ => (Self::default(), true),
            },
            Ok(_) => (Self::default(), true),
            Err(_) => (Self::default(), false),
        }
    }

    fn save(&self, path: &Path) -> Result<(), FollowerError> {
        let partial = path.with_extension("partial");
        let bytes = serde_json::to_vec(self).expect("a journal serializes");
        std::fs::write(&partial, &bytes)
            .and_then(|()| std::fs::rename(&partial, path))
            .map_err(|e| FollowerError::Io(format!("{}: {e}", path.display())))
    }
}

/// The wire shape of one line of `alerts.jsonl`, exactly as `archie-netwatch`
/// serializes `archie_lan::audit::Alert`. Bounded at the line level by
/// [`MAX_LINE_BYTES`]; unknown fields are ignored so a newer netwatch with extra
/// fields still reads.
#[derive(Debug, Deserialize)]
struct WireAlert {
    at_ms: u64,
    severity: WireSeverity,
    rule: String,
    device: String,
    detail: String,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum WireSeverity {
    Notice,
    Warning,
    High,
}

impl WireSeverity {
    fn severity(self) -> AlertSeverity {
        match self {
            Self::Notice => AlertSeverity::Notice,
            Self::Warning => AlertSeverity::Warning,
            Self::High => AlertSeverity::High,
        }
    }
}

/// What one batch's pass came to, carried into the report. The pass record —
/// stages, grading, the refusal code — is in the trace; this is the operator-facing
/// summary of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PassSummary {
    pub trace_id: TraceId,
    /// Alerts the batch stored and referenced (already-known replays included).
    pub records: usize,
    /// `None` when a local model reasoned over the batch; the typed refusal
    /// otherwise — `NoLocalModel` is the expected one on a node without a local
    /// model, and it is recorded here *and* in the trace.
    pub refusal: Option<PassRefusal>,
}

impl PassSummary {
    pub fn reasoned(&self) -> bool {
        self.refusal.is_none()
    }
}

/// What one follow pass did. Counts are exact: every line read is accounted for
/// as parsed, malformed, or held as a partial tail.
#[derive(Debug, Default)]
pub struct FollowReport {
    /// Complete lines consumed from the alerts file (and its rotated sibling).
    pub lines_read: u64,
    /// Lines that were not an alert the netwatch binary writes; skipped, counted.
    pub malformed_lines: u64,
    /// Alerts handed to the bridge across all batches.
    pub alerts_fed: u64,
    /// Parsed alerts the bridge's own validation rejected; counted apart from
    /// malformed lines, which never parsed at all.
    pub alerts_rejected: u64,
    /// One entry per batch that ran a pass, in order.
    pub passes: Vec<PassSummary>,
    /// True when the journal did not parse and the file was re-read from the
    /// start (safe: vault references dedupe the replay).
    pub journal_reset: bool,
    /// True when the alerts file shrank past the journal offset (rotation or
    /// recreation) and the offset restarted.
    pub rotation_reset: bool,
}

impl FollowReport {
    /// Batches whose pass was refused for lack of a local model.
    pub fn refused_no_local_model(&self) -> usize {
        self.passes
            .iter()
            .filter(|p| p.refusal == Some(PassRefusal::NoLocalModel))
            .count()
    }
}

/// The `address-moved` occurrence counts from `told.json`, keyed as the audit
/// keys them (`moved <address>`). An unreadable told file yields an empty map —
/// the count is enrichment, never a reason to drop the alert.
fn told_counts(path: Option<&Path>) -> BTreeMap<String, u64> {
    let Some(path) = path else {
        return BTreeMap::new();
    };
    let Ok(bytes) = std::fs::read(path) else {
        return BTreeMap::new();
    };
    if bytes.len() as u64 > MAX_TOLD_BYTES {
        return BTreeMap::new();
    }
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return BTreeMap::new();
    };
    value
        .get("records")
        .and_then(|r| r.as_object())
        .map(|records| {
            records
                .iter()
                .filter_map(|(key, record)| {
                    let count = record.get("count")?.as_u64()?;
                    Some((key.clone(), count))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Convert one wire alert to the bridge's input, exactly as the bridge's module
/// note specifies. `address-moved` details begin `<address>: ...`, which is how
/// the audit's `moved <address>` told key is recovered.
fn to_input(wire: WireAlert, told: &BTreeMap<String, u64>) -> AlertInput {
    let occurrences = if wire.rule == "address-moved" {
        wire.detail
            .split_once(':')
            .and_then(|(address, _)| told.get(&format!("moved {}", address.trim())))
            .copied()
    } else {
        None
    };
    let mut input = AlertInput::measured(
        wire.at_ms,
        wire.severity.severity(),
        wire.rule,
        wire.device,
        wire.detail,
    );
    if let Some(count) = occurrences {
        input = input.with_occurrences(count);
    }
    input
}

/// Read complete lines starting at `offset`, returning them with the offset of
/// the first unconsumed byte. A trailing partial line stays unconsumed.
/// `Ok(None)` means the file shrank past the offset — rotated or recreated —
/// which the caller handles by draining the rotated sibling first; any genuine
/// I/O failure is the error.
fn read_new_lines(path: &Path, offset: u64) -> Result<Option<(Vec<String>, u64)>, FollowerError> {
    let mut file = std::fs::File::open(path)
        .map_err(|e| FollowerError::Io(format!("{}: {e}", path.display())))?;
    let length = file
        .metadata()
        .map_err(|e| FollowerError::Io(format!("{}: {e}", path.display())))?
        .len();
    if offset > length {
        return Ok(None);
    }
    file.seek(SeekFrom::Start(offset))
        .map_err(|e| FollowerError::Io(format!("{}: {e}", path.display())))?;
    let mut bytes = Vec::new();
    file.take(MAX_READ_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| FollowerError::Io(format!("{}: {e}", path.display())))?;
    let end = match bytes.iter().rposition(|b| *b == b'\n') {
        Some(newline) => newline + 1,
        None => 0, // no complete line yet; the tail waits
    };
    let mut lines = Vec::new();
    for raw in bytes[..end].split(|b| *b == b'\n') {
        if raw.is_empty() {
            continue;
        }
        lines.push(String::from_utf8_lossy(raw).into_owned());
    }
    Ok(Some((lines, offset + end as u64)))
}

/// A Cerebra with the default router, gateway and config — what a local
/// follower needs; the pass's routing still comes from the model registry the
/// caller hands to each pass.
pub fn local_cerebra() -> Cerebra {
    Cerebra::new(
        archie_router::Router::default(),
        archie_vault::ContextGateway::new(),
        crate::CerebraConfig::default(),
    )
}

/// An empty model registry: every restricted pass is refused with
/// [`PassRefusal::NoLocalModel`], the designed outcome on a node without a
/// local model (`docs/PENDIENTES.md` §4). Callers with a real, probed local
/// model build their own registry instead.
pub fn no_local_models() -> ModelRegistry {
    ModelRegistry::new()
}

/// One follow pass: read every complete new alert line, record and consider the
/// alerts in batches, persist vault and journal, and report exactly what
/// happened. Safe to call in a poll loop; a second call over unchanged input
/// reads nothing and feeds nothing.
///
/// `now_ms` is the caller's clock, used for the passes themselves; stored
/// records carry the alerts' own times, per the bridge's honesty rules.
pub fn follow_alerts_once(
    cerebra: &Cerebra,
    paths: &FollowerPaths,
    models: &ModelRegistry,
    task_id: &str,
    now_ms: i64,
) -> Result<FollowReport, FollowerError> {
    let mut report = FollowReport::default();
    let (mut journal, reset) = FollowerJournal::load(&paths.journal);
    report.journal_reset = reset;

    let mut lines = Vec::new();
    // Rotation or recreation: the file shrank past the offset, or its first
    // bytes no longer match the fingerprint the journal kept. Either way the
    // unread tail is drained from the rotated sibling at the old offset first —
    // netwatch renames `alerts.jsonl` to `alerts.1.jsonl` verbatim — then the
    // new file is read from the start. Replays dedupe in the vault.
    let fresh = read_new_lines(&paths.alerts, journal.offset)?;
    let rotated = journal.offset > 0
        && (fresh.is_none() || prefix_hash(&paths.alerts, journal.offset) != journal.prefix_hash);
    if rotated {
        report.rotation_reset = true;
        let sibling = paths.alerts.with_file_name("alerts.1.jsonl");
        if let Ok(Some((tail, _))) = read_new_lines(&sibling, journal.offset) {
            lines.extend(tail);
        }
        let (read, next) = read_new_lines(&paths.alerts, 0)?
            .expect("offset zero is never past the end of the file");
        lines.extend(read);
        journal.offset = next;
    } else {
        let (read, next) = fresh.expect("not rotated means the file covers the offset");
        lines.extend(read);
        journal.offset = next;
    }
    journal.prefix_hash = prefix_hash(&paths.alerts, journal.offset);

    let told = told_counts(paths.told.as_deref());
    let mut alerts = Vec::new();
    for line in &lines {
        report.lines_read += 1;
        if line.len() > MAX_LINE_BYTES {
            report.malformed_lines += 1;
            continue;
        }
        match serde_json::from_str::<WireAlert>(line) {
            Ok(wire) => alerts.push(to_input(wire, &told)),
            Err(_) => report.malformed_lines += 1,
        }
    }
    report.alerts_fed = alerts.len() as u64;

    if !alerts.is_empty() {
        let mut vault = vault_io::open_vault(&paths.vault_dir)?;
        let domain = ContextDomain::personal("owner").map_err(|e| {
            FollowerError::Io(format!("the sentry domain is fixed and must parse: {e}"))
        })?;
        for batch in alerts.chunks(MAX_ALERTS_PER_BATCH) {
            consider_batch(
                cerebra,
                &mut vault,
                models,
                &domain,
                task_id,
                batch,
                now_ms,
                &mut report,
            )?;
        }
        // Vault first, journal second: a crash between them re-reads alerts the
        // vault then dedupes; the other order could lose them.
        vault_io::persist_vault(&paths.vault_dir, &vault)?;
    }
    journal.save(&paths.journal)?;
    Ok(report)
}

/// One batch through the bridge. A batch that fails validation is retried
/// alert by alert: the malformed ones are counted and skipped, the rest still
/// record — one bad line must not wedge the follower behind itself forever.
#[allow(clippy::too_many_arguments)]
fn consider_batch(
    cerebra: &Cerebra,
    vault: &mut archie_vault::Vault,
    models: &ModelRegistry,
    domain: &ContextDomain,
    task_id: &str,
    batch: &[AlertInput],
    now_ms: i64,
    report: &mut FollowReport,
) -> Result<(), FollowerError> {
    let mut attempt =
        |alerts: &[AlertInput], report: &mut FollowReport| -> Result<bool, FollowerError> {
            let trace_id = TraceId::new();
            match consider_alerts(
                cerebra, vault, models, domain, task_id, alerts, trace_id, now_ms,
            ) {
                Ok(AlertPassOutcome::NoAlerts) => Ok(true),
                Ok(AlertPassOutcome::Reasoning { records, .. }) => {
                    report.passes.push(PassSummary {
                        trace_id,
                        records: records.len(),
                        refusal: None,
                    });
                    Ok(true)
                }
                Ok(AlertPassOutcome::Refused {
                    refusal, records, ..
                }) => {
                    report.passes.push(PassSummary {
                        trace_id,
                        records: records.len(),
                        refusal: Some(refusal),
                    });
                    Ok(true)
                }
                Err(BridgeError::InvalidAlert { .. }) => Ok(false),
                Err(other) => Err(FollowerError::Bridge(other)),
            }
        };
    if attempt(batch, report)? {
        return Ok(());
    }
    for alert in batch {
        if !attempt(std::slice::from_ref(alert), report)? {
            report.alerts_rejected += 1;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use archie_router::Router;
    use archie_types::{
        Availability, Capability, DataRetention, Locality, ModelDescriptor, ModelId, PrivacyGrade,
        ProviderKind, TokenCost,
    };
    use archie_vault::{ContextGateway, VaultRef};

    /// Every alert in these tests is a fixture: synthetic hardware addresses and
    /// RFC 1918 addresses written for the test. No capture, no device, no network.
    const FIXTURE_DEVICE: &str = "54:4e:f0:07:3d:e8";

    struct Fixture {
        dir: PathBuf,
        paths: FollowerPaths,
    }

    impl Fixture {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "archie-cerebra-follower-{name}-{}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            let paths = FollowerPaths {
                alerts: dir.join("alerts.jsonl"),
                told: Some(dir.join("told.json")),
                journal: dir.join("follower-journal.json"),
                vault_dir: dir.join("vault"),
            };
            Self { dir, paths }
        }

        fn append(&self, lines: &[&str]) {
            use std::io::Write;
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.paths.alerts)
                .unwrap();
            for line in lines {
                writeln!(file, "{line}").unwrap();
            }
        }

        fn vault_entries(&self) -> usize {
            vault_io::open_vault(&self.paths.vault_dir)
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

    fn cerebra() -> Cerebra {
        Cerebra::new(
            Router::default(),
            ContextGateway::new(),
            crate::CerebraConfig::default(),
        )
    }

    fn new_device_line(at_ms: u64) -> String {
        format!(
            "{{\"at_ms\":{at_ms},\"severity\":\"notice\",\"rule\":\"new-device\",\"device\":\"{FIXTURE_DEVICE}\",\"detail\":\"first seen, at 192.168.1.84\"}}"
        )
    }

    fn port_scan_line(at_ms: u64) -> String {
        format!(
            "{{\"at_ms\":{at_ms},\"severity\":\"high\",\"rule\":\"port-scan\",\"device\":\"{FIXTURE_DEVICE}\",\"detail\":\"opened 100 ports of 192.168.1.1 within a minute\"}}"
        )
    }

    #[test]
    fn new_alerts_are_recorded_and_the_pass_is_refused_without_a_local_model() {
        let fixture = Fixture::new("basic");
        let first = new_device_line(10_000);
        let second = port_scan_line(15_000);
        fixture.append(&[&first, &second]);

        let report = follow_alerts_once(
            &cerebra(),
            &fixture.paths,
            &ModelRegistry::new(),
            "sentry-alerts",
            20_000,
        )
        .unwrap();

        assert_eq!(report.lines_read, 2);
        assert_eq!(report.malformed_lines, 0);
        assert_eq!(report.alerts_fed, 2);
        assert_eq!(report.passes.len(), 1);
        // The designed outcome on a node without a local model: a recorded,
        // typed refusal — not a failure of the follower.
        assert_eq!(report.passes[0].refusal, Some(PassRefusal::NoLocalModel));
        assert_eq!(report.refused_no_local_model(), 1);
        // Both alerts are sealed in the vault under content-derived references.
        assert_eq!(fixture.vault_entries(), 2);
    }

    #[test]
    fn a_restart_does_not_refeed_alerts() {
        let fixture = Fixture::new("restart");
        let line = port_scan_line(15_000);
        fixture.append(&[&line]);
        let cerebra = cerebra();

        let first = follow_alerts_once(
            &cerebra,
            &fixture.paths,
            &ModelRegistry::new(),
            "sentry-alerts",
            20_000,
        )
        .unwrap();
        assert_eq!(first.alerts_fed, 1);
        let entries_after_first = fixture.vault_entries();

        // A "restart": a fresh call over the same files, with the journal the
        // first run persisted. Nothing is re-read, nothing re-fed.
        let second = follow_alerts_once(
            &cerebra,
            &fixture.paths,
            &ModelRegistry::new(),
            "sentry-alerts",
            21_000,
        )
        .unwrap();
        assert_eq!(second.alerts_fed, 0);
        assert!(second.passes.is_empty());
        assert_eq!(fixture.vault_entries(), entries_after_first);

        // Journal lost (operator wiped state, or a restore): the file is
        // re-read, and the vault's content-derived references absorb the
        // replay — the entries count does not move.
        std::fs::remove_file(&fixture.paths.journal).unwrap();
        let third = follow_alerts_once(
            &cerebra,
            &fixture.paths,
            &ModelRegistry::new(),
            "sentry-alerts",
            22_000,
        )
        .unwrap();
        assert_eq!(third.alerts_fed, 1);
        assert_eq!(fixture.vault_entries(), entries_after_first);
    }

    #[test]
    fn a_partial_last_line_is_held_until_its_newline_arrives() {
        let fixture = Fixture::new("partial");
        let cerebra = cerebra();
        // A crash mid-write leaves a line without its newline.
        std::fs::write(
            &fixture.paths.alerts,
            format!("{}\n{{\"at_ms\":15", new_device_line(10_000)),
        )
        .unwrap();
        let first = follow_alerts_once(
            &cerebra,
            &fixture.paths,
            &ModelRegistry::new(),
            "sentry-alerts",
            20_000,
        )
        .unwrap();
        assert_eq!(first.alerts_fed, 1);
        assert_eq!(first.malformed_lines, 0);

        // The writer finishes the line; the follower picks up exactly the tail.
        let rest = "000,\"severity\":\"high\",\"rule\":\"port-scan\",\"device\":\"54:4e:f0:07:3d:e8\",\"detail\":\"opened 100 ports of 192.168.1.1 within a minute\"}\n";
        use std::io::Write;
        std::fs::OpenOptions::new()
            .append(true)
            .open(&fixture.paths.alerts)
            .unwrap()
            .write_all(rest.as_bytes())
            .unwrap();
        let second = follow_alerts_once(
            &cerebra,
            &fixture.paths,
            &ModelRegistry::new(),
            "sentry-alerts",
            21_000,
        )
        .unwrap();
        assert_eq!(second.alerts_fed, 1);
        assert_eq!(second.malformed_lines, 0);
        assert_eq!(fixture.vault_entries(), 2);
    }

    #[test]
    fn rotation_drains_the_rotated_file_then_reads_the_new_one() {
        let fixture = Fixture::new("rotation");
        let cerebra = cerebra();
        let one = new_device_line(10_000);
        fixture.append(&[&one]);
        follow_alerts_once(
            &cerebra,
            &fixture.paths,
            &ModelRegistry::new(),
            "sentry-alerts",
            20_000,
        )
        .unwrap();

        // Two more alerts land, then netwatch rotates: alerts.jsonl becomes
        // alerts.1.jsonl and a fresh alerts.jsonl starts.
        let two = port_scan_line(15_000);
        fixture.append(&[&two]);
        std::fs::rename(&fixture.paths.alerts, fixture.dir.join("alerts.1.jsonl")).unwrap();
        let three = new_device_line(18_000);
        fixture.append(&[&three]);

        let report = follow_alerts_once(
            &cerebra,
            &fixture.paths,
            &ModelRegistry::new(),
            "sentry-alerts",
            21_000,
        )
        .unwrap();
        assert!(report.rotation_reset);
        assert_eq!(
            report.alerts_fed, 2,
            "the rotated tail and the new file both feed"
        );
        // The vault holds all three distinct alerts, each once.
        assert_eq!(fixture.vault_entries(), 3);
    }

    #[test]
    fn malformed_lines_are_counted_and_skipped_not_fatal() {
        let fixture = Fixture::new("malformed");
        let good = port_scan_line(15_000);
        fixture.append(&["this is not an alert", &good, "{\"at_ms\":\"nope\"}"]);
        let report = follow_alerts_once(
            &cerebra(),
            &fixture.paths,
            &ModelRegistry::new(),
            "sentry-alerts",
            20_000,
        )
        .unwrap();
        assert_eq!(report.lines_read, 3);
        assert_eq!(report.malformed_lines, 2);
        assert_eq!(report.alerts_fed, 1);
        assert_eq!(fixture.vault_entries(), 1);
    }

    #[test]
    fn an_address_moved_alert_carries_its_told_count() {
        let fixture = Fixture::new("told-count");
        std::fs::write(
            fixture.paths.told.as_ref().unwrap(),
            "{\"format\":\"archie-netwatch-told-v1\",\"records\":{\"moved 192.168.1.84\":{\"at_ms\":9000,\"count\":3}}}",
        )
        .unwrap();
        let line = "{\"at_ms\":10000,\"severity\":\"notice\",\"rule\":\"address-moved\",\"device\":\"54:4e:f0:07:3d:e8\",\"detail\":\"192.168.1.84: address now claimed by another device; it has now changed hands 3 times, most lately from aa:bb:cc:dd:ee:ff\"}";
        fixture.append(&[line]);
        let report = follow_alerts_once(
            &cerebra(),
            &fixture.paths,
            &ModelRegistry::new(),
            "sentry-alerts",
            20_000,
        )
        .unwrap();
        assert_eq!(report.alerts_fed, 1);
        // The stored record carries occurrences: 3, recovered from told.json.
        let mut vault = vault_io::open_vault(&fixture.paths.vault_dir).unwrap();
        let reference = vault
            .sealed_entries()
            .into_iter()
            .map(|entry| entry.reference)
            .find(|r: &VaultRef| r.path().contains("/sentry/alerts/"))
            .expect("the alert was stored");
        let disclosure = vault
            .disclose("test", &reference, PrivacyGrade::Restricted, 21_000)
            .unwrap();
        let text = disclosure.text().unwrap();
        assert!(text.contains("\"occurrences\":3"), "stored record: {text}");
    }

    #[test]
    fn a_node_with_a_local_model_reasons_over_the_batch() {
        let fixture = Fixture::new("reasoning");
        let line = port_scan_line(15_000);
        fixture.append(&[&line]);
        let mut models = ModelRegistry::new();
        models
            .register(ModelDescriptor {
                id: ModelId::parse("local.fixture").unwrap(),
                display_name: "fixture local adviser".into(),
                provider: ProviderKind::LlamaCpp,
                locality: Locality::Local,
                node: None,
                context_window: 8192,
                max_output_tokens: 2048,
                capabilities: vec![Capability::known("compute.inference")],
                max_privacy_grade: PrivacyGrade::Restricted,
                retention: DataRetention::ZeroVerified,
                availability: Availability::Ready,
                cost: TokenCost::FREE,
                latency_ema_ms: Some(100),
                quality_tier: 60,
                required_accelerator_bytes: None,
                labels: BTreeMap::new(),
            })
            .unwrap();
        let report =
            follow_alerts_once(&cerebra(), &fixture.paths, &models, "sentry-alerts", 20_000)
                .unwrap();
        assert_eq!(report.passes.len(), 1);
        assert!(
            report.passes[0].reasoned(),
            "a local model reasons: {:?}",
            report.passes[0]
        );
    }
}
