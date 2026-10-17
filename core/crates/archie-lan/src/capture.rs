//! What is taken from each frame on the wire, and how TShark is asked for it.
//!
//! The capture itself is TShark's: it has the dissectors, and `dumpcap` beneath it is the
//! one small program that holds the privilege to open the interface. This module names
//! the fields wanted from every frame and reads the line TShark prints for each. They are
//! addresses, ports, flags, three kinds of name (a DNS question, a DHCP host name, a TLS
//! server name), and what devices say about themselves in the clear: the DHCP options a
//! client asks for and its vendor class, the TTL and window of a connection it opens, its
//! TLS client fingerprint (JA4, computed by TShark), an HTTP User-Agent, an LLDP system
//! name, a NetBIOS name, and IPv6 Router Advertisements. From DNS answers, the addresses
//! and the response code; from FTP, POP3, IMAP and SMTP, only the command word a client
//! sent (`PASS`, `LOGIN`, `AUTH`), never its argument. No other payload is asked for, so
//! none arrives: a password is never read, only the fact that one was sent.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// The fields, in the order they are printed, one frame to a line, tab between.
pub const FIELDS: [&str; 46] = [
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
    "icmpv6.nd.na.target_address",
    "dhcp.option.dhcp",
    "dhcp.option.request_list_item",
    "dhcp.option.vendor_class_id",
    "dhcp.option.dhcp_server_id",
    "icmpv6.type",
    "ip.ttl",
    "ipv6.hlim",
    "tcp.window_size_value",
    "tls.handshake.ja4",
    "http.user_agent",
    "lldp.tlv.system.name",
    "nbns.name",
    "dhcp.hw.mac_addr",
    "nbns.flags.response",
    "dhcpv6.msgtype",
    "icmp.type",
    "arp.opcode",
    "arp.dst.proto_ipv4",
    "dns.a",
    "dns.aaaa",
    "dns.flags.rcode",
    "ftp.request.command",
    "pop.request.command",
    "imap.request.command",
    "smtp.req.command",
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
        // Every occurrence, joined by commas: the DHCP options a client asks for are one
        // field per option. Every other field is read from its first occurrence.
        "-E",
        "occurrence=a",
        "-E",
        "aggregator=,",
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
    /// The IPv6 counterpart: a Neighbor Advertisement, by the hardware address that sent
    /// it, and the address it answers for.
    pub neighbor: Option<(String, Ipv6Addr)>,
    pub tcp: Option<(u16, u16)>,
    /// A TCP segment that opens a connection: SYN without ACK.
    pub opening: bool,
    pub udp: Option<(u16, u16)>,
    /// The name asked for, when the frame is a DNS question and not an answer.
    pub dns_question: Option<String>,
    pub dhcp_hostname: Option<String>,
    pub server_name: Option<String>,
    /// The DHCP message type: 1 discover, 2 offer, 3 request, 5 ack, 6 nak, 8 inform.
    pub dhcp_message: Option<u8>,
    /// The options a DHCP client asks for, in its own order ("1,3,6,15,119"): its stack's
    /// fingerprint, the same from every device that runs that stack.
    pub dhcp_requested: Option<String>,
    /// The vendor class a DHCP client names itself by ("MSFT 5.0", "android-dhcp-14").
    pub dhcp_vendor: Option<String>,
    /// The server a DHCP offer or acknowledgement speaks for.
    pub dhcp_server: Option<Ipv4Addr>,
    /// An IPv6 Router Advertisement: the sender offers itself as a router on this link.
    pub router_advertisement: bool,
    /// The TTL (IPv4) or hop limit (IPv6) the frame arrived with.
    pub hops: Option<u8>,
    /// The TCP window it advertised.
    pub window: Option<u16>,
    /// The TLS client fingerprint of a ClientHello, as TShark computes JA4.
    pub ja4: Option<String>,
    pub user_agent: Option<String>,
    /// The system name an LLDP frame announces (switches, access points, phones).
    pub lldp_name: Option<String>,
    /// A NetBIOS name registered or asked for, without its type suffix.
    pub nbns_name: Option<String>,
    /// The client hardware address a DHCP message is about (`chaddr`). A client's own
    /// messages carry its Ethernet address here too; a tool exhausting the pool varies it.
    pub dhcp_client: Option<String>,
    /// The name a device answers for over LLMNR or NetBIOS name service: the names
    /// Windows asks the whole link when DNS does not know them, and which Responder
    /// answers to all.
    pub answered_name: Option<String>,
    /// A DHCPv6 advertise or reply: the sender serves addresses, and DNS servers, to IPv6
    /// clients.
    pub dhcpv6_answer: bool,
    /// An ICMP Redirect (IPv4 type 5, IPv6 type 137): the sender tells a host to send its
    /// traffic for some destination through another router.
    pub redirect: bool,
    /// The address an ARP request asks about ("who has …"), when it is not the sender's
    /// own (a gratuitous announcement).
    pub arp_asked: Option<Ipv4Addr>,
    /// A DNS answer sent from port 53: the sender serves DNS.
    pub dns_answer: bool,
    /// What that answer resolved: the name asked for and the addresses (A and AAAA) it
    /// gave, at most `MAX_RESOLVED` of them.
    pub resolved: Option<(String, Vec<IpAddr>)>,
    /// A DNS answer saying a name does not exist (NXDOMAIN): the device it went to (by
    /// the frame's Ethernet destination) and the name it had asked for.
    pub nxdomain: Option<(String, String)>,
    /// A client sending a password in the clear: the protocol whose login command it
    /// sent (FTP `PASS`, POP3 `PASS`/`APOP`/`AUTH`, IMAP `LOGIN`/`AUTHENTICATE`, SMTP
    /// `AUTH`). Only the command word is read, never what follows it.
    pub cleartext_password: Option<&'static str>,
}

