//! Unified IoT context feed for IRIS: one typed event stream built from the
//! LAN inventory, Sentry execution receipts/observations, passive network
//! analysis and the router's TShark watchdog exports.
//!
//! Honesty rules enforced here:
//! - `observed_at` is always the source's timestamp. A source without one
//!   yields `null`, never "now".
//! - `origin` is copied from the source and never upgraded from hypothesis to
//!   measurement.
//! - `event_id` is a content-derived sha256, so replays dedupe.
//! - Every input and every retained structure is bounded; malformed input is
//!   an error, never a panic.

use crate::{
    contracts::{Observation, ObservationOrigin, ResourceState},
    engine::{ExecutionResult, ExecutionStatus, Stage},
};
use archie_types::Timestamp;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    net::{IpAddr, Ipv4Addr},
    path::{Path, PathBuf},
};

pub const SCHEMA_VERSION: u16 = 1;
pub const MAX_DEVICES: usize = 4096;
pub const MAX_EVENTS_PER_CALL: usize = 4096;
pub const MAX_FACTS: usize = 32;
pub const MAX_FACT_KEY_LEN: usize = 64;
pub const MAX_FACT_VALUE_LEN: usize = 256;
pub const MAX_PROVENANCE: usize = 16;
pub const MAX_PROVENANCE_LEN: usize = 128;
pub const MAX_INPUT_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_EVENT_BYTES: usize = 16 * 1024;
pub const SEGMENT_MAX_BYTES: u64 = 32 * 1024 * 1024;
pub const WATCHDOG_MAX_READ_BYTES: usize = 4 * 1024 * 1024;
pub const WATCHDOG_MAX_LINE_BYTES: usize = 4096;
pub const WATCHDOG_MAX_NAMES_PER_LINE: usize = 64;
pub const WATCHDOG_MAX_SERVICES_PER_HOST: usize = 16;

#[derive(Debug, thiserror::Error)]
pub enum FeedError {
    #[error("malformed feed input: {0}")]
    Malformed(&'static str),
    #[error("feed input exceeds bounds: {0}")]
    Bound(&'static str),
    #[error("event exceeds the serialized size bound")]
    EventTooLarge,
    #[error("feed I/O failed")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    DeviceSeen,
    DeviceGone,
    DeviceStateChanged,
    NetworkObservation,
    AlertObserved,
    IntentExecuted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventSource {
    LanScan,
    SentryEngine,
    PassiveNetwork,
    RouterWatchdog,
}

/// One analytic device-class candidate, mirroring Core's
/// `AnalyticDeviceClassification` wire shape. A candidate is a heuristic
/// classification, not a verified identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssetCandidate {
    pub class: String,
    pub score: u8,
    pub supporting_signals: u8,
}

/// Bounded, typed payload mirroring the producing source. Internally tagged
/// as `type` inside the event's `payload` field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum EventPayload {
    /// Device facts from the LAN inventory. `changed_keys` is non-empty only
    /// for `device_state_changed` and names the facts that differ.
    DeviceFacts {
        facts: BTreeMap<String, String>,
        changed_keys: Vec<String>,
    },
    /// A measured (or hypothesised) resource state from a Sentry adapter:
    /// Tv, Printer, Switch, Light, Climate or Media.
    ResourceState { state: ResourceState },
    /// One host from passive network analysis. `trusted` mirrors the source
    /// and is always `false` today; candidates are analytic, not identity.
    NetworkAsset {
        ipv4_address: String,
        candidates: Vec<AssetCandidate>,
        trusted: bool,
    },
    /// Aggregated mDNS advertisement metadata from the router watchdog.
    /// Service names are reduced to the finite vocabulary the passive
    /// importer already recognises; anything else is only counted.
    WatchdogTraffic {
        ipv4_address: String,
        packets: u32,
        services: Vec<String>,
        unrecognized_names: u32,
    },
    /// Outcome of one Sentry execution, carrying the intent digest so IRIS
    /// can correlate with the reviewed intent without seeing its content.
    ExecutionOutcome {
        status: ExecutionStatus,
        operation_id: u64,
        intent_digest: String,
        replayed: bool,
        dispatched: bool,
        stages: Vec<Stage>,
        before_state: Option<ResourceState>,
        after_state: Option<ResourceState>,
    },
}

/// One typed context event. `observed_at: null` is the explicit marker that
/// the source provided no timestamp; it is never filled with local time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextEvent {
    pub schema_version: u16,
    pub event_id: String,
    pub kind: EventKind,
    pub observed_at: Option<Timestamp>,
    pub origin: ObservationOrigin,
    pub source: EventSource,
    pub subject: String,
    pub payload: EventPayload,
    pub provenance: Vec<String>,
}

