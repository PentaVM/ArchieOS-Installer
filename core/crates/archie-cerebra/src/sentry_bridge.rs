//! The bridge from `archie-netwatch` alerts to the cognitive loop.
//!
//! Today nothing produces a [`PillarContext`]. This module is the Sentry producer: it
//! takes the alerts the continuous audit in `archie-lan` raises (`docs/SENTRY-NETWATCH.md`),
//! records each one in the vault under a content-derived reference, collects the
//! references into a [`PillarContext`] tagged [`Pillar::Sentry`], and — when asked —
//! runs one [`Cerebra`] pass over the batch, returning a typed outcome whose refusal is
//! itself recorded in the trace.
//!
//! # The conversion point
//!
//! This crate deliberately does not depend on `archie-lan`. The integrator — the
//! component that watches `alerts.jsonl` in the running system — converts
//! `archie_lan::audit::Alert` into [`AlertInput`] by copying fields: `at_ms`,
//! `severity`, `rule`, `device`, `detail` map one to one; the audit's `Severity`
//! maps onto [`AlertSeverity`] in order; `origin` is [`AlertOrigin::Measurement`] for
//! netwatch alerts, which are raised from observed frames. The dedup count of an
//! `address-moved` alert (its `ToldRecord::count`) is passed as `occurrences`.
//!
//! # Honesty rules, inherited from the Sentry context feed
//!
//! - Timestamps are the alert's own (`Alert::at_ms` is the frames' time), never the
//!   bridge's clock. The observation for the pass is stamped with the newest alert's
//!   time; `now_ms` belongs to the caller.
//! - [`AlertOrigin`] mirrors `archie_sentry`'s `ObservationOrigin` discipline: it is
//!   copied into the stored record verbatim, and a hypothesis is never upgraded to a
//!   measurement — there is no code path that writes `measurement` but this copy.
//! - Provenance ids are carried, bounded and validated exactly as the feed's are.
//! - The reference path carries a content-derived id, so a replayed `alerts.jsonl`
//!   dedupes; a different alert landing on the same id is a typed collision error,
//!   never a silent overwrite.
//! - Nothing here actuates on the network, and a device is a hardware address, never
//!   a person. The pass requests grade `restricted`; on a node with no local model
//!   the pass is refused, and that refusal is the recorded outcome (`docs/PENDIENTES.md`
//!   §3–4), not a failure of this bridge.

use crate::pass::{Cerebra, CerebraError, Intent, Pass};
use crate::pillars::{Pillar, PillarContext, PillarError, MAX_CONTEXT_REFERENCES};
use crate::{Observation, Trigger};
use archie_registry::ModelRegistry;
use archie_types::{PrivacyGrade, TraceId};
use archie_vault::{ContextDomain, DomainError, EntryClass, Vault, VaultError, VaultRef};
use serde::Serialize;

/// The most alerts one batch may carry: one reference per distinct alert, and the
/// pillar context's budget is the ceiling for both.
pub const MAX_ALERTS_PER_BATCH: usize = MAX_CONTEXT_REFERENCES;

const MAX_RULE_LEN: usize = 64;
const MAX_DEVICE_LEN: usize = 64;
const MAX_DETAIL_LEN: usize = 1024;
const MAX_PROVENANCE: usize = 16;
const MAX_PROVENANCE_LEN: usize = 128;
/// What an alert that arrives without evidence ids is recorded as coming from: the
/// file the netwatch binary appends to. Supplying real evidence ids is better.
const DEFAULT_PROVENANCE: &str = "archie-netwatch:alerts.jsonl";
const RECORD_FORMAT: &str = "archie-sentry-alert-v1";
const GOAL_LABEL: &str = "sentry-alerts";
const ACTOR: &str = "sentry-bridge";

/// How serious the audit judged what it saw. Mirrors `archie_lan::audit::Severity`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum AlertSeverity {
    Notice,
    Warning,
    High,
}

impl AlertSeverity {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Notice => "notice",
            Self::Warning => "warning",
            Self::High => "high",
        }
    }
}

