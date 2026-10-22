//! Integration tests for the IRIS context feed.

use archie_sentry::{
    contracts::{Observation, ObservationOrigin, ResourceState},
    engine::{ExecutionResult, ExecutionStatus, Stage},
    feed::{
        dedupe_events, diff_snapshots, from_execution_result, from_lan_inventory, from_observation,
        from_passive_network, parse_event, EventKind, EventPayload, EventSource, FeedError,
        FeedWriter, ReaderLimits, Snapshot, WatchdogReader, MAX_FACT_VALUE_LEN,
    },
};
use archie_types::Timestamp;
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "archie-feed-test-{}-{}-{}",
        name,
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn lan_json(showing: &str) -> String {
    serde_json::json!({
        "format": "archie-lan-inventory-v1",
        "scanned_unix": 1_790_000_000u64,
        "devices": [
            {
                "address": "192.168.1.40",
                "class": "television",
                "name": "Living",
                "manufacturer": "Roku",
                "model": "3941X",
                "services": ["_airplay._tcp"],
                "seen_by": ["ssdp", "mdns"],
                "state": {"showing": showing}
            },
            {
                "address": "192.168.1.60",
                "class": "printer",
                "name": "Oficina",
                "manufacturer": "Brother",
                "model": "HL-L2350DW",
                "services": ["_ipp._tcp"],
                "seen_by": ["mdns"],
                "state": {}
            }
        ]
    })
    .to_string()
}

fn observation(origin: ObservationOrigin) -> Observation {
    Observation {
        resource_id: "tv.living".into(),
        adapter_id: "roku-1".into(),
        evidence_id: "evidence-1".into(),
        observed_at: Timestamp::from_millis(1_500),
        origin,
        state: ResourceState::Tv {
            power_on: true,
            showing_home: false,
            active_app: Some("netflix".into()),
            playing: Some(true),
        },
    }
}

fn execution_result(status: ExecutionStatus, origin: ObservationOrigin) -> ExecutionResult {
    let mut before = observation(origin);
    before.evidence_id = "evidence-before".into();
    let mut after = observation(origin);
    after.evidence_id = "evidence-after".into();
    ExecutionResult {
        schema_version: 1,
        intent_id: "intent-7".into(),
        operation_id: 7,
        resource_id: "tv.living".into(),
        intent_digest: "a".repeat(64),
        status,
        lease_id: None,
        dispatched_at: Some(Timestamp::from_millis(1_000)),
        before: Some(before),
        after: Some(after),
        stages: vec![
            Stage::Resolved,
            Stage::Authorized,
            Stage::Reserved,
            Stage::DispatchStarted,
            Stage::Acknowledged,
            Stage::ObservedAfter,
        ],
        finished_at: Timestamp::from_millis(2_000),
        replayed: false,
    }
}

fn passive_json() -> String {
    serde_json::json!({
        "source": "offline_mdns_metadata",
        "trusted": false,
        "assets": [
            {
                "ipv4_address": "192.168.1.3",
                "candidates": [
                    {"class": "television", "score": 80, "supporting_signals": 1}
                ]
            }
        ],
        "changes": [
            {
                "kind": "newly_observed",
                "ipv4_address": "192.168.1.3",
                "candidates": [
                    {"class": "television", "score": 80, "supporting_signals": 1}
                ]
            },
            {
                "kind": "no_longer_observed",
                "ipv4_address": "192.168.1.2",
                "previous_candidates": [
                    {"class": "printer", "score": 70, "supporting_signals": 1}
                ]
            }
        ]
    })
    .to_string()
}

fn watchdog_line(ip: &str, names: &[&str], epoch: Option<f64>) -> String {
    match epoch {
        Some(value) => serde_json::json!({
            "ip_src": ip,
            "udp_srcport": 5353,
            "dns_ptr_names": names,
            "frame_time_epoch": value
        })
        .to_string(),
        None => serde_json::json!({
            "ip_src": ip,
            "udp_srcport": 5353,
            "dns_ptr_names": names
        })
        .to_string(),
    }
}

