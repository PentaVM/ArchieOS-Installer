//! Explicit real-database tests. Ignored by default; the isolated harness supplies
//! credentials via environment. No fixture can silently become a passing mock.
use archie_memory::{
    cognitive::{CognitiveEvent, CognitivePayload, FocusRecord},
    execution::{ExecutionCheckpoint, ExecutionKind, ExecutionStatus, RetainedTaskState},
    graph::{GraphNode, GraphSnapshot, NodeKind, Sensitivity},
    shared_thought::{SharedThoughtState, ThoughtItem, ThoughtKind, ThoughtStatus},
    storage::{
        CheckpointStore, EventStore, ExperienceStore, GraphStore, ReplayStore, SharedThoughtStore,
        TrajectoryStore,
    },
    CorrectionEvent, CorrectionKind, CorrectionTrajectory, GraphCorrectionReceipt,
};
use archie_postgres::{PostgresStore, StoreError};
use postgres::{Client, Config, NoTls};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc, Barrier,
};

struct Fixture {
    config: Config,
    schema: String,
}
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let config: Config = std::env::var("ARCHIE_COGNITIVE_TEST_DATABASE_URL")
            .expect("explicit isolated PostgreSQL fixture URL required")
            .parse()
            .expect("valid fixture configuration");
        let schema = format!(
            "archie_test_{}_{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        // Refuse pre-existing names. Cleanup can only affect this newly reserved schema.
        let mut admin = config.connect(NoTls).expect("fixture connection");
        admin
            .batch_execute(&format!("CREATE SCHEMA {schema}"))
            .expect("new fixture schema");
        Self { config, schema }
    }
    fn store(&self) -> PostgresStore {
        PostgresStore::open_local(self.config.clone(), &self.schema).expect("open fixture store")
    }
    fn raw(&self) -> Client {
        let mut client = self
            .config
            .connect(NoTls)
            .expect("fixture admin connection");
        client
            .batch_execute(&format!("SET search_path TO {},pg_catalog", self.schema))
            .unwrap();
        client
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        if let Ok(mut client) = self.config.connect(NoTls) {
            // Generated schema only, never public, user databases or filesystem data.
            let _ = client.batch_execute(&format!("DROP SCHEMA {} CASCADE", self.schema));
        }
    }
}
fn focus(revision: u64, time: i64) -> CognitiveEvent {
    CognitiveEvent {
        schema_version: 1,
        event_id: format!("event:focus:{revision}"),
        recorded_at_ms: time,
        payload: CognitivePayload::FocusUpdated(FocusRecord {
            focus_id: "focus:migration".into(),
            revision,
            objective_ref: "objective:migration".into(),
            parent_focus_id: None,
            suspended: false,
            half_life_ms: 1000,
            updated_at_ms: time,
            activations: vec![],
        }),
    }
}
fn graph(id: &str, time: i64) -> GraphSnapshot {
    GraphSnapshot {
        snapshot_id: id.into(),
        captured_at_ms: time,
        nodes: vec![GraphNode {
            id: "claim:1".into(),
            kind: NodeKind::Claim,
            summary: "fixture evidence".into(),
            aliases: vec![],
            provenance_ref: "evidence:1".into(),
            sensitivity: Sensitivity::LocalRestricted,
            observed_at_ms: 100,
            version: 1,
        }],
        edges: vec![],
    }
}
fn item(id: &str) -> ThoughtItem {
    ThoughtItem {
        id: id.into(),
        kind: ThoughtKind::Hypothesis,
        content_ref: format!("content:{id}"),
        status: ThoughtStatus::Tentative,
        confidence: 0.6,
        evidence_refs: vec!["evidence:1".into()],
        supersedes: None,
    }
}
fn thought() -> SharedThoughtState {
    SharedThoughtState {
        schema_version: 1,
        thought_id: "thought:migration".into(),
        revision: 1,
        updated_at_ms: 100,
        objective_ref: "objective:migration".into(),
        topic_ref: "topic:data".into(),
        focus_id: "focus:migration".into(),
        execution_branch_ref: None,
        items: vec![item("hypothesis:first")],
    }
}
fn contract<S>(store: &S)
where
    S: ReplayStore + GraphStore + SharedThoughtStore,
    <S as EventStore>::Error: std::fmt::Debug,
    <S as GraphStore>::Error: std::fmt::Debug,
    <S as SharedThoughtStore>::Error: std::fmt::Debug,
{
    assert_eq!(store.event_watermark().unwrap(), 0);
    let first = focus(1, 100);
    let sequence = store.append_event(&first).unwrap();
    assert_eq!(store.append_event(&first).unwrap(), sequence);
    assert!(store.append_event(&focus(1, 101)).is_err());
    let watermark = store.event_watermark().unwrap();
    let mut suspended = focus(2, 200);
    if let CognitivePayload::FocusUpdated(f) = &mut suspended.payload {
        f.suspended = true;
    }
    store.append_event(&suspended).unwrap();
    assert!(
        !store
            .focus_at("focus:migration", 150)
            .unwrap()
            .unwrap()
            .suspended
    );
    assert!(
        store
            .focus_at("focus:migration", 200)
            .unwrap()
            .unwrap()
            .suspended
    );
    assert!(store.focus_at("focus:migration", 99).unwrap().is_none());
    assert!(store.append_event(&focus(4, 300)).is_err());
    assert!(store.append_event(&focus(3, 150)).is_err());
    let mut child = focus(1, 50);
    child.event_id = "event:child".into();
    if let CognitivePayload::FocusUpdated(f) = &mut child.payload {
        f.focus_id = "focus:child".into();
        f.parent_focus_id = Some("focus:migration".into());
    }
    assert!(store.append_event(&child).is_err());
    let replay = store.replay_events(0, watermark, 300, 100).unwrap();
    assert_eq!(replay.len(), 1);
    assert_eq!(replay[0].event, first);
    assert!(store
        .replay_events(0, watermark, 99, 100)
        .unwrap()
        .is_empty());
    for limit in [0, 1001] {
        assert!(store.replay_events(0, watermark, 300, limit).is_err());
    }
    assert!(store.replay_events(-1, watermark, 300, 1).is_err());
    store.publish_graph(&graph("graph:1", 100), None).unwrap();
    assert!(store.publish_graph(&graph("graph:2", 200), None).is_err());
    assert!(store
        .publish_graph(&graph("graph:2", 100), Some("graph:1"))
        .is_err());
    store
        .publish_graph(&graph("graph:2", 200), Some("graph:1"))
        .unwrap();
    assert_eq!(store.graph_at(199).unwrap(), Some(graph("graph:1", 100)));
    assert_eq!(store.graph_at(200).unwrap(), Some(graph("graph:2", 200)));
    // Graph publication intentionally retains strict non-idempotent CAS.
    assert!(store
        .publish_graph(&graph("graph:2", 200), Some("graph:1"))
        .is_err());
    let first = thought();
    store.publish_thought(&first, None).unwrap();
    let repaired = first
        .repair("hypothesis:first", item("hypothesis:repaired"), 200)
        .unwrap();
    assert!(store.publish_thought(&repaired, Some(0)).is_err());
    store.publish_thought(&repaired, Some(1)).unwrap();
    store.publish_thought(&repaired, Some(1)).unwrap();
    assert_eq!(
        store.thought_at(&first.thought_id, 199).unwrap(),
        Some(first.clone())
    );
    assert_eq!(
        store.thought_at(&first.thought_id, 200).unwrap(),
        Some(repaired.clone())
    );
    assert!(store.publish_thought(&first, None).is_err());
    let mut rewrite = repaired;
    rewrite.revision = 3;
    rewrite.items[0].content_ref = "content:rewritten".into();
    assert!(store.publish_thought(&rewrite, Some(2)).is_err());
}

