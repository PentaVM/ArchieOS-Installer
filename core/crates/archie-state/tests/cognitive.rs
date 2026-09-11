use archie_memory::cognitive::*;
use archie_state::StateStore;

fn focus(revision: u64, time: i64) -> CognitiveEvent {
    CognitiveEvent {
        schema_version: 1,
        event_id: format!("event:focus:{revision}"),
        recorded_at_ms: time,
        payload: CognitivePayload::FocusUpdated(FocusRecord {
            focus_id: "focus:robot".into(),
            revision,
            objective_ref: "vault:objective".into(),
            parent_focus_id: None,
            suspended: false,
            half_life_ms: 100,
            updated_at_ms: time,
            activations: vec![FocusActivation {
                node_id: "project:robot".into(),
                strength: 1.0,
                activated_at_ms: time,
                pinned: true,
            }],
        }),
    }
}
fn sample(id: &str, time: i64) -> CognitiveEvent {
    CognitiveEvent {
        schema_version: 1,
        event_id: id.into(),
        recorded_at_ms: time,
        payload: CognitivePayload::Training(TrainingEvent {
            event_id: id.into(),
            subject_id: "process:simulation".into(),
            snapshot_id: "snapshot:1".into(),
            focus_id: "focus:robot".into(),
            evidence_ref: "audit:outcome".into(),
            signal: LearningSignal::TaskOutcome,
            observed: UtilityObservation {
                task_success: Some(false),
                latency_ms: Some(42.0),
                ..Default::default()
            },
            recorded_at_ms: time,
        }),
    }
}
#[test]
#[ignore = "needs a PostgreSQL cluster: run `node tests/with-postgres.mjs --core` from tools/engineering-dashboard"]
fn focus_survives_restart_and_suspension_does_not_rewrite_history() {
    let path = std::env::temp_dir().join(format!(
        "archie-cognitive-{}.json",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    {
        let store = StateStore::open(&path).unwrap();
        let first = focus(1, 100);
        assert_eq!(
            store.record_cognitive_event(&first).unwrap(),
            store.record_cognitive_event(&first).unwrap()
        );
        let mut second = focus(2, 200);
        if let CognitivePayload::FocusUpdated(f) = &mut second.payload {
            f.suspended = true;
        }
        store.record_cognitive_event(&second).unwrap();
        assert!(
            !store
                .cognitive_focus_at("focus:robot", 150)
                .unwrap()
                .unwrap()
                .suspended
        );
        assert!(
            store
                .cognitive_focus_at("focus:robot", 200)
                .unwrap()
                .unwrap()
                .suspended
        );
        assert!(store.record_cognitive_event(&focus(4, 300)).is_err());
        assert!(store.record_cognitive_event(&focus(3, 100)).is_err());
        store.record_cognitive_event(&focus(3, 300)).unwrap();
    }
    {
        let store = StateStore::open(&path).unwrap();
        let restored = store
            .cognitive_focus_at("focus:robot", 400)
            .unwrap()
            .unwrap();
        assert_eq!(restored.revision, 3);
        assert!(!restored.suspended);
        assert_eq!(
            store
                .cognitive_events(0, store.cognitive_high_watermark().unwrap(), 400, 100)
                .unwrap()
                .len(),
            3
        );
    }
    std::fs::remove_file(path).unwrap();
}
#[test]
#[ignore = "needs a PostgreSQL cluster: run `node tests/with-postgres.mjs --core` from tools/engineering-dashboard"]
fn replay_is_bounded_idempotent_and_has_a_stable_watermark() {
    let s = StateStore::open_test().unwrap();
    s.record_cognitive_event(&sample("event:a", 100)).unwrap();
    let watermark = s.cognitive_high_watermark().unwrap();
    s.record_cognitive_event(&sample("event:b", 50)).unwrap();
    assert_eq!(s.cognitive_events(0, watermark, 100, 100).unwrap().len(), 1);
    assert!(s.record_cognitive_event(&sample("event:a", 101)).is_err());
    assert!(s.cognitive_events(0, watermark, 100, 1001).is_err());
    assert!(s
        .cognitive_events(0, watermark, 99, 100)
        .unwrap()
        .is_empty());
}
#[test]
#[ignore = "needs a PostgreSQL cluster: run `node tests/with-postgres.mjs --core` from tools/engineering-dashboard"]
fn nested_focus_requires_existing_parent_and_cannot_be_reparented() {
    let s = StateStore::open_test().unwrap();
    let mut child = focus(1, 100);
    child.event_id = "child:1".into();
    if let CognitivePayload::FocusUpdated(f) = &mut child.payload {
        f.focus_id = "child".into();
        f.parent_focus_id = Some("focus:robot".into());
    }
    assert!(s.record_cognitive_event(&child).is_err());
    s.record_cognitive_event(&focus(1, 100)).unwrap();
    s.record_cognitive_event(&child).unwrap();
    let mut parent = focus(2, 200);
    if let CognitivePayload::FocusUpdated(f) = &mut parent.payload {
        f.parent_focus_id = Some("child".into());
    }
    assert!(s.record_cognitive_event(&parent).is_err());
}
#[test]
#[ignore = "needs a PostgreSQL cluster: run `node tests/with-postgres.mjs --core` from tools/engineering-dashboard"]
fn invalid_signals_unknown_fields_and_corrupt_history_are_refused() {
    let mut event = sample("a", 100);
    if let CognitivePayload::Training(t) = &mut event.payload {
        t.observed.task_success = None;
    }
    assert!(event.validate().is_err());
    if let CognitivePayload::Training(t) = &mut event.payload {
        t.observed.task_success = Some(true);
        t.observed.risk = Some(f64::NAN);
    }
    assert!(event.validate().is_err());
    let mut json = serde_json::to_value(sample("a", 100)).unwrap();
    json["payload"]["value"]["raw_message"] = "private text".into();
    assert!(serde_json::from_value::<CognitiveEvent>(json).is_err());
    let path = std::env::temp_dir().join(format!(
        "archie-cognitive-{}.json",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    {
        let s = StateStore::open(&path).unwrap();
        s.record_cognitive_event(&sample("a", 100)).unwrap();
    }
    {
        let db = StateStore::open(&path).unwrap();
        db.test_sql("UPDATE cognitive_events SET recorded_at=99")
            .unwrap();
    }
    {
        let s = StateStore::open(&path).unwrap();
        assert!(s.cognitive_events(0, 1, 100, 10).is_err());
    }
    std::fs::remove_file(path).unwrap();
}
