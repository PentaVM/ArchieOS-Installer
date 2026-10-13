use archie_lan::dns_sd::{convert, Scope};
use mdns_sd::{InterfaceId, IntoTxtProperties, ResolvedService, ScopedIp, ScopedIpV4};
use std::collections::{HashMap, HashSet};

fn service() -> ResolvedService {
    let interface = InterfaceId {
        name: "lan-test".into(),
        index: 7,
    };
    serde_json::from_value(serde_json::json!({
        "ty_domain": "_ipp._tcp.local.", "sub_ty_domain": null,
        "fullname": "Office._ipp._tcp.local.", "host": "printer.local.", "port": 631,
        "addresses": [ScopedIp::V4(ScopedIpV4::new("10.0.0.42".parse().unwrap(), interface))],
        "txt_properties": HashMap::from([("ty".to_string(),"Observed printer".to_string()),
            ("rp".into(), "ipp/print".into()), ("serial".into(), "private".into())]).into_txt_properties(),
    })).unwrap()
}

#[test]
fn library_advertisements_preserve_service_claims_without_actuator_endpoints() {
    let scope = Scope::new(
        "10.0.0.7".parse().unwrap(),
        "255.255.255.0".parse().unwrap(),
        "lan-test",
        7,
    );
    let devices = convert(&service(), &scope);
    assert_eq!(devices.len(), 1);
    assert_eq!(devices[0].address, "10.0.0.42");
    assert_eq!(devices[0].name, "Office");
    assert_eq!(devices[0].model, "Observed printer");
    assert_eq!(devices[0].class.as_deref(), Some("printer"));
    assert!(!devices[0].state.keys().any(|key| key.contains("endpoint")));
    assert!(!format!("{:?}", devices[0]).contains("private"));
}

#[test]
fn advertised_public_foreign_and_wrong_interface_addresses_are_dropped() {
    let scope = Scope::new(
        "10.0.0.7".parse().unwrap(),
        "255.255.255.0".parse().unwrap(),
        "lan-test",
        7,
    );
    for (ip, index) in [("8.8.8.8", 7), ("10.0.1.42", 7), ("10.0.0.42", 8)] {
        let mut info = service();
        info.addresses = HashSet::from([ScopedIp::V4(ScopedIpV4::new(
            ip.parse().unwrap(),
            InterfaceId {
                name: "other".into(),
                index,
            },
        ))]);
        assert!(
            convert(&info, &scope).is_empty(),
            "accepted {ip} on {index}"
        );
    }
}

#[test]
fn private_ipv6_preserves_interface_scope_and_never_becomes_a_raw_http_target() {
    let scope = Scope::new(
        "10.0.0.7".parse().unwrap(),
        "255.255.255.0".parse().unwrap(),
        "lan-test",
        7,
    );
    let mut info = service();
    info.addresses = HashSet::from([serde_json::from_value::<ScopedIp>(
        serde_json::json!({"V6":{"addr":"fe80::42","scope_id":{"name":"lan-test","index":7}}}),
    )
    .unwrap()]);
    let devices = convert(&info, &scope);
    assert_eq!(devices.len(), 1);
    assert_eq!(devices[0].address, "fe80::42");
    assert_eq!(devices[0].state["mdns.interface_index"], "7");
    assert!(!devices[0].state.keys().any(|key| key.contains("endpoint")));
}
