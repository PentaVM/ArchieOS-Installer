//! LAN devices in the world graph: the `archie sentry-devices` unified view,
//! ingested as bounded, provenance-carrying entities.
//!
//! The unified view (`archie-sentry-devices-v1`) is produced by `archie-core`'s
//! `unify` module from the LAN inventory, the netwatch audit and the passive
//! import. This crate deliberately does not depend on `archie-core`, so the
//! document is re-validated here as plain data against the same bounds its
//! writer enforces. Anything unknown, malformed or oversize is rejected, and an
//! ingestion is atomic: the graph never holds half a view, and re-ingesting the
//! same view yields the same graph — no duplicates.
//!
//! Every entity is a *measurement*: something a source observed on the wire,
//! attributed to the sources that saw it and the times they saw it. RFC-008's
//! hypothesis/measurement separation holds here by construction —
//! [`LanEntityOrigin`] has no variant for a hypothesis, so this ingestion path
//! cannot express one.
//!
//! Two clocks, as in RFC-008 §4: `first_seen_unix` / `last_seen_unix` are event
//! time (when the sources saw the device); `ingested_at` is knowledge time
//! (when Archie took the view into the graph). The event-time fields stay in
//! the seconds the view carries; nothing here rescales them.

use archie_types::Timestamp;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::net::IpAddr;

/// The `format` value a view document must carry. The same literal
/// `archie-core::unify::FORMAT` writes; duplicated because this crate has no
/// dependency edge to `archie-core`.
pub const VIEW_FORMAT: &str = "archie-sentry-devices-v1";
/// Bytes of one view document. Mirrors the inventory bound its writer uses.
pub const MAX_VIEW_BYTES: usize = 8 * 1024 * 1024;
/// Entities the graph may hold — the same cap the view's writer enforces.
pub const MAX_LAN_DEVICES: usize = 4096;
/// The longest string a field may carry (names, classes, claim keys/values).
pub const MAX_FIELD_CHARS: usize = 256;
/// Names one entity may carry.
pub const MAX_NAMES: usize = 32;
/// Classes one entity may carry.
pub const MAX_CLASSES: usize = 32;
/// Addresses one entity may carry.
pub const MAX_IPS: usize = 64;
/// Claims one source may attach to one device in the view.
pub const MAX_CLAIMS: usize = 64;

/// Where a claim about a device came from. The same three sources the unified
/// view knows, under the same kebab-case names; a fourth source string is
/// malformed input, not a new variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LanSource {
    /// `archie-lan scan` / `watch`: what the device announced.
    LanScan,
    /// `archie-netwatch`: what the device's own frames showed.
    Netwatch,
    /// Offline TShark mDNS metadata, read from a file.
    PassiveImport,
}

/// What this ingestion path is allowed to believe. There is one variant, and
/// that is the point: devices in the graph are observed facts. A hypothesis
/// would have to arrive by a different path that names it as one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LanEntityOrigin {
    Measurement,
}

/// One source's contribution to one entity: that it saw the device, and when
/// (event time, as the source reported it).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LanSourceProvenance {
    pub first_seen_unix: Option<u64>,
    pub last_seen_unix: Option<u64>,
}

/// One device in the world graph: identity, kind, provenance, observed times.
///
/// `identity` is the hardware address, or `ip:<address>` — visibly prefixed —
/// for a device no source keyed by hardware. `kind` is the device's primary
/// class (its first class in sorted order), or `unclassified`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LanDeviceEntity {
    pub identity: String,
    pub mac: Option<String>,
    pub kind: String,
    pub names: Vec<String>,
    pub classes: Vec<String>,
    pub ips: BTreeSet<IpAddr>,
    pub origin: LanEntityOrigin,
    /// Which sources reported this device, with each source's own times.
    pub sources: BTreeMap<LanSource, LanSourceProvenance>,
    /// Earliest and latest event times across all sources.
    pub first_seen_unix: Option<u64>,
    pub last_seen_unix: Option<u64>,
    /// Knowledge time: when this entity entered the graph.
    pub ingested_at: Timestamp,
}

/// What one successful ingestion did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LanIngestReport {
    pub devices: usize,
    /// Event time of the merge that produced the view, as the view states it.
    pub generated_unix: u64,
}

