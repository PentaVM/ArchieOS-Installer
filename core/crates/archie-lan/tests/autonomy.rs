use archie_lan::{
    autonomy::{Controller, Settings},
    inventory::{Device, Inventory, FORMAT},
};

fn live(now: u64) -> Inventory {
    Inventory {
        format: FORMAT,
        scanned_unix: now,
        devices: vec![Device {
            address: "10.0.0.42".into(),
            name: "Observed player".into(),
            seen_by: vec!["ssdp".into()],
            state: [("power".into(), "PowerOn".into())].into(),
            ..Default::default()
        }],
    }
}

#[test]
fn restart_preserves_known_devices_without_representing_old_state_as_current() {
    let mut controller = Controller::new(Settings::default()).unwrap();
    controller.record_success(&live(100), true).unwrap();
    let bytes = controller.encode().unwrap();
    let mut restarted = Controller::restore(Settings::default(), &bytes).unwrap();
    let output = restarted
        .record_success(
            &Inventory {
                format: FORMAT,
                scanned_unix: 130,
                devices: vec![],
            },
            false,
        )
        .unwrap();
    assert_eq!(output.devices.len(), 1);
    assert_eq!(output.devices[0].name, "Observed player");
    assert_eq!(output.devices[0].seen_by, ["history-cache"]);
    assert!(!output.devices[0].state.contains_key("power"));
    assert_eq!(output.devices[0].state["history.last_response_unix"], "100");
    assert!(!restarted.deep_due(130));
}

#[test]
fn light_and_deep_cadences_are_independent_and_failure_backoff_recovers() {
    let mut controller = Controller::new(Settings::default()).unwrap();
    assert!(controller.deep_due(100));
    controller.record_success(&live(100), true).unwrap();
    assert!(!controller.deep_due(130));
    assert!(controller.deep_due(700));
    assert!(
        !controller.deep_due(99),
        "clock rollback caused another expensive probe"
    );
    let mut ahead = Controller::new(Settings::default()).unwrap();
    ahead.record_success(&live(1_000_000), true).unwrap();
    assert!(
        ahead.deep_due(1_000_000 - 601),
        "a clock stepped back by more than an interval must not suppress deep passes"
    );
    assert_eq!(controller.record_failure("no interface").unwrap(), 30);
    assert_eq!(controller.record_failure("no interface").unwrap(), 60);
    for _ in 0..10 {
        controller.record_failure("no interface").unwrap();
    }
    assert_eq!(controller.record_failure("no interface").unwrap(), 300);
    controller.record_success(&live(800), false).unwrap();
    assert_eq!(controller.record_failure("no interface").unwrap(), 30);
}

#[test]
fn corrupt_oversized_state_and_unsafe_settings_are_rejected() {
    assert!(Controller::restore(Settings::default(), b"not-json").is_err());
    assert!(Controller::restore(Settings::default(), &vec![b' '; 8 * 1024 * 1024 + 1]).is_err());
    assert!(Controller::new(Settings {
        max_devices: 4097,
        ..Default::default()
    })
    .is_err());
    assert!(Controller::new(Settings {
        retention_seconds: 0,
        ..Default::default()
    })
    .is_err());
}

#[test]
fn cache_rows_cannot_refresh_response_time_or_retain_candidates_forever() {
    let settings = Settings {
        retention_seconds: 60,
        ..Default::default()
    };
    let mut controller = Controller::new(settings).unwrap();
    controller.record_success(&live(100), false).unwrap();
    let cache = Inventory {
        format: FORMAT,
        scanned_unix: 130,
        devices: vec![Device {
            address: "10.0.0.42".into(),
            seen_by: vec!["neighbor-cache".into()],
            ..Default::default()
        }],
    };
    let output = controller.record_success(&cache, false).unwrap();
    assert_eq!(output.devices[0].state["history.last_response_unix"], "100");
    let expired = controller
        .record_success(
            &Inventory {
                format: FORMAT,
                scanned_unix: 191,
                devices: vec![],
            },
            false,
        )
        .unwrap();
    assert!(expired.devices.is_empty());
}

