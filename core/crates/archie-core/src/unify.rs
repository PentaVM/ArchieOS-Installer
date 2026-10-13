//! One view of the devices on the owner's network: what the active scan
//! (`archie-lan` inventory), the continuous audit (`archie-netwatch` state) and the
//! passive TShark import each said, merged into one record per device.
//!
//! Devices are keyed by hardware address (MAC); an IP is an attribute that changes.
//! Only the audit knows hardware addresses, so a device the scan or the import knows
//! by IP joins the record of the device that claimed that address on the wire, and
//! gets a record of its own — clearly marked as IP-keyed — when no device did. Two
//! devices no source keyed by hardware never merge.
//!
//! Everything is read from local files, bounded, and malformed or oversize input is
//! an error, never a panic. Nothing here captures, probes, acts on a device, or
//! leaves the machine. A phone is a device; whose it is, is not asked.

use crate::control::{NetworkAssetObservation, NetworkEvidence};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::net::IpAddr;

/// The `format` value of the JSON document [`UnifiedView`] serializes to.
pub const FORMAT: &str = "archie-sentry-devices-v1";
/// Records in one view. Each source is bounded below this; the merged view is too.
pub const MAX_DEVICES: usize = 4096;
/// The longest string a field may carry (names, models, hostnames, classes).
pub const MAX_FIELD_CHARS: usize = 256;
/// Names one device may carry across all sources.
pub const MAX_NAMES: usize = 32;
/// Classes one device may carry across all sources.
pub const MAX_CLASSES: usize = 32;
/// Addresses one device may have used.
pub const MAX_IPS: usize = 64;
/// Claims one source may attach to one device.
pub const MAX_CLAIMS: usize = 64;
/// Bytes of one inventory document (`archie-lan-inventory-v1`).
pub const MAX_INVENTORY_BYTES: usize = 8 * 1024 * 1024;
/// Bytes of one audit state (`archie-netwatch-state-v1`). It can hold 4096 devices
/// with the names they asked for; the cap keeps a corrupt file from reading forever.
pub const MAX_STATE_BYTES: usize = 64 * 1024 * 1024;

/// Where a claim about a device came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Source {
    /// `archie-lan scan` / `watch`: what the device announced over SSDP, mDNS, UPnP, ECP.
    LanScan,
    /// `archie-netwatch`: what the device's own frames showed.
    Netwatch,
    /// Offline TShark mDNS metadata, read from a file.
    PassiveImport,
}

impl Source {
    /// The name the JSON document and the human listing both use.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::LanScan => "lan-scan",
            Self::Netwatch => "netwatch",
            Self::PassiveImport => "passive-import",
        }
    }
}

/// What one source claimed about one device, with the times it saw it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceFact {
    /// Names this source heard for the device.
    pub names: BTreeSet<String>,
    /// Device families or evidence kinds this source assigned.
    pub classes: BTreeSet<String>,
    /// When this source first and last saw the device, where it tracks that.
    pub first_seen_unix: Option<u64>,
    pub last_seen_unix: Option<u64>,
    /// Small typed claims: manufacturer, model, frames, bytes, services, seen_by.
    pub claims: BTreeMap<String, String>,
}

/// One device, as every source together knows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceRecord {
    /// The hardware address, or `ip:<address>` for a device no source keyed by
    /// hardware. The prefix marks the weaker identity so it is never mistaken
    /// for a hardware one.
    pub identity: String,
    /// The hardware address, when any source knew one.
    pub mac: Option<String>,
    /// Every address the device has used, per the sources.
    pub ips: BTreeSet<IpAddr>,
    pub names: BTreeSet<String>,
    pub classes: BTreeSet<String>,
    /// What each source said, under that source.
    pub sources: BTreeMap<Source, SourceFact>,
    pub first_seen_unix: Option<u64>,
    pub last_seen_unix: Option<u64>,
}

/// The merged view: each device once, with per-source provenance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnifiedView {
    pub format: String,
    pub generated_unix: u64,
    /// Hardware-keyed devices first, by address; then IP-keyed ones, by address.
    pub devices: Vec<DeviceRecord>,
}