/// A malformed or oversize view. Nothing panics; everything is reported, and
/// no message quotes the value it rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LanIngestError {
    /// An input exceeded a bound; the string says which.
    TooLarge(&'static str),
    /// An input did not match the format its writer produces; the string says why.
    Malformed(&'static str),
}

impl std::fmt::Display for LanIngestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooLarge(what) => write!(f, "too large: {what}"),
            Self::Malformed(why) => write!(f, "malformed: {why}"),
        }
    }
}

impl std::error::Error for LanIngestError {}

// --- The view document, as `archie-core::unify` writes it --------------------
//
// Plain data, mirrored from the writer's schema and re-validated here; the
// graph never trusts that the bytes came from the real writer.

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ViewDocument {
    format: String,
    generated_unix: u64,
    devices: Vec<ViewDevice>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ViewDevice {
    identity: String,
    mac: Option<String>,
    ips: BTreeSet<IpAddr>,
    names: BTreeSet<String>,
    classes: BTreeSet<String>,
    sources: BTreeMap<LanSource, ViewSourceFact>,
    first_seen_unix: Option<u64>,
    last_seen_unix: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ViewSourceFact {
    names: BTreeSet<String>,
    classes: BTreeSet<String>,
    first_seen_unix: Option<u64>,
    last_seen_unix: Option<u64>,
    claims: BTreeMap<String, String>,
}

/// A hardware address is `xx:xx:xx:xx:xx:xx`, lowercase hex — the only form
/// the writer emits. Anything else means the document is not one.
fn hardware_address(mac: &str) -> Result<(), LanIngestError> {
    let well_formed = mac.len() == 17
        && mac.bytes().enumerate().all(|(i, b)| {
            if i % 3 == 2 {
                b == b':'
            } else {
                b.is_ascii_hexdigit() && !b.is_ascii_uppercase()
            }
        });
    if well_formed {
        Ok(())
    } else {
        Err(LanIngestError::Malformed("hardware address"))
    }
}

fn field(value: &str, what: &'static str) -> Result<(), LanIngestError> {
    if value.chars().count() > MAX_FIELD_CHARS {
        return Err(LanIngestError::TooLarge(what));
    }
    Ok(())
}

fn times(first_seen_unix: Option<u64>, last_seen_unix: Option<u64>) -> Result<(), LanIngestError> {
    if let (Some(first), Some(last)) = (first_seen_unix, last_seen_unix) {
        if first > last {
            return Err(LanIngestError::Malformed("seen times out of order"));
        }
    }
    Ok(())
}

fn validate_device(device: &ViewDevice) -> Result<(), LanIngestError> {
    match &device.mac {
        Some(mac) => {
            hardware_address(mac)?;
            if device.identity != *mac {
                // A hardware-keyed device's identity is its address; anything
                // else is not a document this writer produced.
                return Err(LanIngestError::Malformed("device identity"));
            }
        }
        None => {
            // The weaker identity is marked, and must name a real address.
            let Some(address) = device.identity.strip_prefix("ip:") else {
                return Err(LanIngestError::Malformed("device identity"));
            };
            if address.parse::<IpAddr>().is_err() {
                return Err(LanIngestError::Malformed("device identity"));
            }
        }
    }
    if device.names.len() > MAX_NAMES {
        return Err(LanIngestError::TooLarge("device names"));
    }
    if device.classes.len() > MAX_CLASSES {
        return Err(LanIngestError::TooLarge("device classes"));
    }
    if device.ips.len() > MAX_IPS {
        return Err(LanIngestError::TooLarge("device addresses"));
    }
    for name in device.names.iter().chain(device.classes.iter()) {
        field(name, "device field")?;
    }
    times(device.first_seen_unix, device.last_seen_unix)?;
    for fact in device.sources.values() {
        if fact.names.len() > MAX_NAMES || fact.classes.len() > MAX_CLASSES {
            return Err(LanIngestError::TooLarge("source fields"));
        }
        if fact.claims.len() > MAX_CLAIMS {
            return Err(LanIngestError::TooLarge("source claims"));
        }
        for value in fact.names.iter().chain(fact.classes.iter()) {
            field(value, "source field")?;
        }
        for (key, value) in &fact.claims {
            field(key, "source claim key")?;
            field(value, "source claim value")?;
        }
        times(fact.first_seen_unix, fact.last_seen_unix)?;
    }
    Ok(())
}

fn entity_from(device: &ViewDevice, ingested_at: Timestamp) -> LanDeviceEntity {
    LanDeviceEntity {
        identity: device.identity.clone(),
        mac: device.mac.clone(),
        kind: device
            .classes
            .first()
            .cloned()
            .unwrap_or_else(|| "unclassified".to_string()),
        names: device.names.iter().cloned().collect(),
        classes: device.classes.iter().cloned().collect(),
        ips: device.ips.clone(),
        origin: LanEntityOrigin::Measurement,
        sources: device
            .sources
            .iter()
            .map(|(source, fact)| {
                (
                    *source,
                    LanSourceProvenance {
                        first_seen_unix: fact.first_seen_unix,
                        last_seen_unix: fact.last_seen_unix,
                    },
                )
            })
            .collect(),
        first_seen_unix: device.first_seen_unix,
        last_seen_unix: device.last_seen_unix,
        ingested_at,
    }
}

/// The bounded device graph: one entity per device, keyed by identity.
///
/// Ingestion replaces the graph's contents atomically — the unified view *is*
/// the current view of the LAN — so a rejected view leaves the previous graph
/// untouched and a re-ingested view changes nothing but knowledge time.
#[derive(Debug, Clone, Default)]
pub struct LanGraph {
    entities: BTreeMap<String, LanDeviceEntity>,
    generated_unix: Option<u64>,
    ingested_at: Option<Timestamp>,
}

impl LanGraph {
    pub fn new() -> Self {
        Self::default()
    }

    /// Ingest one `archie-sentry-devices-v1` document. Atomic: on any error the
    /// graph is exactly what it was before the call.
    pub fn ingest(
        &mut self,
        view: &[u8],
        now: Timestamp,
    ) -> Result<LanIngestReport, LanIngestError> {
        if view.len() > MAX_VIEW_BYTES {
            return Err(LanIngestError::TooLarge("lan view"));
        }
        let document: ViewDocument =
            serde_json::from_slice(view).map_err(|_| LanIngestError::Malformed("lan view json"))?;
        if document.format != VIEW_FORMAT {
            return Err(LanIngestError::Malformed("unsupported lan view format"));
        }
        if document.devices.len() > MAX_LAN_DEVICES {
            return Err(LanIngestError::TooLarge("lan view devices"));
        }

        let mut entities = BTreeMap::new();
        for device in &document.devices {
            validate_device(device)?;
            if entities
                .insert(device.identity.clone(), entity_from(device, now))
                .is_some()
            {
                // One record per device is the view's own contract; a document
                // that breaks it is not one this writer produced.
                return Err(LanIngestError::Malformed("duplicate device identity"));
            }
        }

        self.entities = entities;
        self.generated_unix = Some(document.generated_unix);
        self.ingested_at = Some(now);
        Ok(LanIngestReport {
            devices: self.entities.len(),
            generated_unix: document.generated_unix,
        })
    }

    pub fn len(&self) -> usize {
        self.entities.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entities.is_empty()
    }

    pub fn get(&self, identity: &str) -> Option<&LanDeviceEntity> {
        self.entities.get(identity)
    }

    /// Every entity, in identity order — hardware-keyed identities first, since
    /// `0`–`9` and `a`–`f` sort before `ip:`.
    pub fn entities(&self) -> impl Iterator<Item = &LanDeviceEntity> + '_ {
        self.entities.values()
    }

    /// Event time of the merge that produced the current view.
    pub fn generated_unix(&self) -> Option<u64> {
        self.generated_unix
    }

    /// Knowledge time of the current view: when it entered the graph.
    pub fn ingested_at(&self) -> Option<Timestamp> {
        self.ingested_at
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &[u8] = include_bytes!("../tests/fixtures/unified_view_small_lan.json");

    const TV: &str = "54:4e:f0:07:3d:e8";
    const PRINTER: &str = "aa:bb:cc:00:11:22";
    const CAMERA: &str = "ip:192.168.1.77";

    fn ingested(now: Timestamp) -> LanGraph {
        let mut graph = LanGraph::new();
        graph
            .ingest(FIXTURE, now)
            .expect("the fixture is a valid view");
        graph
    }

    #[test]
    fn the_graph_holds_exactly_the_fixture_devices_with_provenance() {
        let now = Timestamp::from_millis(1_790_000_200_000);
        let graph = ingested(now);

        assert_eq!(graph.len(), 3);
        assert_eq!(graph.generated_unix(), Some(1_790_000_100));
        assert_eq!(graph.ingested_at(), Some(now));

        let tv = graph.get(TV).expect("the television is one entity");
        assert_eq!(tv.mac.as_deref(), Some(TV));
        assert_eq!(tv.kind, "mdns-roku", "first class in sorted order");
        assert_eq!(tv.origin, LanEntityOrigin::Measurement);
        assert_eq!(tv.names, ["Living", "roku-living"]);
        assert_eq!(tv.ips.len(), 1);
        // Every source the fixture credited is present, under its own name,
        // with its own event times.
        assert_eq!(
            tv.sources.keys().copied().collect::<Vec<_>>(),
            [
                LanSource::LanScan,
                LanSource::Netwatch,
                LanSource::PassiveImport
            ]
        );
        assert_eq!(
            tv.sources[&LanSource::LanScan].first_seen_unix,
            Some(1_790_000_000)
        );
        assert_eq!(
            tv.sources[&LanSource::Netwatch].last_seen_unix,
            Some(1_790_000_005)
        );
        assert_eq!(tv.sources[&LanSource::PassiveImport].first_seen_unix, None);
        assert_eq!(tv.first_seen_unix, Some(1_789_999_000));
        assert_eq!(tv.last_seen_unix, Some(1_790_000_005));
        assert_eq!(
            tv.ingested_at, now,
            "knowledge time, distinct from event time"
        );

        let printer = graph.get(PRINTER).expect("the printer");
        assert_eq!(printer.kind, "printer");
        assert_eq!(printer.sources.len(), 2);

        // A device no source keyed by hardware keeps its visibly weaker identity.
        let camera = graph.get(CAMERA).expect("the IP-keyed device");
        assert_eq!(camera.mac, None);
        assert_eq!(camera.kind, "unclassified");
        assert_eq!(
            camera.sources.keys().copied().collect::<Vec<_>>(),
            [LanSource::PassiveImport]
        );
    }

    #[test]
    fn re_ingesting_the_same_view_changes_nothing_but_knowledge_time() {
        let now = Timestamp::from_millis(1_790_000_200_000);
        let mut graph = ingested(now);
        let before: Vec<LanDeviceEntity> = graph.entities().cloned().collect();

        let report = graph.ingest(FIXTURE, now).unwrap();
        assert_eq!(report.devices, 3);
        let after: Vec<LanDeviceEntity> = graph.entities().cloned().collect();
        assert_eq!(before, after, "same view, same graph — no duplicates");

        let later = now.plus_millis(60_000);
        graph.ingest(FIXTURE, later).unwrap();
        assert_eq!(graph.len(), 3, "still one entity per device");
        assert_eq!(graph.ingested_at(), Some(later));
        let tv = graph.get(TV).unwrap();
        assert_eq!(tv.ingested_at, later);
        assert_eq!(
            tv.first_seen_unix,
            Some(1_789_999_000),
            "event time untouched"
        );
    }

    #[test]
    fn a_rejected_view_leaves_the_previous_graph_untouched() {
        let now = Timestamp::from_millis(1_790_000_200_000);
        let mut graph = ingested(now);
        let before: Vec<LanDeviceEntity> = graph.entities().cloned().collect();

        assert!(graph.ingest(b"not json", now.plus_millis(1)).is_err());
        assert!(graph
            .ingest(&vec![b' '; MAX_VIEW_BYTES + 1], now.plus_millis(1))
            .is_err());
        let wrong_format = serde_json::json!({
            "format": "archie-lan-inventory-v1", "generated_unix": 0, "devices": []
        });
        assert!(graph
            .ingest(wrong_format.to_string().as_bytes(), now.plus_millis(1))
            .is_err());

        let after: Vec<LanDeviceEntity> = graph.entities().cloned().collect();
        assert_eq!(before, after, "never partial");
        assert_eq!(graph.ingested_at(), Some(now));
    }

    #[test]
    fn malformed_and_oversize_views_are_rejected() {
        let mut graph = LanGraph::new();
        let now = Timestamp::from_millis(1);

        let device = serde_json::json!({
            "identity": TV, "mac": TV, "ips": ["192.168.1.40"], "names": [],
            "classes": [], "sources": {}, "first_seen_unix": null, "last_seen_unix": null
        });
        let view = |devices: Vec<serde_json::Value>| {
            serde_json::json!({
                "format": VIEW_FORMAT, "generated_unix": 0, "devices": devices
            })
            .to_string()
        };

        // A field the writer never emits.
        let mut extra = device.clone();
        extra["surprise"] = serde_json::json!(1);
        assert_eq!(
            graph.ingest(view(vec![extra]).as_bytes(), now),
            Err(LanIngestError::Malformed("lan view json"))
        );
        // Uppercase hardware address: not what the writer emits.
        let mut bad_mac = device.clone();
        bad_mac["mac"] = serde_json::json!(TV.to_uppercase());
        bad_mac["identity"] = serde_json::json!(TV.to_uppercase());
        assert_eq!(
            graph.ingest(view(vec![bad_mac]).as_bytes(), now),
            Err(LanIngestError::Malformed("hardware address"))
        );
        // Identity that does not match the hardware address.
        let mut mismatched = device.clone();
        mismatched["identity"] = serde_json::json!("11:22:33:44:55:66");
        assert_eq!(
            graph.ingest(view(vec![mismatched]).as_bytes(), now),
            Err(LanIngestError::Malformed("device identity"))
        );
        // An unmarked weak identity: a bare IP with no MAC is not trusted.
        let mut unmarked = device.clone();
        unmarked["mac"] = serde_json::Value::Null;
        unmarked["identity"] = serde_json::json!("192.168.1.40");
        assert_eq!(
            graph.ingest(view(vec![unmarked]).as_bytes(), now),
            Err(LanIngestError::Malformed("device identity"))
        );
        // The same device twice.
        assert_eq!(
            graph.ingest(view(vec![device.clone(), device.clone()]).as_bytes(), now),
            Err(LanIngestError::Malformed("duplicate device identity"))
        );
        // Bounds.
        let mut long_name = device.clone();
        long_name["names"] = serde_json::json!(["n".repeat(MAX_FIELD_CHARS + 1)]);
        assert_eq!(
            graph.ingest(view(vec![long_name]).as_bytes(), now),
            Err(LanIngestError::TooLarge("device field"))
        );
        let too_many = view(vec![device.clone(); MAX_LAN_DEVICES + 1]);
        assert_eq!(
            graph.ingest(too_many.as_bytes(), now),
            Err(LanIngestError::TooLarge("lan view devices"))
        );
        // Seen times out of order.
        let mut backwards = device.clone();
        backwards["first_seen_unix"] = serde_json::json!(10);
        backwards["last_seen_unix"] = serde_json::json!(5);
        assert_eq!(
            graph.ingest(view(vec![backwards]).as_bytes(), now),
            Err(LanIngestError::Malformed("seen times out of order"))
        );
        // A source name the writer does not know.
        let mut unknown_source = device.clone();
        unknown_source["sources"] = serde_json::json!({
            "cloud-teaser": {"names": [], "classes": [], "first_seen_unix": null,
                             "last_seen_unix": null, "claims": {}}
        });
        assert!(graph
            .ingest(view(vec![unknown_source]).as_bytes(), now)
            .is_err());
        assert!(graph.is_empty(), "nothing landed");
    }

    #[test]
    fn an_empty_view_ingests_to_an_empty_graph() {
        let mut graph = ingested(Timestamp::from_millis(1));
        let empty = serde_json::json!({
            "format": VIEW_FORMAT, "generated_unix": 42, "devices": []
        });
        let report = graph
            .ingest(empty.to_string().as_bytes(), Timestamp::from_millis(2))
            .unwrap();
        assert_eq!(report.devices, 0);
        assert!(graph.is_empty(), "a vanished device leaves the graph");
        assert_eq!(graph.generated_unix(), Some(42));
    }
}