#[test]
fn changed_interface_scope_cannot_reuse_old_ip_labels() {
    let mut controller = Controller::new(Settings::default()).unwrap();
    controller
        .set_scope("adapter-a|10.0.0.5|255.255.255.0")
        .unwrap();
    controller.record_success(&live(100), true).unwrap();
    let mut restored =
        Controller::restore(Settings::default(), &controller.encode().unwrap()).unwrap();
    restored
        .set_scope("adapter-b|10.0.0.5|255.255.255.0")
        .unwrap();
    assert!(restored.deep_due(130));
    let output = restored
        .record_success(
            &Inventory {
                format: FORMAT,
                scanned_unix: 130,
                devices: vec![],
            },
            false,
        )
        .unwrap();
    assert!(output.devices.is_empty());
}

#[test]
fn long_light_cadence_is_not_shortened_by_failure_backoff() {
    let mut controller = Controller::new(Settings {
        light_seconds: 600,
        deep_seconds: 1200,
        ..Default::default()
    })
    .unwrap();
    assert_eq!(controller.record_failure("failed").unwrap(), 600);
}

#[test]
fn current_cache_metadata_survives_without_reviving_power_or_reusing_a_changed_mac_label() {
    let mut controller = Controller::new(Settings::default()).unwrap();
    let mut first = live(100);
    first.devices[0]
        .state
        .insert("neighbor.mac".into(), "02:00:00:00:00:01".into());
    controller.record_success(&first, false).unwrap();
    let mut cache = Inventory {
        format: FORMAT,
        scanned_unix: 130,
        devices: vec![Device {
            address: "10.0.0.42".into(),
            seen_by: vec!["neighbor-cache".into()],
            state: [
                ("neighbor.mac".into(), "02:00:00:00:00:01".into()),
                (
                    "neighbor.source".into(),
                    "system-cache; reachability unconfirmed".into(),
                ),
            ]
            .into(),
            ..Default::default()
        }],
    };
    let output = controller.record_success(&cache, false).unwrap();
    assert_eq!(output.devices[0].state["neighbor.mac"], "02:00:00:00:00:01");
    assert_eq!(output.devices[0].name, "Observed player");
    assert!(!output.devices[0].state.contains_key("power"));
    cache.devices[0]
        .state
        .insert("neighbor.mac".into(), "02:00:00:00:00:02".into());
    let changed = controller.record_success(&cache, false).unwrap();
    assert_eq!(changed.devices[0].name, "");
    assert!(!changed.devices[0]
        .state
        .contains_key("history.last_response_unix"));
}

#[test]
fn mac_learned_after_a_live_response_is_persisted_without_refreshing_that_response() {
    let mut controller = Controller::new(Settings::default()).unwrap();
    controller.record_success(&live(100), false).unwrap();
    let mut cache = Inventory {
        format: FORMAT,
        scanned_unix: 130,
        devices: vec![Device {
            address: "10.0.0.42".into(),
            seen_by: vec!["neighbor-cache".into()],
            state: [("neighbor.mac".into(), "02:00:00:00:00:01".into())].into(),
            ..Default::default()
        }],
    };
    controller.record_success(&cache, false).unwrap();
    let mut restarted =
        Controller::restore(Settings::default(), &controller.encode().unwrap()).unwrap();
    cache.scanned_unix = 160;
    cache.devices[0]
        .state
        .insert("neighbor.mac".into(), "02:00:00:00:00:02".into());
    let changed = restarted.record_success(&cache, false).unwrap();
    assert_eq!(changed.devices[0].name, "");
    assert!(!changed.devices[0]
        .state
        .contains_key("history.last_response_unix"));
}

#[test]
fn unchanged_live_state_excludes_internal_monitor_timestamps() {
    let mut controller = Controller::new(Settings::default()).unwrap();
    let first = controller.record_success(&live(100), false).unwrap();
    let next = controller.record_success(&live(130), false).unwrap();
    assert_eq!(first.devices[0].state, next.devices[0].state);
}

fn device(address: &str, seen_by: &[&str], state: &[(&str, &str)]) -> Device {
    Device {
        address: address.into(),
        seen_by: seen_by.iter().map(|s| s.to_string()).collect(),
        state: state
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        ..Default::default()
    }
}

fn pass(now: u64, devices: Vec<Device>) -> Inventory {
    Inventory {
        format: FORMAT,
        scanned_unix: now,
        devices,
    }
}

