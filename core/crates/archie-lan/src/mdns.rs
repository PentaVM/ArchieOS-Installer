//! mDNS service discovery: the announcements printers, televisions, speakers and home
//! hubs make so that phones on the same network find them.
//!
//! The browse asks which kinds of service exist (`_services._dns-sd._udp.local`), then
//! asks for each kind it hears of. Questions are sent with the "answer me directly" bit,
//! and a shared multicast listener also receives group-only replies. Failure to open
//! that listener is reported without losing the unicast path. Explicit service queries
//! find responders that do not implement service-type enumeration.

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
    Service {
        target: String,
        port: u16,
    },
    Text(Vec<String>),
    Address(IpAddr),
    Other,
}

/// A question for every `PTR` under `name`, wanting a direct answer.
pub fn question(name: &str) -> Vec<u8> {
    question_type(name, PTR)
}

fn question_type(name: &str, kind: u16) -> Vec<u8> {
    let mut packet = vec![0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0];
    for label in name.split('.') {
        packet.push(label.len().min(63) as u8);
        packet.extend_from_slice(&label.as_bytes()[..label.len().min(63)]);
    }
    packet.push(0);
    packet.extend_from_slice(&kind.to_be_bytes());
    packet.extend_from_slice(&0x8001u16.to_be_bytes());
    packet
}

pub fn initial_questions() -> Vec<Vec<u8>> {
    service_types().iter().map(|kind| question(kind)).collect()
}

pub fn service_types() -> &'static [&'static str] {
    &[
        META,
        "_ipp._tcp.local",
        "_ipps._tcp.local",
        "_printer._tcp.local",
        "_pdl-datastream._tcp.local",
        "_googlecast._tcp.local",
        "_airplay._tcp.local",
        "_raop._tcp.local",
        "_spotify-connect._tcp.local",
        "_companion-link._tcp.local",
        "_apple-mobdev2._tcp.local",
        "_androidtvremote2._tcp.local",
        "_adb-tls-connect._tcp.local",
        "_home-assistant._tcp.local",
        "_hap._tcp.local",
        "_matter._tcp.local",
        "_matterc._udp.local",
        "_matterd._udp.local",
        "_mqtt._tcp.local",
        "_secure-mqtt._tcp.local",
        "_esphomelib._tcp.local",
        "_wled._tcp.local",
        "_shelly._tcp.local",
        "_hue._tcp.local",
        "_http._tcp.local",
        "_https._tcp.local",
        "_rtsp._tcp.local",
        "_ssh._tcp.local",
        "_smb._tcp.local",
        "_workstation._tcp.local",
    ]
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
        labels.push(
            String::from_utf8_lossy(label)
                .chars()
                .filter(|c| !c.is_control())
                .collect(),
        );
        if labels.iter().map(String::len).sum::<usize>() > 255 {
            return None;
        }
        at += 1 + length;
    }
}