#[test]
#[ignore = "requires isolated PostgreSQL 18 fixture"]
fn historical_contract_preserves_history_and_survives_reopen() {
    let fixture = Fixture::new();
    {
        contract(&fixture.store());
    }
    let store = fixture.store();
    assert_eq!(
        store
            .focus_at("focus:migration", 200)
            .unwrap()
            .unwrap()
            .revision,
        2
    );
    assert_eq!(
        store
            .thought_at("thought:migration", 200)
            .unwrap()
            .unwrap()
            .revision,
        2
    );
    assert_eq!(store.graph_at(200).unwrap(), Some(graph("graph:2", 200)));
    let outbox = store.pending_outbox(100).unwrap();
    assert_eq!(outbox.len(), 6);
    assert!(store
        .acknowledge_outbox(outbox[0].sequence, &"0".repeat(64))
        .is_err());
    store
        .acknowledge_outbox(outbox[0].sequence, &outbox[0].payload_sha256)
        .unwrap();
    drop(store);
    let reopened = fixture.store();
    assert_eq!(reopened.pending_outbox(100).unwrap().len(), 5);
    assert_eq!(
        reopened
            .replay_events(0, reopened.event_watermark().unwrap(), 300, 100)
            .unwrap()
            .len(),
        2
    );
}

