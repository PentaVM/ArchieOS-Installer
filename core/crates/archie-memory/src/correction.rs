//! Typed, privacy-preserving supervision for the Analytical Cortex.
//!
//! A correction may change knowledge immediately, but it must first be applied by the
//! trusted graph writer.  This module therefore refuses to create a training trajectory
//! unless it carries the writer's receipt.  It stores opaque vault/provenance references
//! and digests only: conversational text, model output and biometric/private payloads do
//! not get copied into the general state database.

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fmt;
use thiserror::Error;

const MAX_REFERENCE_BYTES: usize = 192;
const MAX_TARGETS: usize = 128;
const MAX_CONTEXT_IDS: usize = 4096;

/// The operator-labelled learning signal.  Names intentionally match RFC-009 and the
/// C++ Cortex contract, but this Rust representation owns no parser or graph mutation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CorrectionKind {
    EntityCorrection,
    RelationCorrection,
    CoreferenceCorrection,
    RetrievalPositive,
    RetrievalNegative,
    MissingContextLabel,
    StaleKnowledgeCorrection,
    ContextPreference,
    AnswerFailure,
}

impl CorrectionKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::EntityCorrection => "entity_correction",
            Self::RelationCorrection => "relation_correction",
            Self::CoreferenceCorrection => "coreference_correction",
            Self::RetrievalPositive => "retrieval_positive",
            Self::RetrievalNegative => "retrieval_negative",
            Self::MissingContextLabel => "missing_context_label",
            Self::StaleKnowledgeCorrection => "stale_knowledge_correction",
            Self::ContextPreference => "context_preference",
            Self::AnswerFailure => "answer_failure",
        }
    }
}

/// A correction after text/voice ingestion has produced immutable vault references.
///
/// `raw_correction_ref` and `rationale_ref` point to the private vault or a derived,
/// policy-governed record. They are never raw text fields. This lets retention, consent
/// and deletion operate at the source while preserving an auditable training reference.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CorrectionEvent {
    pub correction_id: String,
    pub kind: CorrectionKind,
    pub operator_id: String,
    pub graph_snapshot_id: String,
    pub query_digest: String,
    pub raw_correction_ref: String,
    pub rationale_ref: String,
    pub target_ids: Vec<String>,
    pub recorded_at_ms: i64,
}

/// Immutable evidence that the authorized graph writer applied the factual part of a
/// correction. A Cortex worker cannot manufacture this receipt or mutate a snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphCorrectionReceipt {
    pub correction_id: String,
    pub source_graph_snapshot_id: String,
    pub resulting_graph_snapshot_id: String,
    pub graph_update_ref: String,
    pub applied_at_ms: i64,
}

/// One replayable, immutable supervised trajectory. Raw records remain in the vault;
/// the durable control-plane database receives only identifiers, versioned references
/// and SHA-256 digests.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CorrectionTrajectory {
    pub trajectory_id: String,
    pub correction: CorrectionEvent,
    pub graph_update: GraphCorrectionReceipt,
    pub selected_context_ids: Vec<String>,
    pub omitted_candidate_ids: Vec<String>,
    pub response_ref: String,
    pub response_digest: String,
    pub corrected_interpretation_ref: String,
    pub model_id: String,
    pub model_hash: String,
    pub captured_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CorrectionError {
    #[error("{field} must be a non-empty bounded opaque reference")]
    InvalidReference { field: &'static str },
    #[error("{field} must be a lower-case sha256:<64-hex> digest")]
    InvalidDigest { field: &'static str },
    #[error("{field} must be a positive millisecond timestamp")]
    InvalidTimestamp { field: &'static str },
    #[error("correction needs between one and {MAX_TARGETS} target identifiers")]
    InvalidTargets,
    #[error("{field} has duplicate identifiers or exceeds its bound")]
    InvalidIdentifiers { field: &'static str },
    #[error("the graph receipt does not bind to this correction and source snapshot")]
    ReceiptMismatch,
    #[error("the graph receipt must name a successor snapshot")]
    NonSuccessorSnapshot,
}

impl CorrectionEvent {
    pub fn validate(&self) -> Result<(), CorrectionError> {
        reference(&self.correction_id, "correction_id")?;
        reference(&self.operator_id, "operator_id")?;
        reference(&self.graph_snapshot_id, "graph_snapshot_id")?;
        digest(&self.query_digest, "query_digest")?;
        reference(&self.raw_correction_ref, "raw_correction_ref")?;
        reference(&self.rationale_ref, "rationale_ref")?;
        identifiers(&self.target_ids, "target_ids", 1, MAX_TARGETS)?;
        timestamp(self.recorded_at_ms, "recorded_at_ms")
    }
}

impl GraphCorrectionReceipt {
    pub fn validate(&self) -> Result<(), CorrectionError> {
        reference(&self.correction_id, "receipt.correction_id")?;
        reference(
            &self.source_graph_snapshot_id,
            "receipt.source_graph_snapshot_id",
        )?;
        reference(
            &self.resulting_graph_snapshot_id,
            "receipt.resulting_graph_snapshot_id",
        )?;
        reference(&self.graph_update_ref, "receipt.graph_update_ref")?;
        timestamp(self.applied_at_ms, "receipt.applied_at_ms")?;
        if self.source_graph_snapshot_id == self.resulting_graph_snapshot_id {
            return Err(CorrectionError::NonSuccessorSnapshot);
        }
        Ok(())
    }
}

impl CorrectionTrajectory {
    pub fn validate(&self) -> Result<(), CorrectionError> {
        reference(&self.trajectory_id, "trajectory_id")?;
        self.correction.validate()?;
        self.graph_update.validate()?;
        if self.graph_update.correction_id != self.correction.correction_id
            || self.graph_update.source_graph_snapshot_id != self.correction.graph_snapshot_id
            || self.graph_update.applied_at_ms < self.correction.recorded_at_ms
        {
            return Err(CorrectionError::ReceiptMismatch);
        }
        identifiers(
            &self.selected_context_ids,
            "selected_context_ids",
            0,
            MAX_CONTEXT_IDS,
        )?;
        identifiers(
            &self.omitted_candidate_ids,
            "omitted_candidate_ids",
            0,
            MAX_CONTEXT_IDS,
        )?;
        reference(&self.response_ref, "response_ref")?;
        digest(&self.response_digest, "response_digest")?;
        reference(
            &self.corrected_interpretation_ref,
            "corrected_interpretation_ref",
        )?;
        reference(&self.model_id, "model_id")?;
        digest(&self.model_hash, "model_hash")?;
        timestamp(self.captured_at_ms, "captured_at_ms")
    }
}

fn reference(value: &str, field: &'static str) -> Result<(), CorrectionError> {
    if value.is_empty()
        || value.len() > MAX_REFERENCE_BYTES
        || !value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b':' | b'.' | b'_' | b'-' | b'/')
        })
    {
        return Err(CorrectionError::InvalidReference { field });
    }
    Ok(())
}

fn digest(value: &str, field: &'static str) -> Result<(), CorrectionError> {
    let Some(hex) = value.strip_prefix("sha256:") else {
        return Err(CorrectionError::InvalidDigest { field });
    };
    if hex.len() != 64
        || !hex
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(CorrectionError::InvalidDigest { field });
    }
    Ok(())
}

