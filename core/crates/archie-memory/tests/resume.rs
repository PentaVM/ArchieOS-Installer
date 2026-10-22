use archie_memory::{
    cognitive::{CognitiveEvent, FocusRecord},
    execution::{
        ExecutionCheckpoint, ExecutionKind, ExecutionStatus, PinnedConstraints, RetainedTaskState,
    },
    resume::{resume_context, ConstraintSource},
    storage::{CheckpointStore, EventStore},
};
use std::cell::Cell;

struct Store {
    focus: FocusRecord,
    execution: ExecutionCheckpoint,
}
impl EventStore for Store {
    type Error = String;
    fn append_event(&self, _: &CognitiveEvent) -> Result<i64, String> {
        unreachable!()
    }
    fn event_watermark(&self) -> Result<i64, String> {
        Ok(1)
    }
    fn focus_at(&self, focus: &str, as_of: i64) -> Result<Option<FocusRecord>, String> {
        assert_eq!(
            as_of,
            i64::MAX,
            "historical time cannot resurrect authority"
        );
        Ok((focus == self.focus.focus_id).then(|| self.focus.clone()))
    }
}
impl CheckpointStore for Store {
    type Error = String;
    fn append_checkpoint(&self, _: &ExecutionCheckpoint, _: Option<&str>) -> Result<(), String> {
        unreachable!()
    }
    fn checkpoint(&self, _: &str) -> Result<Option<ExecutionCheckpoint>, String> {
        unreachable!()
    }
    fn execution_head_at(&self, _: &str, _: i64) -> Result<Option<ExecutionCheckpoint>, String> {
        unreachable!()
    }
    fn execution_path_at(
        &self,
        focus: &str,
        as_of: i64,
    ) -> Result<Vec<ExecutionCheckpoint>, String> {
        assert_eq!(as_of, i64::MAX);
        Ok(if focus == self.focus.focus_id {
            vec![self.execution.clone()]
        } else {
            vec![]
        })
    }
}
struct Policy(Cell<u32>);
impl ConstraintSource for Policy {
    fn constraints_for(&self, focus: &FocusRecord, now: i64) -> Result<PinnedConstraints, String> {
        self.0.set(self.0.get() + 1);
        Ok(PinnedConstraints {
            focus_id: focus.focus_id.clone(),
            policy_revision_ref: format!("policy:{}", self.0.get()),
            issued_at_ms: now,
            authority_ref: "core:policy".into(),
            privacy_ref: "policy:domain".into(),
            capability_ref: "model:proposal".into(),
            operator_constraint_refs: vec![],
            invariant_refs: vec![],
            approval_requirement_refs: vec![],
        })
    }
}
fn fixture() -> Store {
    Store {
        focus: FocusRecord {
            focus_id: "focus:one".into(),
            revision: 1,
            objective_ref: "objective:one".into(),
            parent_focus_id: None,
            suspended: false,
            half_life_ms: 1000,
            updated_at_ms: 10,
            activations: vec![],
        },
        execution: ExecutionCheckpoint {
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
            state: RetainedTaskState::default(),
        },
    }
}

#[test]
fn memory_resume_refreshes_constraints_without_granting_execution() {
    let store = fixture();
    let policy = Policy(Cell::new(0));
    let first = resume_context(&store, &policy, "focus:one", 30).unwrap();
    let second = resume_context(&store, &policy, "focus:one", 31).unwrap();
    assert_eq!(first.execution, store.execution);
    assert_ne!(
        first.pinned.policy_revision_ref,
        second.pinned.policy_revision_ref
    );
    assert_eq!(second.pinned.issued_at_ms, 31);
}

#[test]
fn memory_resume_rejects_current_terminal_and_suspended_state() {
    let mut store = fixture();
    let policy = Policy(Cell::new(0));
    store.execution.status = ExecutionStatus::Completed;
    assert!(resume_context(&store, &policy, "focus:one", 30).is_err());
    store.execution.status = ExecutionStatus::Current;
    store.focus.suspended = true;
    assert!(resume_context(&store, &policy, "focus:one", 30).is_err());
    store.focus.suspended = false;
    assert!(resume_context(&store, &policy, "focus:one", 19).is_err());
    assert_eq!(policy.0.get(), 0);
}
