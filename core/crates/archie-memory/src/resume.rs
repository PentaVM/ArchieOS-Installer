//! Worker-resumption boundary. Only a trusted Core policy adapter can supply the
//! fresh constraint view. This prepares context, never grants worker execution.
use crate::{
    cognitive::FocusRecord,
    execution::{ExecutionCheckpoint, ExecutionStatus, PinnedConstraints},
    storage::{CheckpointStore, EventStore},
};

pub trait ConstraintSource {
    fn constraints_for(
        &self,
        focus: &FocusRecord,
        now_ms: i64,
    ) -> Result<PinnedConstraints, String>;
}

/// A read-only collaboration view. Historical assertions never supply authority;
/// freshly evaluated policy constraints are carried independently.
#[derive(Debug, Clone, PartialEq)]
pub struct CollaborationContext {
    pub thought: crate::shared_thought::SharedThoughtState,
    pub focus: FocusRecord,
    pub pinned: PinnedConstraints,
}
pub fn collaboration_context<S, P>(
    store: &S,
    policy: &P,
    thought_id: &str,
    now_ms: i64,
) -> Result<CollaborationContext, String>
where
    S: crate::storage::SharedThoughtStore + EventStore,
    P: ConstraintSource,
    <S as crate::storage::SharedThoughtStore>::Error: std::fmt::Display,
    <S as EventStore>::Error: std::fmt::Display,
{
    let thought = store
        .thought_at(thought_id, i64::MAX)
        .map_err(|e| e.to_string())?
        .ok_or("SharedThought not found")?;
    let focus = store
        .focus_at(&thought.focus_id, i64::MAX)
        .map_err(|e| e.to_string())?
        .ok_or("Focus not found")?;
    if thought.updated_at_ms > now_ms
        || focus.updated_at_ms > now_ms
        || focus.suspended
        || thought.objective_ref != focus.objective_ref
    {
        return Err(
            "SharedThought cannot resume with stale objective, future state or suspended Focus"
                .into(),
        );
    }
    let pinned = policy.constraints_for(&focus, now_ms)?;
    pinned.validate_for(&focus.focus_id, now_ms)?;
    Ok(CollaborationContext {
        thought,
        focus,
        pinned,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerResumeContext {
    pub execution: ExecutionCheckpoint,
    pub pinned: PinnedConstraints,
}

pub fn resume_context<S, P>(
    store: &S,
    policy: &P,
    focus_id: &str,
    now_ms: i64,
) -> Result<WorkerResumeContext, String>
where
    S: CheckpointStore + EventStore,
    P: ConstraintSource,
    <S as CheckpointStore>::Error: std::fmt::Display,
    <S as EventStore>::Error: std::fmt::Display,
{
    let focus = store
        .focus_at(focus_id, i64::MAX)
        .map_err(|e| e.to_string())?
        .ok_or("Focus not found")?;
    if focus.suspended || focus.updated_at_ms > now_ms {
        return Err("Focus suspended or request predates current Focus".into());
    }
    let execution = store
        .execution_path_at(focus_id, i64::MAX)
        .map_err(|e| e.to_string())?
        .pop()
        .ok_or("execution state not found")?;
    if execution.status != ExecutionStatus::Current
        || execution.objective_ref != focus.objective_ref
        || execution.recorded_at_ms > now_ms
    {
        return Err("execution cannot resume from terminal or obsolete objective state".into());
    }
    let pinned = policy.constraints_for(&focus, now_ms)?;
    pinned.validate_for(focus_id, now_ms)?;
    Ok(WorkerResumeContext { execution, pinned })
}