/// Addresses kept from one DNS answer: a CDN answers with a handful.
pub const MAX_RESOLVED: usize = 16;

fn flag(text: &str) -> bool {
    matches!(text, "1" | "True" | "true")
}

/// A hardware address of one device: not broadcast, not a multicast group.
pub(crate) fn mac(text: &str) -> Option<String> {
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

/// Text a device chose, kept only when printable, trimmed to `limit` characters.
fn said(text: &str, limit: usize) -> Option<String> {
    let text = text.trim();
    (!text.is_empty() && text.chars().all(|c| !c.is_control()))
        .then(|| text.chars().take(limit).collect())
}

/// The DHCP option numbers a client asked for, as TShark joined them.
fn requested(text: &str) -> Option<String> {
    let numbers: Vec<u8> = text
        .split(',')
        .map(|n| n.trim().parse::<u8>())
        .collect::<Result<_, _>>()
        .ok()?;
    (!numbers.is_empty()).then(|| {
        numbers
            .iter()
            .map(u8::to_string)
            .collect::<Vec<_>>()
            .join(",")
    })
}

/// JA4 is three underscore-separated parts of letters and digits.
fn ja4(text: &str) -> Option<String> {
    let sane = text.len() <= 64
        && text.split('_').count() == 3
        && text.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    sane.then(|| text.to_ascii_lowercase())
}

fn ports(source: &str, destination: &str) -> Option<(u16, u16)> {
    Some((source.parse().ok()?, destination.parse().ok()?))
}

/// The addresses a DNS answer from port 53 gave for the name asked.
fn resolved(all: &[&str], fields: &[&str]) -> Option<(String, Vec<IpAddr>)> {
    let answer = flag(fields[16]) && ports(fields[14], fields[15]).is_some_and(|(s, _)| s == 53);
    if !answer {
        return None;
    }
    let asked = name(fields[17])?;
    let addresses: Vec<IpAddr> = all[39]
        .split(',')
        .chain(all[40].split(','))
        .filter_map(|a| a.parse().ok())
        .take(MAX_RESOLVED)
        .collect();
    (!addresses.is_empty()).then_some((asked, addresses))
}

/// The protocol whose password command a client sent, from the FTP, POP3, IMAP and SMTP
/// command fields in that order.
fn password_command(commands: &[&str]) -> Option<&'static str> {
    let is = |field: &str, words: &[&str]| words.iter().any(|w| field.eq_ignore_ascii_case(w));
    [
        ("FTP", &["PASS"][..]),
        ("POP3", &["PASS", "APOP", "AUTH"][..]),
        ("IMAP", &["LOGIN", "AUTHENTICATE"][..]),
        ("SMTP", &["AUTH"][..]),
    ]
    .into_iter()
    .zip(commands)
    .find(|((_, words), field)| is(field, words))
    .map(|((protocol, _), _)| protocol)
}