#[test]
#[ignore = "requires isolated PostgreSQL 18 fixture"]
fn concurrent_migration_and_writers_keep_one_winner_per_revision() {
    let fixture = Fixture::new();
    let barrier = Arc::new(Barrier::new(4));
    let handles: Vec<_> = (0..4)
        .map(|_| {
            let config = fixture.config.clone();
            let schema = fixture.schema.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                PostgresStore::open_local(config, &schema)
                    .unwrap()
                    .append_event(&focus(1, 100))
                    .unwrap()
            })
        })
        .collect();
    let sequences: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    assert!(sequences.iter().all(|s| *s == sequences[0]));
    let store = fixture.store();
    store.publish_thought(&thought(), None).unwrap();
    store.publish_graph(&graph("graph:1", 100), None).unwrap();
    store
        .append_checkpoint(&frame("checkpoint:root", None, 110), None)
        .unwrap();
    for mode in 0..4 {
        let barrier = Arc::new(Barrier::new(4));
        let handles: Vec<_> = (0..4)
            .map(|i| {
                let config = fixture.config.clone();
                let schema = fixture.schema.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    let store = PostgresStore::open_local(config, &schema).unwrap();
                    barrier.wait();
                    match mode {
                        0 => {
                            let mut event = focus(2, 200);
                            event.event_id = format!("event:competitor:{i}");
                            store.append_event(&event).is_ok()
                        }
                        1 => store
                            .publish_thought(
                                &thought()
                                    .repair(
                                        "hypothesis:first",
                                        item(&format!("hypothesis:{i}")),
                                        200,
                                    )
                                    .unwrap(),
                                Some(1),
                            )
                            .is_ok(),
                        2 => store
                            .publish_graph(
                                &graph(&format!("graph:competitor:{i}"), 200),
                                Some("graph:1"),
                            )
                            .is_ok(),
                        _ => store
                            .append_checkpoint(
                                &frame(
                                    &format!("checkpoint:competitor:{i}"),
                                    Some("checkpoint:root"),
                                    220,
                                ),
                                Some("checkpoint:root"),
                            )
                            .is_ok(),
                    }
                })
            })
            .collect();
        assert_eq!(
            handles
                .into_iter()
                .map(|h| usize::from(h.join().unwrap()))
                .sum::<usize>(),
            1
        );
    }
    assert_eq!(store.pending_outbox(100).unwrap().len(), 8);
}

