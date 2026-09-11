use archie_memory::{cognitive::*, execution::*, storage::*};
use archie_state::StateStore;

fn focus(s: &StateStore, id: &str) {
    s.append_event(&CognitiveEvent {
        schema_version: 1,
        event_id: format!("event:{id}"),
        recorded_at_ms: 10,
        payload: CognitivePayload::FocusUpdated(FocusRecord {
            focus_id: id.into(),
            revision: 1,
            objective_ref: "objective:1".into(),
            parent_focus_id: None,
            suspended: false,
            half_life_ms: 100,
            updated_at_ms: 10,
            activations: vec![],
        }),
    })
    .unwrap();
}
fn frame(id: &str, parent: Option<&str>, time: i64) -> ExecutionCheckpoint {
    ExecutionCheckpoint {
        schema_version: 1,
        checkpoint_id: id.into(),
        focus_id: "focus:a".into(),
        branch_id: "branch:one".into(),
        parent_checkpoint_id: parent.map(Into::into),
        objective_ref: "objective:1".into(),
        evidence_ref: "audit:execution".into(),
        recorded_at_ms: time,
        kind: ExecutionKind::Checkpoint,
        status: ExecutionStatus::Current,
        state: RetainedTaskState::default(),
    }
}
#[test]
#[ignore = "needs a PostgreSQL cluster: run `node tests/with-postgres.mjs --core` from tools/engineering-dashboard"]
fn rollback_restores_ancestor_without_abandoned_branch_contamination() {
    let s = StateStore::open_test().unwrap();
    focus(&s, "focus:a");
    let a = frame("a", None, 20);
    s.append_checkpoint(&a, None).unwrap();
    let mut b = frame("b", Some("a"), 30);
    b.kind = ExecutionKind::Failure;
    b.status = ExecutionStatus::Failed;
    b.state.tool_result_refs = vec!["result:failed-attempt".into()];
    s.append_checkpoint(&b, Some("a")).unwrap();
    let mut c = frame("c", Some("a"), 40);
    c.kind = ExecutionKind::Rollback;
    c.branch_id = "branch:two".into();
    s.append_checkpoint(&c, Some("b")).unwrap();
    let path = s.execution_path_at("focus:a", 40).unwrap();
    assert_eq!(
        path.iter()
            .map(|x| x.checkpoint_id.as_str())
            .collect::<Vec<_>>(),
        vec!["a", "c"]
    );
    assert!(path.iter().all(|x| x.state.tool_result_refs.is_empty()));
    assert_eq!(s.execution_head_at("focus:a", 35).unwrap(), Some(b.clone()));
    assert_eq!(s.checkpoint("b").unwrap(), Some(b));
    // Retrying old writes cannot change the active head.
    s.append_checkpoint(&a, None).unwrap();
    assert_eq!(s.execution_head_at("focus:a", 100).unwrap(), Some(c));
}
#[test]
#[ignore = "needs a PostgreSQL cluster: run `node tests/with-postgres.mjs --core` from tools/engineering-dashboard"]
fn stale_writers_cross_focus_and_inactive_parents_are_rejected_atomically() {
    let s = StateStore::open_test().unwrap();
    focus(&s, "focus:a");
    focus(&s, "focus:b");
    let a = frame("a", None, 20);
    s.append_checkpoint(&a, None).unwrap();
    let b = frame("b", Some("a"), 30);
    s.append_checkpoint(&b, Some("a")).unwrap();
    let mut c = frame("c", Some("a"), 40);
    assert!(s.append_checkpoint(&c, Some("a")).is_err());
    assert!(s.append_checkpoint(&c, Some("b")).is_err()); // implicit rewind
    c.kind = ExecutionKind::Rollback;
    c.branch_id = "branch:two".into();
    s.append_checkpoint(&c, Some("b")).unwrap();
    let mut d = frame("d", Some("b"), 50);
    d.kind = ExecutionKind::Rollback;
    d.branch_id = "branch:three".into();
    assert!(s.append_checkpoint(&d, Some("c")).is_err());
    d.focus_id = "focus:b".into();
    assert!(s.append_checkpoint(&d, None).is_err());
    assert!(s.checkpoint("d").unwrap().is_none());
    assert_eq!(s.cognitive_outbox(100).unwrap().len(), 5); // 2 Focus + 3 successful checkpoints
}
#[test]
#[ignore = "needs a PostgreSQL cluster: run `node tests/with-postgres.mjs --core` from tools/engineering-dashboard"]
fn terminal_checkpoint_requires_explicit_rewind_and_outbox_ack_binds_digest() {
    let s = StateStore::open_test().unwrap();
    focus(&s, "focus:a");
    let mut a = frame("a", None, 20);
    a.status = ExecutionStatus::Completed;
    s.append_checkpoint(&a, None).unwrap();
    assert!(s
        .append_checkpoint(&frame("b", Some("a"), 30), Some("a"))
        .is_err());
    let mut c = frame("c", None, 40);
    c.kind = ExecutionKind::Rollback;
    c.branch_id = "branch:two".into();
    s.append_checkpoint(&c, Some("a")).unwrap();
    let pending = s.cognitive_outbox(100).unwrap();
    assert!(s
        .acknowledge_cognitive_outbox(pending[0].sequence, &"0".repeat(64))
        .is_err());
    s.acknowledge_cognitive_outbox(pending[0].sequence, &pending[0].payload_sha256)
        .unwrap();
    s.acknowledge_cognitive_outbox(pending[0].sequence, &pending[0].payload_sha256)
        .unwrap();
    assert_eq!(s.cognitive_outbox(100).unwrap().len(), 2);
    assert!(s.focus_at("focus:a", 10).unwrap().is_some());
    assert!(s.cognitive_outbox(1001).is_err());
}
#[test]
#[ignore = "needs a PostgreSQL cluster: run `node tests/with-postgres.mjs --core` from tools/engineering-dashboard"]
fn execution_survives_restart_and_corruption_fails_closed() {
    let db = std::env::temp_dir().join(format!(
        "archie-execution-{}-{}.json",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    {
        let s = StateStore::open(&db).unwrap();
        focus(&s, "focus:a");
        s.append_checkpoint(&frame("a", None, 20), None).unwrap();
    }
    {
        let s = StateStore::open(&db).unwrap();
        assert_eq!(s.execution_path_at("focus:a", 20).unwrap().len(), 1);
    }
    {
        let c = StateStore::open(&db).unwrap();
        c.test_sql("UPDATE execution_checkpoints SET parent_id='forged'")
            .unwrap();
    }
    {
        let s = StateStore::open(&db).unwrap();
        assert!(s.execution_path_at("focus:a", 20).is_err());
    }
    std::fs::remove_file(db).unwrap();
}
#[test]
#[ignore = "needs a PostgreSQL cluster: run `node tests/with-postgres.mjs --core` from tools/engineering-dashboard"]
fn suspended_focus_cannot_append_and_retained_state_is_bounded() {
    let s = StateStore::open_test().unwrap();
    focus(&s, "focus:a");
    let mut f = s.focus_at("focus:a", 10).unwrap().unwrap();
    f.revision = 2;
    f.updated_at_ms = 15;
    f.suspended = true;
    s.append_event(&CognitiveEvent {
        schema_version: 1,
        event_id: "suspend".into(),
        recorded_at_ms: 15,
        payload: CognitivePayload::FocusUpdated(f),
    })
    .unwrap();
    assert!(s.append_checkpoint(&frame("a", None, 20), None).is_err());
    assert!(s.append_checkpoint(&frame("a", None, 12), None).is_err());
    let mut c = frame("a", None, 20);
    c.state.plan_refs = vec!["plan:same".into(); 65];
    assert!(c.validate().is_err());
}
