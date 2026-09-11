//! Explicit task state, never hidden model reasoning. A checkpoint is a complete
//! working state; ancestry records execution history, not implicit state merging.
use crate::cognitive::reference;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionKind {
    Task,
    Subtask,
    Attempt,
    Decision,
    ToolCall,
    Result,
    Failure,
    Checkpoint,
    Rollback,
    Outcome,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionStatus {
    Current,
    Completed,
    Failed,
    Abandoned,
}

/// References resolve through policy-filtered evidence storage. A reference in
/// `verified_fact_refs` does not itself confer verification or access permission.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetainedTaskState {
    pub decision_refs: Vec<String>,
    pub verified_fact_refs: Vec<String>,
    pub open_question_refs: Vec<String>,
    pub artifact_refs: Vec<String>,
    pub tool_result_refs: Vec<String>,
    pub plan_refs: Vec<String>,
    pub remaining_work_refs: Vec<String>,
}
impl RetainedTaskState {
    pub fn validate(&self) -> Result<(), String> {
        for refs in [
            &self.decision_refs,
            &self.verified_fact_refs,
            &self.open_question_refs,
            &self.artifact_refs,
            &self.tool_result_refs,
            &self.plan_refs,
            &self.remaining_work_refs,
        ] {
            if refs.len() > 64
                || refs.iter().any(|r| !reference(r))
                || refs.iter().collect::<BTreeSet<_>>().len() != refs.len()
            {
                return Err("invalid retained task references".into());
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionCheckpoint {
    pub schema_version: u32,
    pub checkpoint_id: String,
    pub focus_id: String,
    pub branch_id: String,
    pub parent_checkpoint_id: Option<String>,
    pub objective_ref: String,
    pub evidence_ref: String,
    pub recorded_at_ms: i64,
    pub kind: ExecutionKind,
    pub status: ExecutionStatus,
    pub state: RetainedTaskState,
}
impl ExecutionCheckpoint {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != 1
            || self.recorded_at_ms <= 0
            || [
                &self.checkpoint_id,
                &self.focus_id,
                &self.branch_id,
                &self.objective_ref,
                &self.evidence_ref,
            ]
            .into_iter()
            .any(|r| !reference(r))
            || self
                .parent_checkpoint_id
                .as_ref()
                .is_some_and(|r| !reference(r) || r == &self.checkpoint_id)
            || (self.kind == ExecutionKind::Failure && self.status != ExecutionStatus::Failed)
            || (self.kind == ExecutionKind::Rollback && self.status != ExecutionStatus::Current)
        {
            return Err("invalid execution checkpoint".into());
        }
        self.state.validate()
    }
}

/// Core refreshes these constraints for every worker request; they are never
/// stored in a checkpoint or accepted from the compacted model context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinnedConstraints {
    pub focus_id: String,
    pub policy_revision_ref: String,
    pub issued_at_ms: i64,
    pub authority_ref: String,
    pub privacy_ref: String,
    pub capability_ref: String,
    pub operator_constraint_refs: Vec<String>,
    pub invariant_refs: Vec<String>,
    pub approval_requirement_refs: Vec<String>,
}
impl PinnedConstraints {
    pub fn validate_for(&self, focus: &str, now_ms: i64) -> Result<(), String> {
        if self.focus_id != focus
            || now_ms <= 0
            || self.issued_at_ms != now_ms
            || [
                &self.focus_id,
                &self.policy_revision_ref,
                &self.authority_ref,
                &self.privacy_ref,
                &self.capability_ref,
            ]
            .into_iter()
            .any(|r| !reference(r))
        {
            return Err("constraints must come from a fresh trusted policy view".into());
        }
        for refs in [
            &self.operator_constraint_refs,
            &self.invariant_refs,
            &self.approval_requirement_refs,
        ] {
            if refs.len() > 64 || refs.iter().any(|r| !reference(r)) {
                return Err("invalid pinned constraints".into());
            }
        }
        Ok(())
    }
}
