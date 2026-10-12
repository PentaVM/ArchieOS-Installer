//! Configuration validation for the native IoT adapters (roku, print spools,
//! MQTT, ONVIF telemetry, context feed). Validation-only: no network, no
//! devices, no credential files are touched — constructors are probed with
//! placeholders, exactly like the Home Assistant binding.
use archie_core::config::ArchieConfig;

/// An absolute directory literal valid on the host running the tests.
fn abs_dir(name: &str) -> String {
    if cfg!(windows) {
        format!("C:/archie/{name}")
    } else {
        format!("/etc/archie/{name}")
    }
}

const UUID: &str = "urn:uuid:e3248000-80ce-11db-8000-202b20a4fe3b";

fn full_config() -> String {
    format!(
        "[sentry]\n\
         operator_uids=[1000]\n\
         [[sentry.printers]]\n\
         resource_id='office-printer'\n\
         endpoint='ipp://192.168.1.232/ipp/print'\n\
         expected_uuid='{UUID}'\n\
         spool_dir='{spool}'\n\
         [[sentry.roku]]\n\
         resource_id='living-room-tv'\n\
         endpoint='http://192.168.1.40'\n\
         [sentry.mqtt]\n\
         broker='192.168.1.10:1883'\n\
         username='archie'\n\
         password_ref='env:ARCHIE_MQTT_PASSWORD'\n\
         [sentry.mqtt.resources.porch-light]\n\
         kind='light'\n\
         command_topic='home/porch/set'\n\
         state_topic='home/porch/state'\n\
         payload_on='ON'\n\
         payload_off='OFF'\n\
         brightness_template='{{\"bri\":{{percent}}}}'\n\
         temperature_template='{{celsius}}'\n\
         [[sentry.onvif_cameras]]\n\
         resource_id='gate-camera'\n\
         endpoint='http://192.168.1.20/onvif/device_service'\n\
         [sentry.context_feed]\n\
         watchdog_dir='{watchdog}'\n\
         out_dir='{out}'\n\
         poll_ms=1500\n",
        spool = abs_dir("spool"),
        watchdog = abs_dir("watchdog"),
        out = abs_dir("feed"),
    )
}

#[test]
fn a_full_iot_configuration_is_accepted() {
    let config = ArchieConfig::from_toml(&full_config()).unwrap();
    assert_eq!(config.sentry.roku.len(), 1);
    assert_eq!(
        config.sentry.printers[0].spool_dir.as_deref(),
        Some(abs_dir("spool").as_str())
    );
    assert!(config.sentry.mqtt.is_some());
    assert_eq!(config.sentry.onvif_cameras.len(), 1);
    assert_eq!(config.sentry.context_feed.as_ref().unwrap().poll_ms, 1500);
    // Routing: actuable resources map to their adapters; cameras never do.
    assert_eq!(
        config
            .sentry
            .actuable_route("living-room-tv")
            .map(|(a, _)| a),
        Some("roku")
    );
    assert_eq!(
        config
            .sentry
            .actuable_route("office-printer")
            .map(|(a, _)| a),
        Some("ipp")
    );
    assert_eq!(
        config.sentry.actuable_route("porch-light").map(|(a, _)| a),
        Some("mqtt")
    );
    assert!(config.sentry.actuable_route("gate-camera").is_none());
}

#[test]
fn the_new_sections_default_to_absent_so_old_configs_keep_working() {
    let config = ArchieConfig::default();
    assert!(config.sentry.roku.is_empty());
    assert!(config.sentry.mqtt.is_none());
    assert!(config.sentry.onvif_cameras.is_empty());
    assert!(config.sentry.context_feed.is_none());
    assert!(config.sentry.printers.is_empty());
    // A query-only printer (no spool) is valid and never actuable.
    let config = ArchieConfig::from_toml(&format!(
        "[[sentry.printers]]\nresource_id='office-printer'\nendpoint='ipp://192.168.1.232/ipp/print'\nexpected_uuid='{UUID}'\n"
    ))
    .unwrap();
    assert!(config.sentry.actuable_route("office-printer").is_none());
}

