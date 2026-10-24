use archie_memory::{
    cognitive::{CognitiveEvent, CognitivePayload, FocusRecord},
    execution::{ExecutionCheckpoint, ExecutionKind, ExecutionStatus, RetainedTaskState},
    inference::{
        sha256, ContextDomain, InferenceBinding, InferenceReceipt, InferenceStatus, ObservedUsage,
        OpaqueSessionId, ScopedReference,
    },
    storage::InferenceStore,
};
use archie_state::StateStore;
use postgres::{Config, NoTls};

#[test]
#[ignore = "requires isolated PostgreSQL18 with TimescaleDB2.29.2 and vector0.8.6"]
fn authoritative_state_adapter_reopens_scoped_receipts_and_resume_context() {
    let connection =
        std::env::var("ARCHIE_TEST_DATABASE_URL").expect("explicit isolated fixture required");
    let config: Config = connection.parse().unwrap();
    let schema = format!("archie_test_state_inference_{}", std::process::id());
    config
        .connect(NoTls)
        .unwrap()
        .batch_execute(&format!("CREATE SCHEMA {schema}"))
        .unwrap();
    struct Cleanup {
        config: Config,
        schema: String,
    }
    impl Drop for Cleanup {
        fn drop(&mut self) {
            if let Ok(mut client) = self.config.connect(NoTls) {
                let _ = client.batch_execute(&format!("DROP SCHEMA {} CASCADE", self.schema));
            }
        }
    }
    let _cleanup = Cleanup {
        config,
        schema: schema.clone(),
    };
    let binding = InferenceBinding {
        schema_version: 1,
        domain: ContextDomain::personal("fixture").unwrap(),
        focus_id: "focus:state".into(),
        task_id: "task:state".into(),
        objective_ref: "objective:state".into(),
        runner_id: "runner:api".into(),
        provider_id: "provider:fixture".into(),
        account_id: "account:fixture".into(),
    };
    let result = ScopedReference {
        reference: binding.domain.reference("results/one").unwrap(),
        account_id: binding.account_id.clone(),
    };
    let receipt = InferenceReceipt {
        schema_version: 1,
        receipt_id: "receipt:state".into(),
        checkpoint_id: "checkpoint:state".into(),
        binding: binding.clone(),
        model_id: "model:fixture".into(),
        session_id: Some(OpaqueSessionId::new("thread-state").unwrap()),
        request_sha256: sha256(b"request"),
        output_sha256: Some(sha256(b"result")),
        admitted_evidence_refs: vec![],
        result_ref: Some(result.clone()),
        status: InferenceStatus::Completed,
        error_code: None,
        observed_at_ms: 20,
        usage: ObservedUsage::default(),
    };
    let checkpoint = ExecutionCheckpoint {
        schema_version: 1,
        checkpoint_id: receipt.checkpoint_id.clone(),
        focus_id: binding.focus_id.clone(),
        branch_id: "branch:state".into(),
        parent_checkpoint_id: None,
        objective_ref: binding.objective_ref.clone(),
        evidence_ref: receipt.provenance_ref(),
        recorded_at_ms: 20,
        kind: ExecutionKind::Result,
        status: ExecutionStatus::Current,
        state: RetainedTaskState {
            artifact_refs: vec![result.reference.to_string()],
            ..Default::default()
        },
    };
    {
        let state = StateStore::connect(&connection, &schema).unwrap();
        state
            .record_cognitive_event(&CognitiveEvent {
                schema_version: 1,
                event_id: "event:state".into(),
                recorded_at_ms: 10,
                payload: CognitivePayload::FocusUpdated(FocusRecord {
                    focus_id: binding.focus_id.clone(),
                    revision: 1,
                    objective_ref: binding.objective_ref.clone(),
                    parent_focus_id: None,
                    suspended: false,
                    half_life_ms: 1000,
                    updated_at_ms: 10,
                    activations: vec![],
                }),
            })
            .unwrap();
        InferenceStore::bind_inference(&state, &binding).unwrap();
        state.commit_inference(&receipt, &checkpoint, None).unwrap();
    }
    let state = StateStore::connect(&connection, &schema).unwrap();
    assert_eq!(
        state
            .inference_receipt(&binding, &receipt.receipt_id)
            .unwrap(),
        Some(receipt.clone())
    );
    assert_eq!(
        state.execution_head_at(&binding.focus_id, 100).unwrap(),
        Some(checkpoint)
    );
    assert_eq!(state.cognitive_outbox(100).unwrap().len(), 3);
    let mut next = binding.clone();
    next.task_id = "task:state-next".into();
    state.bind_inference(&next).unwrap();
    assert_eq!(
        state.recent_session_inferences(&next, 32).unwrap(),
        vec![receipt.clone()]
    );
    assert_eq!(state.inference_session(&next).unwrap(), receipt.session_id);
    next.account_id = "account:other".into();
    assert!(state.recent_session_inferences(&next, 32).is_err());
}
