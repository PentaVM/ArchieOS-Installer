//! Deterministic clarification policy. Models may estimate inputs; this module decides.

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use thiserror::Error;

const ASK_THRESHOLD: i32 = 850;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClarificationAssessment {
    pub ambiguity: u16,
    pub missing_information: u16,
    pub decision_impact: u16,
    /// 1000 means easily reversible; 0 means irreversible.
    pub reversibility: u16,
    pub risk: u16,
    pub confidence: u16,
    pub cost_of_wrong_assumption: u16,
    pub cost_of_interrupting_user: u16,
    pub can_progress_without_answer: bool,
    pub expected_information_gain: u16,
    pub latency_cost: u16,
    pub external_effect_pending: bool,
    pub reason_code: String,
    pub refs_consulted: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ClarificationDecision {
    Continue,
    AskNow,
    AskWhenFloorAvailable,
    AskBeforeCommit,
    AskBeforeExternalEffect,
    DeferAndWork,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ClarificationError {
    #[error("clarification assessment is outside its bounded domain")]
    InvalidAssessment,
    #[error("clarification answer does not match the pending question")]
    UnexpectedAnswer,
}

impl ClarificationAssessment {
    pub fn validate(&self) -> Result<(), ClarificationError> {
        let bounded = [
            self.ambiguity,
            self.missing_information,
            self.decision_impact,
            self.reversibility,
            self.risk,
            self.confidence,
            self.cost_of_wrong_assumption,
            self.cost_of_interrupting_user,
            self.expected_information_gain,
            self.latency_cost,
        ]
        .iter()
        .all(|value| *value <= 1000);
        let reason = !self.reason_code.is_empty()
            && self.reason_code.len() <= 96
            && self
                .reason_code
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
        let refs = self.refs_consulted.len() <= 64
            && self
                .refs_consulted
                .iter()
                .all(|r| !r.is_empty() && r.len() <= 256 && !r.chars().any(char::is_whitespace));
        if bounded && reason && refs {
            Ok(())
        } else {
            Err(ClarificationError::InvalidAssessment)
        }
    }

    pub fn utility(&self) -> Result<i32, ClarificationError> {
        self.validate()?;
        let avoided_wrong =
            i32::from(self.cost_of_wrong_assumption) * i32::from(self.decision_impact) / 1000;
        let avoided_risk = i32::from(self.risk) * i32::from(1000 - self.reversibility) / 1000;
        Ok(
            i32::from(self.expected_information_gain) + avoided_wrong + avoided_risk
                - i32::from(self.cost_of_interrupting_user)
                - i32::from(self.latency_cost),
        )
    }

    /// `floor_available` is the trusted result of the existing deterministic floor policy.
    pub fn decide(
        &self,
        floor_available: bool,
    ) -> Result<ClarificationDecision, ClarificationError> {
        let utility = self.utility()?;
        let materially_missing = self.missing_information >= 500 && self.confidence < 800;
        if !materially_missing || utility < ASK_THRESHOLD {
            return Ok(
                if self.can_progress_without_answer && self.ambiguity >= 350 {
                    ClarificationDecision::DeferAndWork
                } else {
                    ClarificationDecision::Continue
                },
            );
        }
        if self.external_effect_pending {
            return Ok(ClarificationDecision::AskBeforeExternalEffect);
        }
        if self.reversibility <= 250 && self.decision_impact >= 600 {
            return Ok(ClarificationDecision::AskBeforeCommit);
        }
        if floor_available {
            Ok(ClarificationDecision::AskNow)
        } else {
            Ok(ClarificationDecision::AskWhenFloorAvailable)
        }
    }
}

/// Content-free persisted state. The question and answer bodies stay in the live session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ClarificationSession {
    pending_reason: Option<String>,
    asked_reasons: BTreeSet<String>,
    answer_refs: Vec<String>,
    confidence_delta: i16,
    task_outcome: Option<String>,
}

impl ClarificationSession {
    pub fn begin(
        &mut self,
        assessment: &ClarificationAssessment,
        floor_available: bool,
    ) -> Result<ClarificationDecision, ClarificationError> {
        let decision = assessment.decide(floor_available)?;
        if matches!(
            decision,
            ClarificationDecision::AskNow
                | ClarificationDecision::AskWhenFloorAvailable
                | ClarificationDecision::AskBeforeCommit
                | ClarificationDecision::AskBeforeExternalEffect
        ) {
            if self.asked_reasons.contains(&assessment.reason_code) {
                return Ok(ClarificationDecision::DeferAndWork);
            }
            self.pending_reason = Some(assessment.reason_code.clone());
            self.asked_reasons.insert(assessment.reason_code.clone());
        }
        Ok(decision)
    }

    pub fn answer(
        &mut self,
        reason_code: &str,
        answer_ref: String,
        confidence_delta: i16,
    ) -> Result<(), ClarificationError> {
        if self.pending_reason.as_deref() != Some(reason_code)
            || answer_ref.is_empty()
            || answer_ref.len() > 256
            || answer_ref.chars().any(char::is_whitespace)
            || !(-1000..=1000).contains(&confidence_delta)
        {
            return Err(ClarificationError::UnexpectedAnswer);
        }
        self.pending_reason = None;
        self.answer_refs.push(answer_ref);
        self.confidence_delta = confidence_delta;
        Ok(())
    }

    pub fn can_resume(&self) -> bool {
        self.pending_reason.is_none()
    }
    pub fn finish(&mut self, outcome_code: &str) -> Result<(), ClarificationError> {
        if outcome_code.is_empty() || outcome_code.chars().any(char::is_whitespace) {
            return Err(ClarificationError::UnexpectedAnswer);
        }
        self.task_outcome = Some(outcome_code.into());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assessment() -> ClarificationAssessment {
        ClarificationAssessment {
            ambiguity: 800,
            missing_information: 900,
            decision_impact: 900,
            reversibility: 100,
            risk: 800,
            confidence: 300,
            cost_of_wrong_assumption: 900,
            cost_of_interrupting_user: 100,
            can_progress_without_answer: false,
            expected_information_gain: 900,
            latency_cost: 50,
            external_effect_pending: false,
            reason_code: "missing_radial_load".into(),
            refs_consulted: vec!["vault:spec-1".into()],
        }
    }

    #[test]
    fn clear_task_does_not_ask() {
        let mut a = assessment();
        a.ambiguity = 0;
        a.missing_information = 0;
        a.confidence = 1000;
        assert_eq!(a.decide(true).unwrap(), ClarificationDecision::Continue);
    }

    #[test]
    fn low_impact_ambiguity_can_progress() {
        let mut a = assessment();
        a.decision_impact = 100;
        a.cost_of_wrong_assumption = 100;
        a.risk = 0;
        a.expected_information_gain = 200;
        a.can_progress_without_answer = true;
        assert_eq!(a.decide(true).unwrap(), ClarificationDecision::DeferAndWork);
    }

    #[test]
    fn critical_missing_data_asks_before_commit() {
        assert_eq!(
            assessment().decide(true).unwrap(),
            ClarificationDecision::AskBeforeCommit
        );
    }

    #[test]
    fn continuing_speaker_defers_question() {
        let mut a = assessment();
        a.reversibility = 700;
        assert_eq!(
            a.decide(false).unwrap(),
            ClarificationDecision::AskWhenFloorAvailable
        );
    }

    #[test]
    fn answer_resumes_and_reason_does_not_loop() {
        let a = assessment();
        let mut session = ClarificationSession::default();
        assert_eq!(
            session.begin(&a, true).unwrap(),
            ClarificationDecision::AskBeforeCommit
        );
        assert!(!session.can_resume());
        session
            .answer("missing_radial_load", "answer:42".into(), 500)
            .unwrap();
        assert!(session.can_resume());
        assert_eq!(
            session.begin(&a, true).unwrap(),
            ClarificationDecision::DeferAndWork
        );
    }

    #[test]
    fn external_effect_has_explicit_boundary() {
        let mut a = assessment();
        a.external_effect_pending = true;
        assert_eq!(
            a.decide(true).unwrap(),
            ClarificationDecision::AskBeforeExternalEffect
        );
    }
}
