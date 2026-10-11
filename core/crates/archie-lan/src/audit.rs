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
//! | `address-moved` | an IPv4 address is claimed over ARP by another device than before |
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
const MAX_REMEMBERED: usize = 16_384;
const WINDOW_MS: u64 = 60_000;
const SCAN_PORTS: usize = 100;
const SWEEP_HOSTS: usize = 50;

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
    told: HashSet<String>,
}

/// Shannon entropy of a label, in bits per character.
fn entropy(label: &str) -> f64 {
    let mut counts = [0usize; 256];
    for byte in label.bytes() {
        counts[byte as usize] += 1;
    }
    let total = label.len() as f64;
    counts.iter().filter(|&&n| n > 0).map(|&n| n as f64 / total).map(|p| -p * p.log2()).sum()
}

/// Whether a DNS question looks like data in disguise: very long, or one long label of
/// near-random characters. A CDN's hashed host name is long; it is not this random.
fn carries_data(question: &str) -> bool {
    question.len() > 180 || question.split('.').any(|label| label.len() >= 40 && entropy(label) >= 4.0)
}

impl Audit {
    pub fn new(mut state: State, settings: Settings) -> Self {
        state.format = "archie-netwatch-state-v1".into();
        Self { state, settings, openings: HashMap::new(), named: HashMap::new(), told: HashSet::new() }
    }