/// The severity an audit rule carries on its own, from the rule table in
/// `archie-lan/src/audit.rs`. `address-moved` is a notice except when the address is
/// the router's — the alert itself says so, and the bridge never lowers it.
fn severity_for_rule(rule: &str) -> Option<AlertSeverity> {
    match rule {
        "new-device" => Some(AlertSeverity::Notice),
        "address-moved" => Some(AlertSeverity::Notice),
        "cleartext-login" => Some(AlertSeverity::Warning),
        "dns-tunnel-suspect" => Some(AlertSeverity::Warning),
        "port-scan" => Some(AlertSeverity::High),
        "host-sweep" => Some(AlertSeverity::High),
        _ => None,
    }
}

/// Where the claim came from. Mirrors `archie_sentry::contracts::ObservationOrigin`;
/// copied, never upgraded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlertOrigin {
    /// Raised from observed frames — the only thing netwatch produces.
    Measurement,
    /// A hypothesis supplied by an analytic component. It stays one.
    Hypothesis,
}

impl AlertOrigin {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Measurement => "measurement",
            Self::Hypothesis => "hypothesis",
        }
    }
}

/// One alert, as handed over from `archie-lan` (see the module's conversion note).
///
/// Fields are public so the conversion is a plain copy; every one is validated at the
/// boundary in [`record_alerts`], and a malformed alert is a typed error, never a
/// panic and never silently trimmed.
#[derive(Debug, Clone)]
pub struct AlertInput {
    /// The alert's own time (the frames' clock), milliseconds.
    pub at_ms: u64,
    pub severity: AlertSeverity,
    /// The audit rule that raised it, e.g. `port-scan`.
    pub rule: String,
    /// The hardware address of the device the alert is about.
    pub device: String,
    /// What was seen, as the audit phrased it.
    pub detail: String,
    /// The alert's own dedup count, where the rule keeps one (`address-moved`).
    pub occurrences: Option<u64>,
    pub origin: AlertOrigin,
    /// Evidence ids. Empty becomes [`DEFAULT_PROVENANCE`].
    pub provenance: Vec<String>,
}

impl AlertInput {
    /// A measured alert without a dedup count and with the default provenance.
    pub fn measured(
        at_ms: u64,
        severity: AlertSeverity,
        rule: impl Into<String>,
        device: impl Into<String>,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            at_ms,
            severity,
            rule: rule.into(),
            device: device.into(),
            detail: detail.into(),
            occurrences: None,
            origin: AlertOrigin::Measurement,
            provenance: Vec::new(),
        }
    }

    /// The alert's own dedup count, when the audit kept one.
    pub fn with_occurrences(mut self, occurrences: u64) -> Self {
        self.occurrences = Some(occurrences);
        self
    }

    /// Mark the alert as a hypothesis. There is no method that marks one measured:
    /// measured is the constructor, and a hypothesis can never walk back.
    pub fn hypothesis(mut self) -> Self {
        self.origin = AlertOrigin::Hypothesis;
        self
    }

    pub fn with_provenance(mut self, provenance: Vec<String>) -> Self {
        self.provenance = provenance;
        self
    }
}

