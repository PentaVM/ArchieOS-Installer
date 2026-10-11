//! mDNS service discovery: the announcements printers, televisions, speakers and home
//! hubs make so that phones on the same network find them.
//!
//! The browse asks which kinds of service exist (`_services._dns-sd._udp.local`), then
//! asks for each kind it hears of. Questions are sent with the "answer me directly" bit,
//! so replies come to this socket and port 5353, which the system's own resolver usually
//! holds, is left alone. A device that ignores that bit and answers only to the group is
//! not heard; SSDP finds most of those.

use crate::is_lan;
use std::collections::{BTreeMap, BTreeSet};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

const GROUP: SocketAddr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(224, 0, 0, 251)), 5353);
const META: &str = "_services._dns-sd._udp.local";
const PTR: u16 = 12;
const TXT: u16 = 16;
const SRV: u16 = 33;
/// The most kinds of service one browse follows: a device may not make it ask forever.
const MAX_KINDS: usize = 64;
/// More hosts than a home or office network holds; past it, new ones are not learned.
const MAX_HOSTS: usize = 1024;
const MAX_SERVICES: usize = 64;

/// What one record says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Data {
    /// `name` points at this: a kind of service, or one instance of a kind.
    Pointer(String),
    Service { target: String, port: u16 },
    Text(Vec<String>),
    Other,
}

/// A question for every `PTR` under `name`, wanting a direct answer.
pub fn question(name: &str) -> Vec<u8> {
    let mut packet = vec![0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0];
    for label in name.split('.') {
        packet.push(label.len().min(63) as u8);
        packet.extend_from_slice(&label.as_bytes()[..label.len().min(63)]);
    }
    packet.push(0);
    packet.extend_from_slice(&PTR.to_be_bytes());
    packet.extend_from_slice(&0x8001u16.to_be_bytes());
    packet
}

/// A name at `at`, following compression pointers. Returns it and where reading goes on.
fn name_at(packet: &[u8], mut at: usize) -> Option<(String, usize)> {
    let mut labels: Vec<String> = Vec::new();
    let mut after = None;
    let mut jumps = 0;
    loop {
        let length = *packet.get(at)? as usize;
        if length == 0 {
            return Some((labels.join("."), after.unwrap_or(at + 1)));
        }
        if length & 0xc0 == 0xc0 {
            // A pointer to an earlier name. A packet that points in circles is dropped.
            jumps += 1;
            if jumps > 16 {
                return None;
            }
            after.get_or_insert(at + 2);
            at = ((length & 0x3f) << 8) | *packet.get(at + 1)? as usize;
            continue;
        }
        let label = packet.get(at + 1..at + 1 + length)?;
        labels.push(String::from_utf8_lossy(label).chars().filter(|c| !c.is_control()).collect());
        if labels.iter().map(String::len).sum::<usize>() > 255 {
            return None;
        }
        at += 1 + length;
    }
}

/// Every record in a response, as (the name it is about, what it says). A packet that is
/// cut short or malformed gives what was read before the damage.
pub fn records(packet: &[u8]) -> Vec<(String, Data)> {
    let count = |at: usize| packet.get(at..at + 2).map_or(0, |b| u16::from_be_bytes([b[0], b[1]]) as usize);
    let mut out = Vec::new();
    if packet.len() < 12 || packet[2] & 0x80 == 0 {
        return out; // too short, or a question rather than an answer
    }
    let mut at = 12;
    for _ in 0..count(4) {
        let Some((_, next)) = name_at(packet, at) else { return out };
        at = next + 4;
    }
    for _ in 0..(count(6) + count(8) + count(10)).min(512) {
        let Some((name, next)) = name_at(packet, at) else { return out };
        let Some(head) = packet.get(next..next + 10) else { return out };
        let kind = u16::from_be_bytes([head[0], head[1]]);
        let length = u16::from_be_bytes([head[8], head[9]]) as usize;
        let start = next + 10;
        let Some(data) = packet.get(start..start + length) else { return out };
        let said = match kind {
            PTR => name_at(packet, start).map_or(Data::Other, |(target, _)| Data::Pointer(target)),
            SRV if length > 6 => name_at(packet, start + 6).map_or(Data::Other, |(target, _)| Data::Service {
                target,
                port: u16::from_be_bytes([data[4], data[5]]),
            }),
            TXT => {
                let mut strings = Vec::new();
                let mut cursor = 0;
                while let Some(&size) = data.get(cursor) {
                    let Some(text) = data.get(cursor + 1..cursor + 1 + size as usize) else { break };
                    if !text.is_empty() {
                        strings.push(String::from_utf8_lossy(text).chars().filter(|c| !c.is_control()).take(200).collect());
                    }
                    cursor += 1 + size as usize;
                }
                Data::Text(strings)
            }
            _ => Data::Other,
        };
        out.push((name, said));
        at = start + length;
    }
    out
}

