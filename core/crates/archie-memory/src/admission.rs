//! The learning-admission firewall between perception and model training.
//!
//! Sentry and Wanda may observe continuously. That does not make their stream a dataset.
//! This module reduces observations to bounded, provenance-only records and makes the
//! distinction mechanical: raw sensory payloads, repetition and unconfirmed inference can
//! never cross the type boundary into [`CuratedTrainingRecord`].

use crate::{cognitive::reference, graph::Sensitivity};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

const MAX_DIGEST_BYTES: usize = 71;
const ABSOLUTE_MAXIMUM_CANDIDATES: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SensorSource {
    Sentry,
    Wanda,
    OperatorCorrection,
    TaskRuntime,
    DerivedKnowledge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentClass {
    RawAudio,
    RawVideo,
    RawImage,
    Transcript,
    StructuredObservation,
    SanitizedAggregate,
    SocialHypothesis,
    CorrectionTrajectory,
    TaskOutcome,
}

impl ContentClass {
    const fn is_raw(self) -> bool {
        matches!(
            self,
            Self::RawAudio | Self::RawVideo | Self::RawImage | Self::Transcript
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Supervision {
    None,
    OperatorCorrection { graph_writer_receipt_ref: String },
    VerifiedTaskOutcome { outcome_receipt_ref: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LearningCandidate {
    pub candidate_id: String,
    pub source: SensorSource,
    pub source_ref: String,
    pub source_root: String,
    pub evidence_ref: String,
    pub evidence_sha256: String,
    pub semantic_key: String,
    pub content_class: ContentClass,
    pub sensitivity: Sensitivity,
    pub machine_analysis_allowed: bool,
    pub consent_required: bool,
    pub consent_granted: bool,
    pub supervision: Supervision,
    pub quality_permille: u16,
    pub relevance_permille: u16,
    pub novelty_permille: u16,
    pub observed_at_ms: i64,
    pub expires_at_ms: Option<i64>,
}

impl LearningCandidate {
    pub fn validate(&self) -> Result<(), &'static str> {
        for value in [
            &self.candidate_id,
            &self.source_ref,
            &self.source_root,
            &self.evidence_ref,
            &self.semantic_key,
        ] {
            if !reference(value) {
                return Err("candidate contains an invalid reference");
            }
        }
        if !valid_digest(&self.evidence_sha256) {
            return Err("candidate evidence digest is invalid");
        }
        if self.quality_permille > 1000
            || self.relevance_permille > 1000
            || self.novelty_permille > 1000
        {
            return Err("candidate scores must be whole permille");
        }
        if self.observed_at_ms <= 0
            || self
                .expires_at_ms
                .is_some_and(|expiry| expiry <= self.observed_at_ms)
        {
            return Err("candidate time or expiry is invalid");
        }
        let supervision_ref = match &self.supervision {
            Supervision::None => None,
            Supervision::OperatorCorrection {
                graph_writer_receipt_ref,
            } => Some(graph_writer_receipt_ref),
            Supervision::VerifiedTaskOutcome {
                outcome_receipt_ref,
            } => Some(outcome_receipt_ref),
        };
        if supervision_ref.is_some_and(|value| !reference(value)) {
            return Err("supervision receipt is invalid");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdmissionPolicy {
    pub max_batch_candidates: usize,
    pub max_training_records: usize,
    pub max_training_records_per_source: usize,
    pub minimum_quality_permille: u16,
    pub minimum_relevance_permille: u16,
    pub minimum_novelty_permille: u16,
    pub allow_local_personal_training: bool,
}

impl Default for AdmissionPolicy {
    fn default() -> Self {
        Self {
            max_batch_candidates: 4096,
            max_training_records: 128,
            max_training_records_per_source: 16,
            minimum_quality_permille: 500,
            minimum_relevance_permille: 400,
            minimum_novelty_permille: 250,
            allow_local_personal_training: false,
        }
    }
}

impl AdmissionPolicy {
    pub fn validate(&self) -> Result<(), AdmissionError> {
        if self.max_batch_candidates == 0
            || self.max_batch_candidates > ABSOLUTE_MAXIMUM_CANDIDATES
            || self.max_training_records == 0
            || self.max_training_records > self.max_batch_candidates
            || self.max_training_records_per_source == 0
            || self.max_training_records_per_source > self.max_training_records
            || self.minimum_quality_permille > 1000
            || self.minimum_relevance_permille > 1000
            || self.minimum_novelty_permille > 1000
        {
            return Err(AdmissionError::InvalidPolicy);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryDisposition {
    Discard,
    WorkingSet,
    Episodic,
    Training,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdmissionReason {
    RawSensoryPayload,
    ConsentAbsent,
    MachineAnalysisForbidden,
    Expired,
    BiometricOrSecret,
    WandaIsUnconfirmed,
    ObservationIsNotSupervision,
    UntrustedSupervision,
    UnverifiedReceipt,
    PersonalTrainingDisabled,
    LowQuality,
    LowRelevance,
    LowNovelty,
    SemanticDuplicate,
    PreviouslyAdmitted,
    SourceQuota,
    BatchQuota,
    SanitizedContext,
    ExplicitVerifiedSupervision,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdmissionDecision {
    pub candidate_id: String,
    pub disposition: MemoryDisposition,
    pub reason: AdmissionReason,
}

/// The only type a trainer is allowed to consume. It cannot contain audio, images,
/// transcripts or free-form sensor content: only immutable references and digests.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CuratedTrainingRecord {
    candidate_id: String,
    source: SensorSource,
    source_ref: String,
    evidence_ref: String,
    evidence_sha256: String,
    semantic_key: String,
    supervision_ref: String,
    observed_at_ms: i64,
}

impl CuratedTrainingRecord {
    pub fn candidate_id(&self) -> &str {
        &self.candidate_id
    }

    pub fn source(&self) -> SensorSource {
        self.source
    }

    pub fn evidence_ref(&self) -> &str {
        &self.evidence_ref
    }

    pub fn supervision_ref(&self) -> &str {
        &self.supervision_ref
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdmissionBatch {
    decisions: Vec<AdmissionDecision>,
    training_records: Vec<CuratedTrainingRecord>,
    reason_counts: BTreeMap<AdmissionReason, usize>,
}

impl AdmissionBatch {
    pub fn decisions(&self) -> &[AdmissionDecision] {
        &self.decisions
    }

    pub fn training_records(&self) -> &[CuratedTrainingRecord] {
        &self.training_records
    }

    pub fn into_training_records(self) -> Vec<CuratedTrainingRecord> {
        self.training_records
    }

    pub fn count(&self, reason: AdmissionReason) -> usize {
        self.reason_counts.get(&reason).copied().unwrap_or(0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum AdmissionError {
    #[error("learning admission policy is invalid")]
    InvalidPolicy,
    #[error("batch has {actual} candidates; policy permits {maximum}")]
    BatchTooLarge { actual: usize, maximum: usize },
    #[error("candidate {index} is invalid: {reason}")]
    InvalidCandidate { index: usize, reason: &'static str },
}

/// Select a small deterministic training set while retaining an audit decision for every
/// input. The caller may persist Episodic records under its own retention policy; only
/// `training_records` may cross into a dataset builder.
pub fn admit_learning(
    candidates: &[LearningCandidate],
    policy: &AdmissionPolicy,
    now_ms: i64,
    verified_supervision_receipts: &BTreeSet<String>,
    previously_admitted_evidence: &BTreeSet<String>,
) -> Result<AdmissionBatch, AdmissionError> {
    policy.validate()?;
    if candidates.len() > policy.max_batch_candidates {
        return Err(AdmissionError::BatchTooLarge {
            actual: candidates.len(),
            maximum: policy.max_batch_candidates,
        });
    }
    for (index, candidate) in candidates.iter().enumerate() {
        candidate
            .validate()
            .map_err(|reason| AdmissionError::InvalidCandidate { index, reason })?;
    }

    let mut decisions: Vec<Option<AdmissionDecision>> = vec![None; candidates.len()];
    let mut eligible = Vec::new();
    for (index, candidate) in candidates.iter().enumerate() {
        if let Some((disposition, reason)) = preflight(
            candidate,
            policy,
            now_ms,
            verified_supervision_receipts,
            previously_admitted_evidence,
        ) {
            decisions[index] = Some(decision(candidate, disposition, reason));
        } else {
            eligible.push(index);
        }
    }

    // Highest information value first, with explicit deterministic tie breakers. Input
    // order cannot give a noisy source priority over a later correction.
    eligible.sort_by(|left, right| {
        let a = &candidates[*left];
        let b = &candidates[*right];
        supervision_rank(&b.supervision)
            .cmp(&supervision_rank(&a.supervision))
            .then_with(|| b.relevance_permille.cmp(&a.relevance_permille))
            .then_with(|| b.novelty_permille.cmp(&a.novelty_permille))
            .then_with(|| b.quality_permille.cmp(&a.quality_permille))
            .then_with(|| b.observed_at_ms.cmp(&a.observed_at_ms))
            .then_with(|| a.candidate_id.cmp(&b.candidate_id))
    });

    let mut records = Vec::new();
    let mut semantic_keys = BTreeSet::new();
    let mut source_counts: BTreeMap<&str, usize> = BTreeMap::new();
    for index in eligible {
        let candidate = &candidates[index];
        let reason = if semantic_keys.contains(&candidate.semantic_key) {
            Some(AdmissionReason::SemanticDuplicate)
        } else if source_counts
            .get(candidate.source_root.as_str())
            .copied()
            .unwrap_or(0)
            >= policy.max_training_records_per_source
        {
            Some(AdmissionReason::SourceQuota)
        } else if records.len() >= policy.max_training_records {
            Some(AdmissionReason::BatchQuota)
        } else {
            None
        };
        if let Some(reason) = reason {
            decisions[index] = Some(decision(candidate, MemoryDisposition::Episodic, reason));
            continue;
        }

        semantic_keys.insert(candidate.semantic_key.clone());
        *source_counts
            .entry(candidate.source_root.as_str())
            .or_default() += 1;
        records.push(training_record(candidate));
        decisions[index] = Some(decision(
            candidate,
            MemoryDisposition::Training,
            AdmissionReason::ExplicitVerifiedSupervision,
        ));
    }

    let decisions: Vec<AdmissionDecision> = decisions
        .into_iter()
        .map(|item| item.expect("every validated candidate receives a decision"))
        .collect();
    let mut reason_counts = BTreeMap::new();
    for item in &decisions {
        *reason_counts.entry(item.reason).or_default() += 1;
    }
    Ok(AdmissionBatch {
        decisions,
        training_records: records,
        reason_counts,
    })
}

fn preflight(
    candidate: &LearningCandidate,
    policy: &AdmissionPolicy,
    now_ms: i64,
    verified_supervision_receipts: &BTreeSet<String>,
    previously_admitted_evidence: &BTreeSet<String>,
) -> Option<(MemoryDisposition, AdmissionReason)> {
    if candidate.content_class.is_raw() {
        return Some((
            MemoryDisposition::Discard,
            AdmissionReason::RawSensoryPayload,
        ));
    }
    if candidate.consent_required && !candidate.consent_granted {
        return Some((MemoryDisposition::Discard, AdmissionReason::ConsentAbsent));
    }
    if !candidate.machine_analysis_allowed {
        return Some((
            MemoryDisposition::Discard,
            AdmissionReason::MachineAnalysisForbidden,
        ));
    }
    if candidate
        .expires_at_ms
        .is_some_and(|expiry| expiry <= now_ms)
    {
        return Some((MemoryDisposition::Discard, AdmissionReason::Expired));
    }
    if matches!(
        candidate.sensitivity,
        Sensitivity::Biometric | Sensitivity::Secret
    ) {
        return Some((
            MemoryDisposition::Discard,
            AdmissionReason::BiometricOrSecret,
        ));
    }
    // This set comes from the durable dataset index. Batch-local semantic collapse is
    // insufficient: a loud sensor could otherwise present the same corrected sample in
    // every batch and slowly fill the training corpus with repetitions.
    if previously_admitted_evidence.contains(&candidate.evidence_sha256) {
        return Some((
            MemoryDisposition::Episodic,
            AdmissionReason::PreviouslyAdmitted,
        ));
    }
    if candidate.source == SensorSource::Wanda {
        return Some((
            MemoryDisposition::WorkingSet,
            AdmissionReason::WandaIsUnconfirmed,
        ));
    }

    let unsupervised = matches!(candidate.supervision, Supervision::None);
    if candidate.quality_permille < policy.minimum_quality_permille {
        return Some((
            if unsupervised {
                MemoryDisposition::Discard
            } else {
                MemoryDisposition::Episodic
            },
            AdmissionReason::LowQuality,
        ));
    }
    if candidate.relevance_permille < policy.minimum_relevance_permille {
        return Some((
            if unsupervised {
                MemoryDisposition::Discard
            } else {
                MemoryDisposition::Episodic
            },
            AdmissionReason::LowRelevance,
        ));
    }
    if candidate.novelty_permille < policy.minimum_novelty_permille {
        return Some((
            if unsupervised {
                MemoryDisposition::Discard
            } else {
                MemoryDisposition::Episodic
            },
            AdmissionReason::LowNovelty,
        ));
    }

    let trusted_supervision = match (
        &candidate.source,
        &candidate.content_class,
        &candidate.supervision,
    ) {
        (
            SensorSource::OperatorCorrection,
            ContentClass::CorrectionTrajectory,
            Supervision::OperatorCorrection {
                graph_writer_receipt_ref,
            },
        ) => {
            if !verified_supervision_receipts.contains(graph_writer_receipt_ref) {
                return Some((
                    MemoryDisposition::Episodic,
                    AdmissionReason::UnverifiedReceipt,
                ));
            }
            true
        }
        (
            SensorSource::TaskRuntime,
            ContentClass::TaskOutcome,
            Supervision::VerifiedTaskOutcome {
                outcome_receipt_ref,
            },
        ) => {
            if !verified_supervision_receipts.contains(outcome_receipt_ref) {
                return Some((
                    MemoryDisposition::Episodic,
                    AdmissionReason::UnverifiedReceipt,
                ));
            }
            true
        }
        (_, _, Supervision::None) => false,
        _ => {
            return Some((
                MemoryDisposition::Episodic,
                AdmissionReason::UntrustedSupervision,
            ))
        }
    };
    if !trusted_supervision {
        return Some((
            if candidate.source == SensorSource::Sentry {
                MemoryDisposition::WorkingSet
            } else if matches!(
                candidate.content_class,
                ContentClass::StructuredObservation | ContentClass::SanitizedAggregate
            ) {
                MemoryDisposition::Episodic
            } else {
                MemoryDisposition::Discard
            },
            if candidate.source == SensorSource::Sentry {
                AdmissionReason::ObservationIsNotSupervision
            } else {
                AdmissionReason::SanitizedContext
            },
        ));
    }
    if matches!(candidate.sensitivity, Sensitivity::Location)
        || (candidate.sensitivity == Sensitivity::PersonalPrivate
            && !policy.allow_local_personal_training)
    {
        return Some((
            MemoryDisposition::Episodic,
            AdmissionReason::PersonalTrainingDisabled,
        ));
    }
    None
}

fn supervision_rank(supervision: &Supervision) -> u8 {
    match supervision {
        Supervision::OperatorCorrection { .. } => 2,
        Supervision::VerifiedTaskOutcome { .. } => 1,
        Supervision::None => 0,
    }
}

fn decision(
    candidate: &LearningCandidate,
    disposition: MemoryDisposition,
    reason: AdmissionReason,
) -> AdmissionDecision {
    AdmissionDecision {
        candidate_id: candidate.candidate_id.clone(),
        disposition,
        reason,
    }
}

fn training_record(candidate: &LearningCandidate) -> CuratedTrainingRecord {
    let supervision_ref = match &candidate.supervision {
        Supervision::OperatorCorrection {
            graph_writer_receipt_ref,
        } => graph_writer_receipt_ref.clone(),
        Supervision::VerifiedTaskOutcome {
            outcome_receipt_ref,
        } => outcome_receipt_ref.clone(),
        Supervision::None => unreachable!("preflight excludes unsupervised candidates"),
    };
    CuratedTrainingRecord {
        candidate_id: candidate.candidate_id.clone(),
        source: candidate.source,
        source_ref: candidate.source_ref.clone(),
        evidence_ref: candidate.evidence_ref.clone(),
        evidence_sha256: candidate.evidence_sha256.clone(),
        semantic_key: candidate.semantic_key.clone(),
        supervision_ref,
        observed_at_ms: candidate.observed_at_ms,
    }
}

fn valid_digest(value: &str) -> bool {
    value.len() == MAX_DIGEST_BYTES
        && value.strip_prefix("sha256:").is_some_and(|hex| {
            hex.bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        })
}