#[test]
fn unknown_fields_are_rejected_in_every_new_section() {
    let full = full_config();
    for bad in [
        full.replace("poll_ms=1500", "poll_ms=1500\nbogus_key=1"),
        full.replace(
            "endpoint='http://192.168.1.40'",
            "endpoint='http://192.168.1.40'\napp='12'",
        ),
        full.replace(
            "broker='192.168.1.10:1883'",
            "broker='192.168.1.10:1883'\npassword='hunter2'",
        ),
        full.replace(
            "endpoint='http://192.168.1.20/onvif/device_service'",
            "endpoint='http://192.168.1.20/onvif/device_service'\npassword='hunter2'",
        ),
        full.replace("kind='light'", "kind='dimmer'"),
    ] {
        assert!(ArchieConfig::from_toml(&bad).is_err(), "{bad}");
    }
}

#[test]
fn resource_ids_are_unique_across_all_sections() {
    let full = full_config();
    for bad in [
        // roku duplicates a printer id
        full.replace(
            "resource_id='living-room-tv'",
            "resource_id='office-printer'",
        ),
        // mqtt duplicates a roku id
        full.replace(
            "[sentry.mqtt.resources.porch-light]",
            "[sentry.mqtt.resources.living-room-tv]",
        ),
        // a camera duplicates an mqtt resource id
        full.replace("resource_id='gate-camera'", "resource_id='porch-light'"),
    ] {
        assert!(ArchieConfig::from_toml(&bad).is_err(), "{bad}");
    }
}

#[test]
fn unsafe_endpoints_spools_and_brokers_are_rejected() {
    let full = full_config();
    let bad_configs = vec![
        // relative spool directory
        full.replace(&abs_dir("spool"), "relative/spool"),
        // roku endpoint off-LAN / with credentials / with a path
        full.replace("http://192.168.1.40", "http://8.8.8.8"),
        full.replace("http://192.168.1.40", "http://user@192.168.1.40"),
        full.replace("http://192.168.1.40", "http://192.168.1.40/query"),
        // broker: hostname, public IP, port zero
        full.replace("192.168.1.10:1883", "mqtt.lan:1883"),
        full.replace("192.168.1.10:1883", "8.8.8.8:1883"),
        full.replace("192.168.1.10:1883", "192.168.1.10:0"),
        // password reference without a username (and vice versa)
        full.replace("username='archie'\n", ""),
        full.replace("password_ref='env:ARCHIE_MQTT_PASSWORD'\n", ""),
        // malformed secret references
        full.replace("env:ARCHIE_MQTT_PASSWORD", "env:1BAD"),
        full.replace("env:ARCHIE_MQTT_PASSWORD", "relative/token"),
        // poll cadence outside the bounds
        full.replace("poll_ms=1500", "poll_ms=100"),
        full.replace("poll_ms=1500", "poll_ms=120000"),
        // relative feed directories
        full.replace(&abs_dir("watchdog"), "watchdog"),
        // camera: plaintext HTTP with credentials off-loopback
        full.replace(
            "endpoint='http://192.168.1.20/onvif/device_service'",
            "endpoint='http://192.168.1.20/onvif/device_service'\nusername='admin'\npassword_ref='env:CAM_PASSWORD'",
        ),
        // malformed resource ids
        full.replace("resource_id='living-room-tv'", "resource_id='bad/resource'"),
        full.replace("[sentry.mqtt.resources.porch-light]", "[sentry.mqtt.resources.'bad/resource']"),
    ];
    for bad in bad_configs {
        assert!(ArchieConfig::from_toml(&bad).is_err(), "{bad}");
    }
}

#[test]
fn roku_bindings_are_probed_through_the_adapter_constructor() {
    // An empty roku vector is fine; an entry with a missing endpoint is not TOML-valid.
    assert!(ArchieConfig::from_toml("[sentry]\n").is_ok());
    assert!(ArchieConfig::from_toml("[[sentry.roku]]\nresource_id='tv'\n").is_err());
    // Duplicate ids inside the roku section alone are rejected.
    let dup = "[[sentry.roku]]\nresource_id='tv'\nendpoint='http://192.168.1.40'\n[[sentry.roku]]\nresource_id='tv'\nendpoint='http://192.168.1.41'\n";
    assert!(ArchieConfig::from_toml(dup).is_err());
}
