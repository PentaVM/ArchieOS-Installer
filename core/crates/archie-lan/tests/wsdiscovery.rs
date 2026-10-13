use archie_lan::wsdiscovery::parse;
use std::net::IpAddr;

fn reply(types: &str, endpoint: &str, relation: &str) -> String {
    format!(
        r#"<s:Envelope xmlns:s="http://www.w3.org/2003/05/soap-envelope"
        xmlns:a="http://schemas.xmlsoap.org/ws/2004/08/addressing"
        xmlns:d="http://schemas.xmlsoap.org/ws/2005/04/discovery"
        xmlns:p="http://schemas.microsoft.com/windows/2006/08/wdp/print"
        xmlns:c="http://www.onvif.org/ver10/network/wsdl">
      <s:Header><a:RelatesTo>{relation}</a:RelatesTo></s:Header>
      <s:Body><d:ProbeMatches><d:ProbeMatch><d:Types>{types}</d:Types>
      <d:XAddrs>{endpoint}</d:XAddrs></d:ProbeMatch></d:ProbeMatches></s:Body>
    </s:Envelope>"#
    )
}

#[test]
fn correlated_wsd_printer_and_onvif_camera_use_namespace_evidence() {
    let from: IpAddr = "10.0.0.42".parse().unwrap();
    let matches = parse(
        from,
        reply(
            "p:PrintDeviceType",
            "http://10.0.0.42:80/wsd",
            "urn:uuid:test",
        )
        .as_bytes(),
        "urn:uuid:test",
    );
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].class.as_deref(), Some("printer"));
    assert_eq!(
        matches[0].endpoint.as_deref(),
        Some("http://10.0.0.42:80/wsd")
    );
    let camera = reply(
        "c:NetworkVideoTransmitter",
        "http://10.0.0.42/onvif",
        "urn:uuid:test",
    )
    .replace(
        "http://schemas.xmlsoap.org/ws/2005/04/discovery",
        "http://docs.oasis-open.org/ws-dd/ns/discovery/2009/01",
    )
    .replace(
        "http://schemas.xmlsoap.org/ws/2004/08/addressing",
        "http://www.w3.org/2005/08/addressing",
    );
    assert_eq!(
        parse(from, camera.as_bytes(), "urn:uuid:test")[0]
            .class
            .as_deref(),
        Some("camera")
    );
}

#[test]
fn foreign_endpoints_and_wrong_correlations_never_become_targets() {
    let from: IpAddr = "10.0.0.42".parse().unwrap();
    for endpoint in [
        "http://8.8.8.8/",
        "http://10.0.0.99/",
        "http://printer.local/",
    ] {
        let result = parse(
            from,
            reply("p:PrintDeviceType", endpoint, "urn:uuid:test").as_bytes(),
            "urn:uuid:test",
        );
        assert_eq!(result[0].endpoint, None);
    }
    assert!(parse(
        from,
        reply("p:PrintDeviceType", "http://10.0.0.42/", "wrong").as_bytes(),
        "urn:uuid:test"
    )
    .is_empty());
    assert!(parse(
        "8.8.8.8".parse().unwrap(),
        reply("p:PrintDeviceType", "http://8.8.8.8/", "urn:uuid:test").as_bytes(),
        "urn:uuid:test"
    )
    .is_empty());
}

#[test]
fn generic_device_unknown_namespace_and_malformed_xml_do_not_invent_classes() {
    let from = "10.0.0.42".parse().unwrap();
    let unknown = reply("p:PrintDeviceType", "http://10.0.0.42/", "urn:uuid:test").replace(
        "http://schemas.microsoft.com/windows/2006/08/wdp/print",
        "urn:untrusted",
    );
    assert_eq!(
        parse(from, unknown.as_bytes(), "urn:uuid:test")[0].class,
        None
    );
    assert!(parse(from, b"<broken", "urn:uuid:test").is_empty());
    assert!(parse(from, &vec![b'x'; 65_537], "urn:uuid:test").is_empty());
}
