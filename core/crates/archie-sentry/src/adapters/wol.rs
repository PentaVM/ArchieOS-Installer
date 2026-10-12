//! Wake-on-LAN adapter: wakes a powered-off or sleeping host by broadcasting a
//! magic packet, and proves nothing by itself.
//!
//! Honesty notes, deliberately part of the module contract:
//! - The magic packet is fire-and-forget UDP broadcast. An accepted `apply`
//!   means 102 bytes left this machine's socket, nothing more. The only real
//!   verification is the host answering on the network afterwards, which is
//!   exactly the postcondition the engine measures through `observe`
//!   (`WakeHost` matches when the host is reachable).
//! - Reachability is read from the local ARP/neighbor cache: `arp -a` on
//!   Windows, `ip neigh show` on Linux, both through a configured absolute
//!   tool path via [`CommandRunner`]. A valid entry proves the target IP
//!   answered ARP recently. A missing or INCOMPLETE entry proves only that
//!   this machine has not heard from the target lately - the cache expires, so
//!   a powered-on but silent host can read as unreachable. That false negative
//!   is accepted: no ping is sent to freshen the cache, because a measurement
//!   must not become an action.
//! - The target MAC comes from resource configuration, validated as lowercase
//!   hex with colons (`7c:10:c9:55:aa:01`). It is never read back from the
//!   network, so the packet can only target the configured host.

use super::process::CommandRunner;
use crate::{
    contracts::{validate_id, Command, Observation, ObservationOrigin, ResourceState},
    engine::{AdapterAck, AdapterError, EdgeAdapter},
    registry::ResourceDescriptor,
};
use archie_types::Timestamp;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    net::{IpAddr, Ipv4Addr},
    path::Path,
    sync::Arc,
    time::Duration,
};
use tokio::net::UdpSocket;

/// Neighbor-cache queries are bounded independently of Core's deadline.
const ARP_TIMEOUT: Duration = Duration::from_secs(3);
/// A UDP send is local work; the bound exists so a wedged stack cannot hang.
const SEND_TIMEOUT: Duration = Duration::from_secs(2);
/// WoL's conventional port; the packet content, not the port, does the waking.
const DEFAULT_PORT: u16 = 9;

/// Per-resource wiring as written in operator configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WolTarget {
    /// Lowercase hex with colons, e.g. `7c:10:c9:55:aa:01`.
    pub mac: String,
    /// The host's IP, used for the neighbor-cache reachability lookup.
    pub ip: String,
}

pub struct Wol {
    edge: String,
    adapter: String,
    /// `"<ipv4>"` or `"<ipv4>:<port>"` the magic packet is sent to.
    broadcast: String,
    /// Absolute path of `arp` (Windows) or `ip` (Linux).
    arp_tool: String,
    targets: BTreeMap<String, ([u8; 6], IpAddr)>,
    runner: Arc<dyn CommandRunner>,
}
impl std::fmt::Debug for Wol {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Wol")
            .field("edge", &self.edge)
            .field("adapter", &self.adapter)
            .field("broadcast", &self.broadcast)
            .field("targets", &self.targets.len())
            .finish_non_exhaustive()
    }
}

/// A MAC address is exactly six lowercase hex octets joined by colons.
pub fn parse_mac(text: &str) -> Option<[u8; 6]> {
    let parts: Vec<&str> = text.split(':').collect();
    if parts.len() != 6 {
        return None;
    }
    let mut mac = [0u8; 6];
    for (slot, part) in mac.iter_mut().zip(parts) {
        if part.len() != 2
            || !part
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return None;
        }
        *slot = u8::from_str_radix(part, 16).ok()?;
    }
    Some(mac)
}

/// 6 bytes of 0xFF followed by the target MAC repeated 16 times: 102 bytes.
pub fn magic_packet(mac: [u8; 6]) -> [u8; 102] {
    let mut packet = [0xFFu8; 102];
    for repeat in 0..16 {
        packet[6 + repeat * 6..6 + (repeat + 1) * 6].copy_from_slice(&mac);
    }
    packet
}

/// Windows `arp -a`: rows are `<ip> <aa-bb-cc-dd-ee-ff> <dynamic|static|invalid>`.
/// Only a row for the exact IP with a well-formed physical address and a type
/// other than `invalid` counts as the host answering.
pub fn parse_arp_windows(output: &str, ip: &IpAddr) -> bool {
    let wanted = ip.to_string();
    for line in output.lines() {
        let mut fields = line.split_whitespace();
        if fields.next() != Some(wanted.as_str()) {
            continue;
        }
        let Some(mac) = fields.next() else {
            return false;
        };
        let well_formed = mac.len() == 17
            && mac.bytes().enumerate().all(|(index, byte)| {
                if index % 3 == 2 {
                    byte == b'-'
                } else {
                    byte.is_ascii_hexdigit()
                }
            });
        let invalid = fields
            .next()
            .is_some_and(|kind| kind.eq_ignore_ascii_case("invalid"));
        return well_formed && !invalid;
    }
    false
}

/// Linux `ip neigh show`: `<ip> dev <if> lladdr <mac> <STATE>`. An entry counts
/// when it carries a link-layer address and its state is not INCOMPLETE or
/// FAILED; REACHABLE/STALE/DELAY/PERMANENT all mean the kernel resolved it.
pub fn parse_ip_neigh(output: &str, ip: &IpAddr) -> bool {
    let wanted = ip.to_string();
    for line in output.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.first() != Some(&wanted.as_str()) {
            continue;
        }
        let resolved = fields
            .windows(2)
            .any(|pair| pair[0] == "lladdr" && pair[1].contains(':'));
        let failed = fields
            .last()
            .is_some_and(|state| matches!(*state, "INCOMPLETE" | "FAILED"));
        return resolved && !failed;
    }
    false
}

