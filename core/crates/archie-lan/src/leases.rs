//! What the router's DHCP server knows: its lease table, in dnsmasq's format.
//!
//! On a switched network Sentry hears a device only when it broadcasts; the router hands
//! every device its address, so its lease table names them all. dnsmasq writes one lease
//! a line (`<expiry> <mac> <ip> <hostname|*> <client-id|*>`) in OpenWrt
//! (`/tmp/dhcp.leases`), Pi-hole, DD-WRT, Tomato, Asuswrt-Merlin and most Linux routers.
//! The owner copies the file, or exports it, and hands it to `archie-netwatch leases`;
//! nothing here logs in to the router. IPv6 leases (after a `duid` line, keyed by IAID
//! instead of a hardware address) are skipped.

use std::net::Ipv4Addr;

/// Lines read from one file, at most: a home router holds a few hundred leases.
pub const MAX_LEASES: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lease {
    /// When the lease ends, in seconds since the epoch; `None` for one that does not.
    pub expires_s: Option<u64>,
    pub mac: String,
    pub address: Ipv4Addr,
    pub hostname: Option<String>,
}

/// The IPv4 leases in a dnsmasq lease file; lines of another shape are skipped.
pub fn parse(text: &str) -> Vec<Lease> {
    text.lines()
        .take(MAX_LEASES)
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let expires: u64 = fields.next()?.parse().ok()?;
            let mac = crate::capture::mac(fields.next()?)?;
            let address: Ipv4Addr = fields.next()?.parse().ok()?;
            let hostname = fields
                .next()
                .filter(|name| *name != "*")
                .filter(|name| {
                    name.len() <= 63
                        && name
                            .chars()
                            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
                })
                .map(str::to_string);
            Some(Lease {
                expires_s: (expires != 0).then_some(expires),
                mac,
                address,
                hostname,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ipv4_leases_are_read_and_the_rest_skipped() {
        let text = "\
1791750000 54:4e:f0:07:3d:e8 192.168.1.84 Living-Room-TV 01:54:4e:f0:07:3d:e8
0 00:50:56:12:34:56 192.168.1.50 * *
1791750000 FF:FF:FF:FF:FF:FF 192.168.1.9 broadcast *
1791750000 00:11:22:33:44:55 192.168.1.300 bad-address *
1791750000 00:11:22:33:44:66 192.168.1.61 <script> *
duid 00:01:00:01:2a:bc:de:f0:00:11:22:33:44:55
1791750000 1234567 fd00::23 laptop 00:01:00:01
garbage
";
        let leases = parse(text);
        assert_eq!(leases.len(), 3, "{leases:?}");
        assert_eq!(
            leases[0],
            Lease {
                expires_s: Some(1_791_750_000),
                mac: "54:4e:f0:07:3d:e8".into(),
                address: "192.168.1.84".parse().unwrap(),
                hostname: Some("Living-Room-TV".into()),
            }
        );
        assert_eq!((leases[1].expires_s, &leases[1].hostname), (None, &None));
        assert_eq!(
            leases[2].hostname, None,
            "a name that is not a host name is dropped"
        );
    }
}