/// A malformed or oversize input. Nothing panics; everything is reported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnifyError {
    /// An input exceeded a bound; the string says which.
    TooLarge(&'static str),
    /// An input did not match the format its writer produces; the string says why.
    Malformed(&'static str),
}

impl std::fmt::Display for UnifyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooLarge(what) => write!(f, "too large: {what}"),
            Self::Malformed(why) => write!(f, "malformed: {why}"),
        }
    }
}

impl std::error::Error for UnifyError {}

// --- The inventory file, as `archie-lan` writes it -------------------------

/// One device in an `archie-lan-inventory-v1` document.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LanDevice {
    pub address: String,
    pub class: Option<String>,
    pub name: String,
    pub manufacturer: String,
    pub model: String,
    pub services: Vec<String>,
    pub seen_by: Vec<String>,
    pub state: BTreeMap<String, String>,
}

/// An `archie-lan-inventory-v1` document, validated at the boundary.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LanInventory {
    pub format: String,
    pub scanned_unix: u64,
    pub devices: Vec<LanDevice>,
}

/// Reads one `archie-lan-inventory-v1` document. Malformed input is an error.
pub fn parse_inventory(text: &str) -> Result<LanInventory, UnifyError> {
    if text.len() > MAX_INVENTORY_BYTES {
        return Err(UnifyError::TooLarge("lan inventory"));
    }
    let inventory: LanInventory =
        serde_json::from_str(text).map_err(|_| UnifyError::Malformed("lan inventory json"))?;
    if inventory.format != "archie-lan-inventory-v1" {
        return Err(UnifyError::Malformed("unsupported lan inventory format"));
    }
    if inventory.devices.len() > MAX_DEVICES {
        return Err(UnifyError::TooLarge("lan inventory devices"));
    }
    for device in &inventory.devices {
        device
            .address
            .parse::<IpAddr>()
            .map_err(|_| UnifyError::Malformed("inventory device address"))?;
        field(&device.name, "inventory device name")?;
        field(&device.manufacturer, "inventory device manufacturer")?;
        field(&device.model, "inventory device model")?;
        if let Some(class) = &device.class {
            field(class, "inventory device class")?;
        }
        if device.services.len() > 64 || device.seen_by.len() > 8 || device.state.len() > 64 {
            return Err(UnifyError::TooLarge("inventory device fields"));
        }
        for service in device.services.iter().chain(device.seen_by.iter()) {
            field(service, "inventory device service")?;
        }
        for (key, value) in &device.state {
            field(key, "inventory state key")?;
            field(value, "inventory state value")?;
        }
    }
    Ok(inventory)
}

// --- The audit state, as `archie-netwatch` writes it -----------------------

/// One device in an `archie-netwatch-state-v1` document, keyed by hardware address.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NetwatchDevice {
    pub first_seen_ms: u64,
    pub last_seen_ms: u64,
    pub addresses: BTreeSet<IpAddr>,
    pub hostname: Option<String>,
    pub frames: u64,
    pub bytes: u64,
    /// Names the device asked DNS for. Carried through the parser so the schema is
    /// strict; never copied into the view, which does not need them.
    pub asked: BTreeSet<String>,
    /// Bytes sent to each place outside the LAN. Not copied into the view either.
    pub sent_to: BTreeMap<String, u64>,
    /// The device's learned usual sending. Accepted, not copied into the view.
    #[serde(default)]
    pub baseline: Option<serde_json::Value>,
}

/// An `archie-netwatch-state-v1` document, validated at the boundary.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NetwatchState {
    pub format: String,
    pub started_ms: u64,
    pub devices: BTreeMap<String, NetwatchDevice>,
    /// Which device last claimed each address: IPv4 over ARP, IPv6 by Neighbor
    /// Advertisement.
    pub owners: BTreeMap<IpAddr, String>,
}

/// Reads one `archie-netwatch-state-v1` document. Malformed input is an error.
pub fn parse_netwatch_state(text: &str) -> Result<NetwatchState, UnifyError> {
    if text.len() > MAX_STATE_BYTES {
        return Err(UnifyError::TooLarge("netwatch state"));
    }
    let state: NetwatchState =
        serde_json::from_str(text).map_err(|_| UnifyError::Malformed("netwatch state json"))?;
    if state.format != "archie-netwatch-state-v1" {
        return Err(UnifyError::Malformed("unsupported netwatch state format"));
    }
    if state.devices.len() > MAX_DEVICES || state.owners.len() > MAX_DEVICES {
        return Err(UnifyError::TooLarge("netwatch devices"));
    }
    for (mac, device) in &state.devices {
        hardware_address(mac)?;
        if device.addresses.len() > MAX_IPS {
            return Err(UnifyError::TooLarge("netwatch device addresses"));
        }
        if let Some(hostname) = &device.hostname {
            field(hostname, "netwatch hostname")?;
        }
        // Bounds the audit keeps in memory, mirrored so a hand-built file cannot
        // carry more than the writer ever would.
        if device.asked.len() > 512 || device.sent_to.len() > 64 {
            return Err(UnifyError::TooLarge("netwatch device fields"));
        }
    }
    for mac in state.owners.values() {
        hardware_address(mac)?;
    }
    Ok(state)
}