/// Every record in a response, as (the name it is about, what it says). A packet that is
/// cut short or malformed gives what was read before the damage.
pub fn records(packet: &[u8]) -> Vec<(String, Data)> {
    let count = |at: usize| {
        packet
            .get(at..at + 2)
            .map_or(0, |b| u16::from_be_bytes([b[0], b[1]]) as usize)
    };
    let mut out = Vec::new();
    if packet.len() < 12 || packet[2] & 0x80 == 0 {
        return out; // too short, or a question rather than an answer
    }
    let mut at = 12;
    for _ in 0..count(4) {
        let Some((_, next)) = name_at(packet, at) else {
            return out;
        };
        at = next + 4;
    }
    for _ in 0..(count(6) + count(8) + count(10)).min(512) {
        let Some((name, next)) = name_at(packet, at) else {
            return out;
        };
        let Some(head) = packet.get(next..next + 10) else {
            return out;
        };
        let kind = u16::from_be_bytes([head[0], head[1]]);
        let length = u16::from_be_bytes([head[8], head[9]]) as usize;
        let start = next + 10;
        let Some(data) = packet.get(start..start + length) else {
            return out;
        };
        let said = match kind {
            1 if length == 4 => Data::Address(IpAddr::V4(Ipv4Addr::new(
                data[0], data[1], data[2], data[3],
            ))),
            28 if length == 16 => Data::Address(IpAddr::V6(std::net::Ipv6Addr::from(
                <[u8; 16]>::try_from(data).unwrap(),
            ))),
            PTR => name_at(packet, start).map_or(Data::Other, |(target, _)| Data::Pointer(target)),
            SRV if length > 6 => {
                name_at(packet, start + 6).map_or(Data::Other, |(target, _)| Data::Service {
                    target,
                    port: u16::from_be_bytes([data[4], data[5]]),
                })
            }
            TXT => {
                let mut strings = Vec::new();
                let mut cursor = 0;
                while let Some(&size) = data.get(cursor) {
                    let Some(text) = data.get(cursor + 1..cursor + 1 + size as usize) else {
                        break;
                    };
                    if !text.is_empty() {
                        strings.push(
                            String::from_utf8_lossy(text)
                                .chars()
                                .filter(|c| !c.is_control())
                                .take(200)
                                .collect(),
                        );
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

fn followup_questions(packet: &[u8]) -> Vec<(String, u16)> {
    let mut queries = Vec::new();
    for (_, data) in records(packet) {
        match data {
            Data::Pointer(instance) if instance_of(&instance).is_some() => {
                queries.push((instance.clone(), SRV));
                queries.push((instance, TXT));
            }
            Data::Service { target, .. } if target.ends_with(".local") => {
                queries.push((target.clone(), 1));
                queries.push((target, 28));
            }
            _ => {}
        }
    }
    queries
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
    pub instances: BTreeMap<String, Service>,
    pub addresses: BTreeMap<String, BTreeSet<IpAddr>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Service {
    pub target: String,
    pub port: u16,
    pub path: Option<String>,
}

impl Host {
    /// A printer URI is emitted only for a SRV target whose A/AAAA record matches
    /// this announcing host. TXT paths belong to the exact same service instance.
    pub fn endpoints(&self, address: IpAddr) -> BTreeMap<String, String> {
        let mut out = BTreeMap::new();
        for (instance, service) in &self.instances {
            let Some((_, kind)) = instance_of(instance) else {
                continue;
            };
            let scheme = match kind.as_str() {
                "_ipp._tcp" => "ipp",
                "_ipps._tcp" => "ipps",
                "_spotify-connect._tcp" => "http",
                _ => continue,
            };
            if service.port == 0
                || !self
                    .addresses
                    .get(&service.target)
                    .is_some_and(|a| a.contains(&address))
            {
                continue;
            }
            let Some(path) = &service.path else { continue };
            if path.is_empty()
                || path.len() > 128
                || path.starts_with("//")
                || !path
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"/._-%".contains(&b))
            {
                continue;
            }
            let authority = match address {
                IpAddr::V4(a) => a.to_string(),
                IpAddr::V6(a) => format!("[{a}]"),
            };
            let endpoint = format!(
                "{scheme}://{authority}:{}/{}",
                service.port,
                path.trim_start_matches('/')
            );
            // Multiple printer instances at one host must not choose a destination
            // arbitrarily: suppress an ambiguous service kind entirely.
            match out.entry(kind) {
                std::collections::btree_map::Entry::Occupied(mut entry) => {
                    entry.insert(String::new());
                }
                std::collections::btree_map::Entry::Vacant(entry) => {
                    entry.insert(endpoint);
                }
            }
        }
        out.retain(|_, value| !value.is_empty());
        out
    }
}

/// `Living Room TV._googlecast._tcp.local` → (`Living Room TV`, `_googlecast._tcp`).
fn instance_of(name: &str) -> Option<(String, String)> {
    let name = name.strip_suffix(".local").unwrap_or(name);
    let at = name.find("._")?;
    let (instance, kind) = (&name[..at], &name[at + 1..]);
    (!instance.is_empty() && !instance.starts_with('_'))
        .then(|| (instance.to_string(), kind.to_string()))
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
            Data::Service { target, port } => {
                if let Some((instance, kind)) = instance_of(&name) {
                    if host.services.len() < MAX_SERVICES {
                        host.services.insert(kind);
                    }
                    host.name.get_or_insert(instance);
                    if host.instances.len() < MAX_SERVICES || host.instances.contains_key(&name) {
                        let service = host.instances.entry(name).or_default();
                        service.target = target;
                        service.port = port;
                    }
                }
            }
            Data::Text(strings) => {
                if instance_of(&name).is_none() {
                    continue;
                }
                for (key, value) in strings.iter().filter_map(|s| s.split_once('=')) {
                    if matches!(key, "md" | "fn" | "ty" | "model" | "am" | "manufacturer")
                        && !value.is_empty()
                    {
                        host.facts
                            .entry(key.to_string())
                            .or_insert_with(|| value.to_string());
                    }
                    if (key == "rp" || key == "CPath")
                        && (host.instances.len() < MAX_SERVICES
                            || host.instances.contains_key(&name))
                    {
                        host.instances.entry(name.clone()).or_default().path =
                            Some(value.to_string());
                    }
                }
            }
            Data::Address(address)
                if is_lan(address)
                    && (host.addresses.len() < MAX_SERVICES
                        || host.addresses.contains_key(&name)) =>
            {
                let addresses = host.addresses.entry(name).or_default();
                if addresses.len() < 8 {
                    addresses.insert(address);
                }
            }
            Data::Address(_) => {}
            Data::Other => {}
        }
    }
    kinds
}

/// Browses the network `local` is on for `listen` and returns what each host announced.
pub fn browse(local: Ipv4Addr, listen: Duration) -> std::io::Result<BTreeMap<IpAddr, Host>> {
    browse_report(local, listen).map(|report| report.hosts)
}

#[derive(Default)]
pub struct BrowseReport {
    pub hosts: BTreeMap<IpAddr, Host>,
    pub multicast: bool,
    pub warning: Option<String>,
}

fn multicast_socket(local: Ipv4Addr) -> std::io::Result<UdpSocket> {
    multicast_socket_on(local, 5353)
}

fn multicast_socket_on(local: Ipv4Addr, port: u16) -> std::io::Result<UdpSocket> {
    use socket2::{Domain, Protocol, Socket, Type};
    let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    socket.set_reuse_address(true)?;
    #[cfg(unix)]
    socket.set_reuse_port(true)?;
    socket.bind(&SocketAddr::from((Ipv4Addr::UNSPECIFIED, port)).into())?;
    socket.join_multicast_v4(&Ipv4Addr::new(224, 0, 0, 251), &local)?;
    socket.set_multicast_if_v4(&local)?;
    socket.set_multicast_ttl_v4(255)?;
    socket.set_nonblocking(true)?;
    Ok(socket.into())
}

pub fn browse_report(local: Ipv4Addr, listen: Duration) -> std::io::Result<BrowseReport> {
    let socket = UdpSocket::bind((local, 0))?;
    socket.set_nonblocking(true)?;
    socket.set_multicast_ttl_v4(255)?;
    let (multicast, warning) = match multicast_socket(local) {
        Ok(socket) => (Some(socket), None),
        Err(error) => (None, Some(format!("multicast mDNS unavailable: {error}"))),
    };
    let questions = initial_questions();
    for query in &questions {
        socket.send_to(query, GROUP)?;
    }
    let mut asked: BTreeSet<String> = BTreeSet::new();
    let mut asked_records = BTreeSet::new();
    let mut hosts: BTreeMap<IpAddr, Host> = BTreeMap::new();
    let until = Instant::now() + listen;
    let mut buffer = [0u8; 9000];
    let mut packets = 0;
    let mut retry = Some(Instant::now() + Duration::from_secs(1));
    while Instant::now() < until {
        if retry.is_some_and(|at| Instant::now() >= at) {
            for query in &questions {
                socket.send_to(query, GROUP)?;
            }
            retry = None;
        }
        for receiver in std::iter::once(&socket).chain(multicast.iter()) {
            for _ in 0..32 {
                match receiver.recv_from(&mut buffer) {
                    Ok((length, from)) if is_lan(from.ip()) => {
                        packets += 1;
                        if packets > 8192 {
                            break;
                        }
                        if hosts.len() >= MAX_HOSTS && !hosts.contains_key(&from.ip()) {
                            continue;
                        }
                        for kind in learn(hosts.entry(from.ip()).or_default(), &buffer[..length]) {
                            let known = kind.ends_with(".local")
                                && kind.starts_with('_')
                                && kind.len() <= 80;
                            if known && asked.len() < MAX_KINDS && asked.insert(kind.clone()) {
                                socket.send_to(&question(&kind), GROUP)?;
                            }
                        }
                        for (name, kind) in followup_questions(&buffer[..length]) {
                            if name.len() <= 255
                                && asked_records.len() < 128
                                && asked_records.insert((name.clone(), kind))
                            {
                                socket.send_to(&question_type(&name, kind), GROUP)?;
                            }
                        }
                    }
                    Ok(_) => {
                        packets += 1;
                    }
                    Err(e)
                        if matches!(
                            e.kind(),
                            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                        ) =>
                    {
                        break
                    }
                    Err(e) => return Err(e),
                }
            }
        }
        if packets > 8192 {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    hosts.retain(|_, host| !host.services.is_empty());
    Ok(BrowseReport {
        hosts,
        multicast: multicast.is_some(),
        warning,
    })
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
    fn initial_questions_find_service_only_printers_and_domotics() {
        let questions = initial_questions();
        for kind in [
            "_ipp._tcp.local",
            "_ipps._tcp.local",
            "_matter._tcp.local",
            "_matterc._udp.local",
            "_hap._tcp.local",
            "_mqtt._tcp.local",
            "_esphomelib._tcp.local",
            "_wled._tcp.local",
        ] {
            assert!(
                questions.contains(&question(kind)),
                "no direct query for {kind}"
            );
        }
        assert!(questions.len() < MAX_KINDS);
    }

    #[test]
    fn printer_endpoint_requires_matching_instance_and_same_host_address() {
        let instance = "Office._ipp._tcp.local";
        let mut srv = vec![0, 0, 0, 0, 2, 119];
        srv.extend(name("printer.local"));
        let packet = response(&[
            (name(instance), SRV, srv),
            (name(instance), TXT, b"\x0crp=ipp/print".to_vec()),
            (name("printer.local"), 1, vec![10, 0, 0, 9]),
            (
                name("Other._ipp._tcp.local"),
                TXT,
                b"\x0crp=attacker/x".to_vec(),
            ),
        ]);
        let mut host = Host::default();
        learn(&mut host, &packet);
        assert_eq!(host.name.as_deref(), Some("Office"));
        assert_eq!(
            host.endpoints("10.0.0.9".parse().unwrap())["_ipp._tcp"],
            "ipp://10.0.0.9:631/ipp/print"
        );
        assert!(host.endpoints("10.0.0.10".parse().unwrap()).is_empty());
    }

    #[test]
    fn unrelated_txt_does_not_classify_or_name_an_announcer() {
        let packet = response(&[(name("unrelated.local"), TXT, b"\x0dfn=Not a bulb".to_vec())]);
        let mut host = Host::default();
        learn(&mut host, &packet);
        assert!(host.facts.is_empty());
        assert!(host.services.is_empty());
    }

    #[test]
    fn spotify_cpath_comes_from_the_same_service_and_stays_on_the_observed_host() {
        let instance = "Player._spotify-connect._tcp.local";
        let mut srv = vec![0, 0, 0, 0, 0x1f, 0x90];
        srv.extend(name("player.local"));
        let packet = response(&[
            (name(instance), SRV, srv),
            (name(instance), TXT, b"\x09CPath=/zc".to_vec()),
            (name("player.local"), 1, vec![10, 0, 0, 9]),
        ]);
        let mut host = Host::default();
        learn(&mut host, &packet);
        assert_eq!(
            host.endpoints("10.0.0.9".parse().unwrap())["_spotify-connect._tcp"],
            "http://10.0.0.9:8080/zc"
        );
    }

    #[test]
    fn ptr_only_replies_trigger_srv_txt_and_service_targets_trigger_address_queries() {
        let instance = "Player._spotify-connect._tcp.local";
        let ptr = response(&[(name("_spotify-connect._tcp.local"), PTR, name(instance))]);
        let queries = followup_questions(&ptr);
        assert!(queries.contains(&(instance.to_string(), SRV)));
        assert!(queries.contains(&(instance.to_string(), TXT)));
        let mut srv = vec![0, 0, 0, 0, 0x1f, 0x90];
        srv.extend(name("player.local"));
        let packet = response(&[(name(instance), SRV, srv)]);
        assert_eq!(
            followup_questions(&packet),
            [("player.local".into(), 1), ("player.local".into(), 28)]
        );
    }

    #[test]
    fn multicast_listener_actually_receives_group_only_datagrams() {
        let local = crate::lan_address().unwrap_or(Ipv4Addr::LOCALHOST);
        let receiver = multicast_socket_on(local, 0).unwrap();
        let port = receiver.local_addr().unwrap().port();
        let sender = socket2::Socket::new(
            socket2::Domain::IPV4,
            socket2::Type::DGRAM,
            Some(socket2::Protocol::UDP),
        )
        .unwrap();
        sender.set_multicast_if_v4(&local).unwrap();
        sender.set_multicast_loop_v4(true).unwrap();
        let message = uuid::Uuid::new_v4().to_string();
        sender
            .send_to(
                message.as_bytes(),
                &SocketAddr::from((Ipv4Addr::new(224, 0, 0, 251), port)).into(),
            )
            .unwrap();
        let until = Instant::now() + Duration::from_secs(2);
        let mut bytes = [0u8; 256];
        loop {
            if let Ok((length, _)) = receiver.recv_from(&mut bytes) {
                if &bytes[..length] == message.as_bytes() {
                    break;
                }
            }
            assert!(
                Instant::now() < until,
                "group-only reply was never received"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn a_television_announcing_itself_is_named_and_its_model_kept() {
        let instance = "Living Room TV._googlecast._tcp.local";
        let mut srv = vec![0, 0, 0, 0, 0x1f, 0x49];
        srv.extend(name("tv-1234.local"));
        let packet = response(&[
            (name("_googlecast._tcp.local"), PTR, name(instance)),
            (name(instance), SRV, srv),
            (
                name(instance),
                TXT,
                b"\x10md=Chromecast HD\x09fn=Living\x05rs=on".to_vec(),
            ),
        ]);
        let mut host = Host::default();
        assert!(learn(&mut host, &packet).is_empty());
        assert_eq!(host.name.as_deref(), Some("Living Room TV"));
        assert!(host.services.contains("_googlecast._tcp"));
        assert_eq!(host.facts["md"], "Chromecast HD");
        assert!(
            !host.facts.contains_key("rs"),
            "only what describes the device is kept"
        );
    }

    #[test]
    fn the_kinds_a_host_lists_are_what_the_browse_asks_about_next() {
        let packet = response(&[
            (name(META), PTR, name("_ipp._tcp.local")),
            (name(META), PTR, name("_airplay._tcp.local")),
        ]);
        let mut host = Host::default();
        assert_eq!(
            learn(&mut host, &packet),
            ["_ipp._tcp.local", "_airplay._tcp.local"]
        );
        assert!(
            host.services.is_empty(),
            "listing a kind is not offering an instance of it"
        );
    }

    #[test]
    fn compressed_names_are_followed_and_a_packet_that_loops_is_dropped() {
        // The owner name sits at offset 12; the PTR data points back at it.
        let mut packet = vec![0, 0, 0x84, 0, 0, 0, 0, 1, 0, 0, 0, 0];
        packet.extend(name("_ipp._tcp.local"));
        packet.extend_from_slice(&[0, 12, 0, 1, 0, 0, 0, 120, 0, 9]);
        packet.extend_from_slice(b"\x06Office\xc0\x0c");
        assert_eq!(
            records(&packet),
            [(
                "_ipp._tcp.local".to_string(),
                Data::Pointer("Office._ipp._tcp.local".into())
            )]
        );

        let looping = [
            0, 0, 0x84, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0xc0, 0x0c, 0, 12, 0, 1, 0, 0, 0, 0, 0, 0,
        ];
        assert!(records(&looping).is_empty());
        assert!(
            records(&packet[..20]).is_empty(),
            "a packet cut short gives nothing it did not finish"
        );
    }
}
