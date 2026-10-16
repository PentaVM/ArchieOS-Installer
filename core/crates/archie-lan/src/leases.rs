//! What the router's DHCP server knows: its lease table.
//!
//! On a switched network Sentry hears a device only when it broadcasts; the router hands
//! every device its address, so its lease table names them all. Three formats are read,
//! told apart by their shape:
//!
//! - dnsmasq's file, one lease a line (`<expiry> <mac> <ip> <hostname|*> <client-id|*>`):
//!   OpenWrt (`/tmp/dhcp.leases`), Pi-hole, DD-WRT, Tomato, Asuswrt-Merlin;
//! - ISC dhcpd's `dhcpd.leases`, `lease <ip> { … }` blocks: pfSense, OPNsense (ISC
//!   backend), Linux servers;
//! - OpenWrt's `ubus call luci-rpc getDHCPLeases` output, JSON, whose `expires` counts
//!   seconds from when it was taken.
//!
//! The owner copies the file, or exports it, and hands it to `archie-netwatch leases`;
//! nothing here logs in to the router. IPv6 leases are skipped.

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

/// The IPv4 leases in a lease table of any of the three formats, read at `now_s` (seconds
/// since the epoch, for OpenWrt's relative expiry); entries of another shape are skipped.
pub fn parse(text: &str, now_s: u64) -> Vec<Lease> {
    let start = text.trim_start();
    if start.starts_with('{') {
        luci(text, now_s)
    } else if text
        .lines()
        .any(|line| line.trim_start().starts_with("lease "))
    {
        isc(text)
    } else {
        dnsmasq(text)
    }
}

/// A host name as a lease gives it, kept only if it is one.
fn host_name(name: &str) -> Option<String> {
    let sane = !name.is_empty()
        && name != "*"
        && name.len() <= 63
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    sane.then(|| name.to_string())
}

fn dnsmasq(text: &str) -> Vec<Lease> {
    text.lines()
        .take(MAX_LEASES)
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let expires: u64 = fields.next()?.parse().ok()?;
            let mac = crate::capture::mac(fields.next()?)?;
            let address: Ipv4Addr = fields.next()?.parse().ok()?;
            let hostname = fields.next().and_then(host_name);
            Some(Lease {
                expires_s: (expires != 0).then_some(expires),
                mac,
                address,
                hostname,
            })
        })
        .collect()
}

/// Days from 1970-01-01 to a proleptic Gregorian date (Howard Hinnant's algorithm).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let day_of_year = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// dhcpd's `ends <weekday> <YYYY/MM/DD> <HH:MM:SS>` (UTC) in seconds since the epoch.
fn isc_time(fields: &[&str]) -> Option<u64> {
    let date: Vec<i64> = fields
        .get(1)?
        .split('/')
        .map(str::parse)
        .collect::<Result<_, _>>()
        .ok()?;
    let time: Vec<i64> = fields
        .get(2)?
        .split(':')
        .map(str::parse)
        .collect::<Result<_, _>>()
        .ok()?;
    let ([year, month, day], [hour, minute, second]) = (date.as_slice(), time.as_slice()) else {
        return None;
    };
    let valid = (1..=12).contains(month)
        && (1..=31).contains(day)
        && *hour < 24
        && *minute < 60
        && *second < 61;
    let seconds =
        days_from_civil(*year, *month, *day) * 86_400 + hour * 3_600 + minute * 60 + second;
    (valid && seconds > 0).then_some(seconds as u64)
}

/// One `lease … { }` block as it is read.
struct Block {
    address: Ipv4Addr,
    mac: Option<String>,
    hostname: Option<String>,
    expires_s: Option<u64>,
    active: bool,
}

