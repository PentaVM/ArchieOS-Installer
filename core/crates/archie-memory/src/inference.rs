//! Attributed inference experience. Storage is not verification or authority.
//!
//! Private bodies and external credentials have no fields here. The caller seals
//! admitted evidence/results in Vault before committing these references; only a
//! committed receipt makes a staged result eligible for recall.
use crate::{
    cognitive::reference,
    execution::{ExecutionCheckpoint, ExecutionKind, ExecutionStatus},
};
pub use archie_vault::ContextDomain;
use archie_vault::VaultRef;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

pub fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InferenceBinding {
    pub schema_version: u32,
    pub domain: ContextDomain,
    pub focus_id: String,
    pub task_id: String,
    pub objective_ref: String,
    pub runner_id: String,
    pub provider_id: String,
    /// Opaque operator/broker account identity; never a credential or session token.
    pub account_id: String,
}
impl InferenceBinding {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != 1
            || [
                &self.focus_id,
                &self.task_id,
                &self.objective_ref,
                &self.runner_id,
                &self.provider_id,
                &self.account_id,
            ]
            .into_iter()
            .any(|value| !reference(value))
        {
            return Err("invalid inference binding".into());
        }
        Ok(())
    }

    /// Sessions can span tasks only within the same exact authorization/runner scope.
    pub fn same_session_scope(&self, other: &Self) -> bool {
        self.domain == other.domain
            && self.focus_id == other.focus_id
            && self.objective_ref == other.objective_ref
            && self.runner_id == other.runner_id
            && self.provider_id == other.provider_id
            && self.account_id == other.account_id
    }
}

/// A provider-owned thread/session identifier, not an OAuth or application token.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct OpaqueSessionId(String);
impl OpaqueSessionId {
    pub fn new(value: impl Into<String>) -> Result<Self, String> {
        let value = value.into();
        if value.is_empty()
            || value.len() > 192
            || !value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._:-".contains(&b))
        {
            return Err("invalid opaque inference session identifier".into());
        }
        Ok(Self(value))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl TryFrom<String> for OpaqueSessionId {
    type Error = String;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}
impl From<OpaqueSessionId> for String {
    fn from(value: OpaqueSessionId) -> Self {
        value.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScopedReference {
    pub reference: VaultRef,
    /// The account under which Core admitted this reference for this request.
    pub account_id: String,
}
impl ScopedReference {
    pub fn validate_for(&self, binding: &InferenceBinding) -> Result<(), String> {
        if self.account_id != binding.account_id
            || !binding.domain.contains(&self.reference)
            || !reference(&self.reference.to_string())
        {
            return Err("inference reference crosses exact domain or account".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InferenceStatus {
    Completed,
    Failed,
    Cancelled,
}

/// None means unobserved. The store never supplies estimated usage as observation.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservedUsage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub duration_ms: Option<u64>,
}
impl ObservedUsage {
    pub fn validate(&self) -> Result<(), String> {
        if self.input_tokens.is_some_and(|v| v > 1_000_000_000)
            || self.output_tokens.is_some_and(|v| v > 1_000_000_000)
            || self.duration_ms.is_some_and(|v| v > 604_800_000)
        {
            return Err("inference observations exceed bounds".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InferenceReceipt {
    pub schema_version: u32,
    pub receipt_id: String,
    pub checkpoint_id: String,
    pub binding: InferenceBinding,
    pub session_id: Option<OpaqueSessionId>,
    /// The actually selected model, assigned by the trusted caller, not model prose.
    pub model_id: String,
    pub request_sha256: String,
    pub output_sha256: Option<String>,
    pub admitted_evidence_refs: Vec<ScopedReference>,
    pub result_ref: Option<ScopedReference>,
    pub status: InferenceStatus,
    pub error_code: Option<String>,
    pub observed_at_ms: i64,
    pub usage: ObservedUsage,
}
impl InferenceReceipt {
    pub fn provenance_ref(&self) -> String {
        format!("inference:{}", self.receipt_id)
    }

    pub fn validate(&self) -> Result<(), String> {
        self.binding.validate()?;
        self.usage.validate()?;
        if self.schema_version != 1
            || !reference(&self.receipt_id)
            || self.receipt_id.len() > 128
            || !reference(&self.checkpoint_id)
            || !reference(&self.model_id)
            || !digest(&self.request_sha256)
            || self.observed_at_ms <= 0
            || self.admitted_evidence_refs.len() > 64
            || self.error_code.as_ref().is_some_and(|code| {
                code.is_empty()
                    || code.len() > 96
                    || !code
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
            })
        {
            return Err("invalid inference receipt".into());
        }
        let mut refs = BTreeSet::new();
        for evidence in &self.admitted_evidence_refs {
            evidence.validate_for(&self.binding)?;
            if !refs.insert(evidence.reference.to_string()) {
                return Err("duplicate admitted inference evidence".into());
            }
        }
        match self.status {
            InferenceStatus::Completed => {
                let result = self.result_ref.as_ref().ok_or("missing sealed result")?;
                result.validate_for(&self.binding)?;
                if self.error_code.is_some()
                    || !self
                        .output_sha256
                        .as_ref()
                        .is_some_and(|value| digest(value))
                    || refs.contains(&result.reference.to_string())
                {
                    return Err("completed inference requires a distinct attributed result".into());
                }
            }
            InferenceStatus::Failed | InferenceStatus::Cancelled => {
                if self.result_ref.is_some()
                    || self.output_sha256.is_some()
                    || (self.status == InferenceStatus::Failed && self.error_code.is_none())
                {
                    return Err("unsuccessful inference cannot claim completed output".into());
                }
            }
        }
        Ok(())
    }

    /// This boundary retains attributed experience, never grants verified-fact status
    /// or implicitly terminates the parent task when a provider attempt fails.
    pub fn validate_checkpoint(&self, checkpoint: &ExecutionCheckpoint) -> Result<(), String> {
        self.validate()?;
        checkpoint.validate()?;
        if checkpoint.checkpoint_id != self.checkpoint_id
            || checkpoint.focus_id != self.binding.focus_id
            || checkpoint.objective_ref != self.binding.objective_ref
            || checkpoint.recorded_at_ms != self.observed_at_ms
            || checkpoint.evidence_ref != self.provenance_ref()
            || checkpoint.status != ExecutionStatus::Current
            || !checkpoint.state.verified_fact_refs.is_empty()
            || match self.status {
                InferenceStatus::Completed => checkpoint.kind != ExecutionKind::Result,
                InferenceStatus::Failed | InferenceStatus::Cancelled => {
                    checkpoint.kind != ExecutionKind::Attempt
                }
            }
        {
            return Err("inference checkpoint identity, attribution or status mismatch".into());
        }
        let admitted: BTreeSet<_> = self
            .admitted_evidence_refs
            .iter()
            .chain(self.result_ref.iter())
            .map(|item| item.reference.to_string())
            .collect();
        for refs in [
            &checkpoint.state.decision_refs,
            &checkpoint.state.open_question_refs,
            &checkpoint.state.artifact_refs,
            &checkpoint.state.tool_result_refs,
            &checkpoint.state.plan_refs,
            &checkpoint.state.remaining_work_refs,
        ] {
            if refs.iter().any(|value| !admitted.contains(value)) {
                return Err(
                    "checkpoint retains evidence not admitted in this account/domain".into(),
                );
            }
        }
        Ok(())
    }
}

fn digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
