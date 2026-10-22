//! Real PostgreSQL-only integration tests, opt in with the isolated fixture URL.
use archie_memory::{
    cognitive::{CognitiveEvent, CognitivePayload, FocusRecord},
    execution::{ExecutionCheckpoint, ExecutionKind, ExecutionStatus, RetainedTaskState},
    inference::{
        sha256, InferenceBinding, InferenceReceipt, InferenceStatus, ObservedUsage,
        OpaqueSessionId, ScopedReference,
    },
    storage::{CheckpointStore, EventStore, InferenceStore},
};
use archie_postgres::{PostgresStore, StoreError};
use archie_vault::ContextDomain;
use postgres::{Client, Config, NoTls};
use std::sync::atomic::{AtomicU64, Ordering};

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
            .unwrap();
        let schema = format!(
            "archie_test_inference_{}_{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        config
            .connect(NoTls)
            .unwrap()
            .batch_execute(&format!("CREATE SCHEMA {schema}"))
            .unwrap();
        Self { config, schema }
    }
    fn store(&self) -> PostgresStore {
        PostgresStore::open_local(self.config.clone(), &self.schema).unwrap()
    }
    fn raw(&self) -> Client {
        let mut client = self.config.connect(NoTls).unwrap();
        client
            .batch_execute(&format!("SET search_path TO {},pg_catalog", self.schema))
            .unwrap();
        client
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        if let Ok(mut client) = self.config.connect(NoTls) {
            let _ = client.batch_execute(&format!("DROP SCHEMA {} CASCADE", self.schema));
        }
    }
}
fn focus(store: &PostgresStore, id: &str) {
    store
        .append_event(&CognitiveEvent {
            schema_version: 1,
            event_id: format!("event:{id}"),
            recorded_at_ms: 10,
            payload: CognitivePayload::FocusUpdated(FocusRecord {
                focus_id: id.into(),
                revision: 1,
                objective_ref: "objective:one".into(),
                parent_focus_id: None,
                suspended: false,
                half_life_ms: 1000,
                updated_at_ms: 10,
                activations: vec![],
            }),
        })
        .unwrap();
}
fn binding() -> InferenceBinding {
    InferenceBinding {
        schema_version: 1,
        domain: ContextDomain::work("studio", "one").unwrap(),
        task_id: "task:one".into(),
        focus_id: "focus:one".into(),
        objective_ref: "objective:one".into(),
        runner_id: "runner:api".into(),
        provider_id: "provider:local".into(),
        account_id: "account:one".into(),
    }
}
fn receipt(binding: &InferenceBinding, id: &str, at: i64) -> InferenceReceipt {
    InferenceReceipt {
        schema_version: 1,
        receipt_id: id.into(),
        checkpoint_id: format!("checkpoint:{id}"),
        binding: binding.clone(),
        session_id: Some(OpaqueSessionId::new("thread-one").unwrap()),
        model_id: "model:one".into(),
        request_sha256: sha256(b"request"),
        output_sha256: Some(sha256(b"result")),
        admitted_evidence_refs: vec![],
        result_ref: Some(ScopedReference {
            reference: binding
                .domain
                .reference(&format!("inference/{}", sha256(id.as_bytes())))
                .unwrap(),
            account_id: binding.account_id.clone(),
        }),
        status: InferenceStatus::Completed,
        error_code: None,
        observed_at_ms: at,
        usage: ObservedUsage::default(),
    }
}
fn checkpoint(receipt: &InferenceReceipt, parent: Option<&str>) -> ExecutionCheckpoint {
    ExecutionCheckpoint {
        schema_version: 1,
        checkpoint_id: receipt.checkpoint_id.clone(),
        focus_id: receipt.binding.focus_id.clone(),
        branch_id: "branch:one".into(),
        parent_checkpoint_id: parent.map(Into::into),
        objective_ref: receipt.binding.objective_ref.clone(),
        evidence_ref: receipt.provenance_ref(),
        recorded_at_ms: receipt.observed_at_ms,
        kind: ExecutionKind::Result,
        status: ExecutionStatus::Current,
        state: RetainedTaskState {
            artifact_refs: receipt
                .result_ref
                .iter()
                .map(|r| r.reference.to_string())
                .collect(),
            ..Default::default()
        },
    }
}