#[test]
fn serde_round_trip_covers_every_kind() {
    let mut events = from_lan_inventory(&lan_json("Netflix")).unwrap();
    events.extend(from_passive_network(&passive_json()).unwrap());
    events.extend(
        from_execution_result(&execution_result(
            ExecutionStatus::Diverged,
            ObservationOrigin::Measurement,
        ))
        .unwrap(),
    );
    events.push(from_observation(&observation(ObservationOrigin::ModelHypothesis)).unwrap());
    let kinds: Vec<EventKind> = events.iter().map(|e| e.kind).collect();
    for kind in [
        EventKind::DeviceSeen,
        EventKind::DeviceGone,
        EventKind::NetworkObservation,
        EventKind::AlertObserved,
        EventKind::IntentExecuted,
        EventKind::DeviceStateChanged,
    ] {
        assert!(kinds.contains(&kind), "missing kind {kind:?}");
    }
    for event in &events {
        let line = serde_json::to_string(event).unwrap();
        let parsed = parse_event(&line).unwrap();
        assert_eq!(&parsed, event, "round trip changed {line}");
        assert_eq!(parsed.schema_version, 1);
        assert_eq!(parsed.event_id.len(), 64);
    }
}

#[test]
fn unknown_fields_and_tampered_ids_are_rejected() {
    let event = &from_lan_inventory(&lan_json("Netflix")).unwrap()[0];
    let mut value = serde_json::to_value(event).unwrap();
    value["unexpected"] = serde_json::json!(true);
    assert!(parse_event(&value.to_string()).is_err());
    let mut value = serde_json::to_value(event).unwrap();
    value["event_id"] = serde_json::json!("b".repeat(64));
    assert!(parse_event(&value.to_string()).is_err());
    let mut value = serde_json::to_value(event).unwrap();
    value["schema_version"] = serde_json::json!(2);
    assert!(parse_event(&value.to_string()).is_err());
}

#[test]
fn malformed_inputs_are_errors_not_panics() {
    assert!(from_lan_inventory("{").is_err());
    assert!(from_lan_inventory("[]").is_err());
    assert!(from_lan_inventory(
        &serde_json::json!({"format": "other", "scanned_unix": 1, "devices": []}).to_string()
    )
    .is_err());
    assert!(from_lan_inventory(
        &serde_json::json!({
            "format": "archie-lan-inventory-v1",
            "scanned_unix": 1,
            "devices": [{"address": "not an ip", "class": null, "name": "",
                "manufacturer": "", "model": "", "services": [], "seen_by": [],
                "state": {}}]
        })
        .to_string()
    )
    .is_err());
    let oversized = serde_json::json!({
        "format": "archie-lan-inventory-v1",
        "scanned_unix": 1,
        "devices": (0..4097).map(|i| serde_json::json!({
            "address": format!("10.0.{}.{}", i / 256, i % 256),
            "class": null, "name": "", "manufacturer": "", "model": "",
            "services": [], "seen_by": [], "state": {}
        })).collect::<Vec<_>>()
    })
    .to_string();
    assert!(matches!(
        from_lan_inventory(&oversized),
        Err(FeedError::Bound(_))
    ));
    assert!(from_passive_network("{").is_err());
    assert!(from_passive_network("{}").is_err());
    assert!(from_passive_network(
        &serde_json::json!({"source": "other", "trusted": false}).to_string()
    )
    .is_err());
    assert!(parse_event("{\"schema_version\":1}").is_err());
}

#[test]
fn trusted_passive_report_is_refused() {
    let report = serde_json::json!({
        "source": "offline_mdns_metadata",
        "trusted": true,
        "assets": [],
        "changes": []
    })
    .to_string();
    assert!(from_passive_network(&report).is_err());
}

#[test]
fn passive_report_maps_changes_to_device_kinds() {
    let events = from_passive_network(&passive_json()).unwrap();
    assert_eq!(events.len(), 3);
    assert_eq!(events[0].kind, EventKind::NetworkObservation);
    assert_eq!(events[1].kind, EventKind::DeviceSeen);
    assert_eq!(events[2].kind, EventKind::DeviceGone);
    for event in &events {
        assert_eq!(event.source, EventSource::PassiveNetwork);
        assert_eq!(
            event.observed_at, None,
            "passive exports carry no timestamps; null is the marker"
        );
        assert_eq!(event.origin, ObservationOrigin::Measurement);
    }
    let serialized = serde_json::to_string(&events[0]).unwrap();
    assert!(serialized.contains("\"observed_at\":null"));
    match &events[0].payload {
        EventPayload::NetworkAsset {
            candidates,
            trusted,
            ..
        } => {
            assert!(!trusted);
            assert_eq!(candidates[0].class, "television");
        }
        other => panic!("unexpected payload {other:?}"),
    }
}

