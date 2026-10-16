//! The floor scenarios, written as the conversations they are.
//!
//! Each test is a moment in a real exchange. They are the acceptance criteria for the
//! whole speech plane: a system that passes them shares a conversation, and one that
//! fails any of them is a voice chatbot that can be interrupted.

use archie_speech::{
    ConversationalFloorState, FloorAction, FloorPolicy, OverlapState, Probability, SilenceScope,
    Speaker, SpeechBudget,
};

fn p(value: f32) -> Probability {
    Probability::new(value)
}

#[test]
fn wire_probabilities_cannot_bypass_the_floor_invariant() {
    for input in ["-0.2", "1.1", "1e38"] {
        assert!(
            serde_json::from_str::<Probability>(input).is_err(),
            "{input}"
        );
    }
    assert_eq!(
        serde_json::from_str::<Probability>("0.25").unwrap().get(),
        0.25
    );
}

#[test]
fn valid_remainder_does_not_resume_over_someone_else() {
    let mut state = floor_is_free_with_an_answer();
    assert!(FloorPolicy::default().may_resume(&state));
    state.current_speaker = Speaker::Owner;
    assert!(!FloorPolicy::default().may_resume(&state));
    state.current_speaker = Speaker::Nobody;
    state.speaker_hold = p(0.9);
    assert!(!FloorPolicy::default().may_resume(&state));
}

#[test]
fn invalid_policy_thresholds_fail_silent() {
    for threshold in [f32::NAN, f32::INFINITY, -0.1, 1.1] {
        let policy = FloorPolicy {
            speaker_hold_threshold: threshold,
            ..FloorPolicy::default()
        };
        assert_eq!(
            policy.decide(&floor_is_free_with_an_answer()).action,
            FloorAction::Silent
        );
    }
}

/// Nelson has asked a complete question and stopped.
fn floor_is_free_with_an_answer() -> ConversationalFloorState {
    ConversationalFloorState {
        current_speaker: Speaker::Nobody,
        thought_completion: p(0.9),
        silence_scope: SilenceScope::EndOfTurn,
        silence_ms: 400,
        response_utility: p(0.9),
        context_sufficiency: p(0.9),
        ..Default::default()
    }
}

// ------------------------------------------------------------------ 21.1 normal turn

/// A complete question gets an answer, and does not have to wait for a timer.
#[test]
fn a_finished_question_is_answered() {
    let decision = FloorPolicy::default().decide(&floor_is_free_with_an_answer());
    assert_eq!(decision.action, FloorAction::TakeFloor);
    assert_eq!(decision.budget, SpeechBudget::Full);
}

// --------------------------------------------------------- 21.2 pause inside a thought

/// "Move Postgres…" [pause] "…to the server, but keep the local backup."
///
/// The single most important test here. A silence timer answers during that pause, and
/// the person has to start again — which is the moment they decide the thing does not
/// listen.
#[test]
fn a_pause_inside_a_thought_does_not_hand_over_the_floor() {
    let mid_thought = ConversationalFloorState {
        current_speaker: Speaker::Nobody,
        speaker_hold: p(0.9),
        thought_completion: p(0.2),
        silence_scope: SilenceScope::WithinClause,
        // Long enough that any timer would have fired.
        silence_ms: 1_200,
        response_utility: p(0.9),
        context_sufficiency: p(0.9),
        ..Default::default()
    };

    let decision = FloorPolicy::default().decide(&mid_thought);
    assert_eq!(
        decision.action,
        FloorAction::Silent,
        "Archie finished somebody's sentence for them: {decision:?}"
    );
}

/// And a trailing "but…" holds the floor even when the clause sounded complete.
#[test]
fn a_promise_of_more_holds_the_floor() {
    let more_coming = ConversationalFloorState {
        current_speaker: Speaker::Nobody,
        speaker_hold: p(0.85),
        thought_completion: p(0.7),
        silence_scope: SilenceScope::BetweenClauses,
        silence_ms: 700,
        response_utility: p(0.9),
        context_sufficiency: p(0.9),
        ..Default::default()
    };

    assert_eq!(
        FloorPolicy::default().decide(&more_coming).action,
        FloorAction::Silent,
        "\"...but\" is a promise of more, whatever the pause length says"
    );
}

// ------------------------------------------------------------------ 21.3 acknowledgment