fn field(value: &str, what: &'static str) -> Result<(), UnifyError> {
    if value.chars().count() > MAX_FIELD_CHARS {
        return Err(UnifyError::TooLarge(what));
    }
    Ok(())
}

/// A hardware address is `xx:xx:xx:xx:xx:xx`, lowercase hex, the form the audit
/// writes. Anything else means the file is not one this program's writers made.
fn hardware_address(mac: &str) -> Result<(), UnifyError> {
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
        Err(UnifyError::Malformed("hardware address"))
    }
}

// --- The merge --------------------------------------------------------------

fn seconds(ms: u64) -> Option<u64> {
    (ms > 0).then_some(ms / 1000)
}

fn evidence_name(evidence: &NetworkEvidence) -> String {
    match evidence {
        NetworkEvidence::RouterAdvertisement => "router-advertisement".to_string(),
        NetworkEvidence::MdnsRoku => "mdns-roku".to_string(),
        NetworkEvidence::MdnsGoogleCast => "mdns-googlecast".to_string(),
        NetworkEvidence::MdnsIpp => "mdns-ipp".to_string(),
        NetworkEvidence::MdnsRtsp => "mdns-rtsp".to_string(),
        NetworkEvidence::MdnsHomeAssistant => "mdns-home-assistant".to_string(),
        NetworkEvidence::SsdpInternetGatewayDevice => "ssdp-internet-gateway-device".to_string(),
        NetworkEvidence::SsdpMediaRenderer => "ssdp-media-renderer".to_string(),
        NetworkEvidence::OpenTcpPort(port) => format!("open-tcp-{port}"),
    }
}

struct Builder {
    /// Devices a source keyed by hardware address, by that address.
    by_mac: BTreeMap<String, DeviceRecord>,
    /// Devices no source keyed by hardware, by the one address they are known by.
    by_ip: BTreeMap<IpAddr, DeviceRecord>,
}

impl Builder {
    fn record(&mut self, record: DeviceRecord) -> Result<(), UnifyError> {
        if self.by_mac.len() + self.by_ip.len() >= MAX_DEVICES {
            return Err(UnifyError::TooLarge("unified devices"));
        }
        if let Some(mac) = &record.mac {
            self.by_mac.entry(mac.clone()).or_insert(record);
        } else {
            let ip = record
                .ips
                .first()
                .copied()
                .ok_or(UnifyError::Malformed("device with no address"))?;
            self.by_ip.entry(ip).or_insert(record);
        }
        Ok(())
    }

    fn fresh(identity: String, mac: Option<String>) -> DeviceRecord {
        DeviceRecord {
            identity,
            mac,
            ips: BTreeSet::new(),
            names: BTreeSet::new(),
            classes: BTreeSet::new(),
            sources: BTreeMap::new(),
            first_seen_unix: None,
            last_seen_unix: None,
        }
    }

    /// Every source's contribution to a record lands in the record's own sets and
    /// under its source. Bounds are checked so a file cannot grow a record forever.
    fn absorb(
        record: &mut DeviceRecord,
        source: Source,
        fact: SourceFact,
        ips: impl IntoIterator<Item = IpAddr>,
    ) -> Result<(), UnifyError> {
        for ip in ips {
            if record.ips.len() >= MAX_IPS && !record.ips.contains(&ip) {
                return Err(UnifyError::TooLarge("device addresses"));
            }
            record.ips.insert(ip);
        }
        for name in &fact.names {
            if record.names.len() >= MAX_NAMES && !record.names.contains(name) {
                return Err(UnifyError::TooLarge("device names"));
            }
            record.names.insert(name.clone());
        }
        for class in &fact.classes {
            if record.classes.len() >= MAX_CLASSES && !record.classes.contains(class) {
                return Err(UnifyError::TooLarge("device classes"));
            }
            record.classes.insert(class.clone());
        }
        if fact.claims.len() > MAX_CLAIMS {
            return Err(UnifyError::TooLarge("source claims"));
        }
        record.first_seen_unix = record
            .first_seen_unix
            .into_iter()
            .chain(fact.first_seen_unix)
            .min();
        record.last_seen_unix = record
            .last_seen_unix
            .into_iter()
            .chain(fact.last_seen_unix)
            .max();
        record.sources.entry(source).or_insert(fact);
        Ok(())
    }
}