    fn tell(&mut self, alerts: &mut Vec<Alert>, key: String, at_ms: u64, severity: Severity, rule: &str, device: &str, detail: String) {
        if self.told.len() >= MAX_REMEMBERED {
            self.told.clear();
        }
        if self.told.insert(key) {
            alerts.push(Alert { at_ms, severity, rule: rule.into(), device: device.into(), detail });
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
        let Some((mac, source)) = packet.source_mac.as_deref().zip(packet.source).filter(|(_, ip)| is_lan(*ip)) else {
            return alerts;
        };
        if !self.state.devices.contains_key(mac) {
            if self.state.devices.len() >= MAX_DEVICES {
                return alerts;
            }
            if packet.at_ms.saturating_sub(self.state.started_ms) >= self.settings.learn_ms {
                self.tell(&mut alerts, format!("new {mac}"), packet.at_ms, Severity::Notice, "new-device", mac, format!("first seen, at {source}"));
            }
        }
        let outside = packet.destination.filter(|ip| !is_lan(*ip) && !ip.is_multicast() && !ip.is_loopback());
        if let Some((peer, name)) = outside.zip(packet.server_name.clone()) {
            if self.named.len() >= MAX_REMEMBERED {
                self.named.clear();
            }
            self.named.insert(peer, name);
        }
        let device = self.state.devices.entry(mac.to_string()).or_insert_with(|| Device { first_seen_ms: packet.at_ms, ..Device::default() });
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
            let label = self.named.get(&peer).cloned().unwrap_or_else(|| peer.to_string());
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
                self.tell(&mut alerts, format!("tunnel {mac} {zone}"), packet.at_ms, Severity::Warning, "dns-tunnel-suspect", mac, format!("asked for {shown} under {zone}"));
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
                    (Severity::Notice, "address now claimed by another device; usually a new DHCP lease")
                };
                self.tell(alerts, format!("moved {address} {mac}"), at_ms, severity, "address-moved", mac, format!("{address}: {what} (was {before})"));
                self.state.owners.insert(address, mac.to_string());
            }
            Some(_) => {}
            None if self.state.owners.len() < MAX_DEVICES => {
                self.state.owners.insert(address, mac.to_string());
            }
            None => {}
        }
    }

    fn opened(&mut self, alerts: &mut Vec<Alert>, at_ms: u64, mac: &str, destination: IpAddr, port: u16) {
        if matches!(port, 21 | 23) {
            let protocol = if port == 23 { "Telnet" } else { "FTP" };
            self.tell(alerts, format!("clear {mac} {destination} {port}"), at_ms, Severity::Warning, "cleartext-login", mac, format!("opened {protocol} to {destination}; its passwords are readable on the wire"));
        }
        if self.openings.len() >= MAX_DEVICES && !self.openings.contains_key(mac) {
            return;
        }
        let window = self.openings.entry(mac.to_string()).or_default();
        if at_ms.saturating_sub(window.since_ms) > WINDOW_MS {
            *window = Openings { since_ms: at_ms, ..Openings::default() };
        }
        let ports = window.ports_of.entry(destination).or_default();
        let scanned = ports.insert(port) && ports.len() == SCAN_PORTS;
        let swept = is_lan(destination) && {
            let hosts = window.hosts_on.entry(port).or_default();
            hosts.insert(destination) && hosts.len() == SWEEP_HOSTS
        };
        let since = window.since_ms;
        if scanned {
            self.tell(alerts, format!("scan {mac} {destination} {since}"), at_ms, Severity::High, "port-scan", mac, format!("opened {SCAN_PORTS} ports of {destination} within a minute"));
        }
        if swept {
            self.tell(alerts, format!("sweep {mac} {port} {since}"), at_ms, Severity::High, "host-sweep", mac, format!("opened port {port} on {SWEEP_HOSTS} hosts of the network within a minute"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TV: &str = "54:4e:f0:07:3d:e8";
    const ROUTER: &str = "e4:ab:89:6e:1a:18";

    fn from(mac: &str, source: &str, at_ms: u64) -> Packet {
        Packet { at_ms, length: 100, source_mac: Some(mac.into()), source: Some(source.parse().unwrap()), ..Packet::default() }
    }

    /// A network whose devices are all taken as known, to look at one rule at a time.
    fn learned() -> Settings {
        Settings { learn_ms: u64::MAX, gateway: None }
    }

    fn rules(alerts: &[Alert]) -> Vec<&str> {
        alerts.iter().map(|a| a.rule.as_str()).collect()
    }

    #[test]
    fn a_device_is_learned_quietly_at_first_and_announced_afterwards() {
        let mut audit = Audit::new(State::default(), Settings { learn_ms: 60_000, gateway: None });
        assert!(audit.see(&from(TV, "192.168.1.84", 1_000)).is_empty());
        let late = audit.see(&from("aa:bb:cc:00:11:22", "192.168.1.90", 70_000));
        assert_eq!(rules(&late), ["new-device"]);
        assert!(audit.see(&from("aa:bb:cc:00:11:22", "192.168.1.90", 71_000)).is_empty(), "told once");
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
        let mut audit = Audit::new(State::default(), Settings { learn_ms: u64::MAX, gateway: Some(gateway) });
        let claim = |mac: &str, ip: &str, at_ms| Packet { at_ms, arp: Some((mac.into(), ip.parse().unwrap())), ..Packet::default() };
        assert!(audit.see(&claim(ROUTER, "192.168.1.1", 1_000)).is_empty());
        assert!(audit.see(&claim(TV, "192.168.1.84", 1_100)).is_empty());
        let spoofed = audit.see(&claim("de:ad:be:ef:00:02", "192.168.1.1", 2_000));
        assert_eq!((spoofed[0].rule.as_str(), spoofed[0].severity), ("address-moved", Severity::High));
        let lease = audit.see(&claim("aa:bb:cc:00:11:22", "192.168.1.84", 3_000));
        assert_eq!((lease[0].rule.as_str(), lease[0].severity), ("address-moved", Severity::Notice));
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
        assert_eq!(rules(&seen), ["port-scan"], "once, when the hundredth port is opened");

        let mut seen = Vec::new();
        for host in 2..80u8 {
            seen.extend(audit.see(&open(format!("192.168.1.{host}"), 445, 200_000 + u64::from(host))));
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
        let open = |port: u16, at_ms| Packet { destination: Some("192.168.1.1".parse().unwrap()), tcp: Some((50_000, port)), opening: true, ..from(TV, "192.168.1.84", at_ms) };
        assert_eq!(rules(&audit.see(&open(23, 1_000))), ["cleartext-login"]);
        assert!(audit.see(&open(23, 2_000)).is_empty());
        assert!(audit.see(&open(443, 3_000)).is_empty());
    }

    #[test]
    fn a_question_that_carries_data_is_told_and_an_ordinary_long_one_is_not() {
        let mut audit = Audit::new(State::default(), learned());
        let ask = |name: &str, at_ms| Packet { dns_question: Some(name.into()), ..from(TV, "192.168.1.84", at_ms) };
        assert!(audit.see(&ask("d3a1b2c4e5f6a7.cloudfront.net", 1_000)).is_empty());
        assert!(audit.see(&ask("connectivity-check.ubuntu.com", 1_100)).is_empty());
        let label = "q7x2mzk9v4p1tc8w0hjy3ns6bfd5rgae4lu7o2iq9xk";
        assert_eq!(rules(&audit.see(&ask(&format!("{label}.t.example.com"), 2_000))), ["dns-tunnel-suspect"]);
        assert!(audit.see(&ask(&format!("a{label}.t.example.com"), 2_100)).is_empty(), "one alert for the zone");
        assert_eq!(audit.state.devices[TV].asked.len(), 4);
    }

    #[test]
    fn what_a_device_sends_outside_is_counted_under_the_server_s_name() {
        let mut audit = Audit::new(State::default(), learned());
        let to = |name: Option<&str>, at_ms| Packet { destination: Some("52.1.2.3".parse().unwrap()), server_name: name.map(String::from), ..from(TV, "192.168.1.84", at_ms) };
        audit.see(&to(Some("api.roku.com"), 1_000));
        audit.see(&to(None, 1_100));
        assert_eq!(audit.state.devices[TV].sent_to.get("api.roku.com"), Some(&200));
        let saved = serde_json::to_string(&audit.state).unwrap();
        assert_eq!(serde_json::from_str::<State>(&saved).unwrap(), audit.state);
    }
}