/// Archie: "I'm already migrating it."  Nelson: "Okay."
///
/// There is no question, no request and no new information. Answering it is noise.
#[test]
fn an_acknowledgement_gets_no_answer() {
    let acknowledged = ConversationalFloorState {
        current_speaker: Speaker::Nobody,
        thought_completion: p(0.95),
        silence_scope: SilenceScope::EndOfTurn,
        silence_ms: 500,
        acknowledgement_only: p(0.95),
        // Even with something sayable available.
        response_utility: p(0.8),
        context_sufficiency: p(0.9),
        ..Default::default()
    };

    let decision = FloorPolicy::default().decide(&acknowledged);
    assert_eq!(decision.action, FloorAction::Silent);
    assert!(
        decision.reason.contains("acknowledgement"),
        "the reason should name what it heard: {}",
        decision.reason
    );
}

// --------------------------------------------------------------- 21.4 barge-in correction

/// Archie is speaking. Nelson: "No, no, I mean the local server."
#[test]
fn a_correction_takes_the_floor_back() {
    let corrected = ConversationalFloorState {
        current_speaker: Speaker::Archie,
        overlap: OverlapState::UserOverArchie,
        acknowledgement_only: p(0.05),
        ..Default::default()
    };

    let decision = FloorPolicy::default().decide(&corrected);
    assert_eq!(decision.action, FloorAction::Yield);
    assert_eq!(decision.budget, SpeechBudget::None);
}

/// And what was being said is not resumed just because it was interrupted.
///
/// Between yielding and resuming, the user changed the question. Finishing the old
/// answer sounds exactly like not having listened.
#[test]
fn a_response_the_correction_invalidated_is_not_resumed() {
    let policy = FloorPolicy::default();

    let invalidated = ConversationalFloorState {
        response_still_valid: p(0.1),
        ..Default::default()
    };
    assert!(!policy.may_resume(&invalidated));

    // But an interruption that changed nothing leaves it resumable, or every "mhm"
    // would throw away a half-delivered answer.
    let untouched = ConversationalFloorState {
        response_still_valid: p(0.95),
        ..Default::default()
    };
    assert!(policy.may_resume(&untouched));
}

// ------------------------------------------------------------------ 21.5 benign overlap

/// Nelson says "yeah" while Archie is finishing a sentence.
///
/// Stopping here would make Archie unusable to anybody who makes listening noises, which
/// is most people.
#[test]
fn a_listening_noise_does_not_stop_archie() {
    let benign = ConversationalFloorState {
        current_speaker: Speaker::Archie,
        overlap: OverlapState::UserOverArchie,
        acknowledgement_only: p(0.9),
        ..Default::default()
    };

    assert_eq!(
        FloorPolicy::default().decide(&benign).action,
        FloorAction::ContinueSpeaking,
        "\"yeah\" is not a bid for the floor"
    );
}

/// The line between the two is the only thing separating this test from the last one.
#[test]
fn the_difference_between_yeah_and_a_correction_is_the_only_thing_that_matters_here() {
    let policy = FloorPolicy::default();
    let base = ConversationalFloorState {
        current_speaker: Speaker::Archie,
        overlap: OverlapState::UserOverArchie,
        ..Default::default()
    };

    let mut acknowledging = base.clone();
    acknowledging.acknowledgement_only = p(0.95);
    let mut correcting = base;
    correcting.acknowledgement_only = p(0.1);

    assert_eq!(
        policy.decide(&acknowledging).action,
        FloorAction::ContinueSpeaking
    );
    assert_eq!(policy.decide(&correcting).action, FloorAction::Yield);
}

// --------------------------------------------------------------------- 21.6 backchannel

/// Nelson is explaining something long. Archie signals presence without taking over.
#[test]
fn a_long_explanation_earns_a_backchannel_not_the_floor() {
    let explaining = ConversationalFloorState {
        current_speaker: Speaker::Owner,
        speaker_hold: p(0.85),
        silence_ms: 1_800,
        ..Default::default()
    };

    let decision = FloorPolicy::default().decide(&explaining);
    assert_eq!(decision.action, FloorAction::Backchannel);
    assert_eq!(
        decision.budget,
        SpeechBudget::Token,
        "a backchannel is a token, not a sentence"
    );
    assert!(!decision.action.is_interruption());
}

/// But not constantly — a backchannel every half second is worse than silence.
#[test]
fn a_backchannel_needs_a_gap_worth_filling() {
    let just_started = ConversationalFloorState {
        current_speaker: Speaker::Owner,
        speaker_hold: p(0.85),
        silence_ms: 200,
        ..Default::default()
    };

    assert_eq!(
        FloorPolicy::default().decide(&just_started).action,
        FloorAction::Silent
    );
}

// -------------------------------------------------------- 21.7 / 21.8 interruption