/// What one host announced.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Host {
    /// The kinds of service it offers, e.g. `_ipp._tcp`.
    pub services: BTreeSet<String>,
    /// The name it gives itself in an announcement, e.g. `Living Room TV`.
    pub name: Option<String>,
    /// `key=value` facts from its announcements, e.g. `md` (model) on a Chromecast.
    pub facts: BTreeMap<String, String>,
}

/// `Living Room TV._googlecast._tcp.local` → (`Living Room TV`, `_googlecast._tcp`).
fn instance_of(name: &str) -> Option<(String, String)> {
    let name = name.strip_suffix(".local").unwrap_or(name);
    let at = name.find("._")?;
    let (instance, kind) = (&name[..at], &name[at + 1..]);
    (!instance.is_empty() && !instance.starts_with('_')).then(|| (instance.to_string(), kind.to_string()))
}

/// Folds one host's records into what is known of it, and returns the kinds of service
/// the packet named, for the browse to ask about next.
pub fn learn(host: &mut Host, packet: &[u8]) -> Vec<String> {
    let mut kinds = Vec::new();
    for (name, data) in records(packet) {
        match data {
            Data::Pointer(target) if name == META => kinds.push(target),
            Data::Pointer(target) => {
                if let Some((instance, kind)) = instance_of(&target) {
                    if host.services.len() < MAX_SERVICES {
                        host.services.insert(kind);
                    }
                    host.name.get_or_insert(instance);
                }
            }
            Data::Service { .. } => {
                if let Some((_, kind)) = instance_of(&name) {
                    if host.services.len() < MAX_SERVICES {
                        host.services.insert(kind);
                    }
                }
            }
            Data::Text(strings) => {
                for (key, value) in strings.iter().filter_map(|s| s.split_once('=')) {
                    if matches!(key, "md" | "fn" | "ty" | "model" | "am" | "manufacturer") && !value.is_empty() {
                        host.facts.entry(key.to_string()).or_insert_with(|| value.to_string());
                    }
                }
            }
            Data::Other => {}
        }
    }
    kinds
}