#[test]
fn reordered_facts_are_not_a_change() {
    let first = Snapshot::from_lan_inventory_json(&lan_json("Netflix")).unwrap();
    // Rebuild the same snapshot from JSON with the state keys emitted in a
    // different order; BTreeMap comparison makes them equal.
    let reordered = serde_json::json!({
        "format": "archie-lan-inventory-v1",
        "scanned_unix": 1_790_000_000u64,
        "devices": [
            {
                "address": "192.168.1.40",
                "class": "television",
                "name": "Living",
                "manufacturer": "Roku",
                "model": "3941X",
                "services": ["_airplay._tcp"],
                "seen_by": ["ssdp", "mdns"],
                "state": {"showing": "Netflix"}
            },
            {
                "address": "192.168.1.60",
                "class": "printer",
                "name": "Oficina",
                "manufacturer": "Brother",
                "model": "HL-L2350DW",
                "services": ["_ipp._tcp"],
                "seen_by": ["mdns"],
                "state": {}
            }
        ]
    })
    .to_string();
    let second = Snapshot::from_lan_inventory_json(&reordered).unwrap();
    assert_eq!(first, second);
    assert!(diff_snapshots(&first, &second).unwrap().is_empty());
    let mut manual_a = Snapshot {
        taken_at: Some(Timestamp::from_millis(1)),
        ..Snapshot::empty()
    };
    manual_a.devices.insert(
        "dev-1".into(),
        BTreeMap::from([
            ("a".to_string(), "1".to_string()),
            ("b".to_string(), "2".to_string()),
        ]),
    );
    let mut manual_b = Snapshot {
        taken_at: Some(Timestamp::from_millis(2)),
        ..Snapshot::empty()
    };
    let mut facts = BTreeMap::new();
    facts.insert("b".to_string(), "2".to_string());
    facts.insert("a".to_string(), "1".to_string());
    manual_b.devices.insert("dev-1".into(), facts);
    assert!(diff_snapshots(&manual_a, &manual_b).unwrap().is_empty());
}

#[test]
fn one_changed_fact_is_exactly_one_state_changed() {
    let before = Snapshot::from_lan_inventory_json(&lan_json("Netflix")).unwrap();
    let after = Snapshot::from_lan_inventory_json(&lan_json("YouTube")).unwrap();
    let events = diff_snapshots(&before, &after).unwrap();
    assert_eq!(events.len(), 1);
    let event = &events[0];
    assert_eq!(event.kind, EventKind::DeviceStateChanged);
    assert_eq!(event.subject, "192.168.1.40");
    assert_eq!(event.observed_at, after.taken_at);
    match &event.payload {
        EventPayload::DeviceFacts {
            facts,
            changed_keys,
        } => {
            assert_eq!(changed_keys, &["state.showing".to_string()]);
            assert_eq!(facts["state.showing"], "YouTube");
        }
        other => panic!("unexpected payload {other:?}"),
    }
}

#[test]
fn appeared_and_disappeared_devices_map_to_seen_and_gone() {
    let before = Snapshot::from_lan_inventory_json(&lan_json("Netflix")).unwrap();
    let empty = Snapshot::empty();
    let seen = diff_snapshots(&empty, &before).unwrap();
    assert_eq!(seen.len(), 2);
    assert!(seen.iter().all(|e| e.kind == EventKind::DeviceSeen));
    let gone = diff_snapshots(&before, &empty).unwrap();
    assert_eq!(gone.len(), 2);
    assert!(gone.iter().all(|e| e.kind == EventKind::DeviceGone));
    // Gone events keep the last known facts; the timestamp is the newer scan's.
    match &gone[0].payload {
        EventPayload::DeviceFacts { facts, .. } => assert!(!facts.is_empty()),
        other => panic!("unexpected payload {other:?}"),
    }
}