#[test]
#[ignore = "requires isolated PostgreSQL 18 fixture"]
fn binding_receipt_checkpoint_session_and_outbox_survive_reopen_atomically() {
    let fixture = Fixture::new();
    let binding = binding();
    let receipt = receipt(&binding, "receipt:one", 20);
    let checkpoint = checkpoint(&receipt, None);
    {
        let store = fixture.store();
        focus(&store, "focus:one");
        store.bind_inference(&binding).unwrap();
        store.bind_inference(&binding).unwrap();
        assert!(store
            .inference_receipt(&binding, &receipt.receipt_id)
            .unwrap()
            .is_none());
        store.commit_inference(&receipt, &checkpoint, None).unwrap();
        store.commit_inference(&receipt, &checkpoint, None).unwrap();
        assert_eq!(store.pending_outbox(100).unwrap().len(), 3);
    }
    let store = fixture.store();
    assert_eq!(
        store
            .inference_receipt(&binding, &receipt.receipt_id)
            .unwrap(),
        Some(receipt.clone())
    );
    assert_eq!(
        store.recent_inferences(&binding, 10).unwrap(),
        vec![receipt.clone()]
    );
    assert_eq!(
        store.execution_head_at(&binding.focus_id, 100).unwrap(),
        Some(checkpoint)
    );
    assert_eq!(
        store.inference_session(&binding).unwrap(),
        receipt.session_id
    );
    let mut altered = binding.clone();
    altered.account_id = "account:two".into();
    assert!(store
        .inference_receipt(&altered, &receipt.receipt_id)
        .is_err());
    assert!(store.inference_session(&altered).is_err());
}

#[test]
#[ignore = "requires isolated PostgreSQL 18 fixture"]
fn account_sessions_are_isolated_and_focus_domain_binding_is_immutable() {
    let fixture = Fixture::new();
    let store = fixture.store();
    focus(&store, "focus:one");
    focus(&store, "focus:two");
    let first = binding();
    store.bind_inference(&first).unwrap();
    let first_receipt = receipt(&first, "receipt:one", 20);
    store
        .commit_inference(&first_receipt, &checkpoint(&first_receipt, None), None)
        .unwrap();
    for mutate in 0..7 {
        let mut other = first.clone();
        match mutate {
            0 => other.domain = ContextDomain::personal("owner").unwrap(),
            1 => other.focus_id = "focus:two".into(),
            2 => other.objective_ref = "objective:two".into(),
            3 => other.runner_id = "runner:other".into(),
            4 => other.provider_id = "provider:other".into(),
            5 => other.account_id = "account:other".into(),
            _ => other.task_id = "task:unbound".into(),
        }
        assert!(store.inference_session(&other).is_err());
        assert!(store.recent_session_inferences(&other, 32).is_err());
    }
    let mut personal = first.clone();
    personal.task_id = "task:personal".into();
    personal.domain = ContextDomain::personal("owner").unwrap();
    assert!(store.bind_inference(&personal).is_err());
    let mut other_account = first.clone();
    other_account.task_id = "task:account-two".into();
    other_account.account_id = "account:two".into();
    store.bind_inference(&other_account).unwrap();
    assert!(store.inference_session(&other_account).unwrap().is_none());
    assert!(store
        .recent_session_inferences(&other_account, 32)
        .unwrap()
        .is_empty());
    assert!(store
        .inference_receipt(&other_account, &first_receipt.receipt_id)
        .unwrap()
        .is_none());
    let borrowed_session = receipt(&other_account, "receipt:borrowed-session", 30);
    assert!(
        store
            .commit_inference(
                &borrowed_session,
                &checkpoint(&borrowed_session, Some(&first_receipt.checkpoint_id)),
                Some(&first_receipt.checkpoint_id),
            )
            .is_err(),
        "an observed external session ID cannot be rebound to a different account"
    );
    let mut continuation = first.clone();
    continuation.task_id = "task:continued".into();
    store.bind_inference(&continuation).unwrap();
    assert_eq!(
        store.inference_session(&continuation).unwrap(),
        first_receipt.session_id
    );
    assert_eq!(
        store.recent_session_inferences(&continuation, 32).unwrap(),
        vec![first_receipt.clone()]
    );
    assert!(store
        .recent_inferences(&continuation, 32)
        .unwrap()
        .is_empty());
    assert!(store.recent_session_inferences(&continuation, 129).is_err());
    let mut wrong_focus = first.clone();
    wrong_focus.task_id = "task:focus-two".into();
    wrong_focus.focus_id = "focus:two".into();
    store.bind_inference(&wrong_focus).unwrap();
    let collision = receipt(&wrong_focus, "receipt:collision", 30);
    assert!(store
        .commit_inference(&collision, &checkpoint(&collision, None), None)
        .is_err());
}

