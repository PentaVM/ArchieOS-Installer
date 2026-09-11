use archie_core::cognitive_context::*;
use archie_memory::{cognitive::*, execution::*};
use archie_state::StateStore;
struct Policy {
    stale: bool,
}

#[test]
#[ignore = "needs a PostgreSQL cluster: run `node tests/with-postgres.mjs --core` from tools/engineering-dashboard"]
fn shared_thought_context_requires_current_focus_and_fresh_policy() {
    use archie_memory::shared_thought::SharedThoughtState;
    let store = StateStore::open_test().unwrap();
    let mut focus = FocusRecord {
        focus_id: "focus:shared".into(),
        revision: 1,
        objective_ref: "objective:shared".into(),
        parent_focus_id: None,
        suspended: false,
        half_life_ms: 100,
        updated_at_ms: 10,
        activations: vec![],
    };
    store
        .record_cognitive_event(&CognitiveEvent {
            schema_version: 1,
            event_id: "event:shared-focus".into(),
            recorded_at_ms: 10,
            payload: CognitivePayload::FocusUpdated(focus.clone()),
        })
        .unwrap();
    store
        .record_shared_thought(
            &SharedThoughtState {
                schema_version: 1,
                thought_id: "thought:shared".into(),
                revision: 1,
                updated_at_ms: 20,
                objective_ref: focus.objective_ref.clone(),
                topic_ref: "topic:database".into(),
                focus_id: focus.focus_id.clone(),
                execution_branch_ref: None,
                items: vec![],
            },
            None,
        )
        .unwrap();
    let context =
        collaboration_context(&store, &Policy { stale: false }, "thought:shared", 30).unwrap();
    assert_eq!(context.pinned.policy_revision_ref, "policy:live");
    assert!(collaboration_context(&store, &Policy { stale: true }, "thought:shared", 30).is_err());
    assert!(collaboration_context(&store, &Policy { stale: false }, "thought:shared", 19).is_err());
    focus.revision = 2;
    focus.updated_at_ms = 40;
    focus.suspended = true;
    store
        .record_cognitive_event(&CognitiveEvent {
            schema_version: 1,
            event_id: "event:shared-suspend".into(),
            recorded_at_ms: 40,
            payload: CognitivePayload::FocusUpdated(focus),
        })
        .unwrap();
    assert!(collaboration_context(&store, &Policy { stale: false }, "thought:shared", 50).is_err());
    assert!(collaboration_context(&store, &Policy { stale: false }, "thought:shared", 30).is_err());
}
impl ConstraintSource for Policy {
    fn constraints_for(&self, f: &FocusRecord, now: i64) -> Result<PinnedConstraints, String> {
        Ok(PinnedConstraints {
            focus_id: f.focus_id.clone(),
            policy_revision_ref: "policy:live".into(),
            issued_at_ms: now - i64::from(self.stale),
            authority_ref: "authority:core".into(),
            privacy_ref: "privacy:local".into(),
            capability_ref: "capability:readonly".into(),
            operator_constraint_refs: vec!["operator:constraint".into()],
            invariant_refs: vec![],
            approval_requirement_refs: vec!["approval:required".into()],
        })
    }
}
#[test]
#[ignore = "needs a PostgreSQL cluster: run `node tests/with-postgres.mjs --core` from tools/engineering-dashboard"]
fn compacted_state_is_combined_with_fresh_constraints_and_never_implicitly_authorizes_execution() {
    let s = StateStore::open_test().unwrap();
    s.record_cognitive_event(&CognitiveEvent {
        schema_version: 1,
        event_id: "focus:event".into(),
        recorded_at_ms: 10,
        payload: CognitivePayload::FocusUpdated(FocusRecord {
            focus_id: "focus:one".into(),
            revision: 1,
            objective_ref: "objective:one".into(),
            parent_focus_id: None,
            suspended: false,
            half_life_ms: 100,
            updated_at_ms: 10,
            activations: vec![],
        }),
    })
    .unwrap();
    let c = ExecutionCheckpoint {
        schema_version: 1,
        checkpoint_id: "checkpoint:one".into(),
        focus_id: "focus:one".into(),
        branch_id: "branch:one".into(),
        parent_checkpoint_id: None,
        objective_ref: "objective:one".into(),
        evidence_ref: "evidence:one".into(),
        recorded_at_ms: 20,
        kind: ExecutionKind::Checkpoint,
        status: ExecutionStatus::Current,
        state: RetainedTaskState {
            remaining_work_refs: vec!["task:remaining".into()],
            ..Default::default()
        },
    };
    s.record_execution_checkpoint(&c, None).unwrap();
    let context = resume_context(&s, &Policy { stale: false }, "focus:one", 30).unwrap();
    assert_eq!(context.execution, c);
    assert_eq!(
        context.pinned.approval_requirement_refs,
        vec!["approval:required"]
    );
    assert!(resume_context(&s, &Policy { stale: true }, "focus:one", 30).is_err());
    assert!(resume_context(&s, &Policy { stale: false }, "focus:other", 30).is_err());
    let mut end = c;
    end.checkpoint_id = "checkpoint:end".into();
    end.parent_checkpoint_id = Some("checkpoint:one".into());
    end.recorded_at_ms = 40;
    end.status = ExecutionStatus::Completed;
    s.record_execution_checkpoint(&end, Some("checkpoint:one"))
        .unwrap();
    assert!(resume_context(&s, &Policy { stale: false }, "focus:one", 50).is_err());
    // A historical timestamp cannot resurrect the previously current checkpoint.
    assert!(resume_context(&s, &Policy { stale: false }, "focus:one", 30).is_err());
}
