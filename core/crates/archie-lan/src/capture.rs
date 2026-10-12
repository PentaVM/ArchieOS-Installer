//! What is taken from each frame on the wire, and how TShark is asked for it.
//!
//! The capture itself is TShark's: it has the dissectors, and `dumpcap` beneath it is the
//! one small program that holds the privilege to open the interface. This module names
//! the fields wanted from every frame and reads the line TShark prints for each. They are
//! addresses, ports, flags and three kinds of name (a DNS question, a DHCP host name, a
//! TLS server name). No payload is asked for, so none arrives.

use std::net::{IpAddr, Ipv4Addr};

/// The fields, in the order they are printed, one frame to a line, tab between.
pub const FIELDS: [&str; 20] = [
    "frame.time_epoch",
    "frame.len",
    "eth.src",
    "eth.dst",
    "ip.src",
    "ip.dst",
    "ipv6.src",
    "ipv6.dst",
    "arp.src.hw_mac",
    "arp.src.proto_ipv4",
    "tcp.srcport",
    "tcp.dstport",
    "tcp.flags.syn",
    "tcp.flags.ack",
    "udp.srcport",
    "udp.dstport",
    "dns.flags.response",
    "dns.qry.name",
    "dhcp.option.hostname",
    "tls.handshake.extensions_server_name",
];

/// Enough of each frame for the names above; the rest is never copied off the interface.
pub const SNAPLEN: &str = "768";

/// The arguments that make TShark print [`FIELDS`], after the ones that say where from.
pub fn field_arguments() -> Vec<String> {
    let mut arguments: Vec<String> = [
        "-n",
        "-l",
        "-Q",
        "-T",
        "fields",
        "-E",
        "separator=/t",
        "-E",
        "occurrence=f",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    for field in FIELDS {
        arguments.push("-e".into());
        arguments.push(field.into());
    }
    arguments
}

/// One frame, as far as the audit cares.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Packet {
    pub at_ms: u64,
    pub length: u64,
    pub source_mac: Option<String>,
    pub source: Option<IpAddr>,
    pub destination: Option<IpAddr>,
    /// The sender an ARP frame names: its hardware address and the IPv4 address it claims.
    pub arp: Option<(String, Ipv4Addr)>,
    pub tcp: Option<(u16, u16)>,
    /// A TCP segment that opens a connection: SYN without ACK.
    pub opening: bool,
    pub udp: Option<(u16, u16)>,
    /// The name asked for, when the frame is a DNS question and not an answer.
    pub dns_question: Option<String>,
    pub dhcp_hostname: Option<String>,
    pub server_name: Option<String>,
}

fn flag(text: &str) -> bool {
    matches!(text, "1" | "True" | "true")
}

/// A hardware address of one device: not broadcast, not a multicast group.
fn mac(text: &str) -> Option<String> {
    let text = text.to_ascii_lowercase();
    let bytes: Vec<u8> = text
        .split(':')
        .filter_map(|part| u8::from_str_radix(part, 16).ok())
        .collect();
    (bytes.len() == 6 && text.len() == 17 && bytes[0] & 1 == 0 && bytes != [0; 6]).then_some(text)
}

/// A name as a device wrote it: lower case, of a sane length, with nothing unprintable.
fn name(text: &str) -> Option<String> {
    let text = text.trim_end_matches('.');
    let sane = !text.is_empty() && text.len() <= 253 && text.chars().all(|c| c.is_ascii_graphic());
    sane.then(|| text.to_ascii_lowercase())
}

fn ports(source: &str, destination: &str) -> Option<(u16, u16)> {
    Some((source.parse().ok()?, destination.parse().ok()?))
}

