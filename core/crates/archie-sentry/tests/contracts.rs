use archie_sentry::contracts::*;
use archie_sentry::registry::*;
use archie_types::{Timestamp, TraceId};

fn intent() -> Intent {
    Intent {
        schema_version: 1,
        intent_id: "intent-1".into(),
        operation_id: 1,
        trace_id: TraceId::new(),
        resource_id: "tv-living".into(),
        command: Command::MediaPause,
        created_at: Timestamp(1000),
        deadline_at: Timestamp(1400),
        max_observation_age_ms: 100,
        verify: true,
    }
}
fn resource() -> ResourceDescriptor {
    ResourceDescriptor {
        resource_id: "tv-living".into(),
        edge_id: "edge-home".into(),
        adapter_id: "ha-home".into(),
        paired: true,
        capabilities: vec![CommandKind::MediaPause],
    }
}
fn observation() -> Observation {
    Observation {
        resource_id: "tv-living".into(),
        adapter_id: "ha-home".into(),
        evidence_id: "measurement-1".into(),
        observed_at: Timestamp(1200),
        origin: ObservationOrigin::Measurement,
        state: ResourceState::Media { playing: false },
    }
}

#[test]
fn intent_bounds_reject_unsafe_or_unversioned_input() {
    let valid = intent();
    assert!(valid.validate().is_ok());
    let mut cases = vec![];
    let mut bad = valid.clone();
    bad.schema_version = 2;
    cases.push(bad);
    let mut bad = valid.clone();
    bad.operation_id = 0;
    cases.push(bad);
    let mut bad = valid.clone();
    bad.resource_id = "http://other".into();
    cases.push(bad);
    let mut bad = valid.clone();
    bad.deadline_at = Timestamp(1000);
    cases.push(bad);
    let mut bad = valid.clone();
    bad.deadline_at = Timestamp(31001);
    cases.push(bad);
    let mut bad = valid.clone();
    bad.max_observation_age_ms = 0;
    cases.push(bad);
    let mut bad = valid.clone();
    bad.verify = false;
    cases.push(bad);
    let mut bad = valid.clone();
    bad.command = Command::LightSetBrightness { percent: 101 };
    cases.push(bad);
    let mut bad = valid.clone();
    bad.command = Command::ClimateSetTemperature {
        milli_celsius: 35001,
    };
    cases.push(bad);
    for bad in cases {
        assert!(bad.validate().is_err(), "{bad:?}");
    }
}

#[test]
fn unknown_wire_fields_are_rejected() {
    let mut value = serde_json::to_value(intent()).unwrap();
    value["authority"] = serde_json::json!("operator-approved");
    assert!(serde_json::from_value::<Intent>(value).is_err());
}

#[test]
fn digest_binds_target_payload_deadline_and_operation_id() {
    let first = intent();
    let hash = first.digest().unwrap();
    assert_eq!(hash.len(), 64);
    for mut second in [first.clone(), first.clone(), first.clone(), first.clone()] {
        second.command = Command::MediaPlay;
        assert_ne!(hash, second.digest().unwrap());
    }
    let mut second = first.clone();
    second.resource_id = "another-tv".into();
    assert_ne!(hash, second.digest().unwrap());
    let mut second = first.clone();
    second.operation_id = 2;
    assert_ne!(hash, second.digest().unwrap());
    let mut second = first;
    second.deadline_at = Timestamp(1401);
    assert_ne!(hash, second.digest().unwrap());
}

#[test]
fn resolver_refuses_unpaired_undeclared_and_duplicate_resources() {
    let mut registry = Registry::default();
    registry.register(resource()).unwrap();
    assert_eq!(
        registry
            .resolve(&intent(), "edge-home", "ha-home")
            .unwrap()
            .resource_id,
        "tv-living"
    );
    assert!(registry.register(resource()).is_err());
    assert!(registry
        .resolve(&intent(), "other-edge", "ha-home")
        .is_err());
    let mut other = intent();
    other.command = Command::MediaPlay;
    assert!(registry.resolve(&other, "edge-home", "ha-home").is_err());
    let mut registry = Registry::default();
    let mut unpaired = resource();
    unpaired.paired = false;
    registry.register(unpaired).unwrap();
    assert!(registry.resolve(&intent(), "edge-home", "ha-home").is_err());
}

#[test]
fn verification_requires_fresh_measurement_from_exact_target_after_dispatch() {
    let request = intent();
    let value = observation();
    assert!(value
        .validate_after(&request, "ha-home", Timestamp(1100), Timestamp(1250))
        .is_ok());
    assert!(request.command.matches(&value.state));
    let mut bad = value.clone();
    bad.origin = ObservationOrigin::ModelHypothesis;
    assert!(bad
        .validate_after(&request, "ha-home", Timestamp(1100), Timestamp(1250))
        .is_err());
    let mut bad = value.clone();
    bad.resource_id = "another-tv".into();
    assert!(bad
        .validate_after(&request, "ha-home", Timestamp(1100), Timestamp(1250))
        .is_err());
    assert!(value
        .validate_after(&request, "other-adapter", Timestamp(1100), Timestamp(1250))
        .is_err());
    assert!(value
        .validate_after(&request, "ha-home", Timestamp(1201), Timestamp(1250))
        .is_err());
    assert!(value
        .validate_after(&request, "ha-home", Timestamp(1100), Timestamp(1199))
        .is_err());
    assert!(value
        .validate_after(&request, "ha-home", Timestamp(1100), Timestamp(1301))
        .is_err());
    assert!(!request.command.matches(&ResourceState::Unknown));
    assert!(!Command::MediaPlay.matches(&value.state));
}