fn timestamp(value: i64, field: &'static str) -> Result<(), CorrectionError> {
    if value <= 0 {
        return Err(CorrectionError::InvalidTimestamp { field });
    }
    Ok(())
}

fn identifiers(
    values: &[String],
    field: &'static str,
    minimum: usize,
    maximum: usize,
) -> Result<(), CorrectionError> {
    if values.len() < minimum || values.len() > maximum {
        return if field == "target_ids" {
            Err(CorrectionError::InvalidTargets)
        } else {
            Err(CorrectionError::InvalidIdentifiers { field })
        };
    }
    let mut unique = BTreeSet::new();
    for value in values {
        reference(value, field)?;
        if !unique.insert(value) {
            return Err(CorrectionError::InvalidIdentifiers { field });
        }
    }
    Ok(())
}

impl fmt::Display for CorrectionKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIGEST: &str = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn event() -> CorrectionEvent {
        CorrectionEvent {
            correction_id: "correction:0001".into(),
            kind: CorrectionKind::MissingContextLabel,
            operator_id: "operator:nelson".into(),
            graph_snapshot_id: "graph:before".into(),
            query_digest: DIGEST.into(),
            raw_correction_ref: "vault:corrections/0001".into(),
            rationale_ref: "knowledge:correction-rationale/0001".into(),
            target_ids: vec!["simulation:motor-x".into()],
            recorded_at_ms: 1_700_000_000_000,
        }
    }

    fn trajectory() -> CorrectionTrajectory {
        CorrectionTrajectory {
            trajectory_id: "trajectory:0001".into(),
            correction: event(),
            graph_update: GraphCorrectionReceipt {
                correction_id: "correction:0001".into(),
                source_graph_snapshot_id: "graph:before".into(),
                resulting_graph_snapshot_id: "graph:after".into(),
                graph_update_ref: "knowledge:update/0001".into(),
                applied_at_ms: 1_700_000_000_001,
            },
            selected_context_ids: vec!["project:robot-arm".into()],
            omitted_candidate_ids: vec!["simulation:motor-x".into()],
            response_ref: "vault:responses/0001".into(),
            response_digest: DIGEST.into(),
            corrected_interpretation_ref: "knowledge:interpretation/0001".into(),
            model_id: "deterministic-v1".into(),
            model_hash: DIGEST.into(),
            captured_at_ms: 1_700_000_000_002,
        }
    }

    #[test]
    fn trajectory_requires_a_graph_writer_receipt_before_training() {
        assert!(trajectory().validate().is_ok());
        let mut invalid = trajectory();
        invalid.graph_update.source_graph_snapshot_id = "graph:other".into();
        assert_eq!(invalid.validate(), Err(CorrectionError::ReceiptMismatch));

        let mut invalid = trajectory();
        invalid.graph_update.resulting_graph_snapshot_id = "graph:before".into();
        assert_eq!(
            invalid.validate(),
            Err(CorrectionError::NonSuccessorSnapshot)
        );
    }

    #[test]
    fn private_payloads_are_references_and_digests_not_free_text() {
        let mut invalid = event();
        invalid.raw_correction_ref = "the operator said this was wrong".into();
        assert!(matches!(
            invalid.validate(),
            Err(CorrectionError::InvalidReference {
                field: "raw_correction_ref"
            })
        ));

        let mut invalid = trajectory();
        invalid.response_digest = "not-a-digest".into();
        assert!(matches!(
            invalid.validate(),
            Err(CorrectionError::InvalidDigest {
                field: "response_digest"
            })
        ));
    }

    #[test]
    fn ids_are_bounded_and_duplicate_safe() {
        let mut invalid = trajectory();
        invalid
            .selected_context_ids
            .push("project:robot-arm".into());
        assert_eq!(
            invalid.validate(),
            Err(CorrectionError::InvalidIdentifiers {
                field: "selected_context_ids"
            })
        );
        let mut invalid = event();
        invalid.target_ids.clear();
        assert_eq!(invalid.validate(), Err(CorrectionError::InvalidTargets));
    }
}