/// Why a batch or an alert was refused. Each case names what is wrong; nothing here
/// panics on input that arrived over the wire or from a tampered file.
#[derive(Debug, thiserror::Error)]
pub enum BridgeError {
    #[error("an alert batch holds {found} alerts; the most one pass may carry is {cap}")]
    BatchTooLarge { found: usize, cap: usize },
    #[error("invalid alert: {field}: {reason}")]
    InvalidAlert {
        field: &'static str,
        reason: &'static str,
    },
    #[error("a different record already holds {reference}")]
    ReferenceCollision { reference: VaultRef },
    #[error("pillar context refused the alert: {0}")]
    Pillar(#[from] PillarError),
    #[error("the alert's domain could not hold its reference: {0}")]
    Domain(#[from] DomainError),
    #[error("the vault refused the alert record: {0}")]
    Vault(#[from] VaultError),
    #[error("the pass could not be recorded: {0}")]
    Cerebra(#[from] CerebraError),
}

/// One validated alert, stored in the vault and referenced from the pillar context.
///
/// `Debug` shows identifiers only: the detail can name what a device asked DNS for,
/// and that is the owner's business, not a log line's.
pub struct AlertRecord {
    reference: VaultRef,
    rule: String,
    device: String,
    at_ms: i64,
    severity: AlertSeverity,
    origin: AlertOrigin,
    occurrences: Option<u64>,
    content: String,
}

impl std::fmt::Debug for AlertRecord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AlertRecord")
            .field("reference", &self.reference)
            .field("rule", &self.rule)
            .field("at_ms", &self.at_ms)
            .field("severity", &self.severity)
            .field("origin", &self.origin)
            .field("occurrences", &self.occurrences)
            .finish_non_exhaustive()
    }
}

impl AlertRecord {
    pub fn reference(&self) -> &VaultRef {
        &self.reference
    }
    pub fn rule(&self) -> &str {
        &self.rule
    }
    pub fn device(&self) -> &str {
        &self.device
    }
    /// The alert's own time, never the bridge's.
    pub fn at_ms(&self) -> i64 {
        self.at_ms
    }
    pub fn severity(&self) -> AlertSeverity {
        self.severity
    }
    pub fn origin(&self) -> AlertOrigin {
        self.origin
    }
    pub fn occurrences(&self) -> Option<u64> {
        self.occurrences
    }
    /// The stored record, verbatim. This is the claim; handle it at its grade.
    pub fn content(&self) -> &str {
        &self.content
    }
}

/// The wire form of one stored alert record. Field order is the encoding order, so
/// the content-derived id is stable; unknown fields have nowhere to hide.
#[derive(Serialize)]
struct AlertContent<'a> {
    format: &'static str,
    rule: &'a str,
    device: &'a str,
    at_ms: u64,
    severity: &'static str,
    origin: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    occurrences: Option<u64>,
    detail: &'a str,
    provenance: &'a [String],
}

/// FNV-1a over the content, twice from different bases. Not cryptography: the id is
/// a dedup key and a path component, and a collision is caught by comparing the full
/// stored content rather than trusted. It is stable across compiler versions, which a
/// persisted reference must be.
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

fn validate(alert: &AlertInput) -> Result<(), BridgeError> {
    let invalid =
        |field: &'static str, reason: &'static str| BridgeError::InvalidAlert { field, reason };
    if alert.at_ms == 0 {
        return Err(invalid(
            "at_ms",
            "an alert without a source time is not an observation",
        ));
    }
    if i64::try_from(alert.at_ms).is_err() {
        return Err(invalid(
            "at_ms",
            "the source time does not fit the trace's clock",
        ));
    }
    if alert.rule.is_empty()
        || alert.rule.len() > MAX_RULE_LEN
        || !alert
            .rule
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    {
        return Err(invalid("rule", "a rule is a bounded kebab-case identifier"));
    }
    if alert.device.is_empty()
        || alert.device.len() > MAX_DEVICE_LEN
        || alert.device.bytes().any(|b| b.is_ascii_control())
    {
        return Err(invalid(
            "device",
            "a subject is a bounded identifier without control characters",
        ));
    }
    if alert.detail.is_empty()
        || alert.detail.len() > MAX_DETAIL_LEN
        || alert.detail.bytes().any(|b| b.is_ascii_control())
    {
        return Err(invalid(
            "detail",
            "a detail is bounded text without control characters",
        ));
    }
    if alert.occurrences == Some(0) {
        return Err(invalid(
            "occurrences",
            "a dedup count of zero counts nothing",
        ));
    }
    if alert.provenance.len() > MAX_PROVENANCE {
        return Err(invalid("provenance", "too many evidence ids"));
    }
    for id in &alert.provenance {
        if id.is_empty()
            || id.len() > MAX_PROVENANCE_LEN
            || id.bytes().any(|b| b.is_ascii_control())
        {
            return Err(invalid(
                "provenance",
                "an evidence id is bounded text without control characters",
            ));
        }
    }
    Ok(())
}

/// Validate one alert and shape it into the record the vault will hold. Pure: nothing
/// is stored here, so a bad alert in a batch leaves the vault untouched.
fn alert_record(domain: &ContextDomain, alert: &AlertInput) -> Result<AlertRecord, BridgeError> {
    validate(alert)?;
    let provenance: Vec<String> = if alert.provenance.is_empty() {
        vec![DEFAULT_PROVENANCE.to_string()]
    } else {
        alert.provenance.clone()
    };
    // The severity of record is the rule's own, raised to the alert's when the alert
    // says more — an `address-moved` on the router's address stays `high`. Never lower.
    let severity = severity_for_rule(&alert.rule)
        .map(|derived| derived.max(alert.severity))
        .unwrap_or(alert.severity);
    let content = AlertContent {
        format: RECORD_FORMAT,
        rule: &alert.rule,
        device: &alert.device,
        at_ms: alert.at_ms,
        severity: severity.as_str(),
        origin: alert.origin.as_str(),
        occurrences: alert.occurrences,
        detail: &alert.detail,
        provenance: &provenance,
    };
    let content = serde_json::to_string(&content).map_err(|_| BridgeError::InvalidAlert {
        field: "detail",
        reason: "the record could not be encoded",
    })?;
    let reference =
        domain.reference(&format!("sentry/alerts/{}", content_id(content.as_bytes())))?;
    Ok(AlertRecord {
        reference,
        rule: alert.rule.clone(),
        device: alert.device.clone(),
        at_ms: alert.at_ms as i64,
        severity,
        origin: alert.origin,
        occurrences: alert.occurrences,
        content,
    })
}

/// A batch of alerts, validated, stored and collected into one pillar context.
pub struct AlertBatch {
    context: PillarContext,
    records: Vec<AlertRecord>,
}

impl std::fmt::Debug for AlertBatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AlertBatch")
            .field("context", &self.context)
            .field("records", &self.records.len())
            .finish()
    }
}

