//! The continuous audit: what the frames say about the devices on the network, and the
//! few things in them worth telling the owner.
//!
//! It keeps, for each device, when it was seen, the addresses and name it used, how much
//! it sent, the names it asked DNS for and whom it talked to outside. From that it raises
//! alerts. Each is a reason to look, with what was seen; none is a verdict, and nothing
//! here acts on a device.
//!
//! | Rule | Raised when |
//! |---|---|
//! | `new-device` | a hardware address not seen before sends from a LAN address |
//! | `address-moved` | an IPv4 address is claimed over ARP by another device than before; a fight over one address is one alert, raised again only after a quiet hour, counting the moves |
//! | `port-scan` | one device opens connections to 100 ports of one host within a minute |
//! | `host-sweep` | one device opens the same port on 50 LAN hosts within a minute |
//! | `cleartext-login` | a device opens Telnet or FTP, which carry passwords in the clear |
//! | `dns-tunnel-suspect` | a DNS question is long and random enough to be carrying data |
//!
//! Time is the frames' own, so a capture file is audited exactly as the live network is.

use crate::capture::Packet;
use crate::is_lan;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::net::{IpAddr, Ipv4Addr};

const MAX_DEVICES: usize = 4096;
const MAX_NAMES: usize = 512;
const MAX_PEERS: usize = 64;
const MAX_ADDRESSES: usize = 16;
const MAX_REMEMBERED: usize = 16_384;
const MAX_NAME_CHARS: usize = 1024;
/// A state file larger than this is not a state this program wrote. A real one is
/// kilobytes; every device of a full one with long names fits well inside.
const MAX_STATE_BYTES: u64 = 256 * 1024 * 1024;
/// Room for the most remembered alerts with long keys, times two.
const MAX_TOLD_BYTES: u64 = 16 * 1024 * 1024;
/// An address that keeps changing hands is told again after this much quiet.
const MOVED_COOLDOWN_MS: u64 = 60 * 60 * 1000;
const WINDOW_MS: u64 = 60_000;
const SCAN_PORTS: usize = 100;
const SWEEP_HOSTS: usize = 50;
const STATE_FORMAT: &str = "archie-netwatch-state-v1";
const TOLD_FORMAT: &str = "archie-netwatch-told-v1";

/// One device, by the hardware address it sends from.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Device {
    pub first_seen_ms: u64,
    pub last_seen_ms: u64,
    pub addresses: BTreeSet<IpAddr>,
    pub hostname: Option<String>,
    pub frames: u64,
    pub bytes: u64,
    /// Names it asked DNS for.
    pub asked: BTreeSet<String>,
    /// Bytes sent to each place outside the LAN, by server name where one was seen.
    pub sent_to: BTreeMap<String, u64>,
}

/// What survives a restart.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct State {
    pub format: String,
    pub started_ms: u64,
    pub devices: BTreeMap<String, Device>,
    /// Which device last claimed each IPv4 address over ARP.
    pub owners: BTreeMap<Ipv4Addr, String>,
}

/// The most a file on disk may hold: the same caps the audit applies as it learns, plus
/// a byte cap on the file itself and a length cap on each stored name. Loading enforces
/// them, so a tampered file is refused with the cap it broke instead of loaded whole.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub state_bytes: u64,
    pub told_bytes: u64,
    pub devices: usize,
    pub names: usize,
    pub peers: usize,
    pub addresses: usize,
    pub name_chars: usize,
    pub told: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            state_bytes: MAX_STATE_BYTES,
            told_bytes: MAX_TOLD_BYTES,
            devices: MAX_DEVICES,
            names: MAX_NAMES,
            peers: MAX_PEERS,
            addresses: MAX_ADDRESSES,
            name_chars: MAX_NAME_CHARS,
            told: MAX_REMEMBERED,
        }
    }
}