#[test]
fn what_a_deep_pass_saw_stands_through_the_light_passes_after_it() {
    let mut controller = Controller::new(Settings::default()).unwrap();
    let mac = ("neighbor.mac", "aa:bb:cc:dd:ee:01");
    // A silent device only the TCP sweep hears, and a TV heard by SSDP with open ports.
    let deep = controller
        .record_success(
            &pass(
                100,
                vec![
                    device(
                        "10.0.0.7",
                        &["tcp-presence", "neighbor-cache"],
                        &[("open_tcp_ports", "80"), mac],
                    ),
                    device(
                        "10.0.0.8",
                        &["ssdp", "tcp-presence"],
                        &[("open_tcp_ports", "8060")],
                    ),
                ],
            ),
            true,
        )
        .unwrap();
    let light = controller
        .record_success(
            &pass(
                130,
                vec![
                    device("10.0.0.7", &["neighbor-cache"], &[mac]),
                    device("10.0.0.8", &["ssdp"], &[]),
                ],
            ),
            false,
        )
        .unwrap();
    assert_eq!(
        light.devices, deep.devices,
        "a light pass changed nothing that was seen"
    );
    // Not even in the cache on a light pass: the deep observation still stands.
    let quiet = controller
        .record_success(&pass(160, vec![]), false)
        .unwrap();
    assert_eq!(quiet.devices[0].seen_by, ["tcp-presence"]);
    assert_eq!(quiet.devices[0].state["open_tcp_ports"], "80");
    // The next deep pass that does not hear it ends it.
    let next = controller
        .record_success(
            &pass(700, vec![device("10.0.0.7", &["neighbor-cache"], &[mac])]),
            true,
        )
        .unwrap();
    assert_eq!(next.devices[0].seen_by, ["history-cache"]);
}

#[test]
fn a_device_the_cache_still_holds_is_kept_past_retention() {
    let mut controller = Controller::new(Settings::default()).unwrap();
    let cached = || {
        device(
            "10.0.0.9",
            &["neighbor-cache"],
            &[("neighbor.mac", "aa:bb:cc:dd:ee:02")],
        )
    };
    controller
        .record_success(&pass(100, vec![cached()]), true)
        .unwrap();
    for now in [50_000, 100_000, 150_000] {
        let output = controller
            .record_success(&pass(now, vec![cached()]), false)
            .unwrap();
        assert_eq!(output.devices.len(), 1, "dropped at {now}");
    }
    // Once nothing has seen it for the whole retention, it goes.
    let output = controller
        .record_success(&pass(150_000 + 86_401, vec![]), false)
        .unwrap();
    assert!(output.devices.is_empty());
}

#[test]
fn times_from_before_a_clock_step_back_still_expire() {
    let mut controller = Controller::new(Settings::default()).unwrap();
    controller
        .record_success(
            &pass(10_000_000, vec![device("10.0.0.10", &["ssdp"], &[])]),
            true,
        )
        .unwrap();
    // The clock is corrected back; the record is now anchored at the corrected time.
    assert_eq!(
        controller
            .record_success(&pass(100, vec![]), false)
            .unwrap()
            .devices
            .len(),
        1
    );
    assert!(controller
        .record_success(&pass(100 + 86_401, vec![]), false)
        .unwrap()
        .devices
        .is_empty());
}

#[test]
fn the_device_limit_makes_room_from_devices_not_seen_now() {
    let settings = Settings {
        max_devices: 2,
        ..Settings::default()
    };
    let mut controller = Controller::new(settings).unwrap();
    controller
        .record_success(
            &pass(
                100,
                vec![
                    device("10.0.0.1", &["ssdp"], &[]),
                    device("10.0.0.2", &["ssdp"], &[]),
                ],
            ),
            true,
        )
        .unwrap();
    let output = controller
        .record_success(
            &pass(
                200,
                vec![
                    device("10.0.0.2", &["ssdp"], &[]),
                    device("10.0.0.3", &["ssdp"], &[]),
                ],
            ),
            false,
        )
        .unwrap();
    let addresses: Vec<_> = output.devices.iter().map(|d| d.address.as_str()).collect();
    assert_eq!(addresses, ["10.0.0.2", "10.0.0.3"]);
}