fn frame(id: &str, parent: Option<&str>, time: i64) -> ExecutionCheckpoint {
    ExecutionCheckpoint {
        schema_version: 1,
        checkpoint_id: id.into(),
        focus_id: "focus:migration".into(),
        branch_id: "branch:first".into(),
        parent_checkpoint_id: parent.map(Into::into),
        objective_ref: "objective:migration".into(),
        evidence_ref: "audit:execution".into(),
        recorded_at_ms: time,
        kind: ExecutionKind::Checkpoint,
        status: ExecutionStatus::Current,
        state: RetainedTaskState::default(),
    }
}
fn execution_contract<S>(store: &S)
where
    S: ExperienceStore,
    <S as EventStore>::Error: std::fmt::Debug,
    <S as CheckpointStore>::Error: std::fmt::Debug,
{
    store.append_event(&focus(1, 100)).unwrap();
    let root = frame("checkpoint:root", None, 110);
    store.append_checkpoint(&root, None).unwrap();
    let mut failed = frame("checkpoint:failed", Some("checkpoint:root"), 120);
    failed.kind = ExecutionKind::Failure;
    failed.status = ExecutionStatus::Failed;
    failed
        .state
        .tool_result_refs
        .push("result:abandoned".into());
    store
        .append_checkpoint(&failed, Some("checkpoint:root"))
        .unwrap();
    assert!(store
        .append_checkpoint(
            &frame("checkpoint:forbidden", Some("checkpoint:failed"), 130),
            Some("checkpoint:failed")
        )
        .is_err());
    let mut rollback = frame("checkpoint:rollback", Some("checkpoint:root"), 130);
    rollback.kind = ExecutionKind::Rollback;
    rollback.branch_id = "branch:new".into();
    assert!(store
        .append_checkpoint(&rollback, Some("checkpoint:root"))
        .is_err());
    store
        .append_checkpoint(&rollback, Some("checkpoint:failed"))
        .unwrap();
    let path = store.execution_path_at("focus:migration", 200).unwrap();
    assert_eq!(path, vec![root.clone(), rollback.clone()]);
    assert!(path.iter().all(|c| c.state.tool_result_refs.is_empty()));
    assert_eq!(
        store.checkpoint("checkpoint:failed").unwrap(),
        Some(failed.clone())
    );
    assert_eq!(
        store.execution_head_at("focus:migration", 125).unwrap(),
        Some(failed)
    );
    store.append_checkpoint(&root, None).unwrap();
    assert_eq!(
        store.execution_head_at("focus:migration", 200).unwrap(),
        Some(rollback)
    );
    let mut other = frame("checkpoint:other", Some("checkpoint:failed"), 140);
    other.kind = ExecutionKind::Rollback;
    other.branch_id = "branch:other".into();
    assert!(store
        .append_checkpoint(&other, Some("checkpoint:rollback"))
        .is_err());
    other.parent_checkpoint_id = None;
    other.branch_id = "branch:first".into();
    assert!(store
        .append_checkpoint(&other, Some("checkpoint:rollback"))
        .is_err());
    let mut suspend = focus(2, 150);
    if let CognitivePayload::FocusUpdated(f) = &mut suspend.payload {
        f.suspended = true;
    }
    store.append_event(&suspend).unwrap();
    other.branch_id = "branch:other".into();
    other.recorded_at_ms = 160;
    assert!(store
        .append_checkpoint(&other, Some("checkpoint:rollback"))
        .is_err());
}
#[test]
#[ignore = "requires isolated PostgreSQL 18 fixture"]
fn execution_rollback_excludes_abandoned_result_contamination() {
    let fixture = Fixture::new();
    {
        execution_contract(&fixture.store());
    }
    let reopened = fixture.store();
    assert_eq!(
        reopened
            .execution_path_at("focus:migration", 200)
            .unwrap()
            .len(),
        2
    );
    assert_eq!(reopened.pending_outbox(100).unwrap().len(), 5);
}
fn correction() -> CorrectionTrajectory {
    let hash = format!("sha256:{}", "a".repeat(64));
    CorrectionTrajectory {
        trajectory_id: "trajectory:1".into(),
        correction: CorrectionEvent {
            correction_id: "correction:1".into(),
            kind: CorrectionKind::EntityCorrection,
            operator_id: "operator:fixture".into(),
            graph_snapshot_id: "graph:source".into(),
            query_digest: hash.clone(),
            raw_correction_ref: "vault:correction".into(),
            rationale_ref: "vault:rationale".into(),
            target_ids: vec!["claim:1".into()],
            recorded_at_ms: 150,
        },
        graph_update: GraphCorrectionReceipt {
            correction_id: "correction:1".into(),
            source_graph_snapshot_id: "graph:source".into(),
            resulting_graph_snapshot_id: "graph:corrected".into(),
            graph_update_ref: "evidence:update".into(),
            applied_at_ms: 200,
        },
        selected_context_ids: vec!["claim:1".into()],
        omitted_candidate_ids: vec![],
        response_ref: "vault:response".into(),
        response_digest: hash.clone(),
        corrected_interpretation_ref: "evidence:interpretation".into(),
        model_id: "model:baseline".into(),
        model_hash: hash,
        captured_at_ms: 201,
    }
}
fn corrected_graph() -> GraphSnapshot {
    let mut graph = graph("graph:corrected", 200);
    graph.nodes[0].version = 2;
    graph.nodes[0].summary = "corrected fixture evidence".into();
    graph
}
fn correction_contract<S>(store: &S)
where
    S: ExperienceStore,
    <S as GraphStore>::Error: std::fmt::Debug,
    <S as TrajectoryStore>::Error: std::fmt::Debug,
{
    let source = graph("graph:source", 100);
    let successor = corrected_graph();
    let trajectory = correction();
    store.publish_graph(&source, None).unwrap();
    let mut bad = successor.clone();
    bad.nodes[0].version = 1;
    assert!(store.apply_correction(&trajectory, &bad).is_err());
    let mut bad_trajectory = trajectory.clone();
    bad_trajectory.correction.target_ids = vec!["claim:missing".into()];
    assert!(store.apply_correction(&bad_trajectory, &successor).is_err());
    bad_trajectory = trajectory.clone();
    bad_trajectory.graph_update.applied_at_ms = 199;
    assert!(store.apply_correction(&bad_trajectory, &successor).is_err());
    assert_eq!(store.graph_at(200).unwrap(), Some(source.clone()));
    store.apply_correction(&trajectory, &successor).unwrap();
    store.apply_correction(&trajectory, &successor).unwrap();
    assert_eq!(store.graph_at(199).unwrap(), Some(source));
    assert_eq!(store.graph_at(200).unwrap(), Some(successor.clone()));
    store
        .publish_graph(&graph("graph:later", 300), Some("graph:corrected"))
        .unwrap();
    store.apply_correction(&trajectory, &successor).unwrap();
    assert_eq!(
        store.graph_at(300).unwrap(),
        Some(graph("graph:later", 300))
    );
    bad_trajectory = trajectory;
    bad_trajectory.corrected_interpretation_ref = "evidence:rewritten".into();
    assert!(store.apply_correction(&bad_trajectory, &successor).is_err());
}
#[test]
#[ignore = "requires isolated PostgreSQL 18 fixture"]
fn atomic_graph_correction_and_receipt_binding_persist() {
    let fixture = Fixture::new();
    {
        correction_contract(&fixture.store());
    }
    let store = fixture.store();
    assert_eq!(
        store.trajectory("trajectory:1").unwrap(),
        Some(correction())
    );
    assert!(
        store
            .apply_correction(&correction(), &corrected_graph())
            .unwrap()
            .already_recorded
    );
    assert_eq!(store.pending_outbox(100).unwrap().len(), 3);
    // A snapshot alone must never prove that a correction was committed.
    let unbound = Fixture::new();
    let store = unbound.store();
    store
        .publish_graph(&graph("graph:source", 100), None)
        .unwrap();
    store
        .publish_graph(&corrected_graph(), Some("graph:source"))
        .unwrap();
    assert!(store
        .apply_correction(&correction(), &corrected_graph())
        .is_err());
}
#[test]
#[ignore = "requires isolated PostgreSQL 18 fixture"]
fn execution_and_correction_abort_on_secondary_write_failure() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let mut raw = fixture.raw();
    store.append_event(&focus(1, 100)).unwrap();
    store
        .publish_graph(&graph("graph:source", 100), None)
        .unwrap();
    raw.batch_execute("CREATE FUNCTION fail_write() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'injected'; END $$; CREATE TRIGGER fail_write BEFORE INSERT ON cognitive_outbox FOR EACH ROW EXECUTE FUNCTION fail_write();").unwrap();
    assert!(store
        .append_checkpoint(&frame("checkpoint:root", None, 110), None)
        .is_err());
    assert!(store.checkpoint("checkpoint:root").unwrap().is_none());
    raw.batch_execute("DROP TRIGGER fail_write ON cognitive_outbox; CREATE TRIGGER fail_write BEFORE INSERT ON cortex_correction_trajectories FOR EACH ROW EXECUTE FUNCTION fail_write();").unwrap();
    assert!(store
        .apply_correction(&correction(), &corrected_graph())
        .is_err());
    assert_eq!(
        store.graph_at(300).unwrap(),
        Some(graph("graph:source", 100))
    );
    assert!(store.trajectory("trajectory:1").unwrap().is_none());
    assert_eq!(store.pending_outbox(100).unwrap().len(), 2);
    raw.batch_execute("DROP TRIGGER fail_write ON cortex_correction_trajectories")
        .unwrap();
    store
        .apply_correction(&correction(), &corrected_graph())
        .unwrap();
    store
        .append_checkpoint(&frame("checkpoint:root", None, 110), None)
        .unwrap();
    raw.batch_execute("UPDATE execution_checkpoints SET parent_id='checkpoint:forged'; UPDATE cortex_correction_trajectories SET model_id='model:forged';").unwrap();
    assert!(store.execution_path_at("focus:migration", 300).is_err());
    assert!(store.trajectory("trajectory:1").is_err());
}

