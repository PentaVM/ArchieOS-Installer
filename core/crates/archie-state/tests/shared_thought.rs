use archie_memory::{cognitive::*, shared_thought::*};
use archie_state::StateStore;
fn temporary_db() -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "archie-thought-check-{}-{}.json",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

#[test]
#[ignore = "needs a PostgreSQL cluster: run `node tests/with-postgres.mjs --core` from tools/engineering-dashboard"]
fn concurrent_writers_have_one_winner_and_one_receipt() {
    let path = temporary_db();
    {
        let s = StateStore::open(&path).unwrap();
        seed(&s);
        s.record_shared_thought(&first(), None).unwrap();
    }
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let handles: Vec<_> = (0..2)
        .map(|i| {
            let path = path.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let s = StateStore::open(path).unwrap();
                let repaired = first()
                    .repair(
                        "hypothesis:stop",
                        item(&format!("hypothesis:replacement-{i}"), "action:migrate"),
                        200,
                    )
                    .unwrap();
                barrier.wait();
                s.record_shared_thought(&repaired, Some(1)).is_ok()
            })
        })
        .collect();
    assert_eq!(
        handles
            .into_iter()
            .map(|h| h.join().unwrap() as usize)
            .sum::<usize>(),
        1
    );
    {
        let s = StateStore::open(&path).unwrap();
        assert_eq!(s.cognitive_outbox(100).unwrap().len(), 3);
    }
    std::fs::remove_file(path).unwrap();
}
fn item(id: &str, content: &str) -> ThoughtItem {
    ThoughtItem {
        id: id.into(),
        kind: ThoughtKind::ProposedAction,
        content_ref: content.into(),
        status: ThoughtStatus::Tentative,
        confidence: 0.6,
        evidence_refs: vec!["evidence:user-correction".into()],
        supersedes: None,
    }
}
fn first() -> SharedThoughtState {
    SharedThoughtState {
        schema_version: 1,
        thought_id: "thought:database".into(),
        revision: 1,
        updated_at_ms: 100,
        objective_ref: "objective:migration".into(),
        topic_ref: "topic:postgres".into(),
        focus_id: "focus:database".into(),
        execution_branch_ref: None,
        items: vec![item("hypothesis:stop", "action:stop-postgres")],
    }
}
fn seed(store: &StateStore) {
    store
        .record_cognitive_event(&CognitiveEvent {
            schema_version: 1,
            event_id: "event:focus".into(),
            recorded_at_ms: 100,
            payload: CognitivePayload::FocusUpdated(FocusRecord {
                focus_id: "focus:database".into(),
                revision: 1,
                objective_ref: "objective:migration".into(),
                parent_focus_id: None,
                suspended: false,
                half_life_ms: 1000,
                updated_at_ms: 100,
                activations: vec![],
            }),
        })
        .unwrap();
}
#[test]
#[ignore = "needs a PostgreSQL cluster: run `node tests/with-postgres.mjs --core` from tools/engineering-dashboard"]
fn correction_survives_restart_and_preserves_historical_objective() {
    let path = std::env::temp_dir().join(format!(
        "archie-thought-{}.json",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let original = first();
    let repaired = original
        .repair(
            "hypothesis:stop",
            item("hypothesis:migrate", "action:migrate-postgres"),
            200,
        )
        .unwrap();
    {
        let store = StateStore::open(&path).unwrap();
        seed(&store);
        store.record_shared_thought(&original, None).unwrap();
        store.record_shared_thought(&repaired, Some(1)).unwrap();
        store.record_shared_thought(&repaired, Some(1)).unwrap();
    }
    {
        let store = StateStore::open(&path).unwrap();
        assert_eq!(
            store.shared_thought_at(&original.thought_id, 150).unwrap(),
            Some(original.clone())
        );
        let restored = store
            .shared_thought_at(&original.thought_id, 300)
            .unwrap()
            .unwrap();
        assert_eq!(restored, repaired);
        assert_eq!(restored.objective_ref, original.objective_ref);
        assert_eq!(restored.items[0].status, ThoughtStatus::Superseded);
        assert_eq!(restored.items[1].content_ref, "action:migrate-postgres");
        assert_eq!(
            store
                .cognitive_outbox(100)
                .unwrap()
                .iter()
                .filter(|e| e.topic == "shared_thought")
                .count(),
            2
        );
        assert!(store.record_shared_thought(&original, None).is_err());
    }
    let _ = std::fs::remove_file(path);
}
#[test]
#[ignore = "needs a PostgreSQL cluster: run `node tests/with-postgres.mjs --core` from tools/engineering-dashboard"]
fn stale_writer_and_history_rewrite_fail_without_receipt() {
    let store = StateStore::open_test().unwrap();
    seed(&store);
    let original = first();
    store.record_shared_thought(&original, None).unwrap();
    let mut changed = original.clone();
    changed.revision = 2;
    changed.updated_at_ms = 200;
    changed.items[0].content_ref = "action:rewritten".into();
    assert!(store.record_shared_thought(&changed, Some(1)).is_err());
    let repair = original
        .repair(
            "hypothesis:stop",
            item("hypothesis:migrate", "action:migrate-postgres"),
            200,
        )
        .unwrap();
    assert!(store.record_shared_thought(&repair, Some(0)).is_err());
    assert_eq!(store.cognitive_outbox(100).unwrap().len(), 2);
}
#[test]
#[ignore = "needs a PostgreSQL cluster: run `node tests/with-postgres.mjs --core` from tools/engineering-dashboard"]
fn rejects_missing_focus_unproven_claim_invalid_confidence_and_raw_fields() {
    let store = StateStore::open_test().unwrap();
    let state = first();
    assert!(store.record_shared_thought(&state, None).is_err());
    let mut bad = state.clone();
    bad.items[0].confidence = f64::NAN;
    assert!(bad.validate().is_err());
    bad = state.clone();
    bad.items[0].evidence_refs.clear();
    assert!(bad.validate().is_err());
    let mut json = serde_json::to_value(state).unwrap();
    json["transcript"] = serde_json::json!("private");
    assert!(serde_json::from_value::<SharedThoughtState>(json).is_err());
}