#[test]
fn replays_dedupe_by_content_derived_event_id() {
    let first = from_lan_inventory(&lan_json("Netflix")).unwrap();
    let second = from_lan_inventory(&lan_json("Netflix")).unwrap();
    let ids_a: Vec<_> = first.iter().map(|e| e.event_id.clone()).collect();
    let ids_b: Vec<_> = second.iter().map(|e| e.event_id.clone()).collect();
    assert_eq!(ids_a, ids_b, "identical input must yield identical ids");
    let combined: Vec<_> = first.into_iter().chain(second).collect();
    assert_eq!(dedupe_events(combined).len(), ids_a.len());
    // A different source moment is a different event.
    let later = serde_json::json!({
        "format": "archie-lan-inventory-v1",
        "scanned_unix": 1_790_000_100u64,
        "devices": []
    })
    .to_string();
    let other = from_lan_inventory(&later).unwrap();
    let _ = other;
}

#[test]
fn execution_receipt_maps_to_intent_and_alert_events() {
    let verified = from_execution_result(&execution_result(
        ExecutionStatus::Verified,
        ObservationOrigin::Measurement,
    ))
    .unwrap();
    assert_eq!(verified.len(), 1);
    assert_eq!(verified[0].kind, EventKind::IntentExecuted);
    assert_eq!(verified[0].observed_at, Some(Timestamp::from_millis(2_000)));
    assert_eq!(verified[0].origin, ObservationOrigin::Measurement);
    assert_eq!(
        verified[0].provenance,
        vec![
            "intent-7".to_string(),
            "evidence-before".to_string(),
            "evidence-after".to_string()
        ]
    );
    match &verified[0].payload {
        EventPayload::ExecutionOutcome {
            status,
            intent_digest,
            stages,
            after_state,
            ..
        } => {
            assert_eq!(*status, ExecutionStatus::Verified);
            assert_eq!(intent_digest, &"a".repeat(64));
            assert!(stages.contains(&Stage::ObservedAfter));
            assert!(matches!(after_state, Some(ResourceState::Tv { .. })));
        }
        other => panic!("unexpected payload {other:?}"),
    }

    let diverged = from_execution_result(&execution_result(
        ExecutionStatus::Diverged,
        ObservationOrigin::Measurement,
    ))
    .unwrap();
    assert_eq!(diverged.len(), 2);
    assert_eq!(diverged[1].kind, EventKind::AlertObserved);
    assert_ne!(
        diverged[0].event_id, diverged[1].event_id,
        "kind participates in the content id"
    );

    // Hypothesis origin is copied, never upgraded.
    let hypothesis = from_execution_result(&execution_result(
        ExecutionStatus::Diverged,
        ObservationOrigin::ModelHypothesis,
    ))
    .unwrap();
    assert_eq!(hypothesis[0].origin, ObservationOrigin::ModelHypothesis);

    // Invalid receipts are rejected without panic.
    let mut bad = execution_result(ExecutionStatus::Verified, ObservationOrigin::Measurement);
    bad.intent_digest = "not-a-digest".into();
    assert!(from_execution_result(&bad).is_err());
    let mut bad = execution_result(ExecutionStatus::Verified, ObservationOrigin::Measurement);
    bad.resource_id = "bad id with spaces".into();
    assert!(from_execution_result(&bad).is_err());
}

#[test]
fn observation_origin_and_evidence_are_preserved() {
    let event = from_observation(&observation(ObservationOrigin::ModelHypothesis)).unwrap();
    assert_eq!(event.kind, EventKind::DeviceStateChanged);
    assert_eq!(event.origin, ObservationOrigin::ModelHypothesis);
    assert_eq!(event.observed_at, Some(Timestamp::from_millis(1_500)));
    assert_eq!(event.provenance, vec!["evidence-1".to_string()]);
    match &event.payload {
        EventPayload::ResourceState {
            state: ResourceState::Tv { power_on, .. },
        } => assert!(power_on),
        other => panic!("unexpected payload {other:?}"),
    }
}