/// Reads one line TShark printed. A line of another shape is not a frame.
pub fn parse(line: &str) -> Option<Packet> {
    let fields: Vec<&str> = line.trim_end_matches(['\r', '\n']).split('\t').collect();
    if fields.len() != FIELDS.len() {
        return None;
    }
    let seconds: f64 = fields[0].parse().ok()?;
    if !seconds.is_finite() || seconds < 0.0 {
        return None;
    }
    let address = |v4: &str, v6: &str| v4.parse::<IpAddr>().or_else(|_| v6.parse::<IpAddr>()).ok();
    let tcp = ports(fields[10], fields[11]);
    Some(Packet {
        at_ms: (seconds * 1000.0) as u64,
        length: fields[1].parse().unwrap_or(0),
        source_mac: mac(fields[2]),
        source: address(fields[4], fields[6]),
        destination: address(fields[5], fields[7]),
        arp: mac(fields[8]).zip(
            fields[9]
                .parse::<Ipv4Addr>()
                .ok()
                .filter(|ip| !ip.is_unspecified()),
        ),
        tcp,
        opening: tcp.is_some() && flag(fields[12]) && !flag(fields[13]),
        udp: ports(fields[14], fields[15]),
        dns_question: if flag(fields[16]) {
            None
        } else {
            name(fields[17])
        },
        dhcp_hostname: name(fields[18]),
        server_name: name(fields[19]),
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A line as TShark prints it, from the fields that are not empty.
    pub(crate) fn line(set: &[(&str, &str)]) -> String {
        FIELDS
            .iter()
            .map(|field| {
                set.iter()
                    .find(|(name, _)| name == field)
                    .map_or("", |(_, value)| value)
            })
            .collect::<Vec<_>>()
            .join("\t")
    }

    #[test]
    fn a_dns_question_is_read_and_its_answer_is_not_a_question() {
        let asked = [
            ("frame.time_epoch", "1791749339.250000000"),
            ("frame.len", "86"),
            ("eth.src", "54:4E:F0:07:3D:E8"),
            ("ip.src", "192.168.1.84"),
            ("ip.dst", "192.168.1.1"),
            ("udp.srcport", "40000"),
            ("udp.dstport", "53"),
            ("dns.flags.response", "False"),
            ("dns.qry.name", "Api.Roku.com."),
        ];
        let packet = parse(&line(&asked)).unwrap();
        assert_eq!(packet.at_ms, 1_791_749_339_250);
        assert_eq!(packet.source_mac.as_deref(), Some("54:4e:f0:07:3d:e8"));
        assert_eq!(packet.dns_question.as_deref(), Some("api.roku.com"));
        assert_eq!(packet.udp, Some((40000, 53)));

        let mut answered = asked.to_vec();
        answered[7] = ("dns.flags.response", "True");
        assert_eq!(parse(&line(&answered)).unwrap().dns_question, None);
    }

    #[test]
    fn only_a_syn_without_ack_opens_a_connection() {
        let segment = |syn: &'static str, ack: &'static str| {
            parse(&line(&[
                ("frame.time_epoch", "10.5"),
                ("tcp.srcport", "5000"),
                ("tcp.dstport", "23"),
                ("tcp.flags.syn", syn),
                ("tcp.flags.ack", ack),
            ]))
            .unwrap()
            .opening
        };
        assert!(segment("1", "0") && segment("True", "False"));
        assert!(!segment("1", "1") && !segment("0", "1"));
    }

    #[test]
    fn broadcast_and_group_addresses_are_nobody_s_and_junk_is_not_a_frame() {
        let from = |mac: &str| {
            parse(&line(&[("frame.time_epoch", "1"), ("eth.src", mac)]))
                .unwrap()
                .source_mac
        };
        assert_eq!(from("ff:ff:ff:ff:ff:ff"), None);
        assert_eq!(from("01:00:5e:00:00:fb"), None);
        assert_eq!(from("00:00:00:00:00:00"), None);
        assert!(from("e4:ab:89:6e:1a:18").is_some());
        assert_eq!(parse("Capturing on 'eth0'"), None);
        assert_eq!(parse(&line(&[("frame.time_epoch", "soon")])), None);
        // A name with a control character in it is not kept.
        assert_eq!(
            parse(&line(&[
                ("frame.time_epoch", "1"),
                ("dhcp.option.hostname", "tv\u{7}")
            ]))
            .unwrap()
            .dhcp_hostname,
            None
        );
    }

    #[test]
    fn an_arp_frame_names_who_claims_an_address() {
        let packet = parse(&line(&[
            ("frame.time_epoch", "2"),
            ("arp.src.hw_mac", "e4:ab:89:6e:1a:18"),
            ("arp.src.proto_ipv4", "192.168.1.1"),
        ]))
        .unwrap();
        assert_eq!(
            packet.arp,
            Some(("e4:ab:89:6e:1a:18".into(), "192.168.1.1".parse().unwrap()))
        );
        let probe = parse(&line(&[
            ("frame.time_epoch", "2"),
            ("arp.src.hw_mac", "e4:ab:89:6e:1a:18"),
            ("arp.src.proto_ipv4", "0.0.0.0"),
        ]))
        .unwrap();
        assert_eq!(probe.arp, None);
    }
}