impl Wol {
    /// `broadcast` is `"<ipv4>"` or `"<ipv4>:<port>"` (default 9) - typically a
    /// subnet broadcast such as `192.168.1.255`. `arp_tool` must be an absolute
    /// path; [`super::process::ProcessRunner`] re-checks existence before any
    /// process exists. `runner` is injected so tests never touch the network
    /// cache of the machine running them.
    pub fn new(
        edge: &str,
        adapter: &str,
        broadcast: &str,
        arp_tool: &str,
        targets: BTreeMap<String, WolTarget>,
        runner: Arc<dyn CommandRunner>,
    ) -> Result<Self, AdapterError> {
        validate_id(edge).map_err(|_| AdapterError::Protocol)?;
        validate_id(adapter).map_err(|_| AdapterError::Protocol)?;
        let (host, port) = match broadcast.rsplit_once(':') {
            Some((host, port)) if !host.contains(':') => {
                (host, port.parse::<u16>().ok().filter(|p| *p != 0))
            }
            _ => (broadcast, Some(DEFAULT_PORT)),
        };
        host.parse::<Ipv4Addr>()
            .map_err(|_| AdapterError::Protocol)?;
        let port = port.ok_or(AdapterError::Protocol)?;
        if !Path::new(arp_tool).is_absolute() {
            return Err(AdapterError::Protocol);
        }
        if targets.is_empty() || targets.len() > 256 {
            return Err(AdapterError::Protocol);
        }
        let mut resolved = BTreeMap::new();
        for (resource, target) in targets {
            validate_id(&resource).map_err(|_| AdapterError::Protocol)?;
            let mac = parse_mac(&target.mac).ok_or(AdapterError::Protocol)?;
            let ip: IpAddr = target.ip.parse().map_err(|_| AdapterError::Protocol)?;
            resolved.insert(resource, (mac, ip));
        }
        Ok(Self {
            edge: edge.into(),
            adapter: adapter.into(),
            broadcast: format!("{host}:{port}"),
            arp_tool: arp_tool.into(),
            targets: resolved,
            runner,
        })
    }
    fn target_of(&self, resource: &ResourceDescriptor) -> Result<([u8; 6], IpAddr), AdapterError> {
        if !resource.paired || resource.adapter_id != self.adapter || resource.edge_id != self.edge
        {
            return Err(AdapterError::Unsupported);
        }
        self.targets
            .get(&resource.resource_id)
            .copied()
            .ok_or(AdapterError::Unsupported)
    }
}

#[async_trait]
impl EdgeAdapter for Wol {
    fn edge_id(&self) -> &str {
        &self.edge
    }
    fn adapter_id(&self) -> &str {
        &self.adapter
    }
    async fn observe(
        &mut self,
        resource: &ResourceDescriptor,
    ) -> Result<Observation, AdapterError> {
        let (_, ip) = self.target_of(resource)?;
        let argv: Vec<String> = if cfg!(target_os = "windows") {
            vec![self.arp_tool.clone(), "-a".into()]
        } else {
            vec![self.arp_tool.clone(), "neigh".into(), "show".into()]
        };
        let output = self.runner.run(&argv, ARP_TIMEOUT)?;
        if output.status != 0 {
            return Err(AdapterError::Unavailable);
        }
        let text = String::from_utf8_lossy(&output.stdout);
        let reachable = if cfg!(target_os = "windows") {
            parse_arp_windows(&text, &ip)
        } else {
            parse_ip_neigh(&text, &ip)
        };
        // WoL can measure reachability and nothing else; the remaining Host
        // fields stay unknown and therefore verify nothing.
        let state = ResourceState::Host {
            reachable,
            awake: None,
            session_locked: None,
            volume_percent: None,
            foreground_app: None,
        };
        let report = serde_json::to_vec(&(&resource.resource_id, &state))
            .map_err(|_| AdapterError::Protocol)?;
        Ok(Observation {
            resource_id: resource.resource_id.clone(),
            adapter_id: self.adapter.clone(),
            // Identical cache reads keep one evidence identity; no freshness
            // is invented for a state that did not change.
            evidence_id: format!("wol-{:x}", Sha256::digest(report)),
            observed_at: Timestamp::now(),
            origin: ObservationOrigin::Measurement,
            state,
        })
    }
    async fn apply(
        &mut self,
        resource: &ResourceDescriptor,
        command: &Command,
    ) -> Result<AdapterAck, AdapterError> {
        command.validate().map_err(|_| AdapterError::Protocol)?;
        if !resource.capabilities.contains(&command.capability()) {
            return Err(AdapterError::Unsupported);
        }
        let (mac, _) = self.target_of(resource)?;
        match command {
            Command::WakeHost => {
                let packet = magic_packet(mac);
                let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))
                    .await
                    .map_err(|_| AdapterError::Transport)?;
                socket
                    .set_broadcast(true)
                    .map_err(|_| AdapterError::Transport)?;
                tokio::time::timeout(SEND_TIMEOUT, socket.send_to(&packet, &self.broadcast))
                    .await
                    .map_err(|_| AdapterError::Unavailable)?
                    .map_err(|_| AdapterError::Transport)?;
                // Fire-and-forget: accepted means the packet left the socket.
                // The engine's after-observe is the only wake-up evidence.
                Ok(AdapterAck { accepted: true })
            }
            _ => Err(AdapterError::Unsupported),
        }
    }
}
