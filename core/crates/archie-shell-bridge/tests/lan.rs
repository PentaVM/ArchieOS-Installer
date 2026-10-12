//! The LAN inventory reader. The positive case writes the file with the REAL
//! `archie_lan::inventory::Inventory` type, so the bridge is held to what the scanner emits
//! and not to a copy of its schema.

use archie_lan::inventory::{Device as LanDevice, Inventory, FORMAT};
use archie_shell_bridge::lan::{read, Lan, MAX_DEVICES, MAX_FILE_BYTES, STALE_AFTER_SECONDS};
use std::collections::BTreeMap;
use std::path::PathBuf;

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("archie-shell-lan-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn device(name: &str, address: &str) -> LanDevice {
    let mut state = BTreeMap::new();
    state.insert("power".to_string(), "on".to_string());
    LanDevice {
        address: address.into(),
        class: Some("television".into()),
        name: name.into(),
        manufacturer: "ExampleCo".into(),
        model: "X1".into(),
        services: vec!["_googlecast._tcp".into()],
        seen_by: vec!["ssdp".into(), "mdns".into()],
        state,
    }
}

fn write_inventory(path: &std::path::Path, scanned_unix: u64, devices: Vec<LanDevice>) {
    let inventory = Inventory {
        format: FORMAT,
        scanned_unix,
        devices,
    };
    std::fs::write(path, serde_json::to_vec_pretty(&inventory).unwrap()).unwrap();
}

#[test]
fn the_format_constant_matches_the_scanner() {
    assert_eq!(archie_shell_bridge::lan::FORMAT, FORMAT);
}

#[test]
fn an_inventory_written_by_the_real_type_is_read_back() {
    let file = scratch("ok").join("lan.json");
    write_inventory(
        &file,
        1_000_000,
        vec![
            device("Sala", "192.168.1.20"),
            device("Cuarto", "192.168.1.21"),
        ],
    );
    match read(Some(&file), 1_000_100) {
        Lan::Ok {
            scanned_unix,
            age_seconds,
            stale,
            total,
            devices,
        } => {
            assert_eq!(scanned_unix, 1_000_000);
            assert_eq!(age_seconds, 100);
            assert!(!stale);
            assert_eq!(total, 2);
            assert_eq!(devices[0].name, "Sala");
            assert_eq!(devices[0].state["power"], "on");
            assert_eq!(devices[0].class.as_deref(), Some("television"));
        }
        other => panic!("got {other:?}"),
    }
}

#[test]
fn an_empty_inventory_is_ok_and_empty_not_missing() {
    let file = scratch("empty").join("lan.json");
    write_inventory(&file, 10, vec![]);
    match read(Some(&file), 20) {
        Lan::Ok { total, devices, .. } => assert!(total == 0 && devices.is_empty()),
        other => panic!("got {other:?}"),
    }
}

#[test]
fn an_old_inventory_is_stale() {
    let file = scratch("stale").join("lan.json");
    write_inventory(&file, 1_000, vec![device("Sala", "192.168.1.20")]);
    match read(Some(&file), 1_000 + STALE_AFTER_SECONDS + 1) {
        Lan::Ok { stale, .. } => assert!(stale),
        other => panic!("got {other:?}"),
    }
    match read(Some(&file), 1_000 + STALE_AFTER_SECONDS) {
        Lan::Ok { stale, .. } => assert!(!stale, "exactly at the limit is not yet stale"),
        other => panic!("got {other:?}"),
    }
}

#[test]
fn a_scan_time_in_the_future_has_age_zero() {
    let file = scratch("future").join("lan.json");
    write_inventory(&file, 5_000, vec![]);
    match read(Some(&file), 100) {
        Lan::Ok {
            age_seconds, stale, ..
        } => assert!(age_seconds == 0 && !stale),
        other => panic!("got {other:?}"),
    }
}

#[test]
fn not_configured_missing_and_unreadable_are_distinct_states() {
    assert_eq!(read(None, 0), Lan::NotConfigured);
    let gone = scratch("gone").join("nope.json");
    assert!(matches!(read(Some(&gone), 0), Lan::Missing { .. }));
}

#[test]
fn a_directory_is_not_an_inventory() {
    let dir = scratch("dir");
    assert!(
        matches!(
            read(Some(&dir), 0),
            Lan::Invalid { .. } | Lan::Unreadable { .. }
        ),
        "got {:?}",
        read(Some(&dir), 0)
    );
}

#[test]
fn malformed_json_and_other_formats_are_invalid() {
    let dir = scratch("bad");
    let junk = dir.join("junk.json");
    std::fs::write(&junk, b"not json").unwrap();
    assert!(matches!(read(Some(&junk), 0), Lan::Invalid { .. }));

    let other = dir.join("other.json");
    std::fs::write(
        &other,
        br#"{"format":"archie-lan-inventory-v2","scanned_unix":1,"devices":[]}"#,
    )
    .unwrap();
    match read(Some(&other), 0) {
        Lan::Invalid { reason } => assert!(reason.contains("v2"), "{reason}"),
        other => panic!("got {other:?}"),
    }

    let no_format = dir.join("nofmt.json");
    std::fs::write(&no_format, br#"{"scanned_unix":1,"devices":[]}"#).unwrap();
    assert!(matches!(read(Some(&no_format), 0), Lan::Invalid { .. }));
}

#[test]
fn an_oversized_file_is_refused_unread() {
    let file = scratch("big").join("lan.json");
    std::fs::write(&file, vec![b' '; (MAX_FILE_BYTES + 10) as usize]).unwrap();
    match read(Some(&file), 0) {
        Lan::Invalid { reason } => assert!(reason.contains("limit"), "{reason}"),
        other => panic!("got {other:?}"),
    }
}

#[test]
fn more_devices_than_the_cap_are_counted_but_not_all_sent() {
    let file = scratch("many").join("lan.json");
    let many: Vec<LanDevice> = (0..MAX_DEVICES + 5)
        .map(|i| device(&format!("d{i}"), "10.0.0.1"))
        .collect();
    write_inventory(&file, 1, many);
    match read(Some(&file), 2) {
        Lan::Ok { total, devices, .. } => {
            assert_eq!(total, MAX_DEVICES + 5);
            assert_eq!(devices.len(), MAX_DEVICES);
        }
        other => panic!("got {other:?}"),
    }
}

#[test]
fn a_frame_carries_the_lan_state_in_its_json_line() {
    use archie_shell_bridge::{poll_with, Client, Sources};
    let addr = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap();
    let file = scratch("frame").join("lan.json");
    write_inventory(&file, 1, vec![device("Sala", "192.168.1.20")]);
    let frame = poll_with(
        &Client::new(addr).unwrap(),
        &Sources {
            lan_file: Some(file.clone()),
            workspaces: vec![],
            ..Default::default()
        },
    );
    let line = serde_json::to_string(&frame).unwrap();
    let value: serde_json::Value = serde_json::from_str(&line).unwrap();
    // displayd is down, yet the LAN file is still reported: the two sources are independent.
    assert_eq!(value["link"]["state"], "displayd_offline");
    assert_eq!(value["lan"]["state"], "ok");
    assert_eq!(value["lan"]["devices"][0]["name"], "Sala");
}