impl AlertBatch {
    pub fn context(&self) -> &PillarContext {
        &self.context
    }
    pub fn records(&self) -> &[AlertRecord] {
        &self.records
    }

    /// The observation for a pass over this batch. Stamped with the newest alert's own
    /// time — the bridge has no fresher source — and graded `restricted`, which is what
    /// `PillarContext` always does (`docs/PENDIENTES.md` §4).
    pub fn observation(&self) -> Option<Observation> {
        let at_ms = self.records.iter().map(|record| record.at_ms).max()?;
        Some(self.context.observation(
            Trigger::SystemEvent {
                source: "archie-netwatch".into(),
            },
            GOAL_LABEL,
            at_ms,
        ))
    }
}

/// Validate, store and collect one batch of alerts into a [`PillarContext`].
///
/// Every alert is validated before the vault is touched: a batch with one malformed
/// alert stores nothing. Identical alerts — a replayed `alerts.jsonl`, or a repeated
/// read — land on one content-derived reference and are stored once. A different
/// record already sitting at that reference is a [`BridgeError::ReferenceCollision`],
/// never a silent overwrite.
pub fn record_alerts(
    vault: &mut Vault,
    domain: &ContextDomain,
    task_id: &str,
    alerts: &[AlertInput],
) -> Result<AlertBatch, BridgeError> {
    if alerts.len() > MAX_ALERTS_PER_BATCH {
        return Err(BridgeError::BatchTooLarge {
            found: alerts.len(),
            cap: MAX_ALERTS_PER_BATCH,
        });
    }
    // Phase one: validate everything, dedupe by the content-derived reference. A batch
    // that fails here has touched nothing.
    let mut records: Vec<AlertRecord> = Vec::new();
    for alert in alerts {
        let record = alert_record(domain, alert)?;
        match records
            .iter()
            .find(|existing| existing.reference == record.reference)
        {
            Some(existing) if existing.content == record.content => continue,
            Some(_) => {
                return Err(BridgeError::ReferenceCollision {
                    reference: record.reference,
                })
            }
            None => records.push(record),
        }
    }
    // Phase two: store. A replayed alert is already there with the same content, which
    // is fine; anything else under its reference is a collision.
    let mut context = PillarContext::new(domain.clone(), task_id)?;
    for record in &records {
        if vault.contains(&record.reference) {
            let existing = vault
                .disclose(
                    ACTOR,
                    &record.reference,
                    PrivacyGrade::Restricted,
                    record.at_ms,
                )
                .ok()
                .and_then(|disclosure| disclosure.text().map(str::to_owned));
            match existing {
                Some(text) if text == record.content => {}
                _ => {
                    return Err(BridgeError::ReferenceCollision {
                        reference: record.reference.clone(),
                    })
                }
            }
        } else {
            vault.store(
                ACTOR,
                record.reference.clone(),
                EntryClass::PrivateFact,
                PrivacyGrade::Restricted,
                &record.content,
                record.at_ms,
                None,
            )?;
        }
        context.include(Pillar::Sentry, record.reference.clone())?;
    }
    Ok(AlertBatch { context, records })
}

