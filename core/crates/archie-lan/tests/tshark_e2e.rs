//! The audit end to end through the real TShark: a capture is built here, byte by byte,
//! `archie-netwatch read` dissects it with TShark and audits it, and the alerts it writes
//! are checked. This is what proves the field names in `capture::FIELDS` exist in the
//! TShark that is installed, which no unit test can.
//!
//! It needs `tshark` on PATH, so it is ignored by default; CI installs TShark and runs
//! it with `--ignored`. The capture is synthetic: it is the frames an attack would put on
//! the wire, not a recording of one.

use std::net::Ipv6Addr;
use std::path::Path;
use std::process::Command;

const ROUTER: [u8; 6] = [0xe4, 0xab, 0x89, 0x6e, 0x1a, 0x18];
const ATTACKER: [u8; 6] = [0xde, 0xad, 0xbe, 0xef, 0x00, 0x02];
const TV: [u8; 6] = [0x54, 0x4e, 0xf0, 0x07, 0x3d, 0xe8];

fn checksum(bytes: &[u8]) -> u16 {
    let mut sum: u32 = bytes
        .chunks(2)
        .map(|pair| u32::from(u16::from_be_bytes([pair[0], *pair.get(1).unwrap_or(&0)])))
        .sum();
    while sum > 0xffff {
        sum = (sum >> 16) + (sum & 0xffff);
    }
    !(sum as u16)
}

fn ethernet(destination: [u8; 6], source: [u8; 6], kind: u16, payload: &[u8]) -> Vec<u8> {
    [&destination[..], &source[..], &kind.to_be_bytes(), payload].concat()
}

fn arp_reply(sender: [u8; 6], sender_ip: [u8; 4], target: [u8; 6], target_ip: [u8; 4]) -> Vec<u8> {
    let body = [
        &[0, 1, 8, 0, 6, 4, 0, 2][..],
        &sender,
        &sender_ip,
        &target,
        &target_ip,
    ]
    .concat();
    ethernet(target, sender, 0x0806, &body)
}

fn ipv4(source: [u8; 4], destination: [u8; 4], protocol: u8, payload: &[u8]) -> Vec<u8> {
    let length = (20 + payload.len()) as u16;
    let mut header = [
        &[0x45, 0][..],
        &length.to_be_bytes(),
        &[0, 1, 0, 0, 64, protocol, 0, 0],
        &source,
        &destination,
    ]
    .concat();
    let sum = checksum(&header);
    header[10..12].copy_from_slice(&sum.to_be_bytes());
    [header, payload.to_vec()].concat()
}

/// An unsolicited Neighbor Advertisement: `sender` answers for `target`.
fn neighbor_advertisement(sender: [u8; 6], target: Ipv6Addr) -> Vec<u8> {
    let all_nodes: Ipv6Addr = "ff02::1".parse().unwrap();
    let mut icmp = [
        &[136, 0, 0, 0, 0x20, 0, 0, 0][..],
        &target.octets(),
        &[2, 1],
        &sender,
    ]
    .concat();
    let pseudo = [
        &target.octets()[..],
        &all_nodes.octets(),
        &(icmp.len() as u32).to_be_bytes(),
        &[0, 0, 0, 58],
        &icmp,
    ]
    .concat();
    let sum = checksum(&pseudo);
    icmp[2..4].copy_from_slice(&sum.to_be_bytes());
    let header = [
        &[0x60, 0, 0, 0][..],
        &(icmp.len() as u16).to_be_bytes(),
        &[58, 255],
        &target.octets(),
        &all_nodes.octets(),
    ]
    .concat();
    ethernet(
        [0x33, 0x33, 0, 0, 0, 1],
        sender,
        0x86dd,
        &[header, icmp].concat(),
    )
}

fn dns_question(name: &str) -> Vec<u8> {
    let mut question = Vec::new();
    for label in name.split('.') {
        question.push(label.len() as u8);
        question.extend_from_slice(label.as_bytes());
    }
    question.extend_from_slice(&[0, 0, 1, 0, 1]);
    let message = [&[0x12, 0x34, 1, 0, 0, 1, 0, 0, 0, 0, 0, 0][..], &question].concat();
    let udp = [
        &40000u16.to_be_bytes()[..],
        &53u16.to_be_bytes(),
        &((8 + message.len()) as u16).to_be_bytes(),
        &[0, 0],
        &message,
    ]
    .concat();
    ethernet(
        ROUTER,
        TV,
        0x0800,
        &ipv4([192, 168, 1, 84], [192, 168, 1, 1], 17, &udp),
    )
}

