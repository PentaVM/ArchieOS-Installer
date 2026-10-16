use archie_core::{config::ArchieConfig, control::ControlRequest};

fn config(entity: &str, actuable: bool) -> String {
    let path = std::env::temp_dir().join("sentry-home-fixture-token");
    let section = if actuable {
        "resources"
    } else {
        "telemetry_resources"
    };
    format!("[sentry.home_assistant]\nendpoint = 'http://127.0.0.1:8123'\ntoken_file = '{}'\nresources = []\n[[sentry.home_assistant.{section}]]\nresource_id = 'room'\nentity = '{entity}'\n", path.display())
        .replace("resources = []\n", if actuable { "" } else { "resources = []\n" })
}
#[test]
fn sensors_are_observation_only_and_old_actuator_configuration_remains_valid() {
    for entity in ["sensor.room", "binary_sensor.door", "device_tracker.phone"] {
        let cfg = ArchieConfig::from_toml(&config(entity, false)).unwrap();
        let ha = cfg.sentry.home_assistant.unwrap();
        assert!(ha.resources.is_empty());
        assert!(ha.telemetry_resources[0].capabilities().is_err());
        assert!(ArchieConfig::from_toml(&config(entity, true)).is_err());
    }
    let cfg = ArchieConfig::from_toml(&config("switch.lamp", true)).unwrap();
    assert!(cfg
        .sentry
        .home_assistant
        .unwrap()
        .telemetry_resources
        .is_empty());
    assert!(ArchieConfig::from_toml(&config("camera.private", false)).is_err());
}
#[test]
fn home_status_is_read_only_and_accepts_no_endpoint_or_token() {
    let request = ControlRequest::SentryHomeStatus {
        resource_id: "room".into(),
    };
    assert!(!request.is_mutating());
    assert_eq!(request.name(), "sentry-home-status");
    for field in ["endpoint", "token"] {
        let mut value = serde_json::to_value(&request).unwrap();
        value[field] = serde_json::json!("unexpected");
        assert!(serde_json::from_value::<ControlRequest>(value).is_err());
    }
}