/// Why a pass over an alert batch ended without a model. Each variant is already a
/// code in the trace; this type is what a caller matches on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PassRefusal {
    /// Restricted context and no local model permitted to see it. This is the designed
    /// outcome on a node without one (`docs/PENDIENTES.md` §4), recorded in the trace.
    NoLocalModel,
    /// The batch was larger than what any permitted model can hold; the caller
    /// decomposes rather than simplifies.
    EscalationRequired,
    /// The vault refused to assemble the alert context; the trace code says why.
    Context(&'static str),
    /// Any other recorded silence. The code is in the trace.
    Other(&'static str),
}

/// What a batch of alerts came to. The pass record — trace id, stages, grading — is
/// inside, whether the pass reasoned or was refused.
pub enum AlertPassOutcome {
    /// An empty batch: nothing was stored, no pass ran, no trace exists. Not an error.
    NoAlerts,
    /// The pass assembled the alert context and a local model was chosen for it.
    Reasoning {
        pass: Pass,
        records: Vec<AlertRecord>,
    },
    /// The pass was refused, and the refusal is the recorded outcome: typed here, and
    /// in the trace as the closing stage.
    Refused {
        pass: Pass,
        refusal: PassRefusal,
        records: Vec<AlertRecord>,
    },
}

impl std::fmt::Debug for AlertPassOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoAlerts => f.write_str("AlertPassOutcome::NoAlerts"),
            Self::Reasoning { pass, .. } => f
                .debug_struct("AlertPassOutcome::Reasoning")
                .field("trace_id", &pass.trace.trace_id)
                .finish_non_exhaustive(),
            Self::Refused { pass, refusal, .. } => f
                .debug_struct("AlertPassOutcome::Refused")
                .field("trace_id", &pass.trace.trace_id)
                .field("refusal", refusal)
                .finish_non_exhaustive(),
        }
    }
}

impl AlertPassOutcome {
    /// The pass record, when a pass ran at all.
    pub fn pass(&self) -> Option<&Pass> {
        match self {
            Self::NoAlerts => None,
            Self::Reasoning { pass, .. } | Self::Refused { pass, .. } => Some(pass),
        }
    }
}

fn classify(reason: &'static str) -> PassRefusal {
    match reason {
        "no_model_permitted" => PassRefusal::NoLocalModel,
        "escalation_required" => PassRefusal::EscalationRequired,
        code if code.starts_with("context_") || code.starts_with("capsule_") => {
            PassRefusal::Context(code)
        }
        code => PassRefusal::Other(code),
    }
}

/// The wiring of section 3: alerts → pillar references → one pass of the existing
/// sequencer, with the outcome — reasoning or a recorded refusal — typed.
///
/// `now_ms` is the caller's clock, used for the pass itself; the observation and the
/// stored records carry the alerts' own times. `trace_id` names the pass and lands in
/// the trace verbatim. An empty batch is [`AlertPassOutcome::NoAlerts`]: no pass, no
/// trace, nothing stored.
#[allow(clippy::too_many_arguments)]
pub fn consider_alerts(
    cerebra: &Cerebra,
    vault: &mut Vault,
    models: &ModelRegistry,
    domain: &ContextDomain,
    task_id: &str,
    alerts: &[AlertInput],
    trace_id: TraceId,
    now_ms: i64,
) -> Result<AlertPassOutcome, BridgeError> {
    if alerts.is_empty() {
        return Ok(AlertPassOutcome::NoAlerts);
    }
    let batch = record_alerts(vault, domain, task_id, alerts)?;
    let Some(observation) = batch.observation() else {
        return Ok(AlertPassOutcome::NoAlerts);
    };
    let pass = cerebra.consider(vault, models, &observation, trace_id, now_ms)?;
    Ok(match &pass.intent {
        Intent::Reason { .. } => AlertPassOutcome::Reasoning {
            pass,
            records: batch.records,
        },
        Intent::StaySilent { reason } => AlertPassOutcome::Refused {
            refusal: classify(reason),
            pass,
            records: batch.records,
        },
    })
}