#[test]
#[ignore = "requires isolated PostgreSQL 18 fixture"]
fn stale_writers_failure_cancel_or_secondary_write_failure_do_not_invent_success() {
    let fixture = Fixture::new();
    let store = fixture.store();
    focus(&store, "focus:one");
    let binding = binding();
    store.bind_inference(&binding).unwrap();
    let first = receipt(&binding, "receipt:first", 20);
    let first_checkpoint = checkpoint(&first, None);
    store
        .commit_inference(&first, &first_checkpoint, None)
        .unwrap();
    let stale = receipt(&binding, "receipt:stale", 30);
    assert!(store
        .commit_inference(
            &stale,
            &checkpoint(&stale, Some(&first.checkpoint_id)),
            None
        )
        .is_err());
    assert!(store
        .inference_receipt(&binding, &stale.receipt_id)
        .unwrap()
        .is_none());
    for (id, at, status) in [
        ("receipt:failed", 30, InferenceStatus::Failed),
        ("receipt:cancelled", 40, InferenceStatus::Cancelled),
    ] {
        let head = store
            .execution_head_at(&binding.focus_id, 100)
            .unwrap()
            .unwrap();
        let mut failed = receipt(&binding, id, at);
        failed.status = status;
        failed.error_code = Some("runner_failure".into());
        failed.result_ref = None;
        failed.output_sha256 = None;
        let mut failed_checkpoint = checkpoint(&failed, Some(&head.checkpoint_id));
        failed_checkpoint.kind = ExecutionKind::Attempt;
        store
            .commit_inference(&failed, &failed_checkpoint, Some(&head.checkpoint_id))
            .unwrap();
        assert_eq!(
            store
                .inference_receipt(&binding, id)
                .unwrap()
                .unwrap()
                .status,
            status
        );
        assert_eq!(
            store
                .execution_head_at(&binding.focus_id, 100)
                .unwrap()
                .unwrap()
                .status,
            ExecutionStatus::Current
        );
    }
    let head = store
        .execution_head_at(&binding.focus_id, 100)
        .unwrap()
        .unwrap();
    let blocked = receipt(&binding, "receipt:blocked", 50);
    fixture.raw().batch_execute("CREATE FUNCTION reject_inference() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'blocked'; END $$; CREATE TRIGGER reject_inference BEFORE INSERT ON inference_receipts FOR EACH ROW EXECUTE FUNCTION reject_inference();").unwrap();
    assert!(store
        .commit_inference(
            &blocked,
            &checkpoint(&blocked, Some(&head.checkpoint_id)),
            Some(&head.checkpoint_id)
        )
        .is_err());
    assert!(store.checkpoint(&blocked.checkpoint_id).unwrap().is_none());
    assert!(store
        .inference_receipt(&binding, &blocked.receipt_id)
        .unwrap()
        .is_none());
    assert_eq!(store.pending_outbox(100).unwrap().len(), 7);
}

#[test]
#[ignore = "requires isolated PostgreSQL 18 fixture"]
fn migration_upgrades_v1_without_rewriting_its_hash_and_rejects_tampering() {
    let fixture = Fixture::new();
    let mut raw = fixture.raw();
    raw.batch_execute(include_str!("../src/schema.sql"))
        .unwrap();
    raw.batch_execute(
        "CREATE TABLE schema_migrations (version INTEGER PRIMARY KEY,schema_sha256 TEXT NOT NULL)",
    )
    .unwrap();
    let v1_hash = sha256(
        include_str!("../src/schema.sql")
            .replace("\r\n", "\n")
            .as_bytes(),
    );
    raw.execute("INSERT INTO schema_migrations VALUES (1,$1)", &[&v1_hash])
        .unwrap();
    let store = fixture.store();
    let hashes = raw
        .query(
            "SELECT version,schema_sha256 FROM schema_migrations ORDER BY version",
            &[],
        )
        .unwrap();
    assert_eq!(hashes.len(), 2);
    assert_eq!(hashes[0].get::<_, String>(1), v1_hash);
    focus(&store, "focus:one");
    raw.execute(
        "UPDATE schema_migrations SET schema_sha256=repeat('0',64) WHERE version=2",
        &[],
    )
    .unwrap();
    assert!(matches!(
        PostgresStore::open_local(fixture.config.clone(), &fixture.schema),
        Err(StoreError::Schema)
    ));
}