#[test]
#[ignore = "requires isolated PostgreSQL 18 fixture"]
fn outbox_failure_rolls_back_all_writes_and_errors_are_sanitized() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let mut raw = fixture.raw();
    store.append_event(&focus(1, 100)).unwrap();
    raw.batch_execute("CREATE FUNCTION fail_receipt() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'private-fixture-sentinel'; END $$; CREATE TRIGGER fail_receipt BEFORE INSERT ON cognitive_outbox FOR EACH ROW EXECUTE FUNCTION fail_receipt();").unwrap();
    let error = store.append_event(&focus(2, 200)).unwrap_err();
    assert!(!format!("{error:?} {error}").contains("private-fixture-sentinel"));
    assert!(store.publish_graph(&graph("graph:1", 100), None).is_err());
    assert!(store.publish_thought(&thought(), None).is_err());
    assert_eq!(
        store
            .focus_at("focus:migration", 300)
            .unwrap()
            .unwrap()
            .revision,
        1
    );
    assert!(store.graph_at(300).unwrap().is_none());
    assert!(store
        .thought_at("thought:migration", 300)
        .unwrap()
        .is_none());
    assert_eq!(store.pending_outbox(100).unwrap().len(), 1);
    raw.batch_execute("DROP TRIGGER fail_receipt ON cognitive_outbox")
        .unwrap();
    store.append_event(&focus(2, 200)).unwrap();
    assert_eq!(
        store
            .replay_events(0, store.event_watermark().unwrap(), 300, 100)
            .unwrap()
            .len(),
        2
    );
    assert_eq!(store.pending_outbox(100).unwrap().len(), 2);
}