fn isc(text: &str) -> Vec<Lease> {
    let mut leases: Vec<Lease> = Vec::new();
    let mut open: Option<Block> = None;
    for line in text.lines() {
        let line = line.trim().trim_end_matches(';');
        let fields: Vec<&str> = line.split_whitespace().collect();
        match fields.as_slice() {
            ["lease", address, "{"] => {
                open = address.parse().ok().map(|address| Block {
                    address,
                    mac: None,
                    hostname: None,
                    expires_s: None,
                    active: true,
                });
            }
            ["}"] => {
                if let Some(Block {
                    address,
                    mac: Some(mac),
                    hostname,
                    expires_s,
                    active: true,
                }) = open.take()
                {
                    // dhcpd appends; the last block for an address is the one that holds.
                    leases.retain(|lease| lease.address != address);
                    leases.push(Lease {
                        expires_s,
                        mac,
                        address,
                        hostname,
                    });
                }
            }
            ["hardware", "ethernet", mac] => {
                if let Some(lease) = open.as_mut() {
                    lease.mac = crate::capture::mac(mac);
                }
            }
            ["client-hostname", name] => {
                if let Some(lease) = open.as_mut() {
                    lease.hostname = host_name(name.trim_matches('"'));
                }
            }
            ["ends", "never"] => {}
            ["ends", rest @ ..] => {
                if let Some(lease) = open.as_mut() {
                    lease.expires_s = isc_time(rest);
                }
            }
            ["binding", "state", state] => {
                if let Some(lease) = open.as_mut() {
                    lease.active = *state == "active";
                }
            }
            _ => {}
        }
        if leases.len() >= MAX_LEASES {
            break;
        }
    }
    leases
}

fn luci(text: &str, now_s: u64) -> Vec<Lease> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else {
        return Vec::new();
    };
    let Some(entries) = value.get("dhcp_leases").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    entries
        .iter()
        .take(MAX_LEASES)
        .filter_map(|entry| {
            Some(Lease {
                expires_s: entry
                    .get("expires")
                    .and_then(serde_json::Value::as_u64)
                    .map(|left| now_s.saturating_add(left)),
                mac: crate::capture::mac(entry.get("macaddr")?.as_str()?)?,
                address: entry.get("ipaddr")?.as_str()?.parse().ok()?,
                hostname: entry
                    .get("hostname")
                    .and_then(|h| h.as_str())
                    .and_then(host_name),
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
        let leases = parse(text, 0);
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

    #[test]
    fn isc_dhcpd_blocks_are_read_the_last_one_holding() {
        let text = r#"
# The format of this file is documented in the dhcpd.leases(5) manual page.
lease 192.168.1.10 {
  starts 4 2026/10/15 10:00:00;
  ends 4 2026/10/15 12:00:00;
  binding state active;
  hardware ethernet 00:11:22:33:44:55;
  client-hostname "nas";
}
lease 192.168.1.10 {
  starts 5 2026/10/16 10:00:00;
  ends 5 2026/10/16 12:00:00;
  binding state active;
  hardware ethernet 00:11:22:33:44:55;
  client-hostname "nas";
}
lease 192.168.1.11 {
  ends never;
  binding state free;
  hardware ethernet 00:11:22:33:44:66;
}
lease 192.168.1.12 {
  ends never;
  hardware ethernet 00:11:22:33:44:77;
}
"#;
        let leases = parse(text, 0);
        assert_eq!(leases.len(), 2, "{leases:?}");
        // 2026-10-16 12:00:00 UTC.
        assert_eq!(leases[0].expires_s, Some(1_792_152_000));
        assert_eq!(leases[0].hostname.as_deref(), Some("nas"));
        assert_eq!(
            (leases[1].mac.as_str(), leases[1].expires_s),
            ("00:11:22:33:44:77", None)
        );
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(2000, 3, 1), 11_017);
    }

    #[test]
    fn openwrt_s_json_counts_expiry_from_when_it_was_taken() {
        let text = r#"{"dhcp_leases":[
            {"expires":3600,"hostname":"phone","macaddr":"DA:A1:19:00:00:07","ipaddr":"192.168.1.30"},
            {"expires":false,"macaddr":"00:50:56:12:34:56","ipaddr":"192.168.1.50"},
            {"expires":10,"macaddr":"nonsense","ipaddr":"192.168.1.31"}
        ],"dhcp6_leases":[{"expires":3600,"duid":"0001","ip6addr":"fd00::30"}]}"#;
        let leases = parse(text, 1_000_000);
        assert_eq!(leases.len(), 2, "{leases:?}");
        assert_eq!(leases[0].expires_s, Some(1_003_600));
        assert_eq!(leases[0].mac, "da:a1:19:00:00:07");
        assert_eq!((leases[1].expires_s, &leases[1].hostname), (None, &None));
        assert!(parse("{not json", 0).is_empty());
    }
}