#[test]
#[ignore = "requires isolated PostgreSQL 18 fixture"]
fn outbox_failure_rolls_back_receipt_checkpoint_and_session_together() {
    let fixture = Fixture::new();
    let store = fixture.store();
    focus(&store, "focus:one");
    let binding = binding();
    store.bind_inference(&binding).unwrap();
    fixture.raw().batch_execute("CREATE FUNCTION reject_inference_outbox() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.topic='inference' THEN RAISE EXCEPTION 'blocked'; END IF; RETURN NEW; END $$; CREATE TRIGGER reject_inference_outbox BEFORE INSERT ON cognitive_outbox FOR EACH ROW EXECUTE FUNCTION reject_inference_outbox();").unwrap();
    let receipt = receipt(&binding, "receipt:orphan", 20);
    assert!(store
        .commit_inference(&receipt, &checkpoint(&receipt, None), None)
        .is_err());
    assert!(store
        .inference_receipt(&binding, &receipt.receipt_id)
        .unwrap()
        .is_none());
    assert!(store.checkpoint(&receipt.checkpoint_id).unwrap().is_none());
    assert!(store.inference_session(&binding).unwrap().is_none());
    assert!(store.recent_inferences(&binding, 10).unwrap().is_empty());
    assert_eq!(store.pending_outbox(100).unwrap().len(), 1);
}

#[test]
#[ignore = "requires isolated PostgreSQL 18 fixture"]
fn rollback_excludes_abandoned_inference_from_default_recall_and_session_resume() {
    let fixture = Fixture::new();
    let store = fixture.store();
    focus(&store, "focus:one");
    let binding = binding();
    store.bind_inference(&binding).unwrap();
    let first = receipt(&binding, "receipt:first", 20);
    let first_checkpoint = checkpoint(&first, None);
    store
        .commit_inference(&first, &first_checkpoint, None)
        .unwrap();
    let mut abandoned = receipt(&binding, "receipt:abandoned", 30);
    abandoned.session_id = Some(OpaqueSessionId::new("thread-abandoned").unwrap());
    store
        .commit_inference(
            &abandoned,
            &checkpoint(&abandoned, Some(&first.checkpoint_id)),
            Some(&first.checkpoint_id),
        )
        .unwrap();
    let rollback = ExecutionCheckpoint {
        checkpoint_id: "checkpoint:rollback".into(),
        parent_checkpoint_id: Some(first.checkpoint_id.clone()),
        branch_id: "branch:rollback".into(),
        recorded_at_ms: 40,
        kind: ExecutionKind::Rollback,
        ..first_checkpoint
    };
    store
        .append_checkpoint(&rollback, Some(&abandoned.checkpoint_id))
        .unwrap();
    assert_eq!(
        store.recent_inferences(&binding, 10).unwrap(),
        vec![first.clone()]
    );
    assert_eq!(
        store.recent_inferences(&binding, 1).unwrap(),
        vec![first.clone()]
    );
    let mut next_task = binding.clone();
    next_task.task_id = "task:next".into();
    store.bind_inference(&next_task).unwrap();
    assert_eq!(
        store.recent_session_inferences(&next_task, 32).unwrap(),
        vec![first.clone()]
    );
    assert_eq!(
        store.recent_session_inferences(&next_task, 1).unwrap(),
        vec![first.clone()]
    );
    assert_eq!(store.inference_session(&binding).unwrap(), first.session_id);
    assert_eq!(
        store
            .inference_receipt(&binding, &abandoned.receipt_id)
            .unwrap(),
        Some(abandoned)
    );
}

#[test]
#[ignore = "requires isolated PostgreSQL 18 fixture"]
fn suspended_or_retargeted_focus_and_corrupt_receipts_fail_closed() {
    let fixture = Fixture::new();
    let store = fixture.store();
    focus(&store, "focus:one");
    let binding = binding();
    store.bind_inference(&binding).unwrap();
    let first = receipt(&binding, "receipt:first", 20);
    store
        .commit_inference(&first, &checkpoint(&first, None), None)
        .unwrap();
    store
        .append_event(&CognitiveEvent {
            schema_version: 1,
            event_id: "event:suspend".into(),
            recorded_at_ms: 30,
            payload: CognitivePayload::FocusUpdated(FocusRecord {
                focus_id: binding.focus_id.clone(),
                revision: 2,
                objective_ref: binding.objective_ref.clone(),
                parent_focus_id: None,
                suspended: true,
                half_life_ms: 1000,
                updated_at_ms: 30,
                activations: vec![],
            }),
        })
        .unwrap();
    assert!(store.bind_inference(&binding).is_err());
    assert!(store.inference_session(&binding).is_err());
    let later = receipt(&binding, "receipt:late", 40);
    assert!(store
        .commit_inference(
            &later,
            &checkpoint(&later, Some(&first.checkpoint_id)),
            Some(&first.checkpoint_id)
        )
        .is_err());
    fixture
        .raw()
        .execute(
            "UPDATE inference_receipts SET observed_at=19 WHERE receipt_id=$1",
            &[&first.receipt_id],
        )
        .unwrap();
    assert!(store
        .inference_receipt(&binding, &first.receipt_id)
        .is_err());
    assert!(store.recent_inferences(&binding, 10).is_err());
}