/// Merges the three sources into one record per device. Any source may be absent;
/// a command run with only one of them still shows what that one knows.
///
/// `generated_unix` is passed in so the merge itself is deterministic.
pub fn unify(
    inventory: Option<&LanInventory>,
    netwatch: Option<&NetwatchState>,
    passive: &[NetworkAssetObservation],
    generated_unix: u64,
) -> Result<UnifiedView, UnifyError> {
    let mut builder = Builder {
        by_mac: BTreeMap::new(),
        by_ip: BTreeMap::new(),
    };

    // The audit is the only source that knows hardware addresses: it seeds the
    // records, and its address history decides which record a scan or import
    // device joins. An address claimed by two devices over time belongs to the
    // one seen last; ties break by hardware-address order, so the merge is
    // deterministic even then.
    let mut claimed: BTreeMap<IpAddr, (u64, &str)> = BTreeMap::new();
    if let Some(state) = netwatch {
        for (mac, device) in &state.devices {
            let mut record = Builder::fresh(mac.clone(), Some(mac.clone()));
            let mut fact = SourceFact {
                first_seen_unix: seconds(device.first_seen_ms),
                last_seen_unix: seconds(device.last_seen_ms),
                ..SourceFact::default()
            };
            if let Some(hostname) = &device.hostname {
                fact.names.insert(hostname.clone());
            }
            fact.claims
                .insert("frames".to_string(), device.frames.to_string());
            fact.claims
                .insert("bytes".to_string(), device.bytes.to_string());
            Builder::absorb(
                &mut record,
                Source::Netwatch,
                fact,
                device.addresses.iter().copied(),
            )?;
            builder.record(record)?;
            for ip in &device.addresses {
                match claimed.get(ip) {
                    Some((seen, _)) if *seen > device.last_seen_ms => {}
                    _ => {
                        claimed.insert(*ip, (device.last_seen_ms, mac));
                    }
                }
            }
        }
    }
    let claim = |ip: &IpAddr| claimed.get(ip).map(|(_, mac)| *mac);

    if let Some(inventory) = inventory {
        for device in &inventory.devices {
            let ip: IpAddr = device
                .address
                .parse()
                .map_err(|_| UnifyError::Malformed("inventory device address"))?;
            let cache_only = !device.seen_by.is_empty()
                && device
                    .seen_by
                    .iter()
                    .all(|source| source == "neighbor-cache");
            let observed_at = (!cache_only).then_some(inventory.scanned_unix);
            let mut fact = SourceFact {
                first_seen_unix: observed_at,
                last_seen_unix: observed_at,
                ..SourceFact::default()
            };
            if !device.name.is_empty() {
                fact.names.insert(device.name.clone());
            }
            if let Some(class) = &device.class {
                fact.classes.insert(class.clone());
            }
            for (key, value) in [
                ("manufacturer", &device.manufacturer),
                ("model", &device.model),
            ] {
                if !value.is_empty() {
                    fact.claims.insert(key.to_string(), value.clone());
                }
            }
            if !device.services.is_empty() {
                fact.claims
                    .insert("services".to_string(), device.services.join(","));
            }
            if !device.seen_by.is_empty() {
                fact.claims
                    .insert("seen_by".to_string(), device.seen_by.join(","));
            }
            // What a device says it is doing (a Roku's power and programme) stays,
            // bounded like everything else.
            for (key, value) in device.state.iter().take(16) {
                fact.claims.insert(format!("state.{key}"), value.clone());
            }
            match claim(&ip) {
                Some(mac) => {
                    let record = builder
                        .by_mac
                        .get_mut(mac)
                        .ok_or(UnifyError::Malformed("claim without record"))?;
                    Builder::absorb(record, Source::LanScan, fact, [ip])?;
                }
                None => {
                    let mut record = Builder::fresh(format!("ip:{ip}"), None);
                    Builder::absorb(&mut record, Source::LanScan, fact, [ip])?;
                    builder.record(record)?;
                }
            }
        }
    }

    for observation in passive {
        let ip = IpAddr::V4(observation.ipv4_address);
        let mut fact = SourceFact::default();
        for sourced in &observation.evidence {
            fact.classes.insert(evidence_name(&sourced.evidence));
        }
        match claim(&ip) {
            Some(mac) => {
                let record = builder
                    .by_mac
                    .get_mut(mac)
                    .ok_or(UnifyError::Malformed("claim without record"))?;
                Builder::absorb(record, Source::PassiveImport, fact, [ip])?;
            }
            None => {
                if let Some(record) = builder.by_ip.get_mut(&ip) {
                    // The scan already listed this address: same device, one record.
                    Builder::absorb(record, Source::PassiveImport, fact, [ip])?;
                } else {
                    let mut record = Builder::fresh(format!("ip:{ip}"), None);
                    Builder::absorb(&mut record, Source::PassiveImport, fact, [ip])?;
                    builder.record(record)?;
                }
            }
        }
    }

    let devices = builder
        .by_mac
        .into_values()
        .chain(builder.by_ip.into_values())
        .collect();
    Ok(UnifiedView {
        format: FORMAT.to_string(),
        generated_unix,
        devices,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control::{NetworkEvidenceSource, SourcedNetworkEvidence};

    const TV_MAC: &str = "54:4e:f0:07:3d:e8";
    const PRINTER_MAC: &str = "aa:bb:cc:00:11:22";

    fn inventory_json() -> String {
        serde_json::json!({
            "format": "archie-lan-inventory-v1",
            "scanned_unix": 1_790_000_000u64,
            "devices": [
                {"address": "192.168.1.40", "class": "television", "name": "Living", "manufacturer": "Roku",
                 "model": "3941X", "services": ["_airplay._tcp"], "seen_by": ["ssdp", "mdns"], "state": {"showing": "Netflix"}},
                {"address": "192.168.1.60", "class": "printer", "name": "Oficina", "manufacturer": "Brother",
                 "model": "HL-L2350DW", "services": ["_ipp._tcp"], "seen_by": ["mdns"], "state": {}}
            ]
        })
        .to_string()
    }

    fn netwatch_json() -> String {
        serde_json::json!({
            "format": "archie-netwatch-state-v1",
            "started_ms": 1_789_999_000_000u64,
            "devices": {
                TV_MAC: {"first_seen_ms": 1_789_999_000_000u64, "last_seen_ms": 1_790_000_005_000u64,
                         "addresses": ["192.168.1.40"], "hostname": "roku-living", "frames": 12, "bytes": 3456,
                         "asked": ["api.roku.com"], "sent_to": {"api.roku.com": 3456}},
                PRINTER_MAC: {"first_seen_ms": 1_789_999_500_000u64, "last_seen_ms": 1_790_000_003_000u64,
                              "addresses": ["192.168.1.60"], "hostname": null, "frames": 4, "bytes": 1024,
                              "asked": [], "sent_to": {}}
            },
            "owners": {"192.168.1.40": TV_MAC}
        })
        .to_string()
    }

    fn passive() -> Vec<NetworkAssetObservation> {
        let observed = |ip: &str, evidence: NetworkEvidence| NetworkAssetObservation {
            ipv4_address: ip.parse().unwrap(),
            evidence: vec![SourcedNetworkEvidence {
                source: NetworkEvidenceSource::PassiveMdns,
                evidence,
            }],
        };
        vec![
            observed("192.168.1.40", NetworkEvidence::MdnsRoku),
            observed("192.168.1.60", NetworkEvidence::MdnsIpp),
        ]
    }

    #[test]
    fn three_sources_describing_one_device_produce_one_record_with_attribution() {
        let inventory = parse_inventory(&inventory_json()).unwrap();
        let netwatch = parse_netwatch_state(&netwatch_json()).unwrap();
        let view = unify(Some(&inventory), Some(&netwatch), &passive(), 1_790_000_100).unwrap();

        assert_eq!(view.format, FORMAT);
        assert_eq!(
            view.devices.len(),
            2,
            "the television is one device, not three"
        );
        let tv = &view.devices[0];
        assert_eq!(tv.mac.as_deref(), Some(TV_MAC));
        assert_eq!(tv.identity, TV_MAC);
        assert_eq!(tv.ips, BTreeSet::from(["192.168.1.40".parse().unwrap()]));
        assert_eq!(
            tv.names,
            BTreeSet::from(["Living".to_string(), "roku-living".to_string()])
        );
        assert!(tv.classes.contains("television") && tv.classes.contains("mdns-roku"));
        // Every source said something, under its own name, with its own times.
        assert_eq!(
            view.devices[0].sources.keys().copied().collect::<Vec<_>>(),
            [Source::LanScan, Source::Netwatch, Source::PassiveImport]
        );
        let scan = &tv.sources[&Source::LanScan];
        assert_eq!(scan.claims["manufacturer"], "Roku");
        assert_eq!(scan.claims["state.showing"], "Netflix");
        assert_eq!(scan.first_seen_unix, Some(1_790_000_000));
        let watch = &tv.sources[&Source::Netwatch];
        assert_eq!(watch.claims["frames"], "12");
        assert_eq!(watch.last_seen_unix, Some(1_790_000_005));
        assert_eq!(
            tv.first_seen_unix,
            Some(1_789_999_000),
            "earliest across sources"
        );
        assert_eq!(tv.last_seen_unix, Some(1_790_000_005));
        // The audit's DNS and destination records stay in the audit, not the view.
        let json = serde_json::to_string(&view).unwrap();
        assert!(!json.contains("api.roku.com"));

        let printer = &view.devices[1];
        assert_eq!(printer.mac.as_deref(), Some(PRINTER_MAC));
        assert_eq!(printer.sources.len(), 3);
        assert!(printer.classes.contains("printer") && printer.classes.contains("mdns-ipp"));
    }

    #[test]
    fn two_hardware_addresses_stay_two_devices() {
        let netwatch = parse_netwatch_state(&netwatch_json()).unwrap();
        let view = unify(None, Some(&netwatch), &[], 0).unwrap();
        assert_eq!(view.devices.len(), 2);
        assert_ne!(view.devices[0].mac, view.devices[1].mac);
        assert_eq!(
            view.devices[0].sources.keys().collect::<Vec<_>>(),
            [&Source::Netwatch]
        );
    }

    #[test]
    fn devices_known_only_by_address_never_merge_with_each_other() {
        // Neither address is claimed by any audited device: two records, not one.
        let view = unify(
            Some(&parse_inventory(&inventory_json()).unwrap()),
            None,
            &passive(),
            0,
        )
        .unwrap();
        assert_eq!(view.devices.len(), 2);
        for device in &view.devices {
            assert_eq!(device.mac, None);
            assert!(
                device.identity.starts_with("ip:"),
                "the weaker identity is marked"
            );
            assert_eq!(
                device.identity,
                format!("ip:{}", device.ips.first().unwrap())
            );
            // The scan and the import describe the same address: one record, two sources.
            assert_eq!(
                device.sources.keys().copied().collect::<Vec<_>>(),
                [Source::LanScan, Source::PassiveImport]
            );
        }
    }

    #[test]
    fn an_address_claimed_by_two_devices_belongs_to_the_one_seen_last() {
        let state = serde_json::json!({
            "format": "archie-netwatch-state-v1", "started_ms": 1u64,
            "devices": {
                TV_MAC: {"first_seen_ms": 1u64, "last_seen_ms": 5_000u64, "addresses": ["192.168.1.40"], "hostname": null,
                         "frames": 1, "bytes": 1, "asked": [], "sent_to": {}},
                PRINTER_MAC: {"first_seen_ms": 1u64, "last_seen_ms": 9_000u64, "addresses": ["192.168.1.40"], "hostname": null,
                              "frames": 1, "bytes": 1, "asked": [], "sent_to": {}}
            },
            "owners": {}
        })
        .to_string();
        let scan = serde_json::json!({
            "format": "archie-lan-inventory-v1", "scanned_unix": 1u64,
            "devices": [{"address": "192.168.1.40", "class": null, "name": "x", "manufacturer": "", "model": "",
                         "services": [], "seen_by": [], "state": {}}]
        })
        .to_string();
        let view = unify(
            Some(&parse_inventory(&scan).unwrap()),
            Some(&parse_netwatch_state(&state).unwrap()),
            &[],
            0,
        )
        .unwrap();
        assert_eq!(view.devices.len(), 2);
        let winner = view
            .devices
            .iter()
            .find(|d| d.sources.contains_key(&Source::LanScan))
            .unwrap();
        assert_eq!(
            winner.mac.as_deref(),
            Some(PRINTER_MAC),
            "seen last at 9000 ms"
        );
    }

    #[test]
    fn malformed_and_oversize_inputs_are_rejected_without_panicking() {
        assert!(parse_inventory("not json").is_err());
        assert!(parse_inventory(
            &serde_json::json!({"format": "other", "scanned_unix": 0, "devices": []}).to_string()
        )
        .is_err());
        // A field the writer never emits means the file is not trusted as one.
        let extra = serde_json::json!({"format": "archie-lan-inventory-v1", "scanned_unix": 0, "devices": [], "surprise": 1});
        assert!(parse_inventory(&extra.to_string()).is_err());
        let bad_address = serde_json::json!({"format": "archie-lan-inventory-v1", "scanned_unix": 0, "devices": [
            {"address": "not-an-ip", "class": null, "name": "", "manufacturer": "", "model": "", "services": [], "seen_by": [], "state": {}}]});
        assert!(parse_inventory(&bad_address.to_string()).is_err());
        let long_name = serde_json::json!({"format": "archie-lan-inventory-v1", "scanned_unix": 0, "devices": [
            {"address": "192.168.1.9", "class": null, "name": "n".repeat(MAX_FIELD_CHARS + 1), "manufacturer": "", "model": "",
             "services": [], "seen_by": [], "state": {}}]});
        assert_eq!(
            parse_inventory(&long_name.to_string()),
            Err(UnifyError::TooLarge("inventory device name"))
        );
        assert!(parse_inventory(&" ".repeat(MAX_INVENTORY_BYTES + 1)).is_err());

        assert!(parse_netwatch_state("not json").is_err());
        let bad_mac = serde_json::json!({"format": "archie-netwatch-state-v1", "started_ms": 0, "owners": {}, "devices": {
            "54:4E:F0:07:3D:E8": {"first_seen_ms": 0, "last_seen_ms": 0, "addresses": [], "hostname": null,
                                  "frames": 0, "bytes": 0, "asked": [], "sent_to": {}}}});
        assert!(
            parse_netwatch_state(&bad_mac.to_string()).is_err(),
            "uppercase is not what the writer emits"
        );
        let too_many: serde_json::Value = serde_json::json!({"format": "archie-lan-inventory-v1", "scanned_unix": 0, "devices":
            vec![serde_json::json!({"address": "192.168.1.9", "class": null, "name": "", "manufacturer": "", "model": "",
                                    "services": [], "seen_by": [], "state": {}}); MAX_DEVICES + 1]});
        assert_eq!(
            parse_inventory(&too_many.to_string()),
            Err(UnifyError::TooLarge("lan inventory devices"))
        );
    }

    #[test]
    fn the_view_serializes_and_parses_back_exactly() {
        let view = unify(
            Some(&parse_inventory(&inventory_json()).unwrap()),
            Some(&parse_netwatch_state(&netwatch_json()).unwrap()),
            &passive(),
            1_790_000_100,
        )
        .unwrap();
        let text = serde_json::to_string_pretty(&view).unwrap();
        assert_eq!(serde_json::from_str::<UnifiedView>(&text).unwrap(), view);
        // Source names in the document are the ones the CLI prints.
        assert!(
            text.contains("\"lan-scan\"")
                && text.contains("\"netwatch\"")
                && text.contains("\"passive-import\"")
        );
    }

    #[test]
    fn every_source_may_be_absent_and_empty_is_empty() {
        let view = unify(None, None, &[], 7).unwrap();
        assert!(view.devices.is_empty());
        assert_eq!(view.generated_unix, 7);
        let scan_only = unify(
            Some(&parse_inventory(&inventory_json()).unwrap()),
            None,
            &[],
            0,
        )
        .unwrap();
        assert_eq!(scan_only.devices.len(), 2);
        assert!(scan_only.devices.iter().all(|d| d.mac.is_none()));
    }
}