/// Browses the network `local` is on for `listen` and returns what each host announced.
pub fn browse(local: Ipv4Addr, listen: Duration) -> std::io::Result<BTreeMap<IpAddr, Host>> {
    let socket = UdpSocket::bind((local, 0))?;
    socket.set_read_timeout(Some(Duration::from_millis(250)))?;
    socket.send_to(&question(META), GROUP)?;
    let mut asked: BTreeSet<String> = BTreeSet::new();
    let mut hosts: BTreeMap<IpAddr, Host> = BTreeMap::new();
    let until = Instant::now() + listen;
    let mut buffer = [0u8; 9000];
    while Instant::now() < until {
        match socket.recv_from(&mut buffer) {
            Ok((length, from)) if is_lan(from.ip()) => {
                if hosts.len() >= MAX_HOSTS && !hosts.contains_key(&from.ip()) {
                    continue;
                }
                for kind in learn(hosts.entry(from.ip()).or_default(), &buffer[..length]) {
                    let known = kind.ends_with(".local") && kind.starts_with('_') && kind.len() <= 80;
                    if known && asked.len() < MAX_KINDS && asked.insert(kind.clone()) {
                        socket.send_to(&question(&kind), GROUP)?;
                    }
                }
            }
            Ok(_) => {}
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
            Err(e) => return Err(e),
        }
    }
    hosts.retain(|_, host| !host.services.is_empty());
    Ok(hosts)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name(text: &str) -> Vec<u8> {
        let mut out = Vec::new();
        for label in text.split('.') {
            out.push(label.len() as u8);
            out.extend_from_slice(label.as_bytes());
        }
        out.push(0);
        out
    }

    /// A response with the given records: (name bytes, type, rdata).
    fn response(records: &[(Vec<u8>, u16, Vec<u8>)]) -> Vec<u8> {
        let mut packet = vec![0, 0, 0x84, 0, 0, 0, 0, records.len() as u8, 0, 0, 0, 0];
        for (owner, kind, data) in records {
            packet.extend_from_slice(owner);
            packet.extend_from_slice(&kind.to_be_bytes());
            packet.extend_from_slice(&[0, 1, 0, 0, 0, 120]);
            packet.extend_from_slice(&(data.len() as u16).to_be_bytes());
            packet.extend_from_slice(data);
        }
        packet
    }

    #[test]
    fn a_question_asks_for_pointers_and_a_direct_answer() {
        let packet = question("_ipp._tcp.local");
        assert_eq!(&packet[..12], [0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0]);
        assert_eq!(&packet[12..17], b"\x04_ipp");
        assert_eq!(&packet[packet.len() - 4..], [0, 12, 0x80, 1]);
        assert!(records(&packet).is_empty(), "a question holds no answers");
    }

    #[test]
    fn a_television_announcing_itself_is_named_and_its_model_kept() {
        let instance = "Living Room TV._googlecast._tcp.local";
        let mut srv = vec![0, 0, 0, 0, 0x1f, 0x49];
        srv.extend(name("tv-1234.local"));
        let packet = response(&[
            (name("_googlecast._tcp.local"), PTR, name(instance)),
            (name(instance), SRV, srv),
            (name(instance), TXT, b"\x10md=Chromecast HD\x09fn=Living\x05rs=on".to_vec()),
        ]);
        let mut host = Host::default();
        assert!(learn(&mut host, &packet).is_empty());
        assert_eq!(host.name.as_deref(), Some("Living Room TV"));
        assert!(host.services.contains("_googlecast._tcp"));
        assert_eq!(host.facts["md"], "Chromecast HD");
        assert!(!host.facts.contains_key("rs"), "only what describes the device is kept");
    }

    #[test]
    fn the_kinds_a_host_lists_are_what_the_browse_asks_about_next() {
        let packet = response(&[
            (name(META), PTR, name("_ipp._tcp.local")),
            (name(META), PTR, name("_airplay._tcp.local")),
        ]);
        let mut host = Host::default();
        assert_eq!(learn(&mut host, &packet), ["_ipp._tcp.local", "_airplay._tcp.local"]);
        assert!(host.services.is_empty(), "listing a kind is not offering an instance of it");
    }

    #[test]
    fn compressed_names_are_followed_and_a_packet_that_loops_is_dropped() {
        // The owner name sits at offset 12; the PTR data points back at it.
        let mut packet = vec![0, 0, 0x84, 0, 0, 0, 0, 1, 0, 0, 0, 0];
        packet.extend(name("_ipp._tcp.local"));
        packet.extend_from_slice(&[0, 12, 0, 1, 0, 0, 0, 120, 0, 9]);
        packet.extend_from_slice(b"\x06Office\xc0\x0c");
        assert_eq!(records(&packet), [("_ipp._tcp.local".to_string(), Data::Pointer("Office._ipp._tcp.local".into()))]);

        let looping = [0, 0, 0x84, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0xc0, 0x0c, 0, 12, 0, 1, 0, 0, 0, 0, 0, 0];
        assert!(records(&looping).is_empty());
        assert!(records(&packet[..20]).is_empty(), "a packet cut short gives nothing it did not finish");
    }
}
