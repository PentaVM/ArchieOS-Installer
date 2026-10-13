use archie_lan::neighbors::{parse_linux, parse_macos, parse_windows};

#[test]
fn neighbor_cache_is_interface_scoped_and_never_an_identity_guess() {
    let text = "Interfaz: 10.0.0.7 --- 0x4\n  Internet Address Physical Address Type\n  10.0.0.42 02-11-22-33-44-55 dynamic\n  10.0.0.43 00-00-00-00-00-00 invalid\n  224.0.0.251 01-00-5e-00-00-fb static\nInterfaz: 172.16.0.1 --- 0x2\n  172.16.0.9 00-aa-bb-cc-dd-ee dynamic\n";
    let records = parse_windows(text, "10.0.0.7".parse().unwrap());
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].address.to_string(), "10.0.0.42");
    assert_eq!(records[0].mac, "02:11:22:33:44:55");
    assert_eq!(records[0].status, "dynamic");
}

#[test]
fn malformed_or_public_neighbor_rows_are_not_devices() {
    let text = "Interface: 10.0.0.7 --- 0x4\n 8.8.8.8 00-11-22-33-44-55 dynamic\n 10.0.0.9 abcd dynamic\n 10.0.0.42 ff-ff-ff-ff-ff-ff static\n";
    assert!(parse_windows(text, "10.0.0.7".parse().unwrap()).is_empty());
}

#[test]
fn linux_and_macos_keep_cached_status_and_ignore_other_interfaces() {
    let linux = r#"[{"dst":"10.0.0.9","dev":"eth0","lladdr":"02:11:22:33:44:55","state":["STALE"]},
      {"dst":"10.0.0.8","dev":"eth1","lladdr":"02:11:22:33:44:56","state":["REACHABLE"]},
      {"dst":"10.0.0.10","dev":"eth0","state":["INCOMPLETE"]}]"#;
    let records = parse_linux(linux, "eth0");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].status, "STALE");
    let mac = parse_macos("? (10.0.0.9) at 2:1:2:3:4:5 on en0 ifscope [ethernet]\n? (10.0.0.8) at 02:11:22:33:44:55 on en1\n", "en0");
    assert_eq!(mac.len(), 1);
    assert_eq!(mac[0].mac, "02:01:02:03:04:05");
    assert_eq!(mac[0].status, "cached");
}