/// A correction worth making before Nelson builds further on a bad premise.
#[test]
fn a_useful_correction_may_interrupt_gently() {
    let worth_saying = ConversationalFloorState {
        current_speaker: Speaker::Owner,
        speaker_hold: p(0.8),
        interruption_utility: p(0.75),
        urgency: p(0.4),
        ..Default::default()
    };

    let decision = FloorPolicy::default().decide(&worth_saying);
    assert_eq!(decision.action, FloorAction::SoftInterrupt);
    assert!(decision.action.is_interruption());
    assert_eq!(decision.budget, SpeechBudget::Sentence);
}

/// And something urgent enough may cut in hard — but it needs both bars, not one.
#[test]
fn cutting_somebody_off_needs_both_utility_and_urgency() {
    let policy = FloorPolicy::default();

    let critical = ConversationalFloorState {
        current_speaker: Speaker::Owner,
        speaker_hold: p(0.9),
        interruption_utility: p(0.95),
        urgency: p(0.95),
        ..Default::default()
    };
    assert_eq!(
        policy.decide(&critical).action,
        FloorAction::StrongInterrupt
    );

    // Useful but not urgent stays gentle.
    let mut useful_only = critical.clone();
    useful_only.urgency = p(0.2);
    assert_eq!(
        policy.decide(&useful_only).action,
        FloorAction::SoftInterrupt,
        "high utility alone must not license cutting somebody off"
    );

    // Urgent but not useful says nothing at all.
    let mut urgent_only = critical;
    urgent_only.interruption_utility = p(0.1);
    assert_eq!(
        policy.decide(&urgent_only).action,
        FloorAction::Silent,
        "urgency about nothing is not a reason to interrupt"
    );
}

// ------------------------------------------------------------------------- the defaults

/// Knowing nothing, Archie says nothing.
///
/// A system that starts talking because it has not measured anything yet is worse than
/// one that waits, and the default state is what it has before the first estimate.
#[test]
fn an_observatory_that_knows_nothing_stays_quiet() {
    let decision = FloorPolicy::default().decide(&ConversationalFloorState::default());
    assert_eq!(decision.action, FloorAction::Silent);
    assert!(!decision.action.produces_speech());
}

/// Having something to say is not a reason to say it.
#[test]
fn a_ready_answer_is_not_by_itself_permission_to_speak() {
    let policy = FloorPolicy::default();
    let mut state = floor_is_free_with_an_answer();

    // The answer is ready and good...
    assert_eq!(policy.decide(&state).action, FloorAction::TakeFloor);

    // ...but the person has not finished.
    state.speaker_hold = p(0.9);
    assert_eq!(policy.decide(&state).action, FloorAction::Silent);
}

/// The floor is free, the thought is finished, Archie understands perfectly — and there
/// is nothing worth saying.
///
/// This is the case a system gets wrong by being helpful. Nelson finishes a sentence that
/// needs no reply, and an assistant that answers every complete utterance fills the room
/// with acknowledgements of acknowledgements. Silence here is not a failure to respond;
/// it is the response.
#[test]
fn a_free_floor_with_nothing_to_say_stays_quiet() {
    let nothing_to_add = ConversationalFloorState {
        current_speaker: Speaker::Nobody,
        thought_completion: p(0.95),
        silence_scope: SilenceScope::EndOfTurn,
        silence_ms: 900,
        // Understood completely, and not worth a word.
        context_sufficiency: p(0.95),
        response_utility: p(0.1),
        ..Default::default()
    };

    let decision = FloorPolicy::default().decide(&nothing_to_add);
    assert_eq!(
        decision.action,
        FloorAction::Silent,
        "an open floor is not an obligation to fill it: {decision:?}"
    );
    assert!(!decision.action.produces_speech());
    assert!(
        decision.reason.contains("nothing worth saying"),
        "{}",
        decision.reason
    );
}

/// An answer Archie does not have enough context for waits rather than guesses.
#[test]
fn insufficient_context_keeps_listening_instead_of_guessing() {
    let mut state = floor_is_free_with_an_answer();
    state.context_sufficiency = p(0.1);

    let decision = FloorPolicy::default().decide(&state);
    assert_eq!(decision.action, FloorAction::Silent);
    assert!(decision.reason.contains("context"), "{}", decision.reason);
}