fn syn(host: u8, port: u16) -> Vec<u8> {
    let tcp = [
        &50000u16.to_be_bytes()[..],
        &port.to_be_bytes(),
        &[0, 0, 0, 1, 0, 0, 0, 0, 0x50, 0x02, 0xfa, 0xf0, 0, 0, 0, 0],
    ]
    .concat();
    ethernet(
        ROUTER,
        TV,
        0x0800,
        &ipv4([192, 168, 1, 84], [192, 168, 1, host], 6, &tcp),
    )
}

fn write_pcap(path: &Path, frames: &[Vec<u8>]) {
    let mut out = Vec::new();
    out.extend_from_slice(&0xa1b2_c3d4u32.to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&4u16.to_le_bytes());
    for word in [0u32, 0, 65535, 1] {
        out.extend_from_slice(&word.to_le_bytes());
    }
    for (i, frame) in frames.iter().enumerate() {
        let micros = 10_000 * (i as u32 + 1);
        for word in [
            1_791_749_339u32,
            micros,
            frame.len() as u32,
            frame.len() as u32,
        ] {
            out.extend_from_slice(&word.to_le_bytes());
        }
        out.extend_from_slice(frame);
    }
    std::fs::write(path, out).unwrap();
}

fn alerts(directory: &Path) -> Vec<serde_json::Value> {
    std::fs::read_to_string(directory.join("alerts.jsonl"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn find<'a>(alerts: &'a [serde_json::Value], rule: &str, needle: &str) -> &'a serde_json::Value {
    alerts
        .iter()
        .find(|a| a["rule"] == rule && a["detail"].as_str().unwrap().contains(needle))
        .unwrap_or_else(|| panic!("no {rule} alert about {needle} in {alerts:#?}"))
}

#[test]
#[ignore = "needs tshark on PATH; CI installs it and runs this with --ignored"]
fn a_capture_through_tshark_raises_the_rules_it_should() {
    let dir = std::env::temp_dir().join(format!("archie-tshark-e2e-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let pcap = dir.join("attack.pcap");
    let router_v6: Ipv6Addr = "fe80::1".parse().unwrap();
    let mut frames = vec![
        arp_reply(ROUTER, [192, 168, 1, 1], TV, [192, 168, 1, 84]),
        arp_reply(TV, [192, 168, 1, 84], ROUTER, [192, 168, 1, 1]),
        neighbor_advertisement(ROUTER, router_v6),
        // ARP spoofing of the router, and its IPv6 twin.
        arp_reply(ATTACKER, [192, 168, 1, 1], TV, [192, 168, 1, 84]),
        neighbor_advertisement(ATTACKER, router_v6),
        dns_question("ujzde8gxd6ncf10epf91dhodzdoc9is0j8ht9lgmxg9edn581u.exfil.example"),
    ];
    frames.extend((2..60).map(|host| syn(host, 445)));
    write_pcap(&pcap, &frames);

    let run = |gateway: &str, state: &str| {
        let state = dir.join(state);
        let output = Command::new(env!("CARGO_BIN_EXE_archie-netwatch"))
            .arg("read")
            .arg(&pcap)
            .arg("--state-dir")
            .arg(&state)
            .args(["--gateway", gateway])
            .output()
            .expect("archie-netwatch runs");
        assert!(
            output.status.success(),
            "archie-netwatch read failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        alerts(&state)
    };

    let v4 = run("192.168.1.1", "v4");
    let spoof = find(&v4, "address-moved", "192.168.1.1");
    assert_eq!(spoof["severity"], "high");
    assert_eq!(spoof["device"], "de:ad:be:ef:00:02");
    assert_eq!(
        find(&v4, "address-moved", "fe80::1")["severity"],
        "notice",
        "the IPv6 move is seen even when the router is named by its IPv4 address"
    );
    assert_eq!(
        find(&v4, "dns-tunnel-suspect", "exfil.example")["severity"],
        "warning"
    );
    assert_eq!(find(&v4, "host-sweep", "port 445")["severity"], "high");

    let v6 = run("fe80::1", "v6");
    assert_eq!(
        find(&v6, "address-moved", "fe80::1")["severity"],
        "high",
        "Neighbor Advertisement spoofing of the router is the serious case"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
