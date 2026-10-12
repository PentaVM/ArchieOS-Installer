//! # archie-lan
//!
//! Sentry's sense of the local network: which devices are on the owner's LAN, what kind
//! of thing each is, and what the ones that offer it say they are doing.
//!
//! It asks the way any phone or television on the same network asks. SSDP and mDNS are
//! the two announcements home devices answer by design: a television says it is a
//! television, a printer that it prints. A device that publishes a description is read
//! ([`upnp`]), and a Roku is asked what is on ([`roku`]), because Roku documents that
//! question and answers it to anyone on the LAN.
//!
//! # What it will not do
//!
//! * **Leave the LAN.** Only private and link-local addresses are spoken to or believed.
//!   An answer from a public address is dropped, and a description is fetched only from
//!   the host that announced it. Scanning the internet tells you a device is exposed,
//!   which is a finding about somebody else's network (RFC-008).
//! * **Probe.** No port scan, no address sweep, no default passwords, no vendor cloud. A
//!   device that does not announce itself is not found, and that is the intended limit.
//! * **Act.** Everything here observes. Changing a device's state is
//!   `DeviceAdapterOperation::ApplyDesiredState`, which needs an operator's grant and is
//!   not in this crate.
//! * **Name people.** A phone on the network is a device of class `mobile`. Whose it is, is
//!   not asked.

#![forbid(unsafe_code)]

pub mod audit;
pub mod capture;
pub mod classify;
pub mod http;
pub mod inventory;
pub mod mdns;
pub mod roku;
pub mod ssdp;
pub mod upnp;

pub use inventory::{Device, Inventory};

use std::net::{IpAddr, Ipv4Addr};

/// Whether an address is on a network the owner runs: private or link-local. Loopback is
/// this machine, and is not a device on the LAN.
pub fn is_lan(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(v4) => is_lan_v4(v4),
        IpAddr::V6(v6) => {
            (v6.segments()[0] & 0xffc0) == 0xfe80 || (v6.segments()[0] & 0xfe00) == 0xfc00
        }
    }
}

pub fn is_lan_v4(address: Ipv4Addr) -> bool {
    address.is_private() || address.is_link_local()
}

/// This machine's address on the network its traffic leaves by, when that is a LAN.
///
/// A multicast question goes out of whichever interface the system picks, and on a machine
/// with virtual adapters (a hypervisor's, a container bridge) that is often one of those:
/// the first scan on such a laptop asked an empty virtual network and found nothing. The
/// sockets are bound to this address so the question goes where the devices are. Connecting
/// a UDP socket sends nothing; it only makes the system choose the route.
pub fn lan_address() -> std::io::Result<Ipv4Addr> {
    let socket = std::net::UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))?;
    socket.connect((Ipv4Addr::new(192, 0, 2, 1), 9))?;
    match socket.local_addr()?.ip() {
        IpAddr::V4(address) if is_lan_v4(address) => Ok(address),
        other => Err(std::io::Error::other(format!(
            "this machine's route out is {other}, which is not a local network"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_owners_own_network_counts_as_the_lan() {
        for inside in [
            "192.168.1.20",
            "10.0.0.7",
            "172.16.4.1",
            "169.254.10.2",
            "fe80::1",
            "fd12:3456::9",
        ] {
            assert!(is_lan(inside.parse().unwrap()), "{inside}");
        }
        for outside in [
            "8.8.8.8",
            "172.32.0.1",
            "127.0.0.1",
            "100.64.0.1",
            "2001:4860::8888",
            "::1",
        ] {
            assert!(!is_lan(outside.parse().unwrap()), "{outside}");
        }
    }
}