fn validate_subject(subject: &str) -> Result<(), FeedError> {
    // Same alphabet as contract identifiers, plus ':' for IPv6 subjects.
    if subject.is_empty()
        || subject.len() > 128
        || !subject
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_' | b':'))
    {
        return Err(FeedError::Malformed("invalid subject identifier"));
    }
    Ok(())
}

fn validate_provenance(provenance: &[String]) -> Result<(), FeedError> {
    if provenance.len() > MAX_PROVENANCE {
        return Err(FeedError::Bound("too many provenance entries"));
    }
    for entry in provenance {
        if entry.is_empty()
            || entry.len() > MAX_PROVENANCE_LEN
            || entry.bytes().any(|b| b.is_ascii_control())
        {
            return Err(FeedError::Malformed("invalid provenance entry"));
        }
    }
    Ok(())
}

fn validate_digest(digest: &str) -> Result<(), FeedError> {
    if digest.len() != 64
        || !digest
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        return Err(FeedError::Malformed("digest must be lowercase sha256 hex"));
    }
    Ok(())
}

impl ContextEvent {
    /// Builds an event, validates every bound and derives the stable id.
    pub fn new(
        kind: EventKind,
        observed_at: Option<Timestamp>,
        origin: ObservationOrigin,
        source: EventSource,
        subject: &str,
        payload: EventPayload,
        provenance: Vec<String>,
    ) -> Result<Self, FeedError> {
        let mut event = Self {
            schema_version: SCHEMA_VERSION,
            event_id: String::new(),
            kind,
            observed_at,
            origin,
            source,
            subject: subject.to_owned(),
            payload,
            provenance,
        };
        event.event_id = event.compute_event_id();
        event.validate()?;
        Ok(event)
    }

    /// Content-derived sha256 over everything except the id itself. Equal
    /// content yields an equal id, so a replayed ingest dedupes by id.
    pub fn compute_event_id(&self) -> String {
        #[derive(Serialize)]
        struct Preimage<'a> {
            schema_version: u16,
            kind: EventKind,
            observed_at: Option<Timestamp>,
            origin: ObservationOrigin,
            source: EventSource,
            subject: &'a str,
            payload: &'a EventPayload,
            provenance: &'a [String],
        }
        let preimage = Preimage {
            schema_version: self.schema_version,
            kind: self.kind,
            observed_at: self.observed_at,
            origin: self.origin,
            source: self.source,
            subject: &self.subject,
            payload: &self.payload,
            provenance: &self.provenance,
        };
        serde_json::to_vec(&preimage)
            .map(|bytes| format!("{:x}", Sha256::digest(bytes)))
            .unwrap_or_default()
    }

    pub fn validate(&self) -> Result<(), FeedError> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(FeedError::Malformed("unsupported feed schema version"));
        }
        validate_digest(&self.event_id)?;
        validate_subject(&self.subject)?;
        validate_provenance(&self.provenance)?;
        match &self.payload {
            EventPayload::DeviceFacts {
                facts,
                changed_keys,
            } => {
                if facts.len() > MAX_FACTS {
                    return Err(FeedError::Bound("too many device facts"));
                }
                for (key, value) in facts {
                    if key.is_empty() || key.len() > MAX_FACT_KEY_LEN {
                        return Err(FeedError::Bound("device fact key out of bounds"));
                    }
                    if value.len() > MAX_FACT_VALUE_LEN {
                        return Err(FeedError::Bound("device fact value out of bounds"));
                    }
                }
                if changed_keys.len() > MAX_FACTS
                    || changed_keys.iter().any(|k| k.len() > MAX_FACT_KEY_LEN)
                {
                    return Err(FeedError::Bound("changed keys out of bounds"));
                }
            }
            EventPayload::ResourceState { .. } => {}
            EventPayload::NetworkAsset {
                ipv4_address,
                candidates,
                ..
            } => {
                ipv4_address
                    .parse::<Ipv4Addr>()
                    .map_err(|_| FeedError::Malformed("invalid asset address"))?;
                if candidates.len() > 8 {
                    return Err(FeedError::Bound("too many asset candidates"));
                }
                for candidate in candidates {
                    if candidate.class.len() > MAX_FACT_KEY_LEN || candidate.score > 100 {
                        return Err(FeedError::Bound("asset candidate out of bounds"));
                    }
                }
            }
            EventPayload::WatchdogTraffic {
                ipv4_address,
                services,
                ..
            } => {
                ipv4_address
                    .parse::<Ipv4Addr>()
                    .map_err(|_| FeedError::Malformed("invalid watchdog address"))?;
                if services.len() > WATCHDOG_MAX_SERVICES_PER_HOST
                    || services.iter().any(|s| s.len() > 255)
                {
                    return Err(FeedError::Bound("watchdog services out of bounds"));
                }
            }
            EventPayload::ExecutionOutcome {
                intent_digest,
                stages,
                ..
            } => {
                validate_digest(intent_digest)?;
                if stages.len() > 16 {
                    return Err(FeedError::Bound("too many execution stages"));
                }
            }
        }
        Ok(())
    }
}