/// Why a file on disk is not one this program will use. Each case names what is wrong:
/// the cap that was broken, or the way the file is not the JSON this program writes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadError {
    /// The file is larger than the byte cap for it.
    TooLarge { bytes: u64, cap: u64 },
    /// The file is not the JSON this program writes.
    Malformed(String),
    /// A format marker written by another version.
    UnknownFormat { found: String },
    /// More of one thing than the audit keeps.
    OverCap {
        what: &'static str,
        found: usize,
        cap: usize,
    },
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoadError::TooLarge { bytes, cap } => write!(
                f,
                "the file is {bytes} bytes; the most it may hold is {cap}"
            ),
            LoadError::Malformed(error) => write!(f, "not the JSON this program writes: {error}"),
            LoadError::UnknownFormat { found } => {
                write!(f, "the format {found:?} is not one this version reads")
            }
            LoadError::OverCap { what, found, cap } => write!(
                f,
                "{found} {what} is more than the {cap} this program keeps"
            ),
        }
    }
}

impl std::error::Error for LoadError {}

fn over(cap: usize, found: usize, what: &'static str) -> Result<(), LoadError> {
    if found > cap {
        Err(LoadError::OverCap { what, found, cap })
    } else {
        Ok(())
    }
}

impl State {
    /// Reads a saved state, refusing one over any cap the audit keeps. A state this
    /// program saved always passes; a tampered one fails naming the cap it broke.
    pub fn parse(bytes: &[u8]) -> Result<State, LoadError> {
        Self::parse_within(bytes, &Limits::default())
    }

    /// The same, against other limits — for proving the limits hold.
    pub fn parse_within(bytes: &[u8], limits: &Limits) -> Result<State, LoadError> {
        if bytes.len() as u64 > limits.state_bytes {
            return Err(LoadError::TooLarge {
                bytes: bytes.len() as u64,
                cap: limits.state_bytes,
            });
        }
        let state: State =
            serde_json::from_slice(bytes).map_err(|e| LoadError::Malformed(e.to_string()))?;
        state.check(limits)?;
        Ok(state)
    }

    fn check(&self, limits: &Limits) -> Result<(), LoadError> {
        if !self.format.is_empty() && self.format != STATE_FORMAT {
            return Err(LoadError::UnknownFormat {
                found: self.format.clone(),
            });
        }
        over(limits.devices, self.devices.len(), "devices")?;
        over(limits.devices, self.owners.len(), "addresses with an owner")?;
        for device in self.devices.values() {
            over(
                limits.addresses,
                device.addresses.len(),
                "addresses of one device",
            )?;
            over(
                limits.names,
                device.asked.len(),
                "names asked by one device",
            )?;
            over(
                limits.peers,
                device.sent_to.len(),
                "outside peers of one device",
            )?;
            for name in device
                .asked
                .iter()
                .chain(device.sent_to.keys())
                .chain(device.hostname.iter())
            {
                over(limits.name_chars, name.len(), "characters in one name")?;
            }
        }
        Ok(())
    }
}

/// One remembered alert: when it was last raised, and for `address-moved` how many moves
/// of its address have been seen in all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToldRecord {
    pub at_ms: u64,
    pub count: u64,
}

/// The alerts already raised. It is kept on disk next to the state, so a restart does
/// not repeat them. It is bounded like everything here: past the cap, the oldest is
/// forgotten first.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Told {
    pub format: String,
    pub records: BTreeMap<String, ToldRecord>,
}

impl Default for Told {
    fn default() -> Self {
        Self {
            format: TOLD_FORMAT.into(),
            records: BTreeMap::new(),
        }
    }
}

impl Told {
    /// Reads a saved `told.json`, held to the same bounds as the live one.
    pub fn parse(bytes: &[u8]) -> Result<Told, LoadError> {
        Self::parse_within(bytes, &Limits::default())
    }

    /// The same, against other limits — for proving the limits hold.
    pub fn parse_within(bytes: &[u8], limits: &Limits) -> Result<Told, LoadError> {
        if bytes.len() as u64 > limits.told_bytes {
            return Err(LoadError::TooLarge {
                bytes: bytes.len() as u64,
                cap: limits.told_bytes,
            });
        }
        let told: Told =
            serde_json::from_slice(bytes).map_err(|e| LoadError::Malformed(e.to_string()))?;
        if !told.format.is_empty() && told.format != TOLD_FORMAT {
            return Err(LoadError::UnknownFormat { found: told.format });
        }
        over(limits.told, told.records.len(), "remembered alerts")?;
        Ok(told)
    }

    /// True the first time a key is told, and not again while it is remembered.
    fn is_new(&mut self, key: &str, at_ms: u64) -> bool {
        if self.records.contains_key(key) {
            return false;
        }
        self.make_room();
        self.records
            .insert(key.to_string(), ToldRecord { at_ms, count: 1 });
        true
    }