#[test]
#[ignore = "requires isolated PostgreSQL 18 fixture"]
fn tampered_indexes_payloads_and_unknown_schema_fail_closed() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let mut raw = fixture.raw();
    store.append_event(&focus(1, 100)).unwrap();
    store.publish_graph(&graph("graph:1", 100), None).unwrap();
    store.publish_thought(&thought(), None).unwrap();
    raw.batch_execute("UPDATE cognitive_events SET recorded_at=99; UPDATE cortex_graph_snapshots SET captured_at=99; UPDATE shared_thought_snapshots SET recorded_at=99;").unwrap();
    assert!(store.focus_at("focus:migration", 100).is_err());
    assert!(store
        .replay_events(0, store.event_watermark().unwrap(), 100, 100)
        .is_err());
    assert!(store.graph_at(100).is_err());
    assert!(store.thought_at("thought:migration", 100).is_err());
    raw.batch_execute("UPDATE cognitive_events SET recorded_at=100,payload_sha256=repeat('0',64); UPDATE cortex_graph_snapshots SET captured_at=100,payload_sha256=repeat('0',64); UPDATE shared_thought_snapshots SET recorded_at=100,hash=repeat('0',64);").unwrap();
    assert!(store.focus_at("focus:migration", 100).is_err());
    assert!(store.graph_at(100).is_err());
    assert!(store.thought_at("thought:migration", 100).is_err());
    raw.batch_execute("UPDATE schema_migrations SET version=99")
        .unwrap();
    assert!(matches!(
        PostgresStore::open_local(fixture.config.clone(), &fixture.schema),
        Err(StoreError::Schema)
    ));
    assert_eq!(
        raw.query_one("SELECT version FROM schema_migrations", &[])
            .unwrap()
            .get::<_, i32>(0),
        99
    );
}

#[test]
#[ignore = "requires isolated PostgreSQL 18 fixture"]
fn writer_lock_times_out_and_replay_watermark_excludes_late_commits() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let mut raw = fixture.raw();
    store.append_event(&focus(1, 100)).unwrap();
    let watermark = store.event_watermark().unwrap();
    let mut tx = raw.transaction().unwrap();
    tx.query_one(
        "SELECT pg_advisory_xact_lock($1)",
        &[&0x4152_4348_4945_5047_i64],
    )
    .unwrap();
    let start = std::time::Instant::now();
    assert!(
        matches!(store.append_event(&focus(2,200)),Err(StoreError::Database(code)) if code == "55P03")
    );
    assert!(start.elapsed() < std::time::Duration::from_secs(8));
    assert_eq!(store.event_watermark().unwrap(), watermark);
    tx.rollback().unwrap();
    // Same-time later revision is valid, but excluded by the old sequence watermark.
    store.append_event(&focus(2, 100)).unwrap();
    assert_eq!(
        store.replay_events(0, watermark, 300, 100).unwrap().len(),
        1
    );
    assert_eq!(
        store
            .replay_events(0, store.event_watermark().unwrap(), 300, 100)
            .unwrap()
            .len(),
        2
    );
}