/// Parses one JSONL line back into an event, checking the schema, the bounds
/// and that the id still matches the content. Unknown fields are rejected.
pub fn parse_event(line: &str) -> Result<ContextEvent, FeedError> {
    if line.len() > MAX_EVENT_BYTES {
        return Err(FeedError::EventTooLarge);
    }
    let event: ContextEvent =
        serde_json::from_str(line).map_err(|_| FeedError::Malformed("event json"))?;
    event.validate()?;
    if event.event_id.is_empty() || event.compute_event_id() != event.event_id {
        return Err(FeedError::Malformed("event id does not match content"));
    }
    Ok(event)
}

/// Removes replays: keeps the first occurrence of each `event_id`, in order.
pub fn dedupe_events(events: Vec<ContextEvent>) -> Vec<ContextEvent> {
    let mut seen = BTreeSet::new();
    events
        .into_iter()
        .filter(|event| seen.insert(event.event_id.clone()))
        .collect()
}

/// A comparable device view of one source at one moment. Facts live in a
/// `BTreeMap`, so a device whose facts merely reordered compares equal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub taken_at: Option<Timestamp>,
    pub source: EventSource,
    pub devices: BTreeMap<String, BTreeMap<String, String>>,
}

impl Default for Snapshot {
    fn default() -> Self {
        Self {
            taken_at: None,
            source: EventSource::LanScan,
            devices: BTreeMap::new(),
        }
    }
}

fn truncate_chars(value: &str, max: usize) -> String {
    if value.len() <= max {
        return value.to_owned();
    }
    match value.char_indices().nth(max).map(|(index, _)| index) {
        Some(boundary) => value[..boundary].to_owned(),
        None => value.to_owned(),
    }
}

