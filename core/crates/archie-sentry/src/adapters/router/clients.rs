//! Read-only DHCP client list, the feed for the unified device view.
//! Pure parsers over leases-file and `ip neigh` output, plus one executor
//! call that picks the source the capability probe found. Malformed lines
//! are skipped, never fatal; everything is bounded.

use crate::engine::AdapterError;
use std::net::Ipv4Addr;

use super::{
    probe::RouterCapabilities,
    ssh::{lease_path_of, SshChannel},
};

/// Never report more clients than this from one read.
pub const MAX_CLIENTS: usize = 1024;
const MAX_HOSTNAME: usize = 64;

/// One LAN client as the router's own tables reported it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouterClient {
    /// Lowercase `aa:bb:cc:dd:ee:ff`.
    pub mac: String,
    pub ip: Ipv4Addr,
    pub hostname: Option<String>,
}

fn valid_mac(raw: &str) -> Option<String> {
    let parts: Vec<&str> = raw.split(':').collect();
    if parts.len() != 6
        || !parts
            .iter()
            .all(|p| p.len() == 2 && p.bytes().all(|b| b.is_ascii_hexdigit()))
    {
        return None;
    }
    let mac = parts.join(":").to_ascii_lowercase();
    if mac == "00:00:00:00:00:00" {
        return None;
    }
    Some(mac)
}

fn valid_hostname(raw: &str) -> Option<String> {
    if raw.is_empty()
        || raw == "*"
        || raw.len() > MAX_HOSTNAME
        || !raw
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_'))
    {
        return None;
    }
    Some(raw.to_owned())
}

/// Parse a dnsmasq-style leases file:
/// `<expiry> <mac> <ip> <hostname> <client-id>`, one client per line.
/// Expiry may be `0` (static). Malformed lines are skipped.
pub fn parse_dnsmasq_leases(text: &str) -> Vec<RouterClient> {
    let mut clients = Vec::new();
    for line in text.lines() {
        if clients.len() >= MAX_CLIENTS {
            break;
        }
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() < 4 || fields[0].parse::<u64>().is_err() {
            continue; // malformed or truncated line
        }
        let (Some(mac), Ok(ip)) = (valid_mac(fields[1]), fields[2].parse::<Ipv4Addr>()) else {
            continue;
        };
        if !(ip.is_private() || ip.is_loopback()) {
            continue;
        }
        clients.push(RouterClient {
            mac,
            ip,
            hostname: valid_hostname(fields[3]),
        });
    }
    clients
}

/// Parse `ip neigh` output:
/// `<ip> dev <if> [lladdr <mac>] <state>`. Lines without an `lladdr` or in
/// FAILED state carry no usable identity and are skipped.
pub fn parse_ip_neigh(text: &str) -> Vec<RouterClient> {
    let mut clients = Vec::new();
    for line in text.lines() {
        if clients.len() >= MAX_CLIENTS {
            break;
        }
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() < 4 {
            continue;
        }
        let Ok(ip) = fields[0].parse::<Ipv4Addr>() else {
            continue;
        };
        if !(ip.is_private() || ip.is_loopback()) {
            continue;
        }
        let Some(mac) = fields
            .windows(2)
            .find(|pair| pair[0] == "lladdr")
            .and_then(|pair| valid_mac(pair[1]))
        else {
            continue;
        };
        if fields.last().is_some_and(|state| *state == "FAILED") {
            continue;
        }
        clients.push(RouterClient {
            mac,
            ip,
            hostname: None,
        });
    }
    clients
}

/// One executor call producing the client list: the leases file the probe
/// found when there is one, `ip neigh` otherwise. Read-only; runs the
/// allowlisted command and parses, nothing else.
pub fn router_clients(
    ssh: &SshChannel,
    capabilities: &RouterCapabilities,
) -> Result<Vec<RouterClient>, AdapterError> {
    if let Some(path) = &capabilities.dhcp_leases_path {
        let spec = super::ssh::READ_COMMANDS
            .iter()
            .find(|spec| lease_path_of(spec) == Some(path.as_str()))
            .ok_or(AdapterError::Unsupported)?;
        let output = ssh.run_spec(spec)?;
        Ok(parse_dnsmasq_leases(&output))
    } else {
        let output = ssh.run_named("ip_neigh")?;
        Ok(parse_ip_neigh(&output))
    }
}