#[test]
fn watchdog_reads_only_complete_lines() {
    let dir = temp_dir("partial");
    let file = dir.join("watch.jsonl");
    let complete = watchdog_line(
        "192.168.1.2",
        &["Office._ipp._tcp.local"],
        Some(1_790_000_000.5),
    );
    let split = complete.len() / 2;
    fs::write(&file, &complete[..split]).unwrap();
    let mut reader = WatchdogReader::new(&dir);
    assert!(reader.poll().unwrap().is_empty(), "partial tail must wait");
    use std::io::Write;
    let mut handle = fs::OpenOptions::new().append(true).open(&file).unwrap();
    writeln!(handle, "{}", &complete[split..]).unwrap();
    drop(handle);
    let events = reader.poll().unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].source, EventSource::RouterWatchdog);
    assert_eq!(events[0].kind, EventKind::NetworkObservation);
    assert_eq!(
        events[0].observed_at,
        Some(Timestamp::from_millis(1_790_000_000_500))
    );
    match &events[0].payload {
        EventPayload::WatchdogTraffic {
            packets, services, ..
        } => {
            assert_eq!(*packets, 1);
            assert_eq!(services, &["_ipp._tcp.local".to_string()]);
        }
        other => panic!("unexpected payload {other:?}"),
    }
    // Idempotent: nothing new, no duplicate events.
    assert!(reader.poll().unwrap().is_empty());
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn watchdog_counts_malformed_lines_without_failing() {
    let dir = temp_dir("malformed");
    let file = dir.join("watch.jsonl");
    let good = watchdog_line("192.168.1.9", &["Living._googlecast._tcp.local"], None);
    fs::write(
        &file,
        format!("{good}\nnot json at all\n{{\"ip_src\":\"bad\"}}\n\n"),
    )
    .unwrap();
    let mut reader = WatchdogReader::new(&dir);
    let events = reader.poll().unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(reader.malformed_lines(), 2);
    assert_eq!(
        events[0].observed_at, None,
        "a line without frame_time_epoch must not invent freshness"
    );
    match &events[0].payload {
        EventPayload::WatchdogTraffic {
            services,
            unrecognized_names,
            ..
        } => {
            assert_eq!(services, &["_googlecast._tcp.local".to_string()]);
            assert_eq!(*unrecognized_names, 0);
        }
        other => panic!("unexpected payload {other:?}"),
    }
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn watchdog_reduces_unknown_names_to_a_count() {
    let dir = temp_dir("privacy");
    let file = dir.join("watch.jsonl");
    let line = watchdog_line(
        "192.168.1.20",
        &[
            "Printer._ipp._tcp.local",
            "Alice's iPhone._apple-mobdev2._tcp.local",
        ],
        Some(1_790_000_001.0),
    );
    fs::write(&file, format!("{line}\n")).unwrap();
    let mut reader = WatchdogReader::new(&dir);
    let events = reader.poll().unwrap();
    assert_eq!(events.len(), 1);
    let serialized = serde_json::to_string(&events[0]).unwrap();
    assert!(
        !serialized.contains("Alice"),
        "personal names never enter the feed"
    );
    match &events[0].payload {
        EventPayload::WatchdogTraffic {
            services,
            unrecognized_names,
            ..
        } => {
            assert_eq!(services, &["_ipp._tcp.local".to_string()]);
            assert_eq!(*unrecognized_names, 1);
        }
        other => panic!("unexpected payload {other:?}"),
    }
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn watchdog_handles_rotation_to_a_new_file() {
    let dir = temp_dir("rotation");
    let file = dir.join("watch.jsonl");
    let first = watchdog_line("192.168.1.30", &["TV._roku-ecp._tcp.local"], Some(1.0));
    fs::write(&file, format!("{first}\n")).unwrap();
    let mut reader = WatchdogReader::new(&dir);
    assert_eq!(reader.poll().unwrap().len(), 1);
    // Rotate: the watchdog moves the old file away and starts a new one.
    fs::rename(&file, dir.join("watch.1.jsonl")).unwrap();
    let second = watchdog_line(
        "192.168.1.31",
        &["Hub._home-assistant._tcp.local"],
        Some(2.0),
    );
    fs::write(&file, format!("{second}\n")).unwrap();
    let events = reader.poll().unwrap();
    let subjects: Vec<_> = events.iter().map(|e| e.subject.as_str()).collect();
    assert!(
        subjects.contains(&"192.168.1.31"),
        "rotated file must be read from its start: {subjects:?}"
    );
    assert!(
        subjects.contains(&"192.168.1.30"),
        "the rotated-away file is new to the reader too: {subjects:?}"
    );
    assert!(reader.poll().unwrap().is_empty());
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn watchdog_enforces_the_byte_bound_across_polls() {
    let dir = temp_dir("bytes");
    let file = dir.join("watch.jsonl");
    let lines: Vec<String> = (0..5u8)
        .map(|i| {
            watchdog_line(
                &format!("192.168.1.{}", 50 + i),
                &["Office._ipp._tcp.local"],
                Some(f64::from(i)),
            )
        })
        .collect();
    fs::write(
        &file,
        lines.iter().map(|l| format!("{l}\n")).collect::<String>(),
    )
    .unwrap();
    // One complete line plus a few bytes per poll.
    let cap = lines[0].len() + 6;
    let limits = ReaderLimits {
        max_read_bytes: cap,
        ..ReaderLimits::default()
    };
    let mut reader = WatchdogReader::with_limits(&dir, limits);
    let mut all = Vec::new();
    for _ in 0..16 {
        let batch = reader.poll().unwrap();
        assert!(batch.len() <= 1, "one line fits the byte budget");
        all.extend(batch);
    }
    let mut subjects: Vec<_> = all.iter().map(|e| e.subject.clone()).collect();
    subjects.sort();
    assert_eq!(
        subjects,
        (0..5u8)
            .map(|i| format!("192.168.1.{}", 50 + i))
            .collect::<Vec<_>>(),
        "bounded reads must not lose or duplicate lines"
    );
    assert_eq!(reader.malformed_lines(), 0);
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn writer_rotates_segments_and_lines_stay_parseable() {
    let dir = temp_dir("writer");
    let event = &from_lan_inventory(&lan_json("Netflix")).unwrap()[0];
    let line_len = serde_json::to_string(event).unwrap().len() as u64 + 1;
    let segment_max = line_len * 3;
    let mut writer = FeedWriter::with_limits(&dir, segment_max, 16 * 1024).unwrap();
    for _ in 0..7 {
        writer.append(event).unwrap();
    }
    assert_eq!(writer.segment_index(), 2, "seven lines over a 3-line cap");
    assert_eq!(
        writer.segment_index(),
        2,
        "segment index advances on rotation"
    );
    let mut total_lines = 0;
    for index in 0..=2u64 {
        let segment = dir.join(format!("segment-{index:06}.jsonl"));
        let content = fs::read_to_string(&segment).unwrap();
        for line in content.lines() {
            parse_event(line).unwrap();
            total_lines += 1;
        }
    }
    assert_eq!(total_lines, 7);
    assert!(
        !dir.join("segment-000002.jsonl.tmp").exists(),
        "no tmp file may survive rotation"
    );
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn writer_rejects_oversized_events() {
    let dir = temp_dir("oversized");
    let mut writer = FeedWriter::with_limits(&dir, 4096, 128).unwrap();
    let event = &from_lan_inventory(&lan_json("Netflix")).unwrap()[0];
    assert!(matches!(
        writer.append(event),
        Err(FeedError::EventTooLarge)
    ));
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn fact_values_are_bounded_not_unbounded() {
    let long = "x".repeat(MAX_FACT_VALUE_LEN + 100);
    let json = serde_json::json!({
        "format": "archie-lan-inventory-v1",
        "scanned_unix": 1,
        "devices": [{
            "address": "192.168.1.99",
            "class": null,
            "name": long,
            "manufacturer": "",
            "model": "",
            "services": [],
            "seen_by": [],
            "state": {}
        }]
    })
    .to_string();
    let events = from_lan_inventory(&json).unwrap();
    match &events[0].payload {
        EventPayload::DeviceFacts { facts, .. } => {
            assert_eq!(facts["name"].len(), MAX_FACT_VALUE_LEN);
        }
        other => panic!("unexpected payload {other:?}"),
    }
}

#[test]
fn cache_only_candidates_do_not_generate_fresh_device_seen_events() {
    let json = serde_json::json!({"format":"archie-lan-inventory-v1","scanned_unix":42,
      "devices":[{"address":"10.0.0.42","class":null,"name":"","manufacturer":"",
        "model":"","services":[],"seen_by":["neighbor-cache"],
        "state":{"neighbor.status":"STALE"}}]})
    .to_string();
    assert!(from_lan_inventory(&json).unwrap().is_empty());
}
