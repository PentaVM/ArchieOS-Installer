//! Conservative replayable floor policy. Input estimates are not learned here.
//! This chooses a speech candidate; it never authorizes a tool or physical action.
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum FloorOutput {
    Silent,
    Backchannel,
    ShortResponse,
    SoftInterrupt,
    StrongInterrupt,
    TakeFloor,
    ContinueSpeaking,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConversationalFloorState {
    pub observed_at_ms: i64,
    pub current_speaker_ref: Option<String>,
    pub speaker_hold_probability: f64,
    pub thought_completion_probability: f64,
    pub expected_silence_horizon_ms: u32,
    pub silence_scope_ref: Option<String>,
    pub interruption_utility: f64,
    pub response_utility: f64,
    pub urgency: f64,
    pub context_sufficiency: f64,
    pub predicted_intent_ref: Option<String>,
    pub intent_confidence: f64,
    pub resume_probability: f64,
    pub overlap: bool,
    pub archie_speaking: bool,
    pub explicit_question: bool,
    pub requested_floor: bool,
    pub acknowledgement_only: bool,
    pub listen_preference: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FloorDecision {
    pub can_speak: bool,
    pub should_speak: bool,
    pub output: FloorOutput,
    pub reason: &'static str,
}
impl ConversationalFloorState {
    pub fn validate(&self) -> Result<(), String> {
        if self.observed_at_ms <= 0
            || self.expected_silence_horizon_ms > 3_600_000
            || [
                self.speaker_hold_probability,
                self.thought_completion_probability,
                self.urgency,
                self.context_sufficiency,
                self.intent_confidence,
                self.resume_probability,
            ]
            .iter()
            .any(|p| !p.is_finite() || !(0.0..=1.0).contains(p))
            || [self.interruption_utility, self.response_utility]
                .iter()
                .any(|p| !p.is_finite() || !(-1.0..=1.0).contains(p))
            || [
                &self.current_speaker_ref,
                &self.silence_scope_ref,
                &self.predicted_intent_ref,
            ]
            .iter()
            .any(|r| r.as_ref().is_some_and(|r| !crate::cognitive::reference(r)))
        {
            return Err("invalid conversational floor observation".into());
        }
        Ok(())
    }
}
/// `can_speak` and critical alert are supplied by trusted runtime policy, not by
/// the estimator. Stale observations fail silent even if a previous floor was free.
pub fn decide_floor(
    state: &ConversationalFloorState,
    can_speak: bool,
    critical_alert: bool,
    now_ms: i64,
) -> FloorDecision {
    let choice = |output, reason| FloorDecision {
        can_speak,
        should_speak: output != FloorOutput::Silent,
        output,
        reason,
    };
    if !can_speak {
        return choice(FloorOutput::Silent, "speech unavailable or disallowed");
    }
    if state.validate().is_err()
        || now_ms < state.observed_at_ms
        || now_ms.saturating_sub(state.observed_at_ms) > 2000
    {
        return choice(FloorOutput::Silent, "invalid or stale observation");
    }
    if critical_alert {
        return choice(FloorOutput::StrongInterrupt, "trusted critical alert");
    }
    if state.acknowledgement_only && !state.explicit_question && !state.requested_floor {
        return choice(
            FloorOutput::Silent,
            "acknowledgement adds no response demand",
        );
    }
    if state.listen_preference {
        return choice(FloorOutput::Silent, "retain context while user explains");
    }
    if state.context_sufficiency < 0.5 || state.response_utility <= 0.0 {
        return choice(
            FloorOutput::Silent,
            "insufficient context or nonpositive response utility",
        );
    }
    if state.overlap || state.speaker_hold_probability >= 0.5 || state.resume_probability >= 0.5 {
        if state.interruption_utility > 0.75 && state.urgency > 0.75 {
            return choice(
                FloorOutput::SoftInterrupt,
                "valuable bounded intervention candidate",
            );
        }
        return choice(FloorOutput::Silent, "speaker likely continuing");
    }
    if state.archie_speaking {
        return choice(
            FloorOutput::ContinueSpeaking,
            "uninterrupted speech remains useful",
        );
    }
    if state.thought_completion_probability < 0.8 {
        return choice(FloorOutput::Silent, "thought not complete");
    }
    if state.explicit_question {
        return choice(FloorOutput::ShortResponse, "completed question");
    }
    if state.requested_floor {
        return choice(FloorOutput::TakeFloor, "explicitly requested contribution");
    }
    choice(
        FloorOutput::Silent,
        "no demonstrated demand for unsolicited speech",
    )
}
