//! Bounded analytical synthesis around future neural heads.
//!
//! Models may rank or interpret evidence, but volume, confidence text and repeated
//! republication do not establish a fact. This module gives every independent source at
//! most one vote, preserves disagreement and makes abstention an ordinary result.

use crate::cognitive::reference;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

const MAX_DIGEST_BYTES: usize = 71;
const ABSOLUTE_MAXIMUM_SIGNALS: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceStance {
    Supports,
    Contradicts,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnalyticalSignal {
    pub signal_id: String,
    pub hypothesis_id: String,
    pub evidence_ref: String,
    pub evidence_sha256: String,
    pub source_root: String,
    pub semantic_key: String,
    pub stance: EvidenceStance,
    pub reliability_permille: u16,
    pub relevance_permille: u16,
    pub observed_at_ms: i64,
    pub expires_at_ms: Option<i64>,
    pub machine_analysis_allowed: bool,
    pub consent_required: bool,
    pub consent_granted: bool,
}

impl AnalyticalSignal {
    fn validate(&self, expected_hypothesis: &str) -> Result<(), &'static str> {
        for value in [
            &self.signal_id,
            &self.hypothesis_id,
            &self.evidence_ref,
            &self.source_root,
            &self.semantic_key,
        ] {
            if !reference(value) {
                return Err("signal contains an invalid reference");
            }
        }
        if self.hypothesis_id != expected_hypothesis {
            return Err("signal belongs to a different hypothesis");
        }
        if !valid_digest(&self.evidence_sha256) {
            return Err("signal evidence digest is invalid");
        }
        if self.reliability_permille > 1000 || self.relevance_permille > 1000 {
            return Err("signal scores must be whole permille");
        }
        if self.observed_at_ms <= 0
            || self
                .expires_at_ms
                .is_some_and(|expiry| expiry <= self.observed_at_ms)
        {
            return Err("signal time or expiry is invalid");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnalysisPolicy {
    pub maximum_signals: usize,
    pub maximum_age_ms: i64,
    pub minimum_reliability_permille: u16,
    pub minimum_relevance_permille: u16,
    pub minimum_independent_sources: usize,
    pub minimum_model_score_permille: u16,
}

impl Default for AnalysisPolicy {
    fn default() -> Self {
        Self {
            maximum_signals: 4096,
            maximum_age_ms: 24 * 60 * 60 * 1000,
            minimum_reliability_permille: 500,
            minimum_relevance_permille: 400,
            minimum_independent_sources: 2,
            minimum_model_score_permille: 600,
        }
    }
}

impl AnalysisPolicy {
    fn validate(&self) -> Result<(), AnalysisError> {
        if self.maximum_signals == 0
            || self.maximum_signals > ABSOLUTE_MAXIMUM_SIGNALS
            || self.maximum_age_ms <= 0
            || self.minimum_reliability_permille > 1000
            || self.minimum_relevance_permille > 1000
            || self.minimum_model_score_permille > 1000
            || self.minimum_independent_sources < 2
            || self.minimum_independent_sources > self.maximum_signals
        {
            return Err(AnalysisError::InvalidPolicy);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SignalReason {
    Used,
    MachineAnalysisForbidden,
    ConsentAbsent,
    Expired,
    Stale,
    LowReliability,
    LowRelevance,
    NoStance,
    DuplicateSource,
    SourceContradiction,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignalDecision {
    pub signal_id: String,
    pub reason: SignalReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssessmentState {
    Unavailable,
    InsufficientEvidence,
    Supported,
    Refuted,
    Disputed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AnalyticalAssessment {
    hypothesis_id: String,
    state: AssessmentState,
    support_strength_permille: u16,
    contradiction_strength_permille: u16,
    independent_support_sources: usize,
    independent_contradiction_sources: usize,
    conflicted_sources: usize,
    required_independent_sources: usize,
    minimum_model_score_permille: u16,
    evidence_refs: Vec<String>,
    decisions: Vec<SignalDecision>,
    evaluated_at_ms: i64,
}

impl AnalyticalAssessment {
    pub fn hypothesis_id(&self) -> &str {
        &self.hypothesis_id
    }

    pub fn state(&self) -> AssessmentState {
        self.state
    }

    pub fn support_strength_permille(&self) -> u16 {
        self.support_strength_permille
    }

    pub fn contradiction_strength_permille(&self) -> u16 {
        self.contradiction_strength_permille
    }

    pub fn independent_support_sources(&self) -> usize {
        self.independent_support_sources
    }

    pub fn independent_contradiction_sources(&self) -> usize {
        self.independent_contradiction_sources
    }

    pub fn conflicted_sources(&self) -> usize {
        self.conflicted_sources
    }

    pub fn evidence_refs(&self) -> &[String] {
        &self.evidence_refs
    }

    pub fn decisions(&self) -> &[SignalDecision] {
        &self.decisions
    }

    pub fn evaluated_at_ms(&self) -> i64 {
        self.evaluated_at_ms
    }

    pub fn independent_sources(&self) -> usize {
        self.independent_support_sources + self.independent_contradiction_sources
    }

    pub fn requires_human_review(&self) -> bool {
        self.state == AssessmentState::Disputed
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum AnalysisError {
    #[error("analysis policy is invalid")]
    InvalidPolicy,
    #[error("analysis time or hypothesis reference is invalid")]
    InvalidRequest,
    #[error("analysis received {actual} signals; policy permits {maximum}")]
    TooManySignals { actual: usize, maximum: usize },
    #[error("analytical signal {index} is invalid: {reason}")]
    InvalidSignal { index: usize, reason: &'static str },
}

/// Fuse sanitized evidence without allowing repetition to become authority.
pub fn assess_hypothesis(
    hypothesis_id: &str,
    signals: &[AnalyticalSignal],
    policy: &AnalysisPolicy,
    now_ms: i64,
) -> Result<AnalyticalAssessment, AnalysisError> {
    policy.validate()?;
    if now_ms <= 0 || !reference(hypothesis_id) {
        return Err(AnalysisError::InvalidRequest);
    }
    if signals.len() > policy.maximum_signals {
        return Err(AnalysisError::TooManySignals {
            actual: signals.len(),
            maximum: policy.maximum_signals,
        });
    }
    for (index, signal) in signals.iter().enumerate() {
        signal
            .validate(hypothesis_id)
            .map_err(|reason| AnalysisError::InvalidSignal { index, reason })?;
    }

    let mut decisions: Vec<Option<SignalDecision>> = vec![None; signals.len()];
    let mut candidates: BTreeMap<(String, EvidenceStance), Vec<usize>> = BTreeMap::new();
    for (index, signal) in signals.iter().enumerate() {
        let reason = filter_reason(signal, policy, now_ms);
        if let Some(reason) = reason {
            decisions[index] = Some(signal_decision(signal, reason));
        } else {
            candidates
                .entry((signal.source_root.clone(), signal.stance))
                .or_default()
                .push(index);
        }
    }

    let mut representatives: BTreeMap<(String, EvidenceStance), usize> = BTreeMap::new();
    for (key, mut indices) in candidates {
        indices.sort_by(|left, right| {
            compare_signal(&signals[*left], &signals[*right], now_ms, policy)
        });
        let representative = indices[0];
        representatives.insert(key, representative);
        for index in indices.into_iter().skip(1) {
            decisions[index] = Some(signal_decision(
                &signals[index],
                SignalReason::DuplicateSource,
            ));
        }
    }

    let roots: BTreeSet<String> = representatives
        .keys()
        .map(|(root, _)| root.clone())
        .collect();
    let mut support = Vec::new();
    let mut contradiction = Vec::new();
    let mut conflicted_sources = 0;
    for root in roots {
        let supports = representatives.get(&(root.clone(), EvidenceStance::Supports));
        let contradicts = representatives.get(&(root, EvidenceStance::Contradicts));
        match (supports, contradicts) {
            (Some(left), Some(right)) => {
                conflicted_sources += 1;
                decisions[*left] = Some(signal_decision(
                    &signals[*left],
                    SignalReason::SourceContradiction,
                ));
                decisions[*right] = Some(signal_decision(
                    &signals[*right],
                    SignalReason::SourceContradiction,
                ));
            }
            (Some(index), None) => support.push(*index),
            (None, Some(index)) => contradiction.push(*index),
            (None, None) => {}
        }
    }

    let mut evidence_refs = BTreeSet::new();
    for index in support.iter().chain(&contradiction) {
        decisions[*index] = Some(signal_decision(&signals[*index], SignalReason::Used));
        evidence_refs.insert(signals[*index].evidence_ref.clone());
    }
    let state = assessment_state(
        support.len(),
        contradiction.len(),
        conflicted_sources,
        policy.minimum_independent_sources,
    );
    Ok(AnalyticalAssessment {
        hypothesis_id: hypothesis_id.to_string(),
        state,
        support_strength_permille: average_strength(&support, signals, now_ms, policy),
        contradiction_strength_permille: average_strength(&contradiction, signals, now_ms, policy),
        independent_support_sources: support.len(),
        independent_contradiction_sources: contradiction.len(),
        conflicted_sources,
        required_independent_sources: policy.minimum_independent_sources,
        minimum_model_score_permille: policy.minimum_model_score_permille,
        evidence_refs: evidence_refs.into_iter().collect(),
        decisions: decisions
            .into_iter()
            .map(|decision| decision.expect("every validated signal receives a decision"))
            .collect(),
        evaluated_at_ms: now_ms,
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelProposal {
    pub proposal_id: String,
    pub hypothesis_id: String,
    pub model_id: String,
    pub model_sha256: String,
    pub stance: EvidenceStance,
    pub score_permille: u16,
    pub evidence_refs: Vec<String>,
    pub produced_at_ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalDisposition {
    Advisory,
    Abstain,
    Rejected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalReason {
    EvidenceAligned,
    EvidenceDisputed,
    InsufficientIndependentEvidence,
    UnsupportedEvidenceReference,
    InsufficientCitations,
    ModelUncertain,
    ContradictsAssessment,
    NoStance,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProposalDecision {
    pub proposal_id: String,
    pub disposition: ProposalDisposition,
    pub reason: ProposalReason,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ProposalError {
    #[error("model proposal is invalid")]
    InvalidProposal,
    #[error("model proposal and assessment refer to different hypotheses")]
    HypothesisMismatch,
}

/// A model can add an advisory interpretation. It cannot establish or erase evidence.
pub fn gate_model_proposal(
    proposal: &ModelProposal,
    assessment: &AnalyticalAssessment,
) -> Result<ProposalDecision, ProposalError> {
    if !reference(&proposal.proposal_id)
        || !reference(&proposal.hypothesis_id)
        || !reference(&proposal.model_id)
        || !valid_digest(&proposal.model_sha256)
        || proposal.score_permille > 1000
        || proposal.produced_at_ms <= 0
        || proposal.evidence_refs.is_empty()
        || proposal.evidence_refs.len() > 128
        || proposal.evidence_refs.iter().any(|item| !reference(item))
        || proposal.evidence_refs.iter().collect::<BTreeSet<_>>().len()
            != proposal.evidence_refs.len()
    {
        return Err(ProposalError::InvalidProposal);
    }
    if proposal.hypothesis_id != assessment.hypothesis_id {
        return Err(ProposalError::HypothesisMismatch);
    }
    if proposal.stance == EvidenceStance::Unknown {
        return Ok(proposal_decision(
            proposal,
            ProposalDisposition::Abstain,
            ProposalReason::NoStance,
        ));
    }
    let admitted: BTreeSet<&str> = assessment
        .evidence_refs
        .iter()
        .map(String::as_str)
        .collect();
    if proposal
        .evidence_refs
        .iter()
        .any(|item| !admitted.contains(item.as_str()))
    {
        return Ok(proposal_decision(
            proposal,
            ProposalDisposition::Rejected,
            ProposalReason::UnsupportedEvidenceReference,
        ));
    }
    let (disposition, reason) = match assessment.state {
        AssessmentState::Disputed => (
            ProposalDisposition::Abstain,
            ProposalReason::EvidenceDisputed,
        ),
        AssessmentState::Unavailable | AssessmentState::InsufficientEvidence => (
            ProposalDisposition::Abstain,
            ProposalReason::InsufficientIndependentEvidence,
        ),
        AssessmentState::Supported | AssessmentState::Refuted
            if proposal.evidence_refs.len() < assessment.required_independent_sources =>
        {
            (
                ProposalDisposition::Abstain,
                ProposalReason::InsufficientCitations,
            )
        }
        AssessmentState::Supported | AssessmentState::Refuted
            if proposal.score_permille < assessment.minimum_model_score_permille =>
        {
            (ProposalDisposition::Abstain, ProposalReason::ModelUncertain)
        }
        AssessmentState::Supported if proposal.stance == EvidenceStance::Supports => (
            ProposalDisposition::Advisory,
            ProposalReason::EvidenceAligned,
        ),
        AssessmentState::Refuted if proposal.stance == EvidenceStance::Contradicts => (
            ProposalDisposition::Advisory,
            ProposalReason::EvidenceAligned,
        ),
        AssessmentState::Supported | AssessmentState::Refuted => (
            ProposalDisposition::Rejected,
            ProposalReason::ContradictsAssessment,
        ),
    };
    Ok(proposal_decision(proposal, disposition, reason))
}

fn filter_reason(
    signal: &AnalyticalSignal,
    policy: &AnalysisPolicy,
    now_ms: i64,
) -> Option<SignalReason> {
    if !signal.machine_analysis_allowed {
        return Some(SignalReason::MachineAnalysisForbidden);
    }
    if signal.consent_required && !signal.consent_granted {
        return Some(SignalReason::ConsentAbsent);
    }
    if signal.expires_at_ms.is_some_and(|expiry| expiry <= now_ms) {
        return Some(SignalReason::Expired);
    }
    if signal.observed_at_ms > now_ms || now_ms - signal.observed_at_ms > policy.maximum_age_ms {
        return Some(SignalReason::Stale);
    }
    if signal.reliability_permille < policy.minimum_reliability_permille {
        return Some(SignalReason::LowReliability);
    }
    if signal.relevance_permille < policy.minimum_relevance_permille {
        return Some(SignalReason::LowRelevance);
    }
    if signal.stance == EvidenceStance::Unknown {
        return Some(SignalReason::NoStance);
    }
    None
}

fn compare_signal(
    left: &AnalyticalSignal,
    right: &AnalyticalSignal,
    now_ms: i64,
    policy: &AnalysisPolicy,
) -> std::cmp::Ordering {
    effective_strength(right, now_ms, policy)
        .cmp(&effective_strength(left, now_ms, policy))
        .then_with(|| right.observed_at_ms.cmp(&left.observed_at_ms))
        .then_with(|| left.signal_id.cmp(&right.signal_id))
}

fn effective_strength(signal: &AnalyticalSignal, now_ms: i64, policy: &AnalysisPolicy) -> u16 {
    let age = now_ms - signal.observed_at_ms;
    let recency = if age <= policy.maximum_age_ms / 4 {
        1000_u32
    } else if age <= policy.maximum_age_ms / 2 {
        850
    } else if age <= policy.maximum_age_ms.saturating_mul(3) / 4 {
        700
    } else {
        550
    };
    let weighted =
        u32::from(signal.reliability_permille) * u32::from(signal.relevance_permille) * recency;
    (weighted / 1_000_000) as u16
}

fn average_strength(
    indices: &[usize],
    signals: &[AnalyticalSignal],
    now_ms: i64,
    policy: &AnalysisPolicy,
) -> u16 {
    if indices.is_empty() {
        return 0;
    }
    let total: u64 = indices
        .iter()
        .map(|index| u64::from(effective_strength(&signals[*index], now_ms, policy)))
        .sum();
    (total / indices.len() as u64) as u16
}

fn assessment_state(
    support: usize,
    contradiction: usize,
    conflicts: usize,
    minimum: usize,
) -> AssessmentState {
    if conflicts > 0 || (support > 0 && contradiction > 0) {
        AssessmentState::Disputed
    } else if support >= minimum {
        AssessmentState::Supported
    } else if contradiction >= minimum {
        AssessmentState::Refuted
    } else if support + contradiction == 0 {
        AssessmentState::Unavailable
    } else {
        AssessmentState::InsufficientEvidence
    }
}

fn signal_decision(signal: &AnalyticalSignal, reason: SignalReason) -> SignalDecision {
    SignalDecision {
        signal_id: signal.signal_id.clone(),
        reason,
    }
}

fn proposal_decision(
    proposal: &ModelProposal,
    disposition: ProposalDisposition,
    reason: ProposalReason,
) -> ProposalDecision {
    ProposalDecision {
        proposal_id: proposal.proposal_id.clone(),
        disposition,
        reason,
    }
}

fn valid_digest(value: &str) -> bool {
    value.len() == MAX_DIGEST_BYTES
        && value.strip_prefix("sha256:").is_some_and(|hex| {
            hex.bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        })
}
