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
//! | `address-moved` | an address is claimed by another device than before — IPv4 over ARP, IPv6 by a Neighbor Advertisement; a fight over one address is one alert, raised again only after a quiet hour, counting the moves |
//! | `port-scan` | one device opens connections to 100 ports of one host within a minute |
//! | `host-sweep` | one device opens the same port on 50 LAN hosts within a minute |
//! | `cleartext-login` | a device opens Telnet or FTP, which carry passwords in the clear |
//! | `login-burst` | a device opens 30 or more connections to one host's login port (FTP, SSH, Telnet, RDP, VNC) within a minute: password guessing, from a person's tool or from malware |
//! | `dns-bypass` | a device resolves names outside the network's own DNS: plain DNS or DNS over TLS to an outside resolver, or DNS over HTTPS to a known provider. The router and the LAN's DNS servers (a Pi-hole) forward upstream and are not told |
//! | `retired-smb` | a device speaks SMB version 1 past the Negotiate (Session Setup, Tree Connect…): the protocol EternalBlue and WannaCry spread through, off by default in Windows since 2017 |
//! | `cleartext-password` | a device sends a login command in the clear (FTP `PASS`, POP3 `PASS`/`AUTH`, IMAP `LOGIN`, SMTP `AUTH`), on any port: high when the server is outside the LAN, a warning inside it. Only the command word is read, never the password |
//! | `dns-tunnel-suspect` | a DNS question is long and random enough to be carrying data |
//! | `unusual-destination` | a device learned for a day sends over 25 MB to an outside destination it had not used before |
//! | `traffic-surge` | a device that has been learned for a day sends far more in ten minutes than it usually does |
//! | `rogue-dhcp-server` | a second device answers DHCP (an offer or acknowledgement) when another already does: it can hand out its own router and DNS |
//! | `beacon-suspect` | a device learned for a day starts calling a new outside destination on a clock (a regular interval, scored as RITA does), the way malware checks in with its controller |
//! | `rogue-dhcpv6-server` | a device that is not the router answers DHCPv6 (an advertise or reply), or, once servers are known, a new one does: mitm6 makes itself every Windows machine's IPv6 DNS server this way |
//! | `lease-conflict` | a device claims (over ARP) an address the router's DHCP server leased to another device, while that lease holds: a static address inside the DHCP range, or someone taking another device's traffic. Needs a lease table read with `archie-netwatch leases` |
//! | `rogue-dns-server` | after the learning time, a device on the LAN that is not the router starts answering DNS to other devices: someone handing out its own DNS can send them anywhere. Servers learned while learning (a Pi-hole) stay quiet |
//! | `arp-sweep` | one device asks ARP for 50 or more different addresses within a minute: a scan of the network (nmap -sn, arp-scan, netdiscover, or a network app on a phone). ARP requests are broadcast, so a switched network shows them all |
//! | `icmp-redirect` | a device other than the router sends an ICMP Redirect (IPv4 or IPv6), telling a host to route through it: a way into the middle of its traffic without touching ARP |
//! | `retired-tls` | a device's TLS client offers nothing newer than TLS 1.1 (RFC 8996 retired 1.0 and 1.1 in 2021): firmware that has not been updated in years, usually a camera or an appliance |
//! | `name-poisoning` | a device answers LLMNR or NetBIOS name queries for three different names, or for `wpad`: Responder's way of taking other machines' traffic and password hashes. An honest Windows machine answers for its own name |
//! | `mac-flood` | 20 or more hardware addresses not known as devices send within a minute, after the first minute: a tool filling the switch's address table so it repeats everyone's traffic to every port. While it lasts, unknown addresses are not kept as devices |
//! | `dhcp-starvation` | 30 or more hardware addresses not known as devices ask DHCP for an address within a minute: a tool exhausting the router's pool, so that a rogue server can answer instead |
//! | `dga-suspect` | a device's DNS questions come back "no such name" (NXDOMAIN) for 25 or more different random-looking names within a minute: malware with a domain generation algorithm trying names until it finds its controller's |
//! | `rogue-router-advertisement` | a device not seen before offers itself as an IPv6 router on the link, or one other than the known router does |
//!
//! Alongside the rules it keeps what each device says about itself in the clear (DHCP
//! options and vendor class, the TTL and window of the connections it opens, TLS JA4,
//! HTTP User-Agents, LLDP and NetBIOS names), which `crate::fingerprint` reads as hints.
//!
//! Time is the frames' own, so a capture file is audited exactly as the live network is.

use crate::capture::Packet;
use crate::is_lan;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

const MAX_DEVICES: usize = 4096;
const MAX_NAMES: usize = 512;
const MAX_PEERS: usize = 64;
const MAX_ADDRESSES: usize = 16;
/// TLS fingerprints, User-Agents and NetBIOS names kept per device.
const MAX_FINGERPRINTS: usize = 16;
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
/// A device's sending is measured in buckets of this length.
const BUCKET_MS: u64 = 10 * 60 * 1000;
/// Buckets learned before a device's usual sending is trusted: one day.
const BASELINE_BUCKETS: u32 = 144;
/// How fast the usual moves: a bucket counts as much as about a day of history.
const BASELINE_ALPHA: f64 = 1.0 / 144.0;
/// A bucket is a surge only above all three: this many bytes, this many times the
/// usual, and this many deviations above it. Criterion, not measurement.
const SURGE_MIN_BYTES: u64 = 50 * 1024 * 1024;
const SURGE_FACTOR: f64 = 4.0;
const SURGE_SIGMAS: f64 = 6.0;
/// Bytes a learned device may send to a destination new to it before that is told.
const NEW_DESTINATION_BYTES: u64 = 25_000_000;
/// One device's surges are told at most once in this long.
const SURGE_COOLDOWN_MS: u64 = 60 * 60 * 1000;
/// How clock-like a rhythm must be to be kept on the device, and to be told.
const BEACON_KEPT_SCORE: u8 = 70;
const BEACON_TOLD_SCORE: u8 = 85;
/// Device and destination pairs whose contacts are kept (33 times each).
const MAX_CONTACTED: usize = 4_096;
const DAY_MS: u64 = 24 * 60 * 60 * 1000;
/// Different addresses one device may ask ARP about within a minute before it is told.
const ARP_SWEEP_TARGETS: usize = 50;
/// Devices whose ARP questions are counted at once.
const MAX_ARP_ASKERS: usize = 256;
/// Different random-looking names a device is told do not exist, within a minute, before
/// `dga-suspect`.
const DGA_NAMES: usize = 25;
/// Different LLMNR or NetBIOS names one device may answer for before it is told.
const POISONING_NAMES: usize = 3;
/// Unknown hardware addresses sending within the window that make a flood.
const FLOOD_SENDERS: usize = 20;
/// Unknown DHCP clients within the window that make a starvation attack.
const STARVATION_CLIENTS: usize = 30;
const STARVATION_WINDOW_MS: u64 = 60 * 1000;
/// DHCP client messages kept for that window, at most.
const MAX_ASKING: usize = 4_096;
const PRUNE_EVERY_MS: u64 = 60 * 1000;
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
    /// How much it usually sends. Absent in files written before it existed.
    #[serde(default)]
    pub baseline: Baseline,
    /// The DHCP options it asks for, in its order.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dhcp_requested: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dhcp_vendor: Option<String>,
    /// The TTL the connections it opens most likely start from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hops: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<u16>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub ja4: BTreeSet<String>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub user_agents: BTreeSet<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lldp_name: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub nbns_names: BTreeSet<String>,
    /// Outside destinations it calls on a clock, by the same label as `sent_to`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub beacons: BTreeMap<String, Beacon>,
    /// When its lease from the router's DHCP server ends, in seconds since the epoch, as
    /// the last lease table read said (`Some(0)` for a lease that does not end).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub leased_until_s: Option<u64>,
    /// What the router's connection tracking showed it had open to the outside, the last
    /// time the router was read: seen even when a switch hides its traffic from Sentry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub router_flows: Option<RouterFlows>,
}

/// A device's connections to the outside at one read of the router.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouterFlows {
    pub at_ms: u64,
    pub connections: u32,
    /// Bytes it sent over them, when the router counts bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes: Option<u64>,
    /// The outside destinations it had the most connections to, by name where one is
    /// known, at most `crate::router::MAX_ROUTER_PEERS`.
    pub peers: Vec<String>,
}

/// The holder of one address in the router's lease table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeaseHolder {
    pub mac: String,
    /// When the lease ends, in seconds since the epoch; absent for one that does not.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub until_s: Option<u64>,
}

impl LeaseHolder {
    fn holds_at(&self, at_ms: u64) -> bool {
        self.until_s
            .is_none_or(|until| until.saturating_mul(1000) >= at_ms)
    }
}

/// A clock-like rhythm of connections to one destination (`crate::beacon`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Beacon {
    /// The median interval between contacts, in seconds.
    pub every_s: u64,
    /// 0..=100: how clock-like, as last scored.
    pub score: u8,
    /// The median bytes it sends from one contact to the next, when counted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes: Option<u64>,
    pub first_ms: u64,
    pub last_ms: u64,
}

/// A device's usual sending, learned per ten-minute bucket as an exponentially
/// weighted mean and variance, so it follows the device without keeping history.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Baseline {
    /// Start of the bucket being filled.
    pub bucket_start_ms: u64,
    /// Bytes sent in it so far.
    pub bucket_bytes: u64,
    /// Buckets closed so far, saturating.
    pub buckets: u32,
    pub mean: f64,
    pub variance: f64,
}

impl Baseline {
    /// Whether enough has been learned to judge a bucket against it.
    pub fn learned(&self) -> bool {
        self.buckets >= BASELINE_BUCKETS
    }

    /// The bytes above which the open bucket is a surge.
    pub fn surge_line(&self) -> u64 {
        let usual = self.mean.max(0.0);
        let spread = usual + SURGE_SIGMAS * self.variance.max(0.0).sqrt();
        (SURGE_MIN_BYTES as f64)
            .max(usual * SURGE_FACTOR)
            .max(spread)
            .min(u64::MAX as f64) as u64
    }

    fn close(&mut self, bytes: f64) {
        if self.buckets == 0 {
            self.mean = bytes;
            self.variance = 0.0;
        } else {
            let delta = bytes - self.mean;
            self.mean += BASELINE_ALPHA * delta;
            self.variance =
                (1.0 - BASELINE_ALPHA) * (self.variance + BASELINE_ALPHA * delta * delta);
        }
        self.buckets = self.buckets.saturating_add(1);
    }

    /// Counts bytes sent at a time. Returns true the moment the open bucket crosses
    /// the surge line of a learned baseline, once per bucket.
    pub fn add(&mut self, at_ms: u64, bytes: u64) -> bool {
        let start = at_ms - at_ms % BUCKET_MS;
        let fresh = self.buckets == 0 && self.bucket_bytes == 0 && self.bucket_start_ms == 0;
        if fresh {
            self.bucket_start_ms = start;
        } else if start > self.bucket_start_ms {
            // Close the bucket, and every empty one since: quiet is part of the usual.
            let gone = (start - self.bucket_start_ms) / BUCKET_MS;
            self.close(self.bucket_bytes as f64);
            for _ in 1..gone.min(u64::from(BASELINE_BUCKETS)) {
                self.close(0.0);
            }
            self.bucket_start_ms = start;
            self.bucket_bytes = 0;
        }
        // A frame from before the open bucket (a reordered capture) counts in it.
        let line = self.surge_line();
        let before = self.bucket_bytes;
        self.bucket_bytes = self.bucket_bytes.saturating_add(bytes);
        self.learned() && before <= line && self.bucket_bytes > line
    }
}