    /// `address-moved`: `Some` of every move seen so far when it should be told — the
    /// first move, and later ones only once an hour of quiet has passed with the count
    /// still growing — and `None` while the fight goes on inside the hour. So two devices
    /// fighting over an address are one alert whose number keeps growing, not a stream
    /// and not a silence.
    fn moved(&mut self, key: &str, at_ms: u64) -> Option<u64> {
        match self.records.get_mut(key) {
            Some(record) => {
                record.count = record.count.saturating_add(1);
                if at_ms.saturating_sub(record.at_ms) >= MOVED_COOLDOWN_MS {
                    record.at_ms = at_ms;
                    Some(record.count)
                } else {
                    None
                }
            }
            None => {
                self.make_room();
                self.records
                    .insert(key.to_string(), ToldRecord { at_ms, count: 1 });
                Some(1)
            }
        }
    }

    /// Room for one more, forgetting the oldest first.
    fn make_room(&mut self) {
        if self.records.len() < MAX_REMEMBERED {
            return;
        }
        let oldest = self
            .records
            .iter()
            .min_by_key(|(key, record)| (record.at_ms, *key))
            .map(|(key, _)| key.clone());
        if let Some(oldest) = oldest {
            self.records.remove(&oldest);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Notice,
    Warning,
    High,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Alert {
    pub at_ms: u64,
    pub severity: Severity,
    pub rule: String,
    /// The hardware address of the device it is about.
    pub device: String,
    pub detail: String,
}

#[derive(Debug, Clone, Default)]
pub struct Settings {
    /// For this long after the first frame, devices are learned without `new-device`.
    pub learn_ms: u64,
    /// The router. Another device claiming its address is the serious case of `address-moved`.
    pub gateway: Option<Ipv4Addr>,
}

#[derive(Default)]
struct Openings {
    since_ms: u64,
    ports_of: HashMap<IpAddr, HashSet<u16>>,
    hosts_on: HashMap<u16, HashSet<IpAddr>>,
}

pub struct Audit {
    pub state: State,
    settings: Settings,
    openings: HashMap<String, Openings>,
    /// Server names seen for outside addresses, to label the traffic that follows.
    named: HashMap<IpAddr, String>,
    /// The alerts already raised, kept on disk so a restart does not repeat them.
    pub told: Told,
}

/// Shannon entropy of a label, in bits per character.
fn entropy(label: &str) -> f64 {
    let mut counts = [0usize; 256];
    for byte in label.bytes() {
        counts[byte as usize] += 1;
    }
    let total = label.len() as f64;
    counts
        .iter()
        .filter(|&&n| n > 0)
        .map(|&n| n as f64 / total)
        .map(|p| -p * p.log2())
        .sum()
}

/// Whether a DNS question looks like data in disguise: very long, or one long label of
/// near-random characters. A CDN's hashed host name is long; it is not this random.
fn carries_data(question: &str) -> bool {
    question.len() > 180
        || question
            .split('.')
            .any(|label| label.len() >= 40 && entropy(label) >= 4.0)
}

impl Audit {
    pub fn new(mut state: State, settings: Settings) -> Self {
        state.format = STATE_FORMAT.into();
        Self {
            state,
            settings,
            openings: HashMap::new(),
            named: HashMap::new(),
            told: Told::default(),
        }
    }

    fn tell(
        &mut self,
        alerts: &mut Vec<Alert>,
        key: String,
        at_ms: u64,
        severity: Severity,
        rule: &str,
        device: &str,
        detail: String,
    ) {
        if self.told.is_new(&key, at_ms) {
            alerts.push(Alert {
                at_ms,
                severity,
                rule: rule.into(),
                device: device.into(),
                detail,
            });
        }
    }

    /// Takes one frame into account and returns what it gives reason to tell.
    pub fn see(&mut self, packet: &Packet) -> Vec<Alert> {
        let mut alerts = Vec::new();
        if self.state.started_ms == 0 {
            self.state.started_ms = packet.at_ms.max(1);
        }
        if let Some((mac, address)) = &packet.arp {
            self.claim(&mut alerts, packet.at_ms, mac, *address);
        }
        // A frame from outside the LAN was put on the wire by the router: its hardware
        // address is the router's, and the traffic is not the router's own.
        let Some((mac, source)) = packet
            .source_mac
            .as_deref()
            .zip(packet.source)
            .filter(|(_, ip)| is_lan(*ip))
        else {
            return alerts;
        };
        if !self.state.devices.contains_key(mac) {
            if self.state.devices.len() >= MAX_DEVICES {
                return alerts;
            }
            if packet.at_ms.saturating_sub(self.state.started_ms) >= self.settings.learn_ms {
                self.tell(
                    &mut alerts,
                    format!("new {mac}"),
                    packet.at_ms,
                    Severity::Notice,
                    "new-device",
                    mac,
                    format!("first seen, at {source}"),
                );
            }
        }
        let outside = packet
            .destination
            .filter(|ip| !is_lan(*ip) && !ip.is_multicast() && !ip.is_loopback());
        if let Some((peer, name)) = outside.zip(packet.server_name.clone()) {
            if self.named.len() >= MAX_REMEMBERED {
                self.named.clear();
            }
            self.named.insert(peer, name);
        }
        let device = self
            .state
            .devices
            .entry(mac.to_string())
            .or_insert_with(|| Device {
                first_seen_ms: packet.at_ms,
                ..Device::default()
            });
        device.last_seen_ms = packet.at_ms;
        device.frames += 1;
        device.bytes += packet.length;
        if device.addresses.len() < 16 {
            device.addresses.insert(source);
        }
        if let Some(hostname) = &packet.dhcp_hostname {
            device.hostname = Some(hostname.clone());
        }
        if let Some(peer) = outside {
            let label = self
                .named
                .get(&peer)
                .cloned()
                .unwrap_or_else(|| peer.to_string());
            if device.sent_to.len() < MAX_PEERS || device.sent_to.contains_key(&label) {
                *device.sent_to.entry(label).or_default() += packet.length;
            }
        }
        if let Some(question) = &packet.dns_question {
            if device.asked.len() < MAX_NAMES {
                device.asked.insert(question.clone());
            }
            if carries_data(question) {
                // By the zone, so one tunnel is one alert and not one per question.
                let labels: Vec<&str> = question.split('.').collect();
                let zone = labels[labels.len().saturating_sub(2)..].join(".");
                let shown: String = question.chars().take(80).collect();
                self.tell(
                    &mut alerts,
                    format!("tunnel {mac} {zone}"),
                    packet.at_ms,
                    Severity::Warning,
                    "dns-tunnel-suspect",
                    mac,
                    format!("asked for {shown} under {zone}"),
                );
            }
        }
        if packet.opening {
            if let Some(((_, port), destination)) = packet.tcp.zip(packet.destination) {
                self.opened(&mut alerts, packet.at_ms, mac, destination, port);
            }
        }
        alerts
    }

    fn claim(&mut self, alerts: &mut Vec<Alert>, at_ms: u64, mac: &str, address: Ipv4Addr) {
        if !crate::is_lan_v4(address) {
            return;
        }
        match self.state.owners.get(&address).cloned() {
            Some(before) if before != mac => {
                let gateway = self.settings.gateway == Some(address);
                let (severity, what) = if gateway {
                    (Severity::High, "the router's address is now claimed by another device; traffic may be passing through it")
                } else {
                    (
                        Severity::Notice,
                        "address now claimed by another device; usually a new DHCP lease",
                    )
                };
                // One alert per address, not one per pair of claimants: it is raised
                // again only after a quiet hour, and it counts every move seen.
                if let Some(count) = self.told.moved(&format!("moved {address}"), at_ms) {
                    let detail = if count == 1 {
                        format!("{address}: {what} (was {before})")
                    } else {
                        format!("{address}: {what}; it has now changed hands {count} times, most lately from {before}")
                    };
                    alerts.push(Alert {
                        at_ms,
                        severity,
                        rule: "address-moved".into(),
                        device: mac.into(),
                        detail,
                    });
                }
                self.state.owners.insert(address, mac.to_string());
            }
            Some(_) => {}
            None if self.state.owners.len() < MAX_DEVICES => {
                self.state.owners.insert(address, mac.to_string());
            }
            None => {}
        }
    }

    fn opened(
        &mut self,
        alerts: &mut Vec<Alert>,
        at_ms: u64,
        mac: &str,
        destination: IpAddr,
        port: u16,
    ) {
        if matches!(port, 21 | 23) {
            let protocol = if port == 23 { "Telnet" } else { "FTP" };
            self.tell(
                alerts,
                format!("clear {mac} {destination} {port}"),
                at_ms,
                Severity::Warning,
                "cleartext-login",
                mac,
                format!(
                    "opened {protocol} to {destination}; its passwords are readable on the wire"
                ),
            );
        }
        if self.openings.len() >= MAX_DEVICES && !self.openings.contains_key(mac) {
            return;
        }
        let window = self.openings.entry(mac.to_string()).or_default();
        if at_ms.saturating_sub(window.since_ms) > WINDOW_MS {
            *window = Openings {
                since_ms: at_ms,
                ..Openings::default()
            };
        }
        let ports = window.ports_of.entry(destination).or_default();
        let scanned = ports.insert(port) && ports.len() == SCAN_PORTS;
        let swept = is_lan(destination) && {
            let hosts = window.hosts_on.entry(port).or_default();
            hosts.insert(destination) && hosts.len() == SWEEP_HOSTS
        };
        let since = window.since_ms;
        if scanned {
            self.tell(
                alerts,
                format!("scan {mac} {destination} {since}"),
                at_ms,
                Severity::High,
                "port-scan",
                mac,
                format!("opened {SCAN_PORTS} ports of {destination} within a minute"),
            );
        }
        if swept {
            self.tell(
                alerts,
                format!("sweep {mac} {port} {since}"),
                at_ms,
                Severity::High,
                "host-sweep",
                mac,
                format!("opened port {port} on {SWEEP_HOSTS} hosts of the network within a minute"),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TV: &str = "54:4e:f0:07:3d:e8";
    const ROUTER: &str = "e4:ab:89:6e:1a:18";

    fn from(mac: &str, source: &str, at_ms: u64) -> Packet {
        Packet {
            at_ms,
            length: 100,
            source_mac: Some(mac.into()),
            source: Some(source.parse().unwrap()),
            ..Packet::default()
        }
    }

    /// A network whose devices are all taken as known, to look at one rule at a time.
    fn learned() -> Settings {
        Settings {
            learn_ms: u64::MAX,
            gateway: None,
        }
    }

    fn rules(alerts: &[Alert]) -> Vec<&str> {
        alerts.iter().map(|a| a.rule.as_str()).collect()
    }

    #[test]
    fn a_device_is_learned_quietly_at_first_and_announced_afterwards() {
        let mut audit = Audit::new(
            State::default(),
            Settings {
                learn_ms: 60_000,
                gateway: None,
            },
        );
        assert!(audit.see(&from(TV, "192.168.1.84", 1_000)).is_empty());
        let late = audit.see(&from("aa:bb:cc:00:11:22", "192.168.1.90", 70_000));
        assert_eq!(rules(&late), ["new-device"]);
        assert!(
            audit
                .see(&from("aa:bb:cc:00:11:22", "192.168.1.90", 71_000))
                .is_empty(),
            "told once"
        );
        assert_eq!(audit.state.devices.len(), 2);
        assert_eq!(audit.state.devices[TV].bytes, 100);
    }

    #[test]
    fn traffic_from_the_internet_is_not_a_device_on_the_lan() {
        let mut audit = Audit::new(State::default(), learned());
        // The router forwards a frame from outside; its hardware address is on it.
        assert!(audit.see(&from(ROUTER, "8.8.8.8", 5_000)).is_empty());
        assert!(audit.state.devices.is_empty());
    }

    #[test]
    fn another_device_claiming_the_router_s_address_is_the_serious_case() {
        let gateway: Ipv4Addr = "192.168.1.1".parse().unwrap();
        let mut audit = Audit::new(
            State::default(),
            Settings {
                learn_ms: u64::MAX,
                gateway: Some(gateway),
            },
        );
        let claim = |mac: &str, ip: &str, at_ms| Packet {
            at_ms,
            arp: Some((mac.into(), ip.parse().unwrap())),
            ..Packet::default()
        };
        assert!(audit.see(&claim(ROUTER, "192.168.1.1", 1_000)).is_empty());
        assert!(audit.see(&claim(TV, "192.168.1.84", 1_100)).is_empty());
        let spoofed = audit.see(&claim("de:ad:be:ef:00:02", "192.168.1.1", 2_000));
        assert_eq!(
            (spoofed[0].rule.as_str(), spoofed[0].severity),
            ("address-moved", Severity::High)
        );
        let lease = audit.see(&claim("aa:bb:cc:00:11:22", "192.168.1.84", 3_000));
        assert_eq!(
            (lease[0].rule.as_str(), lease[0].severity),
            ("address-moved", Severity::Notice)
        );
    }

    #[test]
    fn many_ports_of_one_host_is_a_scan_and_one_port_of_many_hosts_a_sweep() {
        let mut audit = Audit::new(State::default(), learned());
        let open = |destination: String, port: u16, at_ms: u64| Packet {
            destination: Some(destination.parse().unwrap()),
            tcp: Some((50_000, port)),
            opening: true,
            ..from(TV, "192.168.1.84", at_ms)
        };
        let mut seen = Vec::new();
        for port in 1000..1150u16 {
            seen.extend(audit.see(&open("192.168.1.1".into(), port, 10_000 + u64::from(port))));
        }
        assert_eq!(
            rules(&seen),
            ["port-scan"],
            "once, when the hundredth port is opened"
        );

        let mut seen = Vec::new();
        for host in 2..80u8 {
            seen.extend(audit.see(&open(
                format!("192.168.1.{host}"),
                445,
                200_000 + u64::from(host),
            )));
        }
        assert_eq!(rules(&seen), ["host-sweep"]);

        // The same hundred ports over an hour is a device doing its work.
        let mut slow = Audit::new(State::default(), learned());
        let mut seen = Vec::new();
        for port in 1000..1150u16 {
            seen.extend(slow.see(&open("192.168.1.1".into(), port, u64::from(port) * 40_000)));
        }
        assert!(seen.is_empty());
    }

    #[test]
    fn telnet_is_told_once_and_https_never() {
        let mut audit = Audit::new(State::default(), learned());
        let open = |port: u16, at_ms| Packet {
            destination: Some("192.168.1.1".parse().unwrap()),
            tcp: Some((50_000, port)),
            opening: true,
            ..from(TV, "192.168.1.84", at_ms)
        };
        assert_eq!(rules(&audit.see(&open(23, 1_000))), ["cleartext-login"]);
        assert!(audit.see(&open(23, 2_000)).is_empty());
        assert!(audit.see(&open(443, 3_000)).is_empty());
    }

    #[test]
    fn a_question_that_carries_data_is_told_and_an_ordinary_long_one_is_not() {
        let mut audit = Audit::new(State::default(), learned());
        let ask = |name: &str, at_ms| Packet {
            dns_question: Some(name.into()),
            ..from(TV, "192.168.1.84", at_ms)
        };
        assert!(audit
            .see(&ask("d3a1b2c4e5f6a7.cloudfront.net", 1_000))
            .is_empty());
        assert!(audit
            .see(&ask("connectivity-check.ubuntu.com", 1_100))
            .is_empty());
        let label = "q7x2mzk9v4p1tc8w0hjy3ns6bfd5rgae4lu7o2iq9xk";
        assert_eq!(
            rules(&audit.see(&ask(&format!("{label}.t.example.com"), 2_000))),
            ["dns-tunnel-suspect"]
        );
        assert!(
            audit
                .see(&ask(&format!("a{label}.t.example.com"), 2_100))
                .is_empty(),
            "one alert for the zone"
        );
        assert_eq!(audit.state.devices[TV].asked.len(), 4);
    }

    #[test]
    fn what_a_device_sends_outside_is_counted_under_the_server_s_name() {
        let mut audit = Audit::new(State::default(), learned());
        let to = |name: Option<&str>, at_ms| Packet {
            destination: Some("52.1.2.3".parse().unwrap()),
            server_name: name.map(String::from),
            ..from(TV, "192.168.1.84", at_ms)
        };
        audit.see(&to(Some("api.roku.com"), 1_000));
        audit.see(&to(None, 1_100));
        assert_eq!(
            audit.state.devices[TV].sent_to.get("api.roku.com"),
            Some(&200)
        );
        let saved = serde_json::to_string(&audit.state).unwrap();
        assert_eq!(serde_json::from_str::<State>(&saved).unwrap(), audit.state);
    }

    fn claim(mac: &str, ip: &str, at_ms: u64) -> Packet {
        Packet {
            at_ms,
            arp: Some((mac.into(), ip.parse().unwrap())),
            ..Packet::default()
        }
    }

    #[test]
    fn an_address_fought_over_is_one_alert_whose_count_grows() {
        let mut audit = Audit::new(State::default(), learned());
        assert!(
            audit.see(&claim(ROUTER, "192.168.1.1", 1_000)).is_empty(),
            "the first claimer is just learned"
        );
        let first = audit.see(&claim(TV, "192.168.1.1", 2_000));
        assert_eq!(rules(&first), ["address-moved"]);
        assert!(
            first[0].detail.contains("(was"),
            "the first move is told plainly: {}",
            first[0].detail
        );
        // The fight goes on inside the quiet hour: counted, not told again.
        assert!(audit.see(&claim(ROUTER, "192.168.1.1", 3_000)).is_empty());
        assert!(audit.see(&claim(TV, "192.168.1.1", 4_000)).is_empty());
        // After the hour, one alert — counting every move seen, not one per move.
        let again = audit.see(&claim(ROUTER, "192.168.1.1", 2_000 + MOVED_COOLDOWN_MS));
        assert_eq!(rules(&again), ["address-moved"]);
        assert!(
            again[0].detail.contains("4 times"),
            "every change counted: {}",
            again[0].detail
        );
        // And the number keeps growing rather than the alert going silent.
        let later = audit.see(&claim(TV, "192.168.1.1", 2_000 + 2 * MOVED_COOLDOWN_MS));
        assert_eq!(rules(&later), ["address-moved"]);
        assert!(later[0].detail.contains("5 times"), "{}", later[0].detail);
    }

    #[test]
    fn the_oldest_remembered_alert_is_the_first_forgotten() {
        let mut told = Told::default();
        for i in 0..MAX_REMEMBERED as u64 {
            assert!(told.is_new(&format!("alert {i}"), i));
        }
        assert_eq!(told.records.len(), MAX_REMEMBERED);
        assert!(told.is_new("alert new", MAX_REMEMBERED as u64));
        assert_eq!(told.records.len(), MAX_REMEMBERED, "the bound holds");
        assert!(
            !told.records.contains_key("alert 0"),
            "the oldest went first"
        );
        assert!(told.records.contains_key("alert 1"));
        assert!(!told.is_new("alert 1", 99_000), "still told");
    }

    #[test]
    fn what_was_told_is_still_told_after_a_restart() {
        let settings = || Settings {
            learn_ms: 0,
            gateway: None,
        };
        let telnet = Packet {
            destination: Some("192.168.1.1".parse().unwrap()),
            tcp: Some((50_000, 23)),
            opening: true,
            ..from(TV, "192.168.1.84", 2_000)
        };
        let mut first = Audit::new(State::default(), settings());
        assert_eq!(
            rules(&first.see(&from(TV, "192.168.1.84", 1_000))),
            ["new-device"]
        );
        assert_eq!(rules(&first.see(&telnet)), ["cleartext-login"]);
        // The process stops and starts: the state and what was told come back from disk.
        let state = serde_json::to_vec(&first.state).unwrap();
        let told = serde_json::to_vec(&first.told).unwrap();
        let mut second = Audit::new(State::parse(&state).unwrap(), settings());
        second.told = Told::parse(&told).unwrap();
        assert!(
            second.see(&telnet).is_empty(),
            "not told again after the restart"
        );
    }

    #[test]
    fn a_saved_state_and_a_saved_told_load_again() {
        let mut audit = Audit::new(
            State::default(),
            Settings {
                learn_ms: 0,
                gateway: None,
            },
        );
        let packet = Packet {
            dns_question: Some("connectivity-check.ubuntu.com".into()),
            destination: Some("52.1.2.3".parse().unwrap()),
            server_name: Some("api.roku.com".into()),
            ..from(TV, "192.168.1.84", 1_000)
        };
        audit.see(&packet);
        audit.see(&claim(TV, "192.168.1.84", 1_100));
        audit.see(&claim(ROUTER, "192.168.1.84", 1_200));
        let state = serde_json::to_vec(&audit.state).unwrap();
        assert_eq!(
            State::parse(&state).unwrap(),
            audit.state,
            "a saved state loads"
        );
        let told = serde_json::to_vec(&audit.told).unwrap();
        assert_eq!(
            Told::parse(&told).unwrap(),
            audit.told,
            "a saved told loads"
        );
    }

    #[test]
    fn a_state_beyond_each_cap_is_refused_naming_the_cap() {
        let mut audit = Audit::new(State::default(), learned());
        for (i, mac) in [TV, "aa:bb:cc:00:11:01", "aa:bb:cc:00:11:02"]
            .into_iter()
            .enumerate()
        {
            audit.see(&from(mac, "192.168.1.84", 1_000 + i as u64));
        }
        let bytes = serde_json::to_vec(&audit.state).unwrap();

        let tiny = Limits {
            state_bytes: 10,
            ..Limits::default()
        };
        assert_eq!(
            State::parse_within(&bytes, &tiny),
            Err(LoadError::TooLarge {
                bytes: bytes.len() as u64,
                cap: 10
            })
        );

        let few_devices = Limits {
            devices: 2,
            ..Limits::default()
        };
        assert_eq!(
            State::parse_within(&bytes, &few_devices),
            Err(LoadError::OverCap {
                what: "devices",
                found: 3,
                cap: 2
            })
        );

        let mut named = audit.state.clone();
        named.devices.get_mut(TV).unwrap().asked =
            (0..4).map(|i| format!("name-{i}.example.com")).collect();
        let bytes = serde_json::to_vec(&named).unwrap();
        let few_names = Limits {
            names: 3,
            ..Limits::default()
        };
        assert_eq!(
            State::parse_within(&bytes, &few_names),
            Err(LoadError::OverCap {
                what: "names asked by one device",
                found: 4,
                cap: 3
            })
        );

        let mut peering = audit.state.clone();
        peering.devices.get_mut(TV).unwrap().sent_to = (0..3)
            .map(|i| (format!("peer-{i}.example.com"), 100))
            .collect();
        let bytes = serde_json::to_vec(&peering).unwrap();
        let few_peers = Limits {
            peers: 2,
            ..Limits::default()
        };
        assert_eq!(
            State::parse_within(&bytes, &few_peers),
            Err(LoadError::OverCap {
                what: "outside peers of one device",
                found: 3,
                cap: 2
            })
        );

        let mut addressed = audit.state.clone();
        addressed.devices.get_mut(TV).unwrap().addresses =
            (0..17u8).map(|i| IpAddr::from([10, 0, 0, i])).collect();
        let bytes = serde_json::to_vec(&addressed).unwrap();
        assert_eq!(
            State::parse(&bytes),
            Err(LoadError::OverCap {
                what: "addresses of one device",
                found: 17,
                cap: 16
            })
        );

        let mut foreign = audit.state.clone();
        foreign.format = "archie-netwatch-state-v0".into();
        let bytes = serde_json::to_vec(&foreign).unwrap();
        assert_eq!(
            State::parse(&bytes),
            Err(LoadError::UnknownFormat {
                found: "archie-netwatch-state-v0".into()
            })
        );

        assert!(
            matches!(State::parse(b"{ not json"), Err(LoadError::Malformed(_))),
            "malformed is typed too"
        );
    }

    #[test]
    fn a_told_file_over_the_cap_is_refused_naming_the_cap() {
        let mut told = Told::default();
        for i in 0..3 {
            told.records
                .insert(format!("alert {i}"), ToldRecord { at_ms: i, count: 1 });
        }
        let bytes = serde_json::to_vec(&told).unwrap();
        let small = Limits {
            told: 2,
            ..Limits::default()
        };
        assert_eq!(
            Told::parse_within(&bytes, &small),
            Err(LoadError::OverCap {
                what: "remembered alerts",
                found: 3,
                cap: 2
            })
        );
        let tiny = Limits {
            told_bytes: 10,
            ..Limits::default()
        };
        assert_eq!(
            Told::parse_within(&bytes, &tiny),
            Err(LoadError::TooLarge {
                bytes: bytes.len() as u64,
                cap: 10
            })
        );
        assert!(matches!(
            Told::parse(b"{ not json"),
            Err(LoadError::Malformed(_))
        ));
    }
}
