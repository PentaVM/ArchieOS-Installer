use archie_core::{config::ArchieConfig, control::ControlRequest};

const UUID: &str = "urn:uuid:e3248000-80ce-11db-8000-202b20a4fe3b";

fn binding(endpoint: &str, uuid: &str) -> String {
    format!("[[sentry.printers]]\nresource_id = 'office-printer'\nendpoint = '{endpoint}'\nexpected_uuid = '{uuid}'\n")
}

#[test]
fn printer_binding_is_explicit_and_control_request_has_no_endpoint() {
    let config = ArchieConfig::from_toml(&binding("ipp://192.168.1.232/ipp/print", UUID)).unwrap();
    assert_eq!(config.sentry.printers.len(), 1);
    assert_eq!(config.sentry.printers[0].resource_id, "office-printer");
    let request = ControlRequest::SentryPrinter {
        resource_id: "office-printer".into(),
    };
    assert!(!request.is_mutating());
    assert_eq!(request.name(), "sentry-printer");
    let value = serde_json::to_value(request).unwrap();
    assert_eq!(value["resource_id"], "office-printer");
    assert!(value.get("endpoint").is_none());
    assert!(serde_json::from_value::<ControlRequest>(serde_json::json!({
        "request": "sentry_printer", "resource_id": "office-printer", "endpoint": "http://127.0.0.1/anything"
    })).is_err());
}

#[test]
fn printer_config_rejects_ambiguous_or_unsafe_bindings() {
    for endpoint in [
        "ipp://8.8.8.8/ipp/print",
        "ipp://localhost/ipp/print",
        "ipp://secret@127.0.0.1/ipp/print",
        "ipp://127.0.0.1/ipp/print?token=secret",
    ] {
        assert!(ArchieConfig::from_toml(&binding(endpoint, UUID)).is_err());
    }
    for uuid in [
        "",
        "not-a-uuid",
        "urn:uuid:e3248000-80ce-11db-8000-202b20a4fe3B",
    ] {
        assert!(ArchieConfig::from_toml(&binding("ipp://127.0.0.1/ipp/print", uuid)).is_err());
    }
    let entry = binding("ipp://127.0.0.1/ipp/print", UUID);
    assert!(ArchieConfig::from_toml(&format!("{entry}{entry}")).is_err());
    assert!(ArchieConfig::from_toml(&entry.replace("office-printer", "bad/resource")).is_err());
    let too_many = (0..65)
        .map(|n| entry.replace("office-printer", &format!("printer-{n}")))
        .collect::<String>();
    assert!(ArchieConfig::from_toml(&too_many).is_err());
}