/// What survives a restart.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct State {
    pub format: String,
    pub started_ms: u64,
    pub devices: BTreeMap<String, Device>,
    /// Which device last claimed each address: IPv4 over ARP, IPv6 by Neighbor
    /// Advertisement. Older files hold only IPv4 keys, and load as they are.
    pub owners: BTreeMap<IpAddr, String>,
    /// The devices that answer DHCP, by hardware address, with the server they speak for.
    #[serde(default)]
    pub dhcp_servers: BTreeMap<String, Option<Ipv4Addr>>,
    /// The devices that have offered themselves as IPv6 routers.
    #[serde(default)]
    pub router_advertisers: BTreeSet<String>,
    /// Which device the router's DHCP server last leased each address to, from the lease
    /// tables read (`archie-netwatch leases`).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub leased_to: BTreeMap<Ipv4Addr, LeaseHolder>,
    /// The devices on the LAN that have answered DNS.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub dns_servers: BTreeSet<String>,
    /// The devices that have answered DHCPv6.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub dhcpv6_servers: BTreeSet<String>,
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
        over(limits.devices, self.dhcp_servers.len(), "DHCP servers")?;
        over(
            limits.devices,
            self.router_advertisers.len(),
            "IPv6 routers",
        )?;
        over(limits.devices, self.dhcpv6_servers.len(), "DHCPv6 servers")?;
        over(limits.devices, self.dns_servers.len(), "DNS servers")?;
        over(limits.devices, self.leased_to.len(), "leased addresses")?;
        for holder in self.leased_to.values() {
            over(
                limits.name_chars,
                holder.mac.len(),
                "characters in one name",
            )?;
        }
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
            over(limits.peers, device.beacons.len(), "rhythms of one device")?;
            let router_peers = device.router_flows.as_ref().map_or(0, |f| f.peers.len());
            over(
                crate::router::MAX_ROUTER_PEERS,
                router_peers,
                "router-seen peers of one device",
            )?;
            for (count, what) in [
                (device.ja4.len(), "TLS fingerprints of one device"),
                (device.user_agents.len(), "User-Agents of one device"),
                (device.nbns_names.len(), "NetBIOS names of one device"),
            ] {
                over(MAX_FINGERPRINTS, count, what)?;
            }
            for name in device
                .asked
                .iter()
                .chain(device.sent_to.keys())
                .chain(device.beacons.keys())
                .chain(device.router_flows.iter().flat_map(|f| f.peers.iter()))
                .chain(device.hostname.iter())
                .chain(device.dhcp_requested.iter())
                .chain(device.dhcp_vendor.iter())
                .chain(device.ja4.iter())
                .chain(device.user_agents.iter())
                .chain(device.lldp_name.iter())
                .chain(device.nbns_names.iter())
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

    /// True when a key was never told or last told at least `cooldown_ms` ago; then it
    /// counts as told now.
    fn cooled(&mut self, key: &str, at_ms: u64, cooldown_ms: u64) -> bool {
        match self.records.get_mut(key) {
            Some(record) if at_ms.saturating_sub(record.at_ms) >= cooldown_ms => {
                record.at_ms = at_ms;
                record.count = record.count.saturating_add(1);
                true
            }
            Some(_) => false,
            None => {
                self.make_room();
                self.records
                    .insert(key.to_string(), ToldRecord { at_ms, count: 1 });
                true
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
    /// The router's IPv6 address on this link, its link-local one as a rule.
    pub gateway_v6: Option<Ipv6Addr>,
}

#[derive(Default)]
struct Openings {
    since_ms: u64,
    ports_of: HashMap<IpAddr, HashSet<u16>>,
    hosts_on: HashMap<u16, HashSet<IpAddr>>,
    /// Connections opened to each host's login port, for `login-burst`.
    logins: HashMap<(IpAddr, u16), usize>,
}

/// DNS-over-HTTPS endpoints of the public resolvers, by the TLS server name a client asks
/// for; a name ending in one of `DOH_SUFFIXES` counts too.
const DOH_NAMES: [&str; 11] = [
    "dns.google",
    "dns.google.com",
    "one.one.one.one",
    "dns.quad9.net",
    "dns9.quad9.net",
    "dns11.quad9.net",
    "doh.opendns.com",
    "doh.cleanbrowsing.org",
    "dns.adguard-dns.com",
    "doh.mullvad.net",
    "dns.mullvad.net",
];
const DOH_SUFFIXES: [&str; 2] = ["cloudflare-dns.com", "dns.nextdns.io"];

/// How `packet` resolves a name outside the network, if it does: the kind and the resolver.
fn bypass(packet: &Packet, outside: IpAddr) -> Option<(&'static str, String)> {
    let port = packet.tcp.or(packet.udp).map(|(_, port)| port);
    if packet.dns_question.is_some() && port == Some(53) {
        return Some(("plain DNS", outside.to_string()));
    }
    if port == Some(853) && (packet.opening || packet.udp.is_some()) {
        return Some(("DNS over TLS", outside.to_string()));
    }
    let name = packet.server_name.as_deref()?;
    let doh = DOH_NAMES.contains(&name)
        || DOH_SUFFIXES
            .iter()
            .any(|suffix| name == *suffix || name.ends_with(&format!(".{suffix}")));
    doh.then(|| ("DNS over HTTPS", name.to_string()))
}

/// Ports a password is typed into: FTP, SSH, Telnet (and Mirai's 2323), RDP, VNC.
const LOGIN_PORTS: [u16; 6] = [21, 22, 23, 2323, 3389, 5900];
/// Connections to one host's login port within a minute before `login-burst`. Zeek's
/// SSH password-guessing script stops at 30 guesses; a person, or a tool that reuses its
/// connection, opens a handful.
const LOGIN_ATTEMPTS: usize = 30;

pub struct Audit {
    pub state: State,
    settings: Settings,
    openings: HashMap<String, Openings>,
    /// Server names seen for outside addresses, to label the traffic that follows.
    named: HashMap<IpAddr, String>,
    /// Destinations a device first used after its baseline was learned, by hardware
    /// address and label. In memory only: a restart forgets which are new, which can
    /// only make the rule quieter.
    fresh: HashSet<(String, String)>,
    /// The alerts already raised, kept on disk so a restart does not repeat them.
    pub told: Told,
    /// What a device said of itself in frames without an address of its own (a DHCP
    /// client's discover and request come from 0.0.0.0; LLDP has no IP at all), by
    /// hardware address, until the device is first seen with one.
    pending: HashMap<String, Introduced>,
    /// When each device last contacted each outside destination, by hardware address and
    /// label, for `crate::beacon`. In memory only: a restart starts the count again.
    contacts: HashMap<(String, String), crate::beacon::Contacts>,
    /// When `contacts` was last pruned, so a full map is not swept on every frame.
    pruned_ms: u64,
    /// DHCP clients asking for an address in the last minute, for `dhcp-starvation`.
    asking: Burst,
    /// Hardware addresses not known as devices that sent in the last minute, for
    /// `mac-flood`.
    arriving: Burst,
    /// The LLMNR and NetBIOS names each device answered for, for `name-poisoning`. In
    /// memory only.
    answering: HashMap<String, BTreeSet<String>>,
    /// The addresses each device asked ARP about in the last minute, for `arp-sweep`.
    arp_asking: HashMap<String, Burst>,
    /// The random-looking names each device was told do not exist in the last minute, for
    /// `dga-suspect`.
    unanswered: HashMap<String, Burst>,
}

/// What happened in the last minute, by key (a DHCP client's hardware address, or a
/// sender's), with the Ethernet sender that carried it; with the count per key, so a flood
/// costs constant work per frame.
#[derive(Debug, Default)]
struct Burst {
    recent: VecDeque<(u64, String, String)>,
    per_client: HashMap<String, u32>,
}

impl Burst {
    fn add(&mut self, at_ms: u64, client: String, sender: String) {
        while self.recent.len() >= MAX_ASKING
            || self
                .recent
                .front()
                .is_some_and(|(at, _, _)| at_ms.saturating_sub(*at) >= STARVATION_WINDOW_MS)
        {
            let Some((_, gone, _)) = self.recent.pop_front() else {
                break;
            };
            if let Some(count) = self.per_client.get_mut(&gone) {
                *count -= 1;
                if *count == 0 {
                    self.per_client.remove(&gone);
                }
            }
        }
        *self.per_client.entry(client.clone()).or_default() += 1;
        self.recent.push_back((at_ms, client, sender));
    }

    /// The Ethernet sender that carried the most of them, and how many senders there were.
    fn senders(&self) -> (String, usize) {
        let mut count: HashMap<&str, usize> = HashMap::new();
        for (_, _, sender) in &self.recent {
            *count.entry(sender).or_default() += 1;
        }
        let most = count
            .iter()
            .max_by_key(|(sender, n)| (**n, std::cmp::Reverse(**sender)))
            .map_or_else(String::new, |(sender, _)| sender.to_string());
        (most, count.len())
    }
}

/// What a device says of itself outside IP: its DHCP client and its LLDP announcement.
#[derive(Debug, Clone, Default, PartialEq)]
struct Introduced {
    dhcp_requested: Option<String>,
    dhcp_vendor: Option<String>,
    lldp_name: Option<String>,
}

impl Introduced {
    fn from(packet: &Packet) -> Self {
        let client = matches!(packet.dhcp_message, Some(1 | 3 | 8));
        Self {
            dhcp_requested: packet.dhcp_requested.clone().filter(|_| client),
            dhcp_vendor: packet.dhcp_vendor.clone().filter(|_| client),
            lldp_name: packet.lldp_name.clone(),
        }
    }

    fn merge(self, newer: Introduced) -> Introduced {
        Introduced {
            dhcp_requested: newer.dhcp_requested.or(self.dhcp_requested),
            dhcp_vendor: newer.dhcp_vendor.or(self.dhcp_vendor),
            lldp_name: newer.lldp_name.or(self.lldp_name),
        }
    }

    fn apply(self, device: &mut Device) {
        if self.dhcp_requested.is_some() {
            device.dhcp_requested = self.dhcp_requested;
        }
        if self.dhcp_vendor.is_some() {
            device.dhcp_vendor = self.dhcp_vendor;
        }
        if self.lldp_name.is_some() {
            device.lldp_name = self.lldp_name;
        }
    }
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
/// The domain someone registered that `name` is under (`example.co.uk` for
/// `a.b.example.co.uk`), by the Public Suffix List; the name itself when it has none.
fn registered(name: &str) -> &str {
    psl::domain_str(name).unwrap_or(name)
}

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
            fresh: HashSet::new(),
            told: Told::default(),
            pending: HashMap::new(),
            contacts: HashMap::new(),
            pruned_ms: 0,
            asking: Burst::default(),
            arriving: Burst::default(),
            answering: HashMap::new(),
            arp_asking: HashMap::new(),
            unanswered: HashMap::new(),
        }
    }

    #[allow(clippy::too_many_arguments)]
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

    /// Takes one lease from the router's DHCP server into account, read at `at_ms`. A
    /// device the router knows and the network has not shown yet is kept like one heard on
    /// the wire, and told as new once the learning time is over.
    pub fn leased(&mut self, lease: &crate::leases::Lease, at_ms: u64) -> Vec<Alert> {
        let mut alerts = Vec::new();
        let mac = lease.mac.as_str();
        let Some(device) =
            self.known_through_router(&mut alerts, at_ms, mac, lease.address, "DHCP leases")
        else {
            return alerts;
        };
        if device.hostname.is_none() {
            device.hostname = lease.hostname.clone();
        }
        device.leased_until_s = Some(lease.expires_s.unwrap_or(0));
        let holder = LeaseHolder {
            mac: mac.to_string(),
            until_s: lease.expires_s,
        };
        let claimed = self
            .state
            .owners
            .get(&IpAddr::V4(lease.address))
            .filter(|owner| *owner != mac)
            .cloned();
        if let Some(claimant) = claimed.filter(|_| holder.holds_at(at_ms)) {
            self.lease_conflict(&mut alerts, at_ms, &claimant, lease.address, &holder);
        }
        if self.state.leased_to.len() < MAX_DEVICES
            || self.state.leased_to.contains_key(&lease.address)
        {
            self.state.leased_to.insert(lease.address, holder);
        }
        alerts
    }

    /// One complete entry of the router's neighbour (ARP) table: the router reached `mac` at
    /// `address`. Counted as an ARP claim (so `address-moved` and `lease-conflict` hold for
    /// what the router sees too) and, for a device the wire has not shown, kept like a
    /// lease. Not a sighting: `last_seen_ms` stays the wire's.
    pub fn router_saw(&mut self, mac: &str, address: Ipv4Addr, at_ms: u64) -> Vec<Alert> {
        let mut alerts = Vec::new();
        if self.state.started_ms == 0 {
            self.state.started_ms = at_ms.max(1);
        }
        self.claim(&mut alerts, at_ms, mac, IpAddr::V4(address));
        self.known_through_router(&mut alerts, at_ms, mac, address, "neighbour table");
        alerts
    }

    /// What the router's connection tracking showed `mac` had open to the outside. Kept on
    /// the device as the latest view; it adds no bytes to `sent_to`, which counts only what
    /// Sentry itself heard, so nothing is counted twice.
    pub fn router_flows(&mut self, mac: &str, outbound: &crate::router::Outbound, at_ms: u64) {
        let mut busiest: Vec<(&IpAddr, &u32)> = outbound.peers.iter().collect();
        busiest.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
        let peers = busiest
            .into_iter()
            .take(crate::router::MAX_ROUTER_PEERS)
            .map(|(peer, _)| {
                self.named
                    .get(peer)
                    .cloned()
                    .unwrap_or_else(|| peer.to_string())
            })
            .collect();
        if let Some(device) = self.state.devices.get_mut(mac) {
            device.router_flows = Some(RouterFlows {
                at_ms,
                connections: outbound.connections,
                bytes: outbound.bytes,
                peers,
            });
        }
    }

    /// The device the router knows as `mac` at `address`, made if the audit has not met it
    /// (and told as new once the learning time is over); `None` past the device cap.
    fn known_through_router(
        &mut self,
        alerts: &mut Vec<Alert>,
        at_ms: u64,
        mac: &str,
        address: Ipv4Addr,
        table: &str,
    ) -> Option<&mut Device> {
        if self.state.started_ms == 0 {
            self.state.started_ms = at_ms.max(1);
        }
        if !self.state.devices.contains_key(mac) {
            if self.state.devices.len() >= MAX_DEVICES {
                return None;
            }
            if at_ms.saturating_sub(self.state.started_ms) >= self.settings.learn_ms {
                self.tell(
                    alerts,
                    format!("new {mac}"),
                    at_ms,
                    Severity::Notice,
                    "new-device",
                    mac,
                    format!("first seen in the router's {table}, at {address}"),
                );
            }
        }
        let device = self
            .state
            .devices
            .entry(mac.to_string())
            .or_insert_with(|| Device {
                first_seen_ms: at_ms,
                last_seen_ms: at_ms,
                ..Device::default()
            });
        if device.addresses.len() < 16 {
            device.addresses.insert(IpAddr::V4(address));
        }
        Some(device)
    }

    /// `claimant` claims `address`, which the router leased to another device.
    fn lease_conflict(
        &mut self,
        alerts: &mut Vec<Alert>,
        at_ms: u64,
        claimant: &str,
        address: Ipv4Addr,
        holder: &LeaseHolder,
    ) {
        let name = self
            .state
            .devices
            .get(&holder.mac)
            .and_then(|device| device.hostname.clone())
            .map_or_else(String::new, |name| format!(" ({name})"));
        self.tell(
            alerts,
            format!("lease {address} {claimant}"),
            at_ms,
            Severity::Warning,
            "lease-conflict",
            claimant,
            format!(
                "claims {address}, which the router leased to {}{name}: a static address inside \
                 the DHCP range by mistake, or a device taking another's traffic",
                holder.mac
            ),
        );
    }

    /// Takes one frame into account and returns what it gives reason to tell.
    pub fn see(&mut self, packet: &Packet) -> Vec<Alert> {
        let mut alerts = Vec::new();
        if self.state.started_ms == 0 {
            self.state.started_ms = packet.at_ms.max(1);
        }
        if let Some((mac, address)) = &packet.arp {
            self.claim(&mut alerts, packet.at_ms, mac, IpAddr::V4(*address));
        }
        if let Some((mac, address)) = &packet.neighbor {
            self.claim(&mut alerts, packet.at_ms, mac, IpAddr::V6(*address));
        }
        if let Some((mac, asked)) = packet.source_mac.as_deref().zip(packet.arp_asked) {
            self.asked_arp(&mut alerts, packet.at_ms, mac, asked);
        }
        if let Some((device, name)) = &packet.nxdomain {
            self.no_such_name(&mut alerts, packet.at_ms, device, name);
        }
        // A client's own DHCP messages (discover, request, inform) describe its stack, and an
        // LLDP frame names its sender. Neither needs the device to have an address yet.
        let introduced = Introduced::from(packet);
        if let Some(mac) = packet.source_mac.as_deref() {
            if introduced != Introduced::default() {
                if let Some(device) = self.state.devices.get_mut(mac) {
                    introduced.apply(device);
                } else if self.pending.len() < MAX_REMEMBERED || self.pending.contains_key(mac) {
                    let earlier = self.pending.remove(mac).unwrap_or_default();
                    self.pending
                        .insert(mac.to_string(), earlier.merge(introduced));
                }
            }
        }
        let flooding = packet
            .source_mac
            .as_deref()
            .is_some_and(|mac| self.arrived(&mut alerts, packet.at_ms, mac));
        if matches!(packet.dhcp_message, Some(1 | 3)) {
            self.asked_dhcp(&mut alerts, packet);
        }
        // During a flood an unknown address is not made a device: thousands of made-up
        // ones would fill the inventory and keep real newcomers out.
        if flooding {
            return alerts;
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
        let learning = packet.at_ms.saturating_sub(self.state.started_ms) < self.settings.learn_ms;
        if matches!(packet.dhcp_message, Some(2 | 5)) {
            self.answered_dhcp(&mut alerts, packet, mac, source, learning);
        }
        if packet.router_advertisement {
            self.advertised_router(&mut alerts, packet.at_ms, mac, source, learning);
        }
        if packet.redirect {
            self.redirected(&mut alerts, packet.at_ms, mac, source);
        }
        if packet.dns_answer && packet.destination.is_some_and(is_lan) {
            self.answered_dns(&mut alerts, packet.at_ms, mac, source, learning);
        }
        if packet.dhcpv6_answer {
            self.answered_dhcpv6(&mut alerts, packet.at_ms, mac, source, learning);
        }
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
        // Names for outside addresses: the one a TLS client asked for (SNI), and those a
        // DNS answer gave, so a CDN's rotating addresses add up to one destination.
        let resolved = packet.resolved.iter().flat_map(|(name, addresses)| {
            addresses
                .iter()
                .filter(|ip| !is_lan(**ip) && !ip.is_multicast() && !ip.is_loopback())
                .map(move |ip| (*ip, name))
        });
        for (peer, name) in outside
            .zip(packet.server_name.as_ref())
            .into_iter()
            .chain(resolved)
        {
            if self.named.len() >= MAX_REMEMBERED && !self.named.contains_key(&peer) {
                self.named.clear();
            }
            self.named.insert(peer, name.clone());
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
        let mut unusual: Option<String> = None;
        let surge = device.baseline.add(packet.at_ms, packet.length).then(|| {
            format!(
                "sent over {} MB in ten minutes; it usually sends about {} MB",
                device.baseline.surge_line() / 1_000_000,
                (device.baseline.mean.max(0.0) / 1_000_000.0).round() as u64
            )
        });
        if device.addresses.len() < 16 {
            device.addresses.insert(source);
        }
        if let Some(hostname) = &packet.dhcp_hostname {
            device.hostname = Some(hostname.clone());
        }
        if let Some(introduced) = self.pending.remove(mac) {
            introduced.apply(device);
        }
        if packet.opening {
            if let Some(hops) = packet.hops {
                device.hops = Some(crate::fingerprint::initial_hops(hops));
            }
            if let Some(window) = packet.window {
                device.window = Some(window);
            }
        }
        for (seen, kept) in [
            (&packet.ja4, &mut device.ja4),
            (&packet.user_agent, &mut device.user_agents),
            (&packet.nbns_name, &mut device.nbns_names),
        ] {
            if let Some(seen) = seen {
                if kept.len() < MAX_FINGERPRINTS {
                    kept.insert(seen.clone());
                }
            }
        }

        let label = outside.map(|peer| {
            self.named
                .get(&peer)
                .cloned()
                .unwrap_or_else(|| peer.to_string())
        });
        if let Some(label) = label.clone() {
            let known = device.sent_to.contains_key(&label);
            // Past the cap a destination is not tracked at all, so it cannot be new.
            if device.sent_to.len() < MAX_PEERS || known {
                let key = (mac.to_string(), label);
                if !known && device.baseline.learned() && self.fresh.len() < MAX_REMEMBERED {
                    self.fresh.insert(key.clone());
                }
                let total = device.sent_to.entry(key.1.clone()).or_default();
                let before = *total;
                *total += packet.length;
                if before < NEW_DESTINATION_BYTES
                    && *total >= NEW_DESTINATION_BYTES
                    && self.fresh.contains(&key)
                {
                    unusual = Some(key.1);
                }
            }
        }
        if let Some(question) = &packet.dns_question {
            if device.asked.len() < MAX_NAMES {
                device.asked.insert(question.clone());
            }
            if carries_data(question) {
                // By the zone, so one tunnel is one alert and not one per question.
                let zone = registered(question);
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
        if let Some(label) = unusual {
            self.tell(
                &mut alerts,
                format!("unusual {mac} {label}"),
                packet.at_ms,
                Severity::Warning,
                "unusual-destination",
                mac,
                format!(
                    "sent over {} MB to {label}, which it had not used before",
                    NEW_DESTINATION_BYTES / 1_000_000
                ),
            );
        }
        if let Some(detail) = surge {
            if self
                .told
                .cooled(&format!("surge {mac}"), packet.at_ms, SURGE_COOLDOWN_MS)
            {
                alerts.push(Alert {
                    at_ms: packet.at_ms,
                    severity: Severity::Warning,
                    rule: "traffic-surge".into(),
                    device: mac.into(),
                    detail,
                });
            }
        }
        if let Some(label) = label.filter(|_| packet.opening || packet.udp.is_some()) {
            let datagram = packet.udp.is_some();
            self.contacted(&mut alerts, packet.at_ms, mac, label, datagram);
        }
        if let Some(version) = packet
            .ja4
            .as_deref()
            .and_then(crate::fingerprint::tls_version)
            .filter(|v| crate::fingerprint::retired_tls(v))
        {
            let to = packet
                .server_name
                .as_deref()
                .map_or_else(String::new, |name| format!(" (to {name})"));
            self.tell(
                &mut alerts,
                format!("tls {mac}"),
                packet.at_ms,
                Severity::Notice,
                "retired-tls",
                mac,
                format!(
                    "its TLS client offers nothing newer than {version}{to}, retired in 2021: \
                     its firmware has likely not been updated in years"
                ),
            );
        }
        let forwards_dns = self.state.dns_servers.contains(mac)
            || self
                .settings
                .gateway
                .and_then(|gateway| self.state.owners.get(&IpAddr::V4(gateway)))
                .is_some_and(|router| router == mac);
        if let Some((kind, resolver)) = packet
            .destination
            .filter(|ip| !is_lan(*ip) && !ip.is_multicast())
            .filter(|_| !forwards_dns)
            .and_then(|outside| bypass(packet, outside))
        {
            self.tell(
                &mut alerts,
                format!("bypass {mac} {resolver}"),
                packet.at_ms,
                Severity::Notice,
                "dns-bypass",
                mac,
                format!(
                    "resolves names with {kind} through {resolver}, not the network's own DNS: \
                     a filter such as a Pi-hole does not see or block what it looks up"
                ),
            );
        }
        if packet.smb1 {
            let peer = packet
                .destination
                .map_or_else(String::new, |peer| format!(" with {peer}"));
            self.tell(
                &mut alerts,
                format!("smb1 {mac}"),
                packet.at_ms,
                Severity::Warning,
                "retired-smb",
                mac,
                format!(
                    "speaks SMB version 1{peer}, the protocol EternalBlue and WannaCry spread \
                     through; Windows has had it off by default since 2017. Turn SMBv1 off on \
                     it, or replace it"
                ),
            );
        }
        if let Some(protocol) = packet.cleartext_password {
            if let Some(server) = packet.destination {
                let outside = !is_lan(server);
                self.tell(
                    &mut alerts,
                    format!("password {mac} {server} {protocol}"),
                    packet.at_ms,
                    if outside {
                        Severity::High
                    } else {
                        Severity::Warning
                    },
                    "cleartext-password",
                    mac,
                    format!(
                        "sent its {protocol} password to {server} unencrypted{}: anyone on the \
                         way can read it. Use the server's TLS port or change the password",
                        if outside { ", across the internet" } else { "" }
                    ),
                );
            }
        }
        if let Some(name) = &packet.answered_name {
            self.answered_name(&mut alerts, packet.at_ms, mac, source, name);
        }
        if packet.opening {
            if let Some(((_, port), destination)) = packet.tcp.zip(packet.destination) {
                self.opened(&mut alerts, packet.at_ms, mac, destination, port);
            }
        }
        alerts
    }

    /// `mac` opened a connection to the outside destination `label`, or sent it a datagram
    /// (`datagram`), which is a contact only after a quiet spell (`crate::beacon`). A
    /// clock-like rhythm is kept on the device; one to a destination it took up after it
    /// was learned is told.
    fn contacted(
        &mut self,
        alerts: &mut Vec<Alert>,
        at_ms: u64,
        mac: &str,
        label: String,
        datagram: bool,
    ) {
        let key = (mac.to_string(), label);
        // Most datagrams belong to a session already going: settle those first.
        let mut heard = false;
        if datagram {
            if let Some(contacts) = self.contacts.get_mut(&key) {
                if !contacts.datagram(at_ms) {
                    return;
                }
                heard = true;
            }
        }
        if self.contacts.len() >= MAX_CONTACTED && !self.contacts.contains_key(&key) {
            // Forget pairs quiet for a day, at most once a minute; until that frees room,
            // a new pair is not tracked.
            if at_ms.saturating_sub(self.pruned_ms) < PRUNE_EVERY_MS {
                return;
            }
            self.pruned_ms = at_ms;
            self.contacts.retain(|_, contacts| {
                contacts
                    .last()
                    .is_some_and(|last| at_ms.saturating_sub(last) < DAY_MS)
            });
            if self.contacts.len() >= MAX_CONTACTED {
                return;
            }
        }
        // What the device has sent to the destination so far, for the sizes of its contacts.
        let total = self
            .state
            .devices
            .get(mac)
            .and_then(|device| device.sent_to.get(&key.1))
            .copied();
        let contacts = self.contacts.entry(key.clone()).or_default();
        if datagram && !heard {
            // The first datagram to this destination opens a contact.
            contacts.datagram(at_ms);
        }
        if !contacts.note(at_ms, total) {
            return;
        }
        let Some(rhythm) = contacts.rhythm() else {
            return;
        };
        let Some(device) = self.state.devices.get_mut(mac) else {
            return;
        };
        let (mac, label) = key;
        if rhythm.score < BEACON_KEPT_SCORE {
            device.beacons.remove(&label);
            return;
        }
        if device.beacons.len() >= MAX_PEERS && !device.beacons.contains_key(&label) {
            return;
        }
        let first_ms = device.beacons.get(&label).map_or(at_ms, |b| b.first_ms);
        device.beacons.insert(
            label.clone(),
            Beacon {
                every_s: rhythm.every_ms / 1000,
                score: rhythm.score,
                bytes: rhythm.bytes,
                first_ms,
                last_ms: at_ms,
            },
        );
        let key = (mac, label);
        if rhythm.score >= BEACON_TOLD_SCORE && self.fresh.contains(&key) {
            let (mac, label) = key;
            self.tell(
                alerts,
                format!("beacon {mac} {label}"),
                at_ms,
                Severity::Warning,
                "beacon-suspect",
                &mac,
                format!(
                    "calls {label}, which it took up recently, about every {} s{}, as regularly \
                     as a clock ({} of 100)",
                    rhythm.every_ms / 1000,
                    rhythm
                        .bytes
                        .map_or_else(String::new, |bytes| format!(" sending about {bytes} bytes")),
                    rhythm.score
                ),
            );
        }
    }

    /// `mac` asked ARP who has `asked`. A device asks for the few it talks to; one asking
    /// for most of the network is scanning it.
    fn asked_arp(&mut self, alerts: &mut Vec<Alert>, at_ms: u64, mac: &str, asked: Ipv4Addr) {
        if !self.arp_asking.contains_key(mac) && self.arp_asking.len() >= MAX_ARP_ASKERS {
            return;
        }
        let burst = self.arp_asking.entry(mac.to_string()).or_default();
        burst.add(at_ms, asked.to_string(), mac.to_string());
        let asked_for = burst.per_client.len();
        if asked_for >= ARP_SWEEP_TARGETS
            && self
                .told
                .cooled(&format!("arp-sweep {mac}"), at_ms, SURGE_COOLDOWN_MS)
        {
            alerts.push(Alert {
                at_ms,
                severity: Severity::Warning,
                rule: "arp-sweep".into(),
                device: mac.into(),
                detail: format!(
                    "asked ARP for {asked_for} addresses of the network within a minute: a scan \
                     of the LAN (nmap -sn, arp-scan, netdiscover), or a network app on a phone"
                ),
            });
        }
    }

    /// `mac` was told `name` does not exist. A device mistypes or probes a few; one told so
    /// for many names nobody would type is generating them.
    fn no_such_name(&mut self, alerts: &mut Vec<Alert>, at_ms: u64, mac: &str, name: &str) {
        if !crate::fingerprint::random_label(name) {
            return;
        }
        if !self.unanswered.contains_key(mac) && self.unanswered.len() >= MAX_ARP_ASKERS {
            return;
        }
        let burst = self.unanswered.entry(mac.to_string()).or_default();
        burst.add(at_ms, name.to_string(), mac.to_string());
        let names = burst.per_client.len();
        if names >= DGA_NAMES
            && self
                .told
                .cooled(&format!("dga {mac}"), at_ms, SURGE_COOLDOWN_MS)
        {
            alerts.push(Alert {
                at_ms,
                severity: Severity::Warning,
                rule: "dga-suspect".into(),
                device: mac.into(),
                detail: format!(
                    "was told within a minute that {names} random-looking names do not exist \
                     (such as {name}): malware generating names to find its controller"
                ),
            });
        }
    }

    /// A DNS answer from `mac` to a device on the LAN. The router and the servers seen
    /// while learning (a Pi-hole, a NAS) are the network's own; a new one afterwards is told.
    fn answered_dns(
        &mut self,
        alerts: &mut Vec<Alert>,
        at_ms: u64,
        mac: &str,
        source: IpAddr,
        learning: bool,
    ) {
        if self.state.dns_servers.contains(mac) || self.state.dns_servers.len() >= MAX_DEVICES {
            return;
        }
        let known: Vec<String> = self.state.dns_servers.iter().cloned().collect();
        self.state.dns_servers.insert(mac.to_string());
        let router = self
            .settings
            .gateway
            .and_then(|gateway| self.state.owners.get(&IpAddr::V4(gateway)))
            .is_some_and(|router| router == mac);
        if learning || router {
            return;
        }
        let before = if known.is_empty() {
            "no device on the network did before".to_string()
        } else {
            format!("until now only {} did", known.join(", "))
        };
        self.tell(
            alerts,
            format!("dns-server {mac}"),
            at_ms,
            Severity::Warning,
            "rogue-dns-server",
            mac,
            format!(
                "answers DNS from {source} to devices on the network; {before}. A DNS server \
                 that is not the network's own can send them to any address it likes"
            ),
        );
    }

    /// An ICMP Redirect from `mac`. The router may send one; anyone else is offering itself
    /// as the way out. Told once per sender: high when it is not the router, a warning while
    /// the router is not known.
    fn redirected(&mut self, alerts: &mut Vec<Alert>, at_ms: u64, mac: &str, source: IpAddr) {
        let router_v4 = self
            .settings
            .gateway
            .and_then(|gateway| self.state.owners.get(&IpAddr::V4(gateway)));
        let router_v6 = self
            .settings
            .gateway_v6
            .and_then(|gateway| self.state.owners.get(&IpAddr::V6(gateway)));
        let routers: Vec<&String> = router_v4.into_iter().chain(router_v6).collect();
        if routers.iter().any(|router| *router == mac) {
            return;
        }
        let (severity, detail) = if routers.is_empty() {
            (
                Severity::Warning,
                format!(
                    "sent an ICMP Redirect from {source}; the router's hardware address is not \
                     known yet to say whether it is the router"
                ),
            )
        } else {
            (
                Severity::High,
                format!(
                    "sent an ICMP Redirect from {source} and is not the router: it tells hosts \
                     to send their traffic through it"
                ),
            )
        };
        // The warning has its own key, so the router becoming known can still raise it.
        let key = if routers.is_empty() {
            "redirect?"
        } else {
            "redirect"
        };
        self.tell(
            alerts,
            format!("{key} {mac}"),
            at_ms,
            severity,
            "icmp-redirect",
            mac,
            detail,
        );
    }

    /// A DHCPv6 advertise or reply from `mac`. Told like a router advertisement: when it
    /// is not the router, or, once servers are known, when it is a new one. Windows prefers
    /// IPv6 and asks DHCPv6 even where the network uses none, which is how mitm6 makes
    /// itself every Windows machine's DNS server.
    fn answered_dhcpv6(
        &mut self,
        alerts: &mut Vec<Alert>,
        at_ms: u64,
        mac: &str,
        source: IpAddr,
        learning: bool,
    ) {
        if self.state.dhcpv6_servers.contains(mac) || self.state.dhcpv6_servers.len() >= MAX_DEVICES
        {
            return;
        }
        let known: Vec<String> = self.state.dhcpv6_servers.iter().cloned().collect();
        self.state.dhcpv6_servers.insert(mac.to_string());
        let not_the_router = self
            .settings
            .gateway
            .and_then(|gateway| self.state.owners.get(&IpAddr::V4(gateway)))
            .is_some_and(|router| router != mac);
        if learning || (known.is_empty() && !not_the_router) {
            return;
        }
        let (severity, detail) = if not_the_router {
            (
                Severity::High,
                format!(
                    "answers DHCPv6 from {source} and is not the router: it can hand IPv6 \
                     clients its own DNS server, which Windows then prefers (mitm6)"
                ),
            )
        } else {
            (
                Severity::Warning,
                format!(
                    "answers DHCPv6 from {source}; until now only {} did",
                    known.join(", ")
                ),
            )
        };
        self.tell(
            alerts,
            format!("dhcpv6-server {mac}"),
            at_ms,
            severity,
            "rogue-dhcpv6-server",
            mac,
            detail,
        );
    }

    /// `mac` answered an LLMNR or NetBIOS name query for `name`. A machine answers for its
    /// own name; one answering for many, or for `wpad` (the name browsers look up to find
    /// a proxy), is poisoning name resolution.
    fn answered_name(
        &mut self,
        alerts: &mut Vec<Alert>,
        at_ms: u64,
        mac: &str,
        source: IpAddr,
        name: &str,
    ) {
        if !self.answering.contains_key(mac) && self.answering.len() >= MAX_REMEMBERED {
            return;
        }
        let names = self.answering.entry(mac.to_string()).or_default();
        if names.len() < POISONING_NAMES || names.contains(name) {
            names.insert(name.to_string());
        }
        let wpad = name == "wpad";
        if !wpad && names.len() < POISONING_NAMES {
            return;
        }
        let shown: Vec<&str> = names.iter().map(String::as_str).collect();
        let detail = if wpad {
            format!(
                "answers from {source} for wpad, the name browsers look up to find a proxy: \
                 whoever answers it can read their web traffic"
            )
        } else {
            format!(
                "answers from {source} for {} names over LLMNR or NetBIOS ({}): a machine \
                 answers for its own name, Responder answers for all of them to collect \
                 password hashes",
                shown.len(),
                shown.join(", ")
            )
        };
        self.tell(
            alerts,
            format!("poison {mac}"),
            at_ms,
            Severity::High,
            "name-poisoning",
            mac,
            detail,
        );
    }

    /// A frame from `mac`. Many hardware addresses never seen before, within a minute, are a
    /// tool flooding the switch's address table (`macof`): a switch whose table is full
    /// sends every frame to every port, where anyone can read it. Returns whether `mac` is
    /// unknown while that goes on.
    fn arrived(&mut self, alerts: &mut Vec<Alert>, at_ms: u64, mac: &str) -> bool {
        // The first minute (or the learning time) everything is new.
        let warming = self.settings.learn_ms.max(STARVATION_WINDOW_MS);
        if self.state.devices.contains_key(mac)
            || at_ms.saturating_sub(self.state.started_ms) < warming
        {
            return false;
        }
        self.arriving.add(at_ms, mac.to_string(), mac.to_string());
        let senders = self.arriving.per_client.len();
        if senders < FLOOD_SENDERS {
            return false;
        }
        if self.told.cooled("mac-flood", at_ms, SURGE_COOLDOWN_MS) {
            alerts.push(Alert {
                at_ms,
                severity: Severity::High,
                rule: "mac-flood".into(),
                device: mac.into(),
                detail: format!(
                    "{senders} hardware addresses never seen before sent frames within a \
                     minute, {mac} among them. Flooding a switch's address table makes it \
                     send everyone's traffic to every port; while it lasts, new addresses \
                     are not kept as devices"
                ),
            });
        }
        true
    }

    /// A DHCP discover or request. Many clients the network does not know, asking within a
    /// minute, are a tool draining the pool (one Ethernet sender behind many client
    /// addresses gives it away; tools that vary both still show as a flood).
    fn asked_dhcp(&mut self, alerts: &mut Vec<Alert>, packet: &Packet) {
        let Some(sender) = packet.source_mac.clone() else {
            return;
        };
        let client = packet.dhcp_client.clone().unwrap_or_else(|| sender.clone());
        if self.state.devices.contains_key(&client) {
            return;
        }
        self.asking.add(packet.at_ms, client, sender);
        let clients = self.asking.per_client.len();
        if clients < STARVATION_CLIENTS
            || !self
                .told
                .cooled("dhcp-starvation", packet.at_ms, SURGE_COOLDOWN_MS)
        {
            return;
        }
        let (sender, senders) = self.asking.senders();
        let from = if senders == 1 {
            format!("all from the one Ethernet sender {sender}")
        } else {
            format!("from {senders} Ethernet senders, {sender} the busiest")
        };
        alerts.push(Alert {
            at_ms: packet.at_ms,
            severity: Severity::High,
            rule: "dhcp-starvation".into(),
            device: sender,
            detail: format!(
                "{clients} unknown hardware addresses asked DHCP for an address within a \
                 minute, {from}. Draining the router's pool lets a rogue DHCP server answer \
                 instead"
            ),
        });
    }

    /// A DHCP offer or acknowledgement from `mac`. The first server is learned; another
    /// one after it is told, since it can hand clients its own router and DNS.
    fn answered_dhcp(
        &mut self,
        alerts: &mut Vec<Alert>,
        packet: &Packet,
        mac: &str,
        source: IpAddr,
        learning: bool,
    ) {
        if self.state.dhcp_servers.contains_key(mac) || self.state.dhcp_servers.len() >= MAX_DEVICES
        {
            return;
        }
        let server = packet.dhcp_server.or(match source {
            IpAddr::V4(v4) => Some(v4),
            IpAddr::V6(_) => None,
        });
        let known: Vec<String> = self.state.dhcp_servers.keys().cloned().collect();
        self.state.dhcp_servers.insert(mac.to_string(), server);
        if known.is_empty() || learning {
            return;
        }
        let speaks = server.map_or_else(|| "itself".to_string(), |s| s.to_string());
        self.tell(
            alerts,
            format!("dhcp-server {mac}"),
            packet.at_ms,
            Severity::High,
            "rogue-dhcp-server",
            mac,
            format!(
                "answers DHCP as {speaks}; until now only {} did. A second DHCP server can give \
                 clients its own router and DNS",
                known.join(", ")
            ),
        );
    }

    /// An IPv6 Router Advertisement from `mac`. Told when it is not the known router, or,
    /// once routers are known, when it is a new one.
    fn advertised_router(
        &mut self,
        alerts: &mut Vec<Alert>,
        at_ms: u64,
        mac: &str,
        source: IpAddr,
        learning: bool,
    ) {
        if self.state.router_advertisers.contains(mac)
            || self.state.router_advertisers.len() >= MAX_DEVICES
        {
            return;
        }
        let known: Vec<String> = self.state.router_advertisers.iter().cloned().collect();
        self.state.router_advertisers.insert(mac.to_string());
        // Not the router: not the IPv6 router given, or not the device that holds the IPv4
        // router's address. On a network whose router offers no IPv6, the first
        // advertisement is the classic way to become everyone's router.
        let router_mac = self
            .settings
            .gateway
            .and_then(|gateway| self.state.owners.get(&IpAddr::V4(gateway)));
        let not_the_router = match (self.settings.gateway_v6, source) {
            (Some(gateway), IpAddr::V6(v6)) => gateway != v6,
            _ => router_mac.is_some_and(|router| router != mac),
        };
        if learning || (known.is_empty() && !not_the_router) {
            return;
        }
        let (severity, detail) = if not_the_router {
            (
                Severity::High,
                format!("offers itself as an IPv6 router from {source}, which is not the router"),
            )
        } else {
            (
                Severity::Warning,
                format!(
                    "offers itself as an IPv6 router from {source}; until now only {} did",
                    known.join(", ")
                ),
            )
        };
        self.tell(
            alerts,
            format!("router-advertisement {mac}"),
            at_ms,
            severity,
            "rogue-router-advertisement",
            mac,
            detail,
        );
    }

    fn claim(&mut self, alerts: &mut Vec<Alert>, at_ms: u64, mac: &str, address: IpAddr) {
        // An advertisement on this link speaks for an address on it, global ones too;
        // an IPv4 claim counts only for a LAN address.
        let gateway = match address {
            IpAddr::V4(v4) if !crate::is_lan_v4(v4) => return,
            IpAddr::V4(v4) => self.settings.gateway == Some(v4),
            IpAddr::V6(v6) => self.settings.gateway_v6 == Some(v6),
        };
        if let IpAddr::V4(v4) = address {
            let holder = self
                .state
                .leased_to
                .get(&v4)
                .filter(|holder| holder.mac != mac && holder.holds_at(at_ms))
                .cloned();
            if let Some(holder) = holder {
                self.lease_conflict(alerts, at_ms, mac, v4, &holder);
            }
        }
        match self.state.owners.get(&address).cloned() {
            Some(before) if before != mac => {
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
        let attempts = if LOGIN_PORTS.contains(&port) && window.logins.len() < MAX_PEERS {
            let count = window.logins.entry((destination, port)).or_default();
            *count += 1;
            *count
        } else {
            0
        };
        let since = window.since_ms;
        if attempts == LOGIN_ATTEMPTS {
            let service = match port {
                21 => "FTP",
                22 => "SSH",
                3389 => "RDP",
                5900 => "VNC",
                _ => "Telnet",
            };
            self.tell(
                alerts,
                format!("logins {mac} {destination} {port} {since}"),
                at_ms,
                Severity::High,
                "login-burst",
                mac,
                format!(
                    "opened {LOGIN_ATTEMPTS} {service} connections to {destination} within a \
                     minute: someone trying passwords, or malware on it spreading the way \
                     Mirai does"
                ),
            );
        }
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
            gateway_v6: None,
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
                gateway_v6: None,
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
                gateway_v6: None,
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
        // Two tunnels under the same public suffix are two zones, each told.
        for (at_ms, zone) in [(3_000, "one.co.uk"), (3_100, "two.co.uk")] {
            let told = audit.see(&ask(&format!("{label}.t.{zone}"), at_ms));
            assert_eq!(rules(&told), ["dns-tunnel-suspect"]);
            assert!(
                told[0].detail.ends_with(&format!("under {zone}")),
                "{}",
                told[0].detail
            );
        }
    }

    #[test]
    fn the_registered_domain_follows_the_public_suffix_list() {
        assert_eq!(registered("a.b.example.co.uk"), "example.co.uk");
        assert_eq!(registered("x.t.evil.com.br"), "evil.com.br");
        assert_eq!(registered("someone.github.io"), "someone.github.io");
        assert_eq!(registered("api.example.com"), "example.com");
        assert_eq!(registered("localhost"), "localhost");
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
    fn a_device_sending_far_more_than_it_usually_does_is_a_surge_told_once_an_hour() {
        let mut audit = Audit::new(State::default(), learned());
        let sent = |at_ms: u64, length: u64| Packet {
            length,
            ..from(TV, "192.168.1.84", at_ms)
        };
        // A day and a bit of ordinary sending: 2 MB per ten minutes.
        let day = u64::from(BASELINE_BUCKETS) + 6;
        for bucket in 0..day {
            assert!(audit
                .see(&sent(bucket * BUCKET_MS + 1, 2_000_000))
                .is_empty());
        }
        let baseline = &audit.state.devices[TV].baseline;
        assert!(baseline.learned());
        assert!(
            (baseline.mean - 2_000_000.0).abs() < 1.0,
            "{}",
            baseline.mean
        );

        // Then 60 MB inside one bucket: told once, when it crosses the line.
        let now = day * BUCKET_MS + 1;
        let mut told = Vec::new();
        for i in 0..60 {
            told.extend(audit.see(&sent(now + i * 1_000, 1_000_000)));
        }
        assert_eq!(rules(&told), ["traffic-surge"]);
        assert_eq!(told[0].severity, Severity::Warning);
        assert!(
            told[0].detail.contains("usually sends about 2 MB"),
            "{}",
            told[0].detail
        );

        // The next bucket surges too, inside the hour: not told again.
        let next = now + BUCKET_MS;
        let quiet: Vec<Alert> = (0..60)
            .flat_map(|i| audit.see(&sent(next + i * 1_000, 1_000_000)))
            .collect();
        assert!(quiet.is_empty());
        // After the hour, it is.
        let later = now + SURGE_COOLDOWN_MS + BUCKET_MS;
        let again: Vec<Alert> = (0..200)
            .flat_map(|i| audit.see(&sent(later + i * 1_000, 1_000_000)))
            .collect();
        assert_eq!(rules(&again), ["traffic-surge"]);
    }

    #[test]
    fn the_router_s_leases_name_devices_the_wire_did_not_show() {
        let mut audit = Audit::new(
            State::default(),
            Settings {
                learn_ms: 60_000,
                gateway: None,
                gateway_v6: None,
            },
        );
        let lease = |mac: &str, ip: &str, name: Option<&str>| crate::leases::Lease {
            expires_s: Some(1_791_750_000),
            mac: mac.into(),
            address: ip.parse().unwrap(),
            hostname: name.map(str::to_string),
        };
        // Seen on the wire first: the lease adds what the router knows, nothing is told.
        audit.see(&from(TV, "192.168.1.84", 1_000));
        assert!(audit
            .leased(&lease(TV, "192.168.1.84", Some("living-room-tv")), 2_000)
            .is_empty());
        let tv = &audit.state.devices[TV];
        assert_eq!(tv.hostname.as_deref(), Some("living-room-tv"));
        assert_eq!(tv.leased_until_s, Some(1_791_750_000));
        assert_eq!(tv.last_seen_ms, 1_000, "a lease is not a sighting");
        // While learning, a device only the router knows is kept quietly.
        assert!(audit
            .leased(&lease("00:11:22:33:44:01", "192.168.1.20", None), 3_000)
            .is_empty());
        // Afterwards it is a new device, told once whether by lease or on the wire.
        let camera = "00:12:12:00:00:02";
        let told = audit.leased(&lease(camera, "192.168.1.61", Some("ipcam")), 120_000);
        assert_eq!(rules(&told), ["new-device"]);
        assert!(
            told[0].detail.contains("router's DHCP leases"),
            "{}",
            told[0].detail
        );
        assert!(audit.see(&from(camera, "192.168.1.61", 130_000)).is_empty());
        assert_eq!(audit.state.devices.len(), 3);
    }

    #[test]
    fn the_router_s_neighbour_table_counts_as_what_it_saw() {
        let gateway: Ipv4Addr = "192.168.1.1".parse().unwrap();
        let mut audit = Audit::new(
            State::default(),
            Settings {
                learn_ms: 60_000,
                gateway: Some(gateway),
                gateway_v6: None,
            },
        );
        // While learning: devices are kept quietly, with the router's view of their address.
        assert!(audit
            .router_saw(TV, "192.168.1.84".parse().unwrap(), 1_000)
            .is_empty());
        assert!(audit.router_saw(ROUTER, gateway, 1_000).is_empty());
        assert_eq!(audit.state.owners[&IpAddr::V4(gateway)], ROUTER);
        assert_eq!(audit.state.devices[TV].last_seen_ms, 1_000);
        // Afterwards a device only the router reached is new, said where it was seen.
        let told = audit.router_saw("00:12:12:00:00:03", "192.168.1.62".parse().unwrap(), 90_000);
        assert_eq!(rules(&told), ["new-device"]);
        assert!(
            told[0].detail.contains("neighbour table"),
            "{}",
            told[0].detail
        );
        // The router's own table showing another device on the router's address is high.
        let spoof = audit.router_saw("de:ad:be:ef:00:01", gateway, 95_000);
        assert!(spoof
            .iter()
            .any(|a| a.rule == "address-moved" && a.severity == Severity::High));
    }

    #[test]
    fn claiming_an_address_the_router_leased_to_another_is_told() {
        let mut audit = Audit::new(State::default(), learned());
        let lease = |mac: &str, ip: &str, until_s: Option<u64>| crate::leases::Lease {
            expires_s: until_s,
            mac: mac.into(),
            address: ip.parse().unwrap(),
            hostname: None,
        };
        audit.leased(&lease(TV, "192.168.1.84", Some(2_000_000)), 1_000);
        assert_eq!(audit.state.leased_to.len(), 1);
        // The holder claiming its own address: quiet.
        assert!(audit.see(&claim(TV, "192.168.1.84", 2_000)).is_empty());
        // Another device claiming it while the lease holds: told once.
        let laptop = "02:00:5e:aa:bb:cc";
        let told: Vec<Alert> = (0..3)
            .flat_map(|i| audit.see(&claim(laptop, "192.168.1.84", 3_000 + i)))
            .filter(|a| a.rule == "lease-conflict")
            .collect();
        assert_eq!(told.len(), 1);
        assert_eq!(told[0].device, laptop);
        assert!(told[0].detail.contains(TV), "{}", told[0].detail);
        // After the lease ended the address is free to move: no conflict.
        audit.leased(&lease(TV, "192.168.1.90", Some(10)), 1_000);
        assert!(audit
            .see(&claim(laptop, "192.168.1.90", 20_000))
            .iter()
            .all(|a| a.rule != "lease-conflict"));
        // A lease table read after the wire already showed someone else on the address.
        audit.see(&claim("02:00:5e:00:00:07", "192.168.1.120", 30_000));
        let late = audit.leased(&lease("00:11:22:33:44:01", "192.168.1.120", None), 31_000);
        assert_eq!(rules(&late), ["lease-conflict"]);
        assert_eq!(late[0].device, "02:00:5e:00:00:07");
        // The state keeps the lease table and still loads.
        let saved = serde_json::to_vec(&audit.state).unwrap();
        assert_eq!(State::parse(&saved).unwrap().leased_to.len(), 3);
    }

    #[test]
    fn a_new_dns_server_on_the_lan_is_told() {
        let gateway: Ipv4Addr = "192.168.1.1".parse().unwrap();
        let mut audit = Audit::new(
            State::default(),
            Settings {
                learn_ms: 60_000,
                gateway: Some(gateway),
                gateway_v6: None,
            },
        );
        audit.see(&Packet {
            at_ms: 1,
            arp: Some((ROUTER.into(), gateway)),
            ..Packet::default()
        });
        let answer = |mac: &str, ip: &str, to: &str, at_ms: u64| Packet {
            dns_answer: true,
            destination: Some(to.parse().unwrap()),
            ..from(mac, ip, at_ms)
        };
        let pihole = "b8:27:eb:00:00:53";
        // A Pi-hole answering while learning, and the router afterwards: quiet.
        assert!(audit
            .see(&answer(pihole, "192.168.1.53", "192.168.1.84", 2_000))
            .is_empty());
        assert!(audit
            .see(&answer(ROUTER, "192.168.1.1", "192.168.1.84", 90_000))
            .into_iter()
            .all(|a| a.rule != "rogue-dns-server"));
        // A laptop starts answering DNS: told once, naming the ones before it.
        let rogue = "02:00:5e:aa:bb:cc";
        let told: Vec<Alert> = (0..3)
            .flat_map(|i| audit.see(&answer(rogue, "192.168.1.66", "192.168.1.84", 100_000 + i)))
            .filter(|a| a.rule != "new-device")
            .collect();
        assert_eq!(rules(&told), ["rogue-dns-server"]);
        assert!(told[0].detail.contains(pihole), "{}", told[0].detail);
        // A DNS answer to the internet (the router's own forwarding) is not this.
        assert!(audit
            .see(&answer(
                "02:00:5e:00:00:99",
                "192.168.1.70",
                "8.8.8.8",
                110_000
            ))
            .into_iter()
            .all(|a| a.rule != "rogue-dns-server"));
    }

    #[test]
    fn asking_arp_for_most_of_the_network_is_a_scan() {
        let mut audit = Audit::new(State::default(), learned());
        let who_has = |mac: &str, asked: u64, at_ms: u64| Packet {
            at_ms,
            source_mac: Some(mac.into()),
            arp_asked: Some(Ipv4Addr::from(0xc0a8_0100 + asked as u32)),
            ..Packet::default()
        };
        // A laptop asks for the router and a printer, over and over: quiet.
        let usual: Vec<Alert> = (0..200)
            .flat_map(|i| audit.see(&who_has(TV, 1 + (i % 2) * 20, i * 100)))
            .collect();
        assert!(usual.is_empty());
        // A slow walk over 60 addresses, one every 1.5 s, never has 50 in a minute.
        let slow: Vec<Alert> = (1..61)
            .flat_map(|i| audit.see(&who_has("02:00:5e:00:00:01", i, 100_000 + i * 1_500)))
            .collect();
        assert!(slow.is_empty(), "{slow:?}");
        // nmap -sn over the /24 in two seconds: told once.
        let scanner = "02:00:5e:12:34:56";
        let told: Vec<Alert> = (1..255)
            .flat_map(|i| audit.see(&who_has(scanner, i, 300_000 + i * 8)))
            .collect();
        assert_eq!(rules(&told), ["arp-sweep"]);
        assert_eq!(told[0].device, scanner);
        assert!(
            told[0].detail.contains("50 addresses"),
            "{}",
            told[0].detail
        );
    }

    #[test]
    fn the_router_s_view_of_a_device_s_connections_is_kept_by_name() {
        let mut audit = Audit::new(State::default(), learned());
        audit.see(&Packet {
            destination: Some("142.250.184.14".parse().unwrap()),
            server_name: Some("www.google.com".into()),
            ..from(TV, "192.168.1.84", 1_000)
        });
        let outbound = crate::router::Outbound {
            connections: 12,
            bytes: Some(40_000),
            peers: (1..=12u8)
                .map(|i| (IpAddr::from([198, 51, 100, i]), u32::from(i)))
                .chain([("142.250.184.14".parse().unwrap(), 30)])
                .collect(),
        };
        audit.router_flows(TV, &outbound, 300_000);
        let flows = audit.state.devices[TV].router_flows.clone().unwrap();
        assert_eq!((flows.connections, flows.bytes), (12, Some(40_000)));
        assert_eq!(flows.peers.len(), crate::router::MAX_ROUTER_PEERS);
        assert_eq!(flows.peers[0], "www.google.com", "busiest first, by name");
        assert_eq!(flows.peers[1], "198.51.100.12");
        // Nothing is added to what Sentry heard itself.
        assert_eq!(audit.state.devices[TV].sent_to["www.google.com"], 100);
        // A device the audit has not met is not made from connections alone.
        audit.router_flows("00:99:99:00:00:01", &outbound, 300_000);
        assert!(!audit.state.devices.contains_key("00:99:99:00:00:01"));
        // It survives a save and a load, and too many peers in a file are refused.
        let saved = serde_json::to_vec(&audit.state).unwrap();
        assert_eq!(
            State::parse(&saved).unwrap().devices[TV].router_flows,
            Some(flows)
        );
        let mut forged = audit.state.clone();
        forged
            .devices
            .get_mut(TV)
            .unwrap()
            .router_flows
            .as_mut()
            .unwrap()
            .peers = (0..20).map(|i| format!("p{i}")).collect();
        assert!(State::parse(&serde_json::to_vec(&forged).unwrap()).is_err());
    }

    #[test]
    fn many_logins_to_one_host_within_a_minute_are_a_burst() {
        let mut audit = Audit::new(State::default(), learned());
        let open = |mac: &str, host: &str, port: u16, at_ms: u64| Packet {
            destination: Some(host.parse().unwrap()),
            tcp: Some((40_000, port)),
            opening: true,
            ..from(mac, "192.168.1.84", at_ms)
        };
        // An admin's few SSH sessions, and a browser's many HTTPS ones: quiet.
        let usual: Vec<Alert> = (0..10)
            .map(|i| open(TV, "192.168.1.5", 22, i * 1_000))
            .chain((0..100).map(|i| open(TV, "192.168.1.5", 443, 20_000 + i * 100)))
            .flat_map(|p| audit.see(&p))
            .collect();
        assert!(usual.is_empty(), "{usual:?}");
        // Thirty logins spread over more than a minute never fill one window.
        let slow: Vec<Alert> = (0..40)
            .flat_map(|i| audit.see(&open(TV, "192.168.1.6", 22, 100_000 + i * 3_000)))
            .collect();
        assert!(slow.is_empty(), "{slow:?}");
        // A camera trying Telnet passwords on the NAS, ten a second: told once.
        let camera = "00:12:12:00:00:01";
        let told: Vec<Alert> = (0..100)
            .flat_map(|i| audit.see(&open(camera, "192.168.1.5", 23, 300_000 + i * 100)))
            .collect();
        // Telnet itself is told too, once.
        assert_eq!(rules(&told), ["cleartext-login", "login-burst"]);
        assert_eq!(told[1].severity, Severity::High);
        assert!(told[1].detail.contains("Telnet"), "{}", told[1].detail);
    }

    #[test]
    fn resolving_outside_the_network_s_dns_is_told_once_per_resolver() {
        let gateway: Ipv4Addr = "192.168.1.1".parse().unwrap();
        let mut audit = Audit::new(
            State::default(),
            Settings {
                learn_ms: u64::MAX,
                gateway: Some(gateway),
                gateway_v6: None,
            },
        );
        audit.see(&Packet {
            arp: Some((ROUTER.into(), gateway)),
            ..from(ROUTER, "192.168.1.1", 1)
        });
        let ask = |mac: &str, source: &str, to: &str, at_ms: u64| Packet {
            destination: Some(to.parse().unwrap()),
            udp: Some((50_000, 53)),
            dns_question: Some("www.example.com".into()),
            ..from(mac, source, at_ms)
        };
        // Asking the router is the network's own DNS; the router asking upstream forwards.
        assert!(audit
            .see(&ask(TV, "192.168.1.84", "192.168.1.1", 1_000))
            .is_empty());
        assert!(audit
            .see(&ask(ROUTER, "203.0.113.5", "8.8.8.8", 1_100))
            .is_empty());
        // A Chromecast asking 8.8.8.8 itself: told once.
        let told = audit.see(&ask(TV, "192.168.1.84", "8.8.8.8", 2_000));
        assert_eq!(rules(&told), ["dns-bypass"]);
        assert_eq!(told[0].severity, Severity::Notice);
        assert!(
            told[0].detail.contains("plain DNS through 8.8.8.8"),
            "{}",
            told[0].detail
        );
        assert!(audit
            .see(&ask(TV, "192.168.1.84", "8.8.8.8", 3_000))
            .is_empty());
        // DNS over TLS and over HTTPS (by the name the browser asks for) are told apart.
        let dot = audit.see(&Packet {
            destination: Some("1.1.1.1".parse().unwrap()),
            tcp: Some((50_001, 853)),
            opening: true,
            ..from(TV, "192.168.1.84", 4_000)
        });
        assert!(dot[0].detail.contains("DNS over TLS"), "{}", dot[0].detail);
        let doh = audit.see(&Packet {
            destination: Some("104.16.248.249".parse().unwrap()),
            tcp: Some((50_002, 443)),
            server_name: Some("mozilla.cloudflare-dns.com".into()),
            ..from(TV, "192.168.1.84", 5_000)
        });
        assert!(
            doh[0].detail.contains("DNS over HTTPS"),
            "{}",
            doh[0].detail
        );
        // An ordinary site over HTTPS is not.
        assert!(audit
            .see(&Packet {
                destination: Some("93.184.216.34".parse().unwrap()),
                tcp: Some((50_003, 443)),
                server_name: Some("www.example.com".into()),
                ..from(TV, "192.168.1.84", 6_000)
            })
            .is_empty());
    }

    #[test]
    fn a_device_speaking_smb1_is_told_once() {
        let mut audit = Audit::new(State::default(), learned());
        let nas = "00:11:32:00:00:05";
        let smb = |at_ms: u64| Packet {
            destination: Some("192.168.1.60".parse().unwrap()),
            tcp: Some((445, 43_000)),
            smb1: true,
            ..from(nas, "192.168.1.5", at_ms)
        };
        let told = audit.see(&smb(1_000));
        assert_eq!(rules(&told), ["retired-smb"]);
        assert_eq!(told[0].device, nas);
        assert!(
            told[0].detail.contains("with 192.168.1.60"),
            "{}",
            told[0].detail
        );
        assert!(audit.see(&smb(2_000)).is_empty(), "once per device");
    }

    #[test]
    fn a_password_sent_in_the_clear_is_told_by_where_it_went() {
        let mut audit = Audit::new(State::default(), learned());
        let login = |server: &str, protocol: &'static str, at_ms: u64| Packet {
            destination: Some(server.parse().unwrap()),
            tcp: Some((40_000, 2121)),
            cleartext_password: Some(protocol),
            ..from(TV, "192.168.1.84", at_ms)
        };
        let told = audit.see(&login("198.51.100.21", "POP3", 1_000));
        assert_eq!(rules(&told), ["cleartext-password"]);
        assert_eq!(told[0].severity, Severity::High);
        assert!(
            told[0].detail.contains("across the internet"),
            "{}",
            told[0].detail
        );
        assert!(
            audit.see(&login("198.51.100.21", "POP3", 2_000)).is_empty(),
            "once"
        );
        let nas = audit.see(&login("192.168.1.5", "FTP", 3_000));
        assert_eq!(rules(&nas), ["cleartext-password"]);
        assert_eq!(nas[0].severity, Severity::Warning);
    }

    #[test]
    fn many_random_names_that_do_not_exist_are_a_dga() {
        let mut audit = Audit::new(State::default(), learned());
        let nx = |device: &str, name: String, at_ms: u64| Packet {
            at_ms,
            source_mac: Some(ROUTER.into()),
            nxdomain: Some((device.into(), name)),
            ..Packet::default()
        };
        // A Pi-hole blocking trackers, and Chrome's three probes, over and over: quiet.
        let usual: Vec<Alert> = (0..300u64)
            .flat_map(|i| {
                let name = [
                    "securepubads.g.doubleclick.net",
                    "connectivitycheck.gstatic.com",
                    "kqzvtrwbxl",
                    "pmxwqhzjtc",
                    "zbvnrtqwlx",
                ][i as usize % 5];
                audit.see(&nx(TV, name.into(), i * 200))
            })
            .collect();
        assert!(usual.is_empty(), "{usual:?}");
        // A bot trying a generated name every half second: told once.
        let mut seed = 0x9e37_79b9_u64;
        let told: Vec<Alert> = (0..120u64)
            .flat_map(|i| {
                let name: String = (0..13)
                    .map(|_| {
                        seed ^= seed << 13;
                        seed ^= seed >> 7;
                        seed ^= seed << 17;
                        char::from(b'a' + (seed % 26) as u8)
                    })
                    .collect();
                audit.see(&nx(
                    "de:ad:be:ef:00:07",
                    format!("{name}.com"),
                    100_000 + i * 500,
                ))
            })
            .collect();
        assert_eq!(rules(&told), ["dga-suspect"]);
        assert_eq!(told[0].device, "de:ad:be:ef:00:07");
    }

    #[test]
    fn a_redirect_from_anyone_but_the_router_is_told() {
        let gateway: Ipv4Addr = "192.168.1.1".parse().unwrap();
        let settings = Settings {
            learn_ms: u64::MAX,
            gateway: Some(gateway),
            gateway_v6: None,
        };
        let redirect = |mac: &str, ip: &str, at_ms: u64| Packet {
            redirect: true,
            ..from(mac, ip, at_ms)
        };
        // The router not known yet: a warning.
        let mut early = Audit::new(State::default(), settings.clone());
        let unsure = early.see(&redirect("02:00:5e:aa:bb:cc", "192.168.1.66", 1));
        assert_eq!(rules(&unsure), ["icmp-redirect"]);
        assert_eq!(unsure[0].severity, Severity::Warning);
        // Once the router is known, the same sender is high.
        early.see(&Packet {
            at_ms: 2,
            arp: Some((ROUTER.into(), gateway)),
            ..Packet::default()
        });
        let sure = early.see(&redirect("02:00:5e:aa:bb:cc", "192.168.1.66", 3));
        assert_eq!(sure[0].severity, Severity::High);
        // Known: the router's own redirect is quiet, anyone else's is high, once.
        let mut audit = Audit::new(State::default(), settings);
        audit.see(&Packet {
            at_ms: 1,
            arp: Some((ROUTER.into(), gateway)),
            ..Packet::default()
        });
        assert!(audit.see(&redirect(ROUTER, "192.168.1.1", 2)).is_empty());
        let told: Vec<Alert> = (0..3)
            .flat_map(|i| {
                audit.see(&redirect(
                    "02:00:5e:aa:bb:cc",
                    "fe80::5eff:feaa:bbcc",
                    3 + i,
                ))
            })
            .collect();
        assert_eq!(rules(&told), ["icmp-redirect"]);
        assert_eq!(told[0].severity, Severity::High);
    }

    #[test]
    fn a_device_offering_only_retired_tls_is_told_once() {
        let mut audit = Audit::new(State::default(), learned());
        let hello = |mac: &str, ja4: &str, at_ms: u64| Packet {
            ja4: Some(ja4.into()),
            server_name: Some("cam.example.net".into()),
            ..from(mac, "192.168.1.60", at_ms)
        };
        assert!(audit
            .see(&hello(TV, "t13d1516h2_8daaf6152771_02713d6af862", 1))
            .is_empty());
        let camera = "00:12:12:00:00:01";
        let told: Vec<Alert> = (0..3)
            .flat_map(|i| {
                audit.see(&hello(
                    camera,
                    "t10d030100_5704bdfc91c6_e3b0c44298fc",
                    10 + i,
                ))
            })
            .collect();
        assert_eq!(rules(&told), ["retired-tls"]);
        assert_eq!(told[0].severity, Severity::Notice);
        assert!(
            told[0].detail.contains("TLS 1.0 (to cam.example.net)"),
            "{}",
            told[0].detail
        );
    }

    #[test]
    fn a_dhcpv6_server_that_is_not_the_router_is_told() {
        let gateway: Ipv4Addr = "192.168.1.1".parse().unwrap();
        let mut audit = Audit::new(
            State::default(),
            Settings {
                learn_ms: 60_000,
                gateway: Some(gateway),
                gateway_v6: None,
            },
        );
        let arp = Packet {
            at_ms: 1_000,
            arp: Some((ROUTER.into(), gateway)),
            ..Packet::default()
        };
        audit.see(&arp);
        let answer = |mac: &str, ip: &str, at_ms: u64| Packet {
            dhcpv6_answer: true,
            ..from(mac, ip, at_ms)
        };
        // The router serving DHCPv6, while learning and after: quiet.
        assert!(audit.see(&answer(ROUTER, "fe80::1", 2_000)).is_empty());
        assert!(audit.see(&answer(ROUTER, "fe80::1", 90_000)).is_empty());
        // mitm6 on a laptop, after learning: told once.
        let mitm6 = "02:00:5e:77:88:99";
        let told: Vec<Alert> = audit
            .see(&answer(mitm6, "fe80::77:8899", 100_000))
            .into_iter()
            .filter(|a| a.rule != "new-device")
            .collect();
        assert_eq!(rules(&told), ["rogue-dhcpv6-server"]);
        assert_eq!(told[0].severity, Severity::High);
        assert!(audit
            .see(&answer(mitm6, "fe80::77:8899", 101_000))
            .is_empty());
        // Kept in the state, and the state still loads.
        let saved = serde_json::to_vec(&audit.state).unwrap();
        assert_eq!(State::parse(&saved).unwrap().dhcpv6_servers.len(), 2);
    }

    #[test]
    fn answering_for_other_machines_names_is_told_once() {
        let mut audit = Audit::new(State::default(), learned());
        let answer = |mac: &str, ip: &str, name: &str, at_ms: u64| Packet {
            answered_name: Some(name.into()),
            ..from(mac, ip, at_ms)
        };
        // A Windows machine answers for its own name, again and again: quiet.
        let own: Vec<Alert> = (0..10)
            .flat_map(|i| audit.see(&answer(TV, "192.168.1.84", "desktop-archie", i)))
            .collect();
        assert!(own.is_empty());
        // Responder answers for whatever is asked.
        let rogue = "02:00:5e:11:22:33";
        let mut told = Vec::new();
        for (i, name) in ["fileserv", "printer01", "nas", "intranet"]
            .iter()
            .enumerate()
        {
            told.extend(audit.see(&answer(rogue, "192.168.1.99", name, 100 + i as u64)));
        }
        assert_eq!(rules(&told), ["name-poisoning"]);
        assert_eq!(told[0].severity, Severity::High);
        assert!(
            told[0].detail.contains("fileserv, nas, printer01"),
            "{}",
            told[0].detail
        );
        // wpad alone is enough.
        let wpad = audit.see(&answer("02:00:5e:44:55:66", "192.168.1.98", "wpad", 200));
        assert_eq!(rules(&wpad), ["name-poisoning"]);
        assert!(wpad[0].detail.contains("proxy"), "{}", wpad[0].detail);
    }

    #[test]
    fn a_flood_of_made_up_hardware_addresses_is_one_alert_and_kept_out() {
        let mut audit = Audit::new(
            State::default(),
            Settings {
                learn_ms: 0,
                gateway: None,
                gateway_v6: None,
            },
        );
        let made_up = |i: u64| format!("0a:00:00:00:{:02x}:{:02x}", i / 256, i % 256);
        // The first minute everything is new: a busy network's devices arriving is quiet.
        let arriving: Vec<Alert> = (0..40)
            .flat_map(|i| audit.see(&from(&made_up(i), "192.168.1.20", 1 + i * 1_000)))
            .filter(|a| a.rule == "mac-flood")
            .collect();
        assert!(arriving.is_empty());
        assert_eq!(audit.state.devices.len(), 40);
        // Afterwards a guest's phone and laptop are two new devices, not a flood.
        let guests: Vec<Alert> = (0..2)
            .flat_map(|i| audit.see(&from(&made_up(500 + i), "192.168.1.30", 120_000 + i)))
            .collect();
        assert_eq!(rules(&guests), ["new-device", "new-device"]);
        // macof: a thousand made-up senders in a second, most from outside addresses.
        let flood: Vec<Alert> = (1_000..2_000)
            .flat_map(|i| {
                let source = if i % 10 == 0 {
                    "192.168.1.77"
                } else {
                    "45.3.2.1"
                };
                audit.see(&from(&made_up(i), source, 300_000 + i))
            })
            .collect();
        let told = rules(&flood);
        assert_eq!(
            told.iter().filter(|r| **r == "mac-flood").count(),
            1,
            "{told:?}"
        );
        assert!(
            told.iter().filter(|r| **r == "new-device").count() < FLOOD_SENDERS,
            "{told:?}"
        );
        assert!(audit.state.devices.len() < 42 + FLOOD_SENDERS);
        // Known devices go on being seen during it.
        audit.see(&from(&made_up(3), "192.168.1.20", 300_500));
        assert_eq!(audit.state.devices[&made_up(3)].last_seen_ms, 300_500);
    }

    #[test]
    fn a_flood_of_unknown_dhcp_clients_is_told_once_an_hour() {
        let mut audit = Audit::new(State::default(), learned());
        let ask = |client: String, sender: &str, at_ms: u64| Packet {
            at_ms,
            source_mac: Some(sender.into()),
            source: Some("0.0.0.0".parse().unwrap()),
            dhcp_message: Some(1),
            dhcp_client: Some(client),
            ..Packet::default()
        };
        let client = |i: u64| format!("02:00:00:00:{:02x}:{:02x}", i / 256, i % 256);
        // A busy evening: 29 new clients in a minute, and known ones renewing, is quiet.
        audit.see(&from(TV, "192.168.1.84", 1));
        let evening: Vec<Alert> = (0..29)
            .map(|i| ask(client(i), &client(i), 1_000 + i * 2_000))
            .chain((0..50).map(|i| ask(TV.into(), TV, 2_000 + i * 1_000)))
            .flat_map(|p| audit.see(&p))
            .collect();
        assert!(evening.is_empty(), "{evening:?}");
        // Spread over more than a minute, more clients are still not a flood.
        let later: Vec<Alert> = (100..140)
            .flat_map(|i| audit.see(&ask(client(i), &client(i), 200_000 + i * 3_000)))
            .collect();
        assert!(later.is_empty(), "{later:?}");
        // One sender behind 40 client addresses in ten seconds is.
        let attacker = "de:ad:be:ef:00:66";
        let flood: Vec<Alert> = (1_000..1_200)
            .flat_map(|i| audit.see(&ask(client(i), attacker, 900_000 + i * 50)))
            .collect();
        assert_eq!(rules(&flood), ["dhcp-starvation"]);
        assert_eq!(flood[0].severity, Severity::High);
        assert_eq!(flood[0].device, attacker);
        assert!(
            flood[0].detail.contains("one Ethernet sender"),
            "{}",
            flood[0].detail
        );
        // An hour later it is told again if it goes on.
        let again: Vec<Alert> = (2_000..2_040)
            .flat_map(|i| {
                audit.see(&ask(
                    client(i),
                    attacker,
                    900_000 + SURGE_COOLDOWN_MS + i * 50,
                ))
            })
            .collect();
        assert_eq!(rules(&again), ["dhcp-starvation"]);
    }

    #[test]
    fn a_learned_device_calling_somewhere_new_on_a_clock_is_told_once() {
        let mut audit = Audit::new(State::default(), learned());
        let open = |peer: &str, at_ms: u64| Packet {
            destination: Some(peer.parse().unwrap()),
            tcp: Some((50_000, 443)),
            opening: true,
            ..from(TV, "192.168.1.84", at_ms)
        };
        // Its vendor's keep-alive, every ten minutes from the start: kept, not told.
        let day = u64::from(BASELINE_BUCKETS) + 1;
        for bucket in 1..=day {
            audit.see(&open("198.51.100.20", bucket * BUCKET_MS));
        }
        assert!(audit.state.devices[TV].baseline.learned());
        assert_eq!(
            audit.state.devices[TV].beacons["198.51.100.20"].every_s,
            BUCKET_MS / 1000
        );
        // Then it starts checking in with somewhere new every minute, with jitter.
        let now = (day + 1) * BUCKET_MS;
        let mut told: Vec<Alert> = (0..40u64)
            .flat_map(|i| {
                audit.see(&open(
                    "203.0.113.9",
                    now + i * 60_000 + ((i * 2_654_435_761) >> 7) % 6_000,
                ))
            })
            .collect();
        told.extend(audit.see(&open("198.51.100.20", now + 41 * 60_000)));
        assert_eq!(rules(&told), ["beacon-suspect"]);
        assert!(told[0].detail.contains("203.0.113.9"), "{}", told[0].detail);
        let beacon = &audit.state.devices[TV].beacons["203.0.113.9"];
        assert!((58..=66).contains(&beacon.every_s), "{beacon:?}");
        assert!(beacon.score >= BEACON_TOLD_SCORE, "{beacon:?}");
        // It survives a save and a load.
        let saved = serde_json::to_vec(&audit.state).unwrap();
        assert_eq!(State::parse(&saved).unwrap().devices[TV].beacons.len(), 2);
    }

    #[test]
    fn a_clock_over_udp_is_scored_by_its_sessions_not_its_datagrams() {
        let mut audit = Audit::new(State::default(), learned());
        let send = |peer: &str, port: u16, at_ms: u64| Packet {
            destination: Some(peer.parse().unwrap()),
            udp: Some((50_000, port)),
            ..from(TV, "192.168.1.84", at_ms)
        };
        let day = u64::from(BASELINE_BUCKETS) + 1;
        for bucket in 1..=day {
            audit.see(&send("198.51.100.20", 123, bucket * BUCKET_MS));
        }
        assert!(audit.state.devices[TV].baseline.learned());
        let now = (day + 1) * BUCKET_MS;
        // A long QUIC session, a datagram every 50 ms for ten minutes: one contact, no rhythm.
        for i in 0..12_000u64 {
            assert!(audit.see(&send("192.0.2.80", 443, now + i * 50)).is_empty());
        }
        assert!(!audit.state.devices[TV].beacons.contains_key("192.0.2.80"));
        // A check-in over UDP every two minutes, with jitter, each a burst of three
        // datagrams: one contact per burst, told once.
        let told: Vec<Alert> = (0..30u64)
            .flat_map(|i| {
                let at = now + i * 120_000 + ((i * 2_654_435_761) >> 7) % 8_000;
                (0..3).map(move |d| at + d * 200)
            })
            .flat_map(|at| audit.see(&send("203.0.113.9", 4_500, at)))
            .collect();
        assert_eq!(rules(&told), ["beacon-suspect"]);
        let beacon = &audit.state.devices[TV].beacons["203.0.113.9"];
        assert!((116..=128).contains(&beacon.every_s), "{beacon:?}");
    }

    #[test]
    fn a_cdn_s_rotating_addresses_add_up_to_the_name_the_device_resolved() {
        let mut audit = Audit::new(State::default(), learned());
        let day = u64::from(BASELINE_BUCKETS) + 1;
        for bucket in 1..=day {
            audit.see(&Packet {
                destination: Some("198.51.100.20".parse().unwrap()),
                tcp: Some((50_000, 443)),
                opening: true,
                ..from(TV, "192.168.1.84", bucket * BUCKET_MS)
            });
        }
        let now = (day + 1) * BUCKET_MS;
        // Each minute it resolves its controller's name and gets another address for it.
        let told: Vec<Alert> = (0..40u64)
            .flat_map(|i| {
                let at = now + i * 60_000 + ((i * 2_654_435_761) >> 7) % 6_000;
                let address: IpAddr = format!("203.0.113.{}", 10 + i % 30).parse().unwrap();
                let answer = Packet {
                    destination: Some("192.168.1.84".parse().unwrap()),
                    udp: Some((53, 50_000)),
                    dns_answer: true,
                    resolved: Some(("c2.example.net".into(), vec![address])),
                    ..from(ROUTER, "192.168.1.1", at)
                };
                let call = Packet {
                    destination: Some(address),
                    tcp: Some((50_000, 443)),
                    opening: true,
                    ..from(TV, "192.168.1.84", at + 20)
                };
                [answer, call]
            })
            .flat_map(|packet| audit.see(&packet))
            .collect();
        assert_eq!(rules(&told), ["beacon-suspect"]);
        assert!(
            told[0].detail.contains("c2.example.net"),
            "{}",
            told[0].detail
        );
        assert!(!audit.state.devices[TV].sent_to.contains_key("203.0.113.10"));
        // An answer naming a LAN address does not name it.
        let lan = Packet {
            destination: Some("192.168.1.84".parse().unwrap()),
            udp: Some((53, 50_000)),
            dns_answer: true,
            resolved: Some((
                "nas.example.net".into(),
                vec!["192.168.1.5".parse().unwrap()],
            )),
            ..from(ROUTER, "192.168.1.1", now + 50 * 60_000)
        };
        audit.see(&lan);
        assert!(!audit
            .named
            .contains_key(&"192.168.1.5".parse::<IpAddr>().unwrap()));
    }

    #[test]
    fn a_learned_device_sending_much_to_somewhere_new_is_told_once() {
        let mut audit = Audit::new(State::default(), learned());
        let to = |peer: &str, at_ms: u64, length: u64| Packet {
            length,
            destination: Some(peer.parse().unwrap()),
            ..from(TV, "192.168.1.84", at_ms)
        };
        // Before the device is learned, a big new destination is learning, not news.
        let early: Vec<Alert> = (0..30)
            .flat_map(|i| audit.see(&to("9.9.9.9", 1 + i, 1_000_000)))
            .collect();
        assert!(early.is_empty());
        // A day of ordinary traffic to its usual place.
        let day = u64::from(BASELINE_BUCKETS) + 1;
        for bucket in 1..=day {
            audit.see(&to("1.1.1.1", bucket * BUCKET_MS, 1_000_000));
        }
        assert!(audit.state.devices[TV].baseline.learned());
        let now = (day + 1) * BUCKET_MS;
        // Much more to its usual place, and to 9.9.9.9 (seen while learning): no news.
        let usual: Vec<Alert> = (0..30)
            .flat_map(|i| {
                let mut a = audit.see(&to("1.1.1.1", now + i, 1_000_000));
                a.extend(audit.see(&to("9.9.9.9", now + i, 1_000_000)));
                a
            })
            .filter(|a| a.rule == "unusual-destination")
            .collect();
        assert!(usual.is_empty());
        // Somewhere it never used: told once, when it crosses the line.
        let told: Vec<Alert> = (0..60)
            .flat_map(|i| audit.see(&to("203.0.113.7", now + 100 + i, 1_000_000)))
            .filter(|a| a.rule == "unusual-destination")
            .collect();
        assert_eq!(rules(&told), ["unusual-destination"]);
        assert!(told[0].detail.contains("203.0.113.7"), "{}", told[0].detail);
        assert_eq!(told[0].severity, Severity::Warning);
    }

    #[test]
    fn nothing_is_judged_before_a_day_is_learned_and_small_devices_have_a_floor() {
        let mut audit = Audit::new(State::default(), learned());
        // A burst on the first day is learning, not news.
        let burst: Vec<Alert> = (0..100)
            .flat_map(|i| {
                audit.see(&Packet {
                    length: 1_000_000,
                    ..from(TV, "192.168.1.84", 1 + i * 1_000)
                })
            })
            .collect();
        assert!(burst.is_empty());
        // A device that usually sends nothing still needs the floor to surge.
        let quiet = Baseline {
            buckets: BASELINE_BUCKETS,
            ..Baseline::default()
        };
        assert_eq!(quiet.surge_line(), SURGE_MIN_BYTES);
        // A state written before baselines loads with an empty one.
        let old = r#"{"first_seen_ms":1,"last_seen_ms":2,"addresses":[],"hostname":null,"frames":1,"bytes":1,"asked":[],"sent_to":{}}"#;
        let device: Device = serde_json::from_str(old).unwrap();
        assert_eq!(device.baseline, Baseline::default());
    }

    fn advert(mac: &str, ip: &str, at_ms: u64) -> Packet {
        Packet {
            at_ms,
            neighbor: Some((mac.into(), ip.parse().unwrap())),
            ..Packet::default()
        }
    }

    #[test]
    fn an_ipv6_address_answered_for_by_another_device_is_a_move_too() {
        let mut audit = Audit::new(
            State::default(),
            Settings {
                learn_ms: u64::MAX,
                gateway: None,
                gateway_v6: Some("fe80::1".parse().unwrap()),
            },
        );
        assert!(audit.see(&advert(ROUTER, "fe80::1", 1_000)).is_empty());
        assert!(audit.see(&advert(TV, "2001:db8::84", 1_100)).is_empty());
        // Neighbor Advertisement spoofing is ARP spoofing's IPv6 twin.
        let spoofed = audit.see(&advert("de:ad:be:ef:00:02", "fe80::1", 2_000));
        assert_eq!(
            (spoofed[0].rule.as_str(), spoofed[0].severity),
            ("address-moved", Severity::High)
        );
        let other = audit.see(&advert("aa:bb:cc:00:11:22", "2001:db8::84", 3_000));
        assert_eq!(
            (other[0].rule.as_str(), other[0].severity),
            ("address-moved", Severity::Notice)
        );
        // The same device answering again is no news.
        assert!(audit
            .see(&advert("aa:bb:cc:00:11:22", "2001:db8::84", 4_000))
            .is_empty());
    }

    #[test]
    fn a_state_written_before_ipv6_owners_still_loads() {
        let old = r#"{"format":"archie-netwatch-state-v1","started_ms":5,"devices":{},"owners":{"192.168.1.1":"e4:ab:89:6e:1a:18"}}"#;
        let state: State = serde_json::from_str(old).unwrap();
        assert_eq!(
            state
                .owners
                .get(&"192.168.1.1".parse::<IpAddr>().unwrap())
                .map(String::as_str),
            Some("e4:ab:89:6e:1a:18")
        );
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
            gateway_v6: None,
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
                gateway_v6: None,
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

    fn dhcp(mac: &str, source: &str, message: u8, at_ms: u64) -> Packet {
        Packet {
            dhcp_message: Some(message),
            dhcp_server: source.parse().ok(),
            ..from(mac, source, at_ms)
        }
    }

    #[test]
    fn a_second_dhcp_server_is_told_and_the_first_is_learned() {
        let mut audit = Audit::new(
            State::default(),
            Settings {
                learn_ms: 60_000,
                ..learned()
            },
        );
        // The router answers while the network is being learned: nothing to tell.
        assert!(audit.see(&dhcp(ROUTER, "192.168.1.1", 2, 1_000)).is_empty());
        assert!(audit
            .see(&dhcp(ROUTER, "192.168.1.1", 5, 90_000))
            .is_empty());
        let rogue = audit.see(&dhcp("02:00:5e:aa:bb:cc", "192.168.1.66", 2, 95_000));
        assert!(rules(&rogue).contains(&"rogue-dhcp-server"));
        let alert = rogue
            .iter()
            .find(|a| a.rule == "rogue-dhcp-server")
            .unwrap();
        assert_eq!(alert.severity, Severity::High);
        assert!(alert.detail.contains(ROUTER));
        // Told once, and a client's own DHCP request is never a server.
        assert!(
            !rules(&audit.see(&dhcp("02:00:5e:aa:bb:cc", "192.168.1.66", 5, 96_000)))
                .contains(&"rogue-dhcp-server")
        );
        assert!(
            !rules(&audit.see(&dhcp(TV, "192.168.1.84", 3, 97_000))).contains(&"rogue-dhcp-server")
        );
        assert_eq!(audit.state.dhcp_servers.len(), 2);
    }

    #[test]
    fn a_router_advertisement_from_anyone_but_the_router_is_told() {
        let settings = Settings {
            gateway: Some("192.168.1.1".parse().unwrap()),
            ..learned()
        };
        let mut audit = Audit::new(State::default(), settings);
        audit.settings.learn_ms = 0;
        let arp = Packet {
            arp: Some((ROUTER.into(), "192.168.1.1".parse().unwrap())),
            ..Packet::default()
        };
        audit.see(&arp);
        let advert = |mac: &str, at_ms| Packet {
            router_advertisement: true,
            ..from(mac, "fe80::1", at_ms)
        };
        // The router itself offering IPv6 is the router.
        assert!(!rules(&audit.see(&advert(ROUTER, 1_000))).contains(&"rogue-router-advertisement"));
        let rogue = audit.see(&advert("02:00:5e:aa:bb:cc", 2_000));
        let alert = rogue
            .iter()
            .find(|a| a.rule == "rogue-router-advertisement")
            .expect("told");
        assert_eq!(alert.severity, Severity::High);
    }

    #[test]
    fn what_a_device_says_before_it_has_an_address_is_kept_for_it() {
        let mut audit = Audit::new(State::default(), learned());
        // A DHCP discover from 0.0.0.0 and an LLDP announcement carry no LAN address.
        audit.see(&Packet {
            source_mac: Some(TV.into()),
            source: Some("0.0.0.0".parse().unwrap()),
            dhcp_message: Some(1),
            dhcp_requested: Some("1,3,6,15,119,252".into()),
            dhcp_vendor: Some("android-dhcp-14".into()),
            ..Packet::default()
        });
        audit.see(&Packet {
            source_mac: Some(TV.into()),
            lldp_name: Some("living-room".into()),
            ..Packet::default()
        });
        assert!(audit.state.devices.is_empty());
        audit.see(&Packet {
            opening: true,
            tcp: Some((50_000, 443)),
            destination: Some("93.184.216.34".parse().unwrap()),
            hops: Some(63),
            ja4: Some("t13d1516h2_8daaf6152771_02713d6af862".into()),
            ..from(TV, "192.168.1.84", 5_000)
        });
        let device = &audit.state.devices[TV];
        assert_eq!(device.dhcp_requested.as_deref(), Some("1,3,6,15,119,252"));
        assert_eq!(device.dhcp_vendor.as_deref(), Some("android-dhcp-14"));
        assert_eq!(device.lldp_name.as_deref(), Some("living-room"));
        assert_eq!(device.hops, Some(64));
        assert_eq!(device.ja4.len(), 1);
        let profile = crate::fingerprint::profile(TV, device);
        assert!(profile.hints.iter().any(|hint| hint.says == "Android"));
        // It survives a restart, and a file written before these fields still loads.
        let saved = serde_json::to_vec(&audit.state).unwrap();
        assert_eq!(State::parse(&saved).unwrap().devices[TV], *device);
        let old = serde_json::json!({"format": "archie-netwatch-state-v1", "started_ms": 1,
            "devices": {}, "owners": {}});
        assert!(State::parse(&serde_json::to_vec(&old).unwrap()).is_ok());
    }
}
