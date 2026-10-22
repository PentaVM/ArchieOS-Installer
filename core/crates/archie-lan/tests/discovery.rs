use archie_lan::{
    inventory::{assemble, merge_observations},
    wsdiscovery::Match,
};
use std::collections::BTreeMap;

#[test]
fn wsd_printer_and_tcp_presence_merge_without_changing_the_inventory_contract() {
    let mut inventory = assemble(&[], &BTreeMap::new(), &BTreeMap::new(), &BTreeMap::new(), 1);
    let printer = "10.0.0.42".parse().unwrap();
    merge_observations(
        &mut inventory,
        &[Match {
            from: printer,
            class: Some("printer".into()),
            types: vec![
                "{http://schemas.microsoft.com/windows/2006/08/wdp/print}PrintDeviceType".into(),
            ],
            endpoint: Some("http://10.0.0.42/wsd".into()),
        }],
        &[
            ("10.0.0.42".parse().unwrap(), vec![80, 9100]),
            ("10.0.0.43".parse().unwrap(), vec![631]),
        ],
    );
    assert_eq!(inventory.devices.len(), 2);
    assert_eq!(inventory.devices[0].class.as_deref(), Some("printer"));
    assert_eq!(inventory.devices[0].state["open_tcp_ports"], "80,9100");
    assert!(inventory.devices[0]
        .seen_by
        .contains(&"ws-discovery".into()));
    assert_eq!(
        inventory.devices[1].class, None,
        "open port alone is not a printer identity"
    );
    assert_eq!(inventory.devices[1].name, "");
    assert_eq!(
        serde_json::to_value(inventory).unwrap()["format"],
        "archie-lan-inventory-v1"
    );
}

#[test]
fn invalid_cli_scope_is_rejected_before_any_scan_runs() {
    for args in [
        vec!["scan", "--subnet", "8.8.8.8/32"],
        vec!["scan", "--seconds", "0"],
        vec!["scan", "--unknown"],
        vec!["scan", "--bind", "8.8.8.8"],
        vec!["scan", "--probe-local", "--subnet", "10.0.0.0/24"],
    ] {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_archie-lan"))
            .args(args)
            .output()
            .unwrap();
        assert!(!output.status.success(), "invalid scope succeeded");
        assert!(
            output.stdout.is_empty(),
            "invalid scope produced an inventory"
        );
    }
}

#[test]
fn an_unassigned_private_binding_cannot_succeed_through_an_empty_neighbor_cache() {
    let interfaces = if_addrs::get_if_addrs().unwrap();
    let address = ["10.254.254.254", "192.168.255.254"]
        .into_iter()
        .map(|ip| ip.parse::<std::net::Ipv4Addr>().unwrap())
        .find(|a| {
            !interfaces
                .iter()
                .any(|i| i.addr.ip() == std::net::IpAddr::V4(*a))
        })
        .unwrap();
    let result = archie_lan::inventory::scan_with(
        archie_lan::inventory::ScanOptions {
            bind: Some(address),
            ..Default::default()
        },
        std::time::Duration::ZERO,
    );
    assert!(
        result.is_err(),
        "an unassigned interface reported a successful scan"
    );
}
