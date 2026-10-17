//! archie-netwatch reading a real capture through a real TShark: the field names it asks
//! for exist, and a rogue DHCP server, a rogue IPv6 router, a DHCP starvation flood, a
//! Responder poisoning names, mitm6 answering DHCPv6, an ICMP Redirect, a rogue DNS server,
//! an ARP sweep, a domain generation algorithm, passwords sent in the clear and a switch
//! table flood are told, with what the devices said of themselves kept for them.
//!
//! ```text
//! cargo test -p archie-lan --test netwatch_tshark -- --ignored
//! ```

use std::process::Command;

#[test]
#[ignore = "needs tshark on PATH"]
fn a_rogue_dhcp_server_and_a_rogue_router_are_told_from_a_real_capture() {
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/rogue-dhcp-and-ra.pcap");
    let state = std::env::temp_dir().join(format!("netwatch-tshark-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&state);
    std::fs::create_dir_all(&state).unwrap();
    let netwatch = env!("CARGO_BIN_EXE_archie-netwatch");
    let read = Command::new(netwatch)
        .arg("read")
        .arg(&fixture)
        .arg("--state-dir")
        .arg(&state)
        .args(["--gateway", "192.168.1.1"])
        .output()
        .unwrap();
    assert!(
        read.status.success(),
        "{}",
        String::from_utf8_lossy(&read.stderr)
    );
    let report = Command::new(netwatch)
        .arg("report")
        .arg("--state-dir")
        .arg(&state)
        .output()
        .unwrap();
    let report: serde_json::Value = serde_json::from_slice(&report.stdout).unwrap();
    let rules: Vec<&str> = report["alerts"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|a| a["severity"] == "high")
        .map(|a| a["rule"].as_str().unwrap())
        .collect();
    assert_eq!(
        rules,
        [
            "rogue-dhcp-server",
            "rogue-router-advertisement",
            "dhcp-starvation",
            "name-poisoning",
            "rogue-dhcpv6-server",
            "icmp-redirect",
            "cleartext-password",
            "cleartext-password",
            "mac-flood"
        ]
    );
    let starvation = report["alerts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["rule"] == "dhcp-starvation")
        .unwrap();
    assert_eq!(starvation["device"], "02:00:5e:dd:ee:ff");
    // The rogue answering DNS is told.
    assert!(report["alerts"]
        .as_array()
        .unwrap()
        .iter()
        .any(|a| a["rule"] == "rogue-dns-server" && a["device"] == "02:00:5e:aa:bb:cc"));
    // The ARP sweep is told, as a warning, with the scanner's address.
    assert!(report["alerts"]
        .as_array()
        .unwrap()
        .iter()
        .any(|a| a["rule"] == "arp-sweep" && a["device"] == "02:00:5e:12:34:56"));
    // The generated names the Windows client was told do not exist are told.
    assert!(report["alerts"]
        .as_array()
        .unwrap()
        .iter()
        .any(|a| a["rule"] == "dga-suspect" && a["device"] == "00:50:56:12:34:56"));
    // The old camera is told, as a notice, and its profile says why.
    assert!(report["alerts"]
        .as_array()
        .unwrap()
        .iter()
        .any(|a| a["rule"] == "retired-tls" && a["device"] == "00:12:12:00:00:01"));
    let camera = report["profiles"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["mac"] == "00:12:12:00:00:01")
        .unwrap();
    assert_eq!(camera["tls_versions"][0], "TLS 1.0");
    // The flood's made-up addresses are not kept as devices.
    assert!(report["profiles"].as_array().unwrap().len() < 10);
    let windows = report["profiles"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["mac"] == "00:50:56:12:34:56")
        .unwrap();
    assert_eq!(windows["vendor"], "VMware, Inc.");
    assert_eq!(
        windows["dhcp_fingerprint"],
        "1,3,6,15,31,33,43,44,46,47,119,121,249,252"
    );
    assert_eq!(windows["netbios_names"][0], "DESKTOP-ARCHIE");
    assert_eq!(windows["ja4"].as_array().unwrap().len(), 1);
    std::fs::remove_dir_all(&state).unwrap();
    assert!(windows["hints"]
        .as_array()
        .unwrap()
        .iter()
        .all(|hint| hint["says"] == "Windows"));
}