fn push_fact(facts: &mut BTreeMap<String, String>, key: String, value: &str) {
    if value.is_empty() || facts.len() >= MAX_FACTS {
        return;
    }
    facts.insert(
        truncate_chars(&key, MAX_FACT_KEY_LEN),
        truncate_chars(value, MAX_FACT_VALUE_LEN),
    );
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LanDeviceWire {
    address: String,
    class: Option<String>,
    name: String,
    manufacturer: String,
    model: String,
    services: Vec<String>,
    seen_by: Vec<String>,
    state: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LanInventoryWire {
    format: String,
    scanned_unix: u64,
    devices: Vec<LanDeviceWire>,
}

impl Snapshot {
    pub fn empty() -> Self {
        Self::default()
    }

    /// Parses one `archie-lan-inventory-v1` document into a comparable
    /// snapshot. Malformed input is an error, never a panic.
    pub fn from_lan_inventory_json(json: &str) -> Result<Self, FeedError> {
        if json.len() > MAX_INPUT_BYTES {
            return Err(FeedError::Bound("lan inventory too large"));
        }
        let inventory: LanInventoryWire =
            serde_json::from_str(json).map_err(|_| FeedError::Malformed("lan inventory json"))?;
        if inventory.format != "archie-lan-inventory-v1" {
            return Err(FeedError::Malformed("unsupported lan inventory format"));
        }
        if inventory.devices.len() > MAX_DEVICES {
            return Err(FeedError::Bound("too many inventory devices"));
        }
        let seconds = i64::try_from(inventory.scanned_unix)
            .map_err(|_| FeedError::Malformed("invalid scan timestamp"))?;
        let taken_at = Some(Timestamp::from_millis(
            seconds
                .checked_mul(1000)
                .ok_or(FeedError::Malformed("invalid scan timestamp"))?,
        ));
        let mut devices = BTreeMap::new();
        for device in &inventory.devices {
            device
                .address
                .parse::<IpAddr>()
                .map_err(|_| FeedError::Malformed("invalid device address"))?;
            validate_subject(&device.address)?;
            let mut facts = BTreeMap::new();
            if let Some(class) = &device.class {
                push_fact(&mut facts, "class".to_owned(), class);
            }
            push_fact(&mut facts, "name".to_owned(), &device.name);
            push_fact(&mut facts, "manufacturer".to_owned(), &device.manufacturer);
            push_fact(&mut facts, "model".to_owned(), &device.model);
            push_fact(
                &mut facts,
                "services".to_owned(),
                &device.services.join(","),
            );
            push_fact(&mut facts, "seen_by".to_owned(), &device.seen_by.join(","));
            for (key, value) in &device.state {
                push_fact(&mut facts, format!("state.{key}"), value);
            }
            devices.insert(device.address.clone(), facts);
        }
        Ok(Self {
            taken_at,
            source: EventSource::LanScan,
            devices,
        })
    }
}

/// Emits `device_seen`, `device_gone` and `device_state_changed` events for
/// the actual differences between two snapshots. A device whose facts only
/// reordered produces no event.
pub fn diff_snapshots(prev: &Snapshot, next: &Snapshot) -> Result<Vec<ContextEvent>, FeedError> {
    if prev.devices.len() > MAX_DEVICES || next.devices.len() > MAX_DEVICES {
        return Err(FeedError::Bound("too many snapshot devices"));
    }
    for (subject, facts) in prev.devices.iter().chain(next.devices.iter()) {
        validate_subject(subject)?;
        if facts.len() > MAX_FACTS {
            return Err(FeedError::Bound("too many device facts"));
        }
    }
    let mut events = Vec::new();
    for (subject, facts) in &next.devices {
        let event = match prev.devices.get(subject) {
            None => Some((EventKind::DeviceSeen, facts, Vec::new())),
            Some(before) if before != facts => {
                let changed: Vec<String> = facts
                    .iter()
                    .filter(|(key, value)| before.get(*key) != Some(*value))
                    .map(|(key, _)| key.clone())
                    .chain(
                        before
                            .keys()
                            .filter(|key| !facts.contains_key(*key))
                            .cloned(),
                    )
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect();
                Some((EventKind::DeviceStateChanged, facts, changed))
            }
            Some(_) => None,
        };
        if let Some((kind, facts, changed_keys)) = event {
            events.push(ContextEvent::new(
                kind,
                next.taken_at,
                ObservationOrigin::Measurement,
                next.source,
                subject,
                EventPayload::DeviceFacts {
                    facts: facts.clone(),
                    changed_keys,
                },
                Vec::new(),
            )?);
        }
    }
    for (subject, facts) in &prev.devices {
        if next.devices.contains_key(subject) {
            continue;
        }
        events.push(ContextEvent::new(
            EventKind::DeviceGone,
            next.taken_at,
            ObservationOrigin::Measurement,
            next.source,
            subject,
            EventPayload::DeviceFacts {
                facts: facts.clone(),
                changed_keys: Vec::new(),
            },
            Vec::new(),
        )?);
    }
    if events.len() > MAX_EVENTS_PER_CALL {
        return Err(FeedError::Bound("too many events"));
    }
    Ok(events)
}

/// Ingests one LAN inventory document as `device_seen` events against an
/// empty baseline. Repeating the same document yields the same event ids.
pub fn from_lan_inventory(json: &str) -> Result<Vec<ContextEvent>, FeedError> {
    let snapshot = Snapshot::from_lan_inventory_json(json)?;
    diff_snapshots(&Snapshot::empty(), &snapshot)
}

/// One Sentry observation as a `device_state_changed` event. The origin is
/// copied, never upgraded: a hypothesis stays a hypothesis.
pub fn from_observation(observation: &Observation) -> Result<ContextEvent, FeedError> {
    validate_subject(&observation.resource_id)?;
    crate::contracts::validate_id(&observation.evidence_id)
        .map_err(|_| FeedError::Malformed("invalid evidence identifier"))?;
    ContextEvent::new(
        EventKind::DeviceStateChanged,
        Some(observation.observed_at),
        observation.origin,
        EventSource::SentryEngine,
        &observation.resource_id,
        EventPayload::ResourceState {
            state: observation.state.clone(),
        },
        vec![observation.evidence_id.clone()],
    )
}

/// One execution receipt as an `intent_executed` event, plus an
/// `alert_observed` event when the effect diverged or dispatch left the
/// outcome unknown. `observed_at` is the receipt's own `finished_at`.
pub fn from_execution_result(result: &ExecutionResult) -> Result<Vec<ContextEvent>, FeedError> {
    if result.schema_version != SCHEMA_VERSION {
        return Err(FeedError::Malformed("unsupported execution receipt schema"));
    }
    validate_subject(&result.resource_id)?;
    validate_digest(&result.intent_digest)?;
    if result.intent_id.is_empty()
        || result.intent_id.len() > MAX_PROVENANCE_LEN
        || result.intent_id.bytes().any(|b| b.is_ascii_control())
    {
        return Err(FeedError::Malformed("invalid intent identifier"));
    }
    for observation in [&result.before, &result.after].into_iter().flatten() {
        crate::contracts::validate_id(&observation.evidence_id)
            .map_err(|_| FeedError::Malformed("invalid evidence identifier"))?;
    }
    let origin = if [&result.before, &result.after]
        .into_iter()
        .flatten()
        .any(|observation| observation.origin == ObservationOrigin::ModelHypothesis)
    {
        ObservationOrigin::ModelHypothesis
    } else {
        ObservationOrigin::Measurement
    };
    let mut provenance = vec![result.intent_id.clone()];
    for observation in [&result.before, &result.after].into_iter().flatten() {
        provenance.push(observation.evidence_id.clone());
    }
    let payload = EventPayload::ExecutionOutcome {
        status: result.status,
        operation_id: result.operation_id,
        intent_digest: result.intent_digest.clone(),
        replayed: result.replayed,
        dispatched: result.dispatched_at.is_some(),
        stages: result.stages.clone(),
        before_state: result.before.as_ref().map(|o| o.state.clone()),
        after_state: result.after.as_ref().map(|o| o.state.clone()),
    };
    let mut events = vec![ContextEvent::new(
        EventKind::IntentExecuted,
        Some(result.finished_at),
        origin,
        EventSource::SentryEngine,
        &result.resource_id,
        payload.clone(),
        provenance.clone(),
    )?];
    let anomalous = result.status == ExecutionStatus::Diverged
        || (result.status == ExecutionStatus::Indeterminate && result.dispatched_at.is_some());
    if anomalous {
        events.push(ContextEvent::new(
            EventKind::AlertObserved,
            Some(result.finished_at),
            origin,
            EventSource::SentryEngine,
            &result.resource_id,
            payload,
            provenance,
        )?);
    }
    Ok(events)
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PassiveCandidateWire {
    class: String,
    score: u8,
    supporting_signals: u8,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PassiveAssetWire {
    ipv4_address: String,
    #[serde(default)]
    candidates: Vec<PassiveCandidateWire>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum PassiveChangeWire {
    NewlyObserved {
        ipv4_address: String,
        #[serde(default)]
        candidates: Vec<PassiveCandidateWire>,
    },
    NoLongerObserved {
        ipv4_address: String,
        #[serde(default)]
        previous_candidates: Vec<PassiveCandidateWire>,
    },
    ClassificationChanged {
        ipv4_address: String,
        // Declared so unknown-field rejection still matches the wire schema;
        // the event payload carries the current candidates.
        #[serde(default, rename = "previous_candidates")]
        _previous_candidates: Vec<PassiveCandidateWire>,
        #[serde(default)]
        current_candidates: Vec<PassiveCandidateWire>,
    },
}

/// Wire shape of `archie sentry-network-analyze --json`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PassiveReportWire {
    source: String,
    trusted: bool,
    #[serde(default)]
    assets: Vec<PassiveAssetWire>,
    #[serde(default)]
    changes: Vec<PassiveChangeWire>,
}

fn passive_candidates(
    candidates: &[PassiveCandidateWire],
) -> Result<Vec<AssetCandidate>, FeedError> {
    if candidates.len() > 8 {
        return Err(FeedError::Bound("too many asset candidates"));
    }
    candidates
        .iter()
        .map(|candidate| {
            if candidate.score > 100 {
                return Err(FeedError::Malformed("invalid candidate score"));
            }
            Ok(AssetCandidate {
                class: truncate_chars(&candidate.class, MAX_FACT_KEY_LEN),
                score: candidate.score,
                supporting_signals: candidate.supporting_signals,
            })
        })
        .collect()
}

fn passive_address(address: &str) -> Result<(), FeedError> {
    address
        .parse::<Ipv4Addr>()
        .map_err(|_| FeedError::Malformed("invalid asset address"))?;
    validate_subject(address)
}

/// Ingests the JSON output of `archie sentry-network-analyze --json`. The
/// export carries no timestamps, so events carry `observed_at: null` — the
/// explicit missing-source-timestamp marker. A report claiming to be trusted
/// is refused: passive advertisements are untrusted analytic evidence.
pub fn from_passive_network(json: &str) -> Result<Vec<ContextEvent>, FeedError> {
    if json.len() > MAX_INPUT_BYTES {
        return Err(FeedError::Bound("passive network report too large"));
    }
    let report: PassiveReportWire =
        serde_json::from_str(json).map_err(|_| FeedError::Malformed("passive network json"))?;
    if report.source != "offline_mdns_metadata" {
        return Err(FeedError::Malformed("unsupported passive network source"));
    }
    if report.trusted {
        return Err(FeedError::Malformed(
            "passive advertisements must remain untrusted",
        ));
    }
    if report.assets.len() + report.changes.len() > MAX_EVENTS_PER_CALL {
        return Err(FeedError::Bound("too many passive network entries"));
    }
    let mut events = Vec::new();
    for asset in &report.assets {
        passive_address(&asset.ipv4_address)?;
        events.push(ContextEvent::new(
            EventKind::NetworkObservation,
            None,
            ObservationOrigin::Measurement,
            EventSource::PassiveNetwork,
            &asset.ipv4_address,
            EventPayload::NetworkAsset {
                ipv4_address: asset.ipv4_address.clone(),
                candidates: passive_candidates(&asset.candidates)?,
                trusted: false,
            },
            Vec::new(),
        )?);
    }
    for change in &report.changes {
        let (kind, address, candidates) = match change {
            PassiveChangeWire::NewlyObserved {
                ipv4_address,
                candidates,
            } => (EventKind::DeviceSeen, ipv4_address, candidates),
            PassiveChangeWire::NoLongerObserved {
                ipv4_address,
                previous_candidates,
            } => (EventKind::DeviceGone, ipv4_address, previous_candidates),
            PassiveChangeWire::ClassificationChanged {
                ipv4_address,
                current_candidates,
                ..
            } => (
                EventKind::DeviceStateChanged,
                ipv4_address,
                current_candidates,
            ),
        };
        passive_address(address)?;
        events.push(ContextEvent::new(
            kind,
            None,
            ObservationOrigin::Measurement,
            EventSource::PassiveNetwork,
            address,
            EventPayload::NetworkAsset {
                ipv4_address: address.clone(),
                candidates: passive_candidates(candidates)?,
                trusted: false,
            },
            Vec::new(),
        )?);
    }
    Ok(events)
}

#[cfg(unix)]
fn file_id(metadata: &std::fs::Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt;
    metadata.ino()
}

/// Windows exposes no stable inode; rotation there is detected by the head
/// fingerprint and length regression in `WatchdogReader::poll`.
#[cfg(not(unix))]
fn file_id(_metadata: &std::fs::Metadata) -> u64 {
    0
}

/// Content fingerprint of the first bytes of a file. Combined with the
/// inode, it detects rotation even when the filesystem reuses an inode for
/// the replacement file.
fn head_fingerprint(path: &Path) -> Result<String, FeedError> {
    let file = File::open(path)?;
    let mut head = Vec::new();
    file.take(64).read_to_end(&mut head)?;
    Ok(format!("{:x}", Sha256::digest(&head)))
}

/// One JSON object per line, as written by the router's TShark export:
/// the same three fields the passive importer accepts, plus an optional
/// `frame.time_epoch`. Example:
/// `{"ip_src":"192.168.1.2","udp_srcport":5353,"dns_ptr_names":["Office._ipp._tcp.local"],"frame_time_epoch":1790000000.5}`
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WatchdogLine {
    ip_src: String,
    udp_srcport: u16,
    dns_ptr_names: Vec<String>,
    #[serde(default)]
    frame_time_epoch: Option<f64>,
}

const WATCHDOG_SERVICES: [&str; 6] = [
    "_roku-ecp._tcp.local",
    "_googlecast._tcp.local",
    "_ipp._tcp.local",
    "_ipps._tcp.local",
    "_rtsp._tcp.local",
    "_home-assistant._tcp.local",
];

fn recognized_service(name: &str) -> Option<&'static str> {
    WATCHDOG_SERVICES
        .iter()
        .find(|service| {
            name == **service
                || name
                    .strip_suffix(**service)
                    .is_some_and(|prefix| prefix.ends_with('.'))
        })
        .copied()
}

fn parse_watchdog_line(text: &str) -> Result<(Ipv4Addr, Vec<&'static str>, u32, Option<i64>), ()> {
    let line: WatchdogLine = serde_json::from_str(text).map_err(|_| ())?;
    if line.udp_srcport != 5353 || line.dns_ptr_names.len() > WATCHDOG_MAX_NAMES_PER_LINE {
        return Err(());
    }
    let ip: Ipv4Addr = line.ip_src.parse().map_err(|_| ())?;
    let mut services: Vec<&'static str> = Vec::new();
    let mut unrecognized = 0u32;
    for name in &line.dns_ptr_names {
        if name.is_empty() || name.len() > 255 || name.chars().any(char::is_control) {
            return Err(());
        }
        let normalized = name.trim_end_matches('.').to_ascii_lowercase();
        match recognized_service(&normalized) {
            Some(service) => {
                if !services.contains(&service) {
                    services.push(service);
                }
            }
            None => unrecognized = unrecognized.saturating_add(1),
        }
    }
    let frame_ms = match line.frame_time_epoch {
        None => None,
        Some(epoch) if epoch.is_finite() && epoch >= 0.0 && epoch * 1000.0 <= i64::MAX as f64 => {
            Some((epoch * 1000.0).round() as i64)
        }
        Some(_) => return Err(()),
    };
    Ok((ip, services, unrecognized, frame_ms))
}

#[derive(Debug, Default)]
struct HostAggregate {
    packets: u32,
    services: BTreeSet<&'static str>,
    unrecognized: u32,
    last_frame_ms: Option<i64>,
}

/// Bounds for one watchdog poll. Defaults: 4 MiB per read, 8192 lines,
/// 4096 hosts and 256 tracked files.
#[derive(Debug, Clone, Copy)]
pub struct ReaderLimits {
    pub max_read_bytes: usize,
    pub max_lines_per_poll: usize,
    pub max_hosts_per_poll: usize,
    pub max_tracked_files: usize,
}

impl Default for ReaderLimits {
    fn default() -> Self {
        Self {
            max_read_bytes: WATCHDOG_MAX_READ_BYTES,
            max_lines_per_poll: 8192,
            max_hosts_per_poll: MAX_DEVICES,
            max_tracked_files: 256,
        }
    }
}

#[derive(Debug)]
struct Cursor {
    file_id: u64,
    head: String,
    offset: u64,
}

/// Reads a directory of rotated line-delimited JSON exports from the
/// router's TShark watchdog. Tracks per-file inode, head fingerprint and
/// offset, reads only complete lines, skips a partial tail until it is
/// completed, and never fails on malformed lines: they are counted and
/// skipped. Re-reads are idempotent; a rotated file (new inode or changed
/// head) restarts at offset zero.
pub struct WatchdogReader {
    dir: PathBuf,
    limits: ReaderLimits,
    cursors: BTreeMap<PathBuf, Cursor>,
    malformed_lines: u64,
}

impl WatchdogReader {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self::with_limits(dir, ReaderLimits::default())
    }

    pub fn with_limits(dir: impl Into<PathBuf>, limits: ReaderLimits) -> Self {
        Self {
            dir: dir.into(),
            limits,
            cursors: BTreeMap::new(),
            malformed_lines: 0,
        }
    }

    /// Total malformed lines counted since this reader was created.
    pub fn malformed_lines(&self) -> u64 {
        self.malformed_lines
    }

    pub fn tracked_files(&self) -> usize {
        self.cursors.len()
    }

    /// Reads new complete lines since the last poll and returns one
    /// `network_observation` event per host seen in this read, with
    /// `observed_at` taken from the newest frame timestamp present, or
    /// `null` when the export carries no timestamps.
    pub fn poll(&mut self) -> Result<Vec<ContextEvent>, FeedError> {
        let limits = self.limits;
        let mut entries: Vec<PathBuf> = std::fs::read_dir(&self.dir)?
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "jsonl"))
            .filter(|path| path.is_file())
            .collect();
        entries.sort();
        entries.truncate(limits.max_tracked_files);
        let mut budget = limits.max_read_bytes;
        let mut lines_read = 0usize;
        let mut hosts: BTreeMap<Ipv4Addr, HostAggregate> = BTreeMap::new();
        for path in entries {
            if budget == 0 || lines_read >= limits.max_lines_per_poll {
                break;
            }
            let metadata = std::fs::metadata(&path)?;
            let id = file_id(&metadata);
            let len = metadata.len();
            let head = head_fingerprint(&path)?;
            let cursor = self.cursors.entry(path.clone()).or_insert_with(|| Cursor {
                file_id: id,
                head: head.clone(),
                offset: 0,
            });
            if cursor.file_id != id || cursor.head != head || len < cursor.offset {
                cursor.file_id = id;
                cursor.head = head;
                cursor.offset = 0;
            }
            if len <= cursor.offset {
                continue;
            }
            let remaining = len - cursor.offset;
            let take_n = remaining.min(budget as u64);
            let budget_limited = remaining > take_n;
            let mut file = File::open(&path)?;
            file.seek(SeekFrom::Start(cursor.offset))?;
            let mut buffer = Vec::new();
            file.take(take_n).read_to_end(&mut buffer)?;
            budget = budget.saturating_sub(buffer.len());
            let mut consumed = 0usize;
            for raw in buffer.split_inclusive(|byte| *byte == b'\n') {
                if !raw.ends_with(b"\n") || lines_read >= limits.max_lines_per_poll {
                    break;
                }
                consumed += raw.len();
                lines_read += 1;
                let text = match std::str::from_utf8(&raw[..raw.len() - 1]) {
                    Ok(text) => text.trim_end_matches('\r'),
                    Err(_) => {
                        self.malformed_lines += 1;
                        continue;
                    }
                };
                if text.is_empty() {
                    continue;
                }
                if text.len() > WATCHDOG_MAX_LINE_BYTES {
                    self.malformed_lines += 1;
                    continue;
                }
                match parse_watchdog_line(text) {
                    Ok((ip, services, unrecognized, frame_ms)) => {
                        if !hosts.contains_key(&ip) && hosts.len() >= limits.max_hosts_per_poll {
                            self.malformed_lines += 1;
                            continue;
                        }
                        let aggregate = hosts.entry(ip).or_default();
                        aggregate.packets = aggregate.packets.saturating_add(1);
                        for service in services {
                            if aggregate.services.len() < WATCHDOG_MAX_SERVICES_PER_HOST {
                                aggregate.services.insert(service);
                            }
                        }
                        aggregate.unrecognized =
                            aggregate.unrecognized.saturating_add(unrecognized);
                        if let Some(ms) = frame_ms {
                            aggregate.last_frame_ms =
                                Some(aggregate.last_frame_ms.map_or(ms, |prev| prev.max(ms)));
                        }
                    }
                    Err(()) => {
                        self.malformed_lines += 1;
                    }
                }
            }
            if consumed == 0 && budget_limited {
                // The read bound cut a line: keep the offset, resume here on
                // the next poll with a fresh budget. A single line larger
                // than the read bound violates the export contract and
                // stalls only its own file, never the reader.
                break;
            }
            cursor.offset += consumed as u64;
        }
        let mut events = Vec::new();
        for (ip, aggregate) in hosts {
            let subject = ip.to_string();
            events.push(ContextEvent::new(
                EventKind::NetworkObservation,
                aggregate.last_frame_ms.map(Timestamp::from_millis),
                ObservationOrigin::Measurement,
                EventSource::RouterWatchdog,
                &subject,
                EventPayload::WatchdogTraffic {
                    ipv4_address: ip.to_string(),
                    packets: aggregate.packets,
                    services: aggregate.services.iter().map(|s| (*s).to_owned()).collect(),
                    unrecognized_names: aggregate.unrecognized,
                },
                Vec::new(),
            )?);
        }
        Ok(events)
    }
}

