//! Opt-in TCP presence, with explicit local scope and no application payload.
use crate::is_lan_v4;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::str::FromStr;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy)]
pub struct Subnet {
    network: u32,
    prefix: u8,
}

impl FromStr for Subnet {
    type Err = String;
    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        let (address, prefix) = raw.split_once('/').ok_or("subnet requires IPv4/CIDR")?;
        let address: Ipv4Addr = address.parse().map_err(|_| "invalid IPv4 subnet")?;
        let prefix: u8 = prefix.parse().map_err(|_| "invalid subnet prefix")?;
        if !(24..=32).contains(&prefix) {
            return Err("subnet must contain at most 256 addresses (/24..=/32)".into());
        }
        let mask = u32::MAX << (32 - prefix);
        let network = u32::from(address) & mask;
        let last = network | !mask;
        if !is_lan_v4(Ipv4Addr::from(network)) || !is_lan_v4(Ipv4Addr::from(last)) {
            return Err("only private or link-local subnets may be probed".into());
        }
        Ok(Self { network, prefix })
    }
}

impl Subnet {
    pub fn hosts(&self) -> Vec<Ipv4Addr> {
        let count = 1u32 << (32 - self.prefix);
        let range = if self.prefix < 31 {
            1..count - 1
        } else {
            0..count
        };
        range
            .map(|offset| Ipv4Addr::from(self.network + offset))
            .collect()
    }
    pub fn contains(&self, address: Ipv4Addr) -> bool {
        u32::from(address) & (u32::MAX << (32 - self.prefix)) == self.network
    }
    pub fn cidr(&self) -> String {
        format!("{}/{}", Ipv4Addr::from(self.network), self.prefix)
    }
    pub fn for_interface(local: Ipv4Addr) -> Result<Self, String> {
        for interface in if_addrs::get_if_addrs().map_err(|e| e.to_string())? {
            if let if_addrs::IfAddr::V4(address) = interface.addr {
                if address.ip == local {
                    let mask = u32::from(address.netmask);
                    if mask.leading_ones() + mask.trailing_zeros() != 32 {
                        return Err("interface has a noncontiguous subnet mask".into());
                    }
                    return format!("{local}/{}", mask.leading_ones()).parse();
                }
            }
        }
        Err("selected address is not a local IPv4 interface".into())
    }
}

pub const PORTS: &[u16] = &[
    80, 443, 631, 9100, 1883, 8883, 554, 8009, 8060, 22, 445, 3389, 8080, 8123, 5357,
];

/// A successful TCP connection is an open-port claim, not device classification.
pub fn probe_ports(address: Ipv4Addr, ports: &[u16], budget: Duration) -> Vec<u16> {
    let until = Instant::now() + budget;
    let mut open = Vec::new();
    for port in ports {
        let left = until.saturating_duration_since(Instant::now());
        if left.is_zero() {
            break;
        }
        if TcpStream::connect_timeout(
            &SocketAddr::from((address, *port)),
            left.min(Duration::from_millis(250)),
        )
        .is_ok()
        {
            open.push(*port);
        }
    }
    open
}

#[derive(Debug, Serialize)]
pub struct Report {
    pub subnet: String,
    pub hosts_total: usize,
    pub hosts_attempted: usize,
    pub probes_total: usize,
    pub probes_attempted: usize,
    pub completed: bool,
    pub responders: Vec<(Ipv4Addr, Vec<u16>)>,
}

fn work_item(index: usize, hosts: &[Ipv4Addr], ports: &[u16]) -> Option<(Ipv4Addr, u16)> {
    if hosts.is_empty() {
        return None;
    }
    Some((hosts[index % hosts.len()], *ports.get(index / hosts.len())?))
}

pub fn scan(subnet: Subnet, local: Ipv4Addr, budget: Duration) -> Report {
    let hosts = Arc::new(
        subnet
            .hosts()
            .into_iter()
            .filter(|a| *a != local)
            .collect::<Vec<_>>(),
    );
    let next = Arc::new(AtomicUsize::new(0));
    let complete = Arc::new(AtomicUsize::new(0));
    let out = Arc::new(Mutex::new(BTreeMap::<Ipv4Addr, BTreeSet<u16>>::new()));
    let until = Instant::now() + budget;
    let mut workers = Vec::new();
    for _ in 0..16.min(hosts.len()) {
        let (hosts, next, complete, out) =
            (hosts.clone(), next.clone(), complete.clone(), out.clone());
        workers.push(std::thread::spawn(move || {
            while Instant::now() < until {
                let index = next.fetch_add(1, Ordering::Relaxed);
                let Some((address, port)) = work_item(index, &hosts, PORTS) else {
                    break;
                };
                let open = probe_ports(
                    address,
                    &[port],
                    until.saturating_duration_since(Instant::now()),
                );
                if Instant::now() < until {
                    complete.fetch_add(1, Ordering::Relaxed);
                }
                if !open.is_empty() {
                    out.lock().unwrap().entry(address).or_default().extend(open);
                }
            }
        }));
    }
    for worker in workers {
        let _ = worker.join();
    }
    let responders = std::mem::take(&mut *out.lock().unwrap())
        .into_iter()
        .map(|(address, ports)| (address, ports.into_iter().collect()))
        .collect();
    let probes_total = hosts.len() * PORTS.len();
    let probes_attempted = next.load(Ordering::Relaxed).min(probes_total);
    Report {
        subnet: subnet.cidr(),
        hosts_total: hosts.len(),
        hosts_attempted: probes_attempted.min(hosts.len()),
        probes_total,
        probes_attempted,
        completed: complete.load(Ordering::Relaxed) == probes_total,
        responders,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ports_are_scheduled_across_the_entire_subnet_before_deeper_checks() {
        let hosts = [
            Ipv4Addr::new(10, 0, 0, 1),
            Ipv4Addr::new(10, 0, 0, 2),
            Ipv4Addr::new(10, 0, 0, 3),
        ];
        assert_eq!(
            work_item(1, &hosts, &[80, 631]),
            Some((Ipv4Addr::new(10, 0, 0, 2), 80))
        );
        assert_eq!(
            work_item(3, &hosts, &[80, 631]),
            Some((Ipv4Addr::new(10, 0, 0, 1), 631))
        );
        assert_eq!(work_item(6, &hosts, &[80, 631]), None);
        assert_eq!(work_item(0, &[], &[80]), None);
    }
}
