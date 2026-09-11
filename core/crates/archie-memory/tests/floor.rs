use archie_memory::floor::*;
fn state() -> ConversationalFloorState {
    ConversationalFloorState {
        observed_at_ms: 100,
        current_speaker_ref: Some("speaker:owner".into()),
        speaker_hold_probability: 0.0,
        thought_completion_probability: 1.0,
        expected_silence_horizon_ms: 1000,
        silence_scope_ref: None,
        interruption_utility: 0.0,
        response_utility: 0.8,
        urgency: 0.0,
        context_sufficiency: 1.0,
        predicted_intent_ref: None,
        intent_confidence: 0.5,
        resume_probability: 0.0,
        overlap: false,
        archie_speaking: false,
        explicit_question: false,
        requested_floor: false,
        acknowledgement_only: false,
        listen_preference: false,
    }
}
#[test]
fn okay_remains_silent_while_listening() {
    let mut s = state();
    s.acknowledgement_only = true;
    let d = decide_floor(&s, true, false, 100);
    assert!(d.can_speak);
    assert!(!d.should_speak);
    assert_eq!(d.output, FloorOutput::Silent);
}
#[test]
fn explanation_and_overlap_defer_response() {
    let mut s = state();
    s.explicit_question = true;
    s.listen_preference = true;
    assert_eq!(
        decide_floor(&s, true, false, 100).output,
        FloorOutput::Silent
    );
    s.listen_preference = false;
    s.overlap = true;
    assert_eq!(
        decide_floor(&s, true, false, 100).output,
        FloorOutput::Silent
    );
    s.overlap = false;
    assert_eq!(
        decide_floor(&s, true, false, 100).output,
        FloorOutput::ShortResponse
    );
}
#[test]
fn stale_invalid_and_disallowed_observations_fail_silent() {
    let mut s = state();
    s.explicit_question = true;
    assert!(!decide_floor(&s, false, true, 100).should_speak);
    assert!(!decide_floor(&s, true, false, 2101).should_speak);
    assert!(!decide_floor(&s, true, false, 99).should_speak);
    s.response_utility = f64::NAN;
    assert!(!decide_floor(&s, true, false, 100).should_speak);
}
#[test]
fn only_trusted_critical_alert_overrides_listen_preference() {
    let mut s = state();
    s.listen_preference = true;
    s.urgency = 1.0;
    s.interruption_utility = 1.0;
    assert_eq!(
        decide_floor(&s, true, false, 100).output,
        FloorOutput::Silent
    );
    assert_eq!(
        decide_floor(&s, true, true, 100).output,
        FloorOutput::StrongInterrupt
    );
}