/// JSONL segment writer for the feed: one `ContextEvent` per line, each
/// serialized event bounded to 16 KiB, segments rotated at 32 MiB. A new
/// segment appears atomically (tmp file + rename); within a segment, lines
/// are appended plainly. No authentication: the directory is trusted local
/// state.
pub struct FeedWriter {
    dir: PathBuf,
    segment_max: u64,
    event_max: usize,
    segment_index: u64,
    segment_size: u64,
    created: bool,
}

impl FeedWriter {
    pub fn open(dir: impl Into<PathBuf>) -> Result<Self, FeedError> {
        Self::with_limits(dir, SEGMENT_MAX_BYTES, MAX_EVENT_BYTES)
    }

    pub fn with_limits(
        dir: impl Into<PathBuf>,
        segment_max: u64,
        event_max: usize,
    ) -> Result<Self, FeedError> {
        let dir = dir.into();
        std::fs::create_dir_all(&dir)?;
        let mut highest: Option<(u64, u64)> = None;
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            let Some(index) = name
                .strip_prefix("segment-")
                .and_then(|rest| rest.strip_suffix(".jsonl"))
                .and_then(|digits| digits.parse::<u64>().ok())
            else {
                continue;
            };
            let size = entry.metadata()?.len();
            highest = Some(match highest {
                Some((current, current_size)) if current >= index => (current, current_size),
                _ => (index, size),
            });
        }
        let (segment_index, segment_size, created) = match highest {
            Some((index, size)) if size < segment_max => (index, size, true),
            Some((index, _)) => (index + 1, 0, false),
            None => (0, 0, false),
        };
        Ok(Self {
            dir,
            segment_max,
            event_max,
            segment_index,
            segment_size,
            created,
        })
    }

    pub fn segment_index(&self) -> u64 {
        self.segment_index
    }

    fn segment_path(&self) -> PathBuf {
        self.dir
            .join(format!("segment-{:06}.jsonl", self.segment_index))
    }

    /// Appends one event as one line. Events larger than the 16 KiB bound
    /// are rejected; they are never truncated into a lie.
    pub fn append(&mut self, event: &ContextEvent) -> Result<(), FeedError> {
        let mut line = serde_json::to_string(event)
            .map_err(|_| FeedError::Malformed("event encoding failed"))?;
        line.push('\n');
        let bytes = line.len() as u64;
        if bytes > self.event_max as u64 {
            return Err(FeedError::EventTooLarge);
        }
        if self.created && self.segment_size + bytes > self.segment_max {
            self.segment_index += 1;
            self.segment_size = 0;
            self.created = false;
        }
        if !self.created {
            // Atomic appearance of a new segment: write the tmp file, then
            // rename it into place. Appends within a segment are plain.
            let tmp = self
                .dir
                .join(format!("segment-{:06}.jsonl.tmp", self.segment_index));
            File::create(&tmp)?;
            std::fs::rename(&tmp, self.segment_path())?;
            self.created = true;
        }
        let mut file = OpenOptions::new().append(true).open(self.segment_path())?;
        file.write_all(line.as_bytes())?;
        self.segment_size += bytes;
        Ok(())
    }
}