/// Reads one line TShark printed. A line of another shape is not a frame.
pub fn parse(line: &str) -> Option<Packet> {
    let all: Vec<&str> = line.trim_end_matches(['\r', '\n']).split('\t').collect();
    if all.len() != FIELDS.len() {
        return None;
    }
    // Each field's first occurrence; the DHCP option list keeps all of them.
    let fields: Vec<&str> = all
        .iter()
        .map(|field| field.split(',').next().unwrap_or(""))
        .collect();
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
        neighbor: mac(fields[2]).zip(
            fields[20]
                .parse::<Ipv6Addr>()
                .ok()
                .filter(|ip| !ip.is_unspecified() && !ip.is_multicast() && !ip.is_loopback()),
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
        dhcp_message: fields[21].parse().ok(),
        dhcp_requested: requested(all[22]),
        dhcp_vendor: said(fields[23], 128),
        dhcp_server: fields[24].parse().ok(),
        router_advertisement: fields[25] == "134",
        hops: fields[26].parse().or_else(|_| fields[27].parse()).ok(),
        window: fields[28].parse().ok(),
        ja4: ja4(fields[29]),
        user_agent: said(all[30], 256),
        lldp_name: said(fields[31], 128),
        nbns_name: said(fields[32].split('<').next().unwrap_or(""), 64),
        dhcp_client: mac(fields[33]),
        answered_name: if flag(fields[16])
            && ports(fields[14], fields[15]).is_some_and(|(s, _)| s == 5355)
        {
            name(fields[17])
        } else if flag(fields[34]) {
            name(fields[32].split('<').next().unwrap_or("").trim())
        } else {
            None
        },
        dhcpv6_answer: matches!(fields[35], "2" | "7")
            && ports(fields[14], fields[15]).is_some_and(|(s, _)| s == 547),
        redirect: fields[36] == "5" || fields[25] == "137",
        arp_asked: (fields[37] == "1")
            .then(|| fields[38].parse::<Ipv4Addr>().ok())
            .flatten()
            .filter(|asked| fields[9].parse::<Ipv4Addr>().ok() != Some(*asked)),
        dns_answer: flag(fields[16]) && ports(fields[14], fields[15]).is_some_and(|(s, _)| s == 53),
        resolved: resolved(&all, &fields),
        nxdomain: (fields[41] == "3"
            && flag(fields[16])
            && ports(fields[14], fields[15]).is_some_and(|(s, _)| s == 53))
        .then(|| mac(fields[3]).zip(name(fields[17])))
        .flatten(),
        cleartext_password: password_command(&fields[42..46]),
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
    fn a_neighbor_advertisement_names_who_answers_for_an_ipv6_address() {
        let advert = [
            ("frame.time_epoch", "1791749339.250000000"),
            ("eth.src", "E4:AB:89:6E:1A:18"),
            ("ipv6.src", "fe80::1"),
            ("ipv6.dst", "ff02::1"),
            ("icmpv6.nd.na.target_address", "fe80::1"),
        ];
        assert_eq!(
            parse(&line(&advert)).unwrap().neighbor,
            Some(("e4:ab:89:6e:1a:18".into(), "fe80::1".parse().unwrap()))
        );
        // A multicast target is no claim on an address.
        let group = [
            ("frame.time_epoch", "1791749339.250000000"),
            ("eth.src", "e4:ab:89:6e:1a:18"),
            ("icmpv6.nd.na.target_address", "ff02::1"),
        ];
        assert_eq!(parse(&line(&group)).unwrap().neighbor, None);
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

    #[test]
    fn what_devices_say_about_themselves_is_read() {
        let mut fields = vec![
            ("frame.time_epoch", "20.0"),
            ("eth.src", "00:50:56:12:34:56"),
            ("ip.src", "0.0.0.0"),
            ("dhcp.option.dhcp", "1"),
            (
                "dhcp.option.request_list_item",
                "1,3,6,15,31,33,43,44,46,47,119,121,249,252",
            ),
            ("dhcp.option.vendor_class_id", "MSFT 5.0"),
            ("ip.ttl", "128"),
            ("tcp.window_size_value", "64240"),
            ("tls.handshake.ja4", "t13d1516h2_8daaf6152771_02713d6af862"),
            ("http.user_agent", "Mozilla/5.0 (KHTML, like Gecko)"),
            ("nbns.name", "DESKTOP-ARCHIE<00>"),
            ("dhcp.hw.mac_addr", "00:50:56:12:34:56"),
        ];
        let packet = parse(&line(&fields)).unwrap();
        assert_eq!(packet.dhcp_message, Some(1));
        assert_eq!(
            packet.dhcp_requested.as_deref(),
            Some("1,3,6,15,31,33,43,44,46,47,119,121,249,252")
        );
        assert_eq!(packet.dhcp_vendor.as_deref(), Some("MSFT 5.0"));
        assert_eq!(packet.dhcp_client.as_deref(), Some("00:50:56:12:34:56"));
        assert_eq!((packet.hops, packet.window), (Some(128), Some(64240)));
        assert_eq!(
            packet.ja4.as_deref(),
            Some("t13d1516h2_8daaf6152771_02713d6af862")
        );
        // A User-Agent keeps its commas; NetBIOS loses its type suffix.
        assert_eq!(
            packet.user_agent.as_deref(),
            Some("Mozilla/5.0 (KHTML, like Gecko)")
        );
        assert_eq!(packet.nbns_name.as_deref(), Some("DESKTOP-ARCHIE"));
        assert_eq!(packet.answered_name, None, "a question is not an answer");
        let llmnr = parse(&line(&[
            ("frame.time_epoch", "1"),
            ("udp.srcport", "5355"),
            ("udp.dstport", "50000"),
            ("dns.flags.response", "True"),
            ("dns.qry.name", "FileServ"),
        ]))
        .unwrap();
        assert_eq!(llmnr.answered_name.as_deref(), Some("fileserv"));
        let nbns = parse(&line(&[
            ("frame.time_epoch", "1"),
            ("nbns.flags.response", "True"),
            ("nbns.name", "WPAD<00> (Workstation/Redirector)"),
        ]))
        .unwrap();
        assert_eq!(nbns.answered_name.as_deref(), Some("wpad"));
        let advertise = |source_port: &str| {
            parse(&line(&[
                ("frame.time_epoch", "1"),
                ("udp.srcport", source_port),
                ("udp.dstport", "546"),
                ("dhcpv6.msgtype", "2"),
            ]))
            .unwrap()
            .dhcpv6_answer
        };
        assert!(advertise("547"));
        for (field, value) in [("icmp.type", "5"), ("icmpv6.type", "137")] {
            let redirect = parse(&line(&[("frame.time_epoch", "1"), (field, value)])).unwrap();
            assert!(redirect.redirect, "{field}");
        }
        let echo = parse(&line(&[("frame.time_epoch", "1"), ("icmp.type", "8")])).unwrap();
        assert!(!echo.redirect);
        let who_has = |asked: &str| {
            parse(&line(&[
                ("frame.time_epoch", "1"),
                ("arp.src.hw_mac", "02:00:5e:12:34:56"),
                ("arp.src.proto_ipv4", "192.168.1.77"),
                ("arp.opcode", "1"),
                ("arp.dst.proto_ipv4", asked),
            ]))
            .unwrap()
            .arp_asked
        };
        assert_eq!(who_has("192.168.1.9"), Some("192.168.1.9".parse().unwrap()));
        assert_eq!(who_has("192.168.1.77"), None, "a gratuitous announcement");
        let answer = |port: &str| {
            parse(&line(&[
                ("frame.time_epoch", "1"),
                ("udp.srcport", port),
                ("udp.dstport", "50000"),
                ("dns.flags.response", "True"),
                ("dns.qry.name", "example.com"),
            ]))
            .unwrap()
            .dns_answer
        };
        assert!(answer("53"));
        assert!(!answer("5353"), "mDNS is not a DNS server");
        let resolved = |port: &str| {
            parse(&line(&[
                ("frame.time_epoch", "1"),
                ("udp.srcport", port),
                ("udp.dstport", "50000"),
                ("dns.flags.response", "True"),
                ("dns.qry.name", "Telemetry.Example.com."),
                ("dns.a", "203.0.113.7,203.0.113.8,not-an-address"),
                ("dns.aaaa", "2001:db8::7"),
            ]))
            .unwrap()
            .resolved
        };
        let (asked, addresses) = resolved("53").unwrap();
        assert_eq!(asked, "telemetry.example.com");
        assert_eq!(
            addresses,
            ["203.0.113.7", "203.0.113.8", "2001:db8::7"].map(|a| a.parse::<IpAddr>().unwrap())
        );
        assert_eq!(
            resolved("5353"),
            None,
            "only a DNS server's answers name addresses"
        );
        let nxdomain = |rcode: &str| {
            parse(&line(&[
                ("frame.time_epoch", "1"),
                ("eth.dst", "54:4E:F0:07:3D:E8"),
                ("udp.srcport", "53"),
                ("udp.dstport", "50000"),
                ("dns.flags.response", "True"),
                ("dns.qry.name", "Kq7vx2pzm9wd.com"),
                ("dns.flags.rcode", rcode),
            ]))
            .unwrap()
            .nxdomain
        };
        assert_eq!(
            nxdomain("3"),
            Some(("54:4e:f0:07:3d:e8".into(), "kq7vx2pzm9wd.com".into()))
        );
        assert_eq!(nxdomain("0"), None, "an answer that resolved");
        let sent = |field: &str, command: &str| {
            parse(&line(&[("frame.time_epoch", "1"), (field, command)]))
                .unwrap()
                .cleartext_password
        };
        assert_eq!(sent("ftp.request.command", "PASS"), Some("FTP"));
        assert_eq!(sent("ftp.request.command", "USER"), None);
        assert_eq!(sent("pop.request.command", "pass"), Some("POP3"));
        assert_eq!(sent("imap.request.command", "LOGIN"), Some("IMAP"));
        assert_eq!(sent("smtp.req.command", "AUTH"), Some("SMTP"));
        assert_eq!(sent("smtp.req.command", "EHLO"), None);
        assert!(!advertise("546"), "only a server's port answers");
        // A second occurrence of an address field is not the frame's own (ICMP errors).
        fields.push(("ip.dst", "192.168.1.1,10.0.0.1"));
        assert_eq!(
            parse(&line(&fields)).unwrap().destination,
            Some("192.168.1.1".parse().unwrap())
        );
        // Garbage in a fingerprint field is dropped, not stored.
        let bad = parse(&line(&[
            ("frame.time_epoch", "1"),
            ("tls.handshake.ja4", "<script>"),
            ("dhcp.option.request_list_item", "1,x,3"),
            ("icmpv6.type", "134"),
        ]))
        .unwrap();
        assert_eq!((bad.ja4, bad.dhcp_requested), (None, None));
        assert!(bad.router_advertisement);
    }
}