#[test]
#[ignore = "requires isolated PostgreSQL 18 fixture"]
fn tampered_scope_metadata_cannot_launder_other_account_evidence() {
    let fixture = Fixture::new();
    let store = fixture.store();
    focus(&store, "focus:one");
    let first_binding = binding();
    store.bind_inference(&first_binding).unwrap();
    let first = receipt(&first_binding, "receipt:one", 20);
    store
        .commit_inference(&first, &checkpoint(&first, None), None)
        .unwrap();
    let mut second_binding = first_binding.clone();
    second_binding.task_id = "task:second".into();
    second_binding.account_id = "account:second".into();
    store.bind_inference(&second_binding).unwrap();
    let mut second = receipt(&second_binding, "receipt:second", 30);
    second.session_id = Some(OpaqueSessionId::new("thread-second").unwrap());
    store
        .commit_inference(
            &second,
            &checkpoint(&second, Some(&first.checkpoint_id)),
            Some(&first.checkpoint_id),
        )
        .unwrap();
    let mut raw = fixture.raw();
    raw.execute("UPDATE inference_receipts SET scope_sha256=(SELECT scope_sha256 FROM inference_receipts WHERE receipt_id=$1) WHERE receipt_id=$2", &[&first.receipt_id, &second.receipt_id]).unwrap();
    // The exact-task SQL read never decodes the other account's payload.
    assert!(store
        .inference_receipt(&first_binding, &second.receipt_id)
        .unwrap()
        .is_none());
    // A tampered scope index that tries to inject it into context fails closed.
    assert!(store.recent_session_inferences(&first_binding, 32).is_err());
    assert!(store.inference_session(&first_binding).is_err());
    raw.execute(
        "UPDATE inference_receipts SET task_id=$1 WHERE receipt_id=$2",
        &[&first_binding.task_id, &second.receipt_id],
    )
    .unwrap();
    assert!(store
        .inference_receipt(&first_binding, &second.receipt_id)
        .is_err());
}

#[test]
#[ignore = "requires isolated PostgreSQL 18 fixture"]
fn original_commit_deadline_cannot_expire_behind_the_database_writer_lock() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let worker = fixture.store();
    focus(&store, "focus:one");
    let binding = binding();
    store.bind_inference(&binding).unwrap();
    let expired = receipt(&binding, "receipt:expired-commit", 20);
    let expired_checkpoint = checkpoint(&expired, None);
    let mut blocker = fixture.raw();
    let mut tx = blocker.transaction().unwrap();
    tx.query_one(
        "SELECT pg_advisory_xact_lock($1)",
        &[&0x4152_4348_4945_5047_i64],
    )
    .unwrap();
    let deadline = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
        + 40;
    let attempt = expired.clone();
    let writing = std::thread::spawn(move || {
        worker.commit_inference_before(&attempt, &expired_checkpoint, None, deadline)
    });
    std::thread::sleep(std::time::Duration::from_millis(120));
    tx.rollback().unwrap();
    assert!(
        writing.join().unwrap().is_err(),
        "expired success committed after writer-lock wait"
    );
    assert!(store
        .inference_receipt(&binding, &expired.receipt_id)
        .unwrap()
        .is_none());
    assert!(store.checkpoint(&expired.checkpoint_id).unwrap().is_none());
    assert_eq!(store.pending_outbox(100).unwrap().len(), 1);
    let current = receipt(&binding, "receipt:current-commit", 30);
    let deadline = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
        + 5000;
    store
        .commit_inference_before(&current, &checkpoint(&current, None), None, deadline)
        .unwrap();
    assert_eq!(
        store
            .inference_receipt(&binding, &current.receipt_id)
            .unwrap(),
        Some(current)
    );
}
