use archie_lan::inventory::{assemble, merge_observations};
use std::collections::BTreeMap;

#[test]
fn autonomous_refresh_does_not_emit_state_changes_for_internal_history_clocks() {
    let mut controller = archie_lan::autonomy::Controller::new(Default::default()).unwrap();
    let mut live = archie_lan::Inventory {
        format: archie_lan::inventory::FORMAT,
        scanned_unix: 100,
        devices: vec![archie_lan::Device {
            address: "10.0.0.42".into(),
            name: "Observed player".into(),
            seen_by: vec!["ssdp".into()],
            state: [("power".into(), "PowerOn".into())].into(),
            ..Default::default()
        }],
    };
    let first = controller.record_success(&live, false).unwrap();
    live.scanned_unix = 130;
    let next = controller.record_success(&live, false).unwrap();
    let prev = archie_sentry::feed::Snapshot::from_lan_inventory_json(
        &serde_json::to_string(&first).unwrap(),
    )
    .unwrap();
    let current = archie_sentry::feed::Snapshot::from_lan_inventory_json(
        &serde_json::to_string(&next).unwrap(),
    )
    .unwrap();
    assert!(archie_sentry::feed::diff_snapshots(&prev, &current)
        .unwrap()
        .is_empty());
}

#[test]
fn expanded_discovery_is_consumed_by_the_existing_core_inventory_parser() {
    let mut inventory = assemble(
        &[],
        &BTreeMap::new(),
        &BTreeMap::new(),
        &BTreeMap::new(),
        42,
    );
    merge_observations(
        &mut inventory,
        &[archie_lan::wsdiscovery::Match {
            from: "10.0.0.42".parse().unwrap(),
            class: Some("printer".into()),
            types: vec![
                "{http://schemas.microsoft.com/windows/2006/08/wdp/print}PrintDeviceType".into(),
            ],
            endpoint: Some("http://10.0.0.42/wsd".into()),
        }],
        &[("10.0.0.42".parse().unwrap(), vec![80, 9100])],
    );
    inventory.devices[0]
        .state
        .insert("neighbor.mac".into(), "02:11:22:33:44:55".into());
    inventory.devices[0]
        .state
        .insert("discovery_tags".into(), "matter,mqtt".into());
    inventory.devices[0].seen_by.push("neighbor-cache".into());
    let parsed =
        archie_core::unify::parse_inventory(&serde_json::to_string(&inventory).unwrap()).unwrap();
    assert_eq!(parsed.devices.len(), 1);
    assert_eq!(parsed.devices[0].state["open_tcp_ports"], "80,9100");
    assert_eq!(parsed.devices[0].state["neighbor.mac"], "02:11:22:33:44:55");
    assert_eq!(parsed.devices[0].class.as_deref(), Some("printer"));
}

#[test]
fn a_neighbor_cache_candidate_does_not_refresh_peer_observation_time() {
    let mut inventory = assemble(
        &[],
        &BTreeMap::new(),
        &BTreeMap::new(),
        &BTreeMap::new(),
        42,
    );
    merge_observations(
        &mut inventory,
        &[],
        &[("10.0.0.42".parse().unwrap(), vec![80])],
    );
    inventory.devices[0].seen_by = vec!["neighbor-cache".into()];
    inventory.devices[0].state.clear();
    inventory.devices[0]
        .state
        .insert("neighbor.status".into(), "STALE".into());
    let json = serde_json::to_string(&inventory).unwrap();
    let parsed = archie_core::unify::parse_inventory(&json).unwrap();
    let unified = archie_core::unify::unify(Some(&parsed), None, &[], 43).unwrap();
    assert_eq!(unified.devices[0].last_seen_unix, None);
    assert_eq!(unified.devices[0].first_seen_unix, None);
    assert!(archie_sentry::feed::from_lan_inventory(&json)
        .unwrap()
        .is_empty());
}

#[test]
fn historical_inventory_retains_metadata_without_refreshing_presence_or_context() {
    let mut inventory = assemble(
        &[],
        &BTreeMap::new(),
        &BTreeMap::new(),
        &BTreeMap::new(),
        42,
    );
    merge_observations(
        &mut inventory,
        &[],
        &[("10.0.0.42".parse().unwrap(), vec![80])],
    );
    inventory.devices[0].seen_by = vec!["history-cache".into()];
    inventory.devices[0].state.clear();
    inventory.devices[0]
        .state
        .insert("history.last_response_unix".into(), "10".into());
    let json = serde_json::to_string(&inventory).unwrap();
    let parsed = archie_core::unify::parse_inventory(&json).unwrap();
    let unified = archie_core::unify::unify(Some(&parsed), None, &[], 43).unwrap();
    assert_eq!(unified.devices[0].last_seen_unix, None);
    assert!(archie_sentry::feed::from_lan_inventory(&json)
        .unwrap()
        .is_empty());
}