/// Every decision explains itself, because an interruption a person cannot account for
/// is one they learn to distrust.
#[test]
fn every_decision_carries_a_reason() {
    let policy = FloorPolicy::default();
    for state in [
        ConversationalFloorState::default(),
        floor_is_free_with_an_answer(),
        ConversationalFloorState {
            current_speaker: Speaker::Archie,
            overlap: OverlapState::UserOverArchie,
            ..Default::default()
        },
        ConversationalFloorState {
            current_speaker: Speaker::Owner,
            interruption_utility: p(0.95),
            urgency: p(0.95),
            ..Default::default()
        },
    ] {
        let decision = policy.decide(&state);
        assert!(
            decision.reason.len() > 15,
            "a decision nobody can account for: {decision:?}"
        );
    }
}

/// A NaN estimate must not silently mean "always" or "never".
///
/// Every comparison against NaN is false, so a NaN utility would read as "never speak"
/// and a NaN hold as "always take the floor" — erratic behaviour with no error anywhere.
#[test]
fn a_broken_estimate_fails_quiet_rather_than_erratic() {
    assert_eq!(Probability::new(f32::NAN).get(), 0.0);
    assert_eq!(Probability::new(-5.0).get(), 0.0);
    assert_eq!(Probability::new(9.0).get(), 1.0);

    let broken = ConversationalFloorState {
        current_speaker: Speaker::Owner,
        interruption_utility: Probability::new(f32::NAN),
        urgency: Probability::new(f32::NAN),
        ..Default::default()
    };
    assert_eq!(
        FloorPolicy::default().decide(&broken).action,
        FloorAction::Silent,
        "an unusable estimate must not become an interruption"
    );
}

/// The state crosses process boundaries, because the estimator and the policy may not
/// share one.
#[test]
fn floor_state_round_trips_through_json() {
    let state = floor_is_free_with_an_answer();
    let text = serde_json::to_string(&state).expect("serialisable");
    let back: ConversationalFloorState = serde_json::from_str(&text).expect("deserialisable");
    assert_eq!(back, state);

    let decision = FloorPolicy::default().decide(&state);
    let text = serde_json::to_string(&decision).expect("serialisable");
    assert_eq!(
        serde_json::from_str::<archie_speech::FloorDecision>(&text).unwrap(),
        decision
    );
}

/// A budget large enough for an answer is granted only to an action that says one.
///
/// The session leans on this: it starts the cognitive plane on the budget alone, and a
/// policy that ever granted a sentence to `Silent` or `Yield` would have Archie generating
/// an answer to accompany its own silence. Swept over every combination the policy can
/// see, rather than asserted about the cases somebody thought of.
#[test]
fn speaking_actions_are_exactly_the_ones_granted_words() {
    let levels = [0.0_f32, 0.6, 0.95];
    let policy = FloorPolicy::default();
    let mut seen = 0_u32;

    for speaker in [
        Speaker::Owner,
        Speaker::Archie,
        Speaker::Other,
        Speaker::Unidentified,
        Speaker::Nobody,
    ] {
        for scope in [
            SilenceScope::WithinClause,
            SilenceScope::BetweenClauses,
            SilenceScope::EndOfTurn,
            SilenceScope::Unknown,
        ] {
            for overlap in [
                OverlapState::None,
                OverlapState::UserOverArchie,
                OverlapState::ArchieOverUser,
            ] {
                for silence_ms in [0_u32, 2_000] {
                    for hold in levels {
                        for completion in levels {
                            for interruption in levels {
                                for utility in levels {
                                    for urgency in levels {
                                        for context in levels {
                                            for acknowledgement in levels {
                                                let state = ConversationalFloorState {
                                                    current_speaker: speaker,
                                                    silence_scope: scope,
                                                    overlap,
                                                    silence_ms,
                                                    speaker_hold: p(hold),
                                                    thought_completion: p(completion),
                                                    interruption_utility: p(interruption),
                                                    response_utility: p(utility),
                                                    urgency: p(urgency),
                                                    context_sufficiency: p(context),
                                                    acknowledgement_only: p(acknowledgement),
                                                    ..Default::default()
                                                };
                                                let decision = policy.decide(&state);
                                                seen += 1;
                                                if matches!(
                                                    decision.budget,
                                                    SpeechBudget::Sentence | SpeechBudget::Full
                                                ) {
                                                    assert!(
                                                        decision.action.produces_speech(),
                                                        "{:?} was granted {:?}",
                                                        decision.action,
                                                        decision.budget
                                                    );
                                                }
                                                if !decision.action.produces_speech() {
                                                    assert_eq!(
                                                        decision.budget,
                                                        SpeechBudget::None,
                                                        "{:?} says nothing and needs no words",
                                                        decision.action
                                                    );
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    assert!(seen > 100_000, "the sweep actually swept: {seen}");
}
