//! One conversation, end to end, with no model in it.
//!
//! The floor decides whether Archie may speak; the cognitive plane decides the words; the
//! backends listen and pronounce. These tests are about what happens where those three
//! meet, which is the only place their disagreements are still fixable.

use archie_speech::adapter::{SpeechCapabilities, SpokenContentSource};
use archie_speech::cognitive::{
    CancelReason, CognitiveModelAdapter, ScriptedCognitiveModel, ScriptedReply,
};
use archie_speech::floor::{
    ConversationalFloorState, FloorAction, FloorPolicy, OverlapState, Probability, SilenceScope,
    Speaker, SpeechBudget,
};
use archie_speech::session::{
    SessionError, SessionEvent, SessionPlan, SpeechSession, SpokenSegment,
};

const ASKED: &str = "how is the disk";
const FIRST: &str = "The array is healthy.";
const SECOND: &str = "One drive is at eighty percent.";

// A transport may race cancellation. The session must fence its own output,
// regardless of whether the backend has stopped returning queued events yet.
struct LateBackend(ScriptedCognitiveModel);
impl CognitiveModelAdapter for LateBackend {
    fn languages(&self) -> &[String] {
        self.0.languages()
    }
    fn begin(
        &mut self,
        request: archie_speech::CognitiveRequest,
    ) -> Result<(), archie_speech::CognitiveError> {
        self.0.begin(request)
    }
    fn poll(&mut self) -> Option<archie_speech::CognitiveEvent> {
        self.0.poll()
    }
    fn cancel(&mut self, _: &str, _: CancelReason) {}
    fn running_turn(&self) -> Option<&str> {
        self.0.running_turn()
    }
}

#[test]
fn cancellation_fences_late_speech_and_tool_proposals() {
    let brain = brain().answering(
        ASKED,
        ScriptedReply::saying(&[FIRST]).proposing("open_app", "{}"),
    );
    let mut session =
        SpeechSession::new(plan("en"), FloorPolicy::default(), LateBackend(brain)).unwrap();
    session
        .heard("late", ASKED, &floor_is_free_with_a_real_answer())
        .unwrap();
    session.cancel(CancelReason::Interrupted);
    assert!(
        session.pump().is_empty(),
        "cancelled events must never be released"
    );
    assert!(session.written_text().is_empty());
}

fn renderer(languages: &[&str]) -> SpeechCapabilities {
    SpeechCapabilities {
        accepts_external_response_text: true,
        renders_external_response_verbatim: true,
        languages: languages.iter().map(|l| (*l).to_string()).collect(),
        ..Default::default()
    }
}

fn ears(languages: &[&str]) -> SpeechCapabilities {
    SpeechCapabilities {
        partial_transcript: true,
        languages: languages.iter().map(|l| (*l).to_string()).collect(),
        ..Default::default()
    }
}

/// The shape NVIDIA's VoiceChat card describes: hears and speaks at once, and writes its
/// own words.
fn unified_duplex() -> SpeechCapabilities {
    SpeechCapabilities {
        native_full_duplex: true,
        simultaneous_listen_speak: true,
        barge_in: true,
        native_turn_taking: true,
        generates_semantic_response: true,
        accepts_external_response_text: false,
        renders_external_response_verbatim: false,
        partial_transcript: true,
        semantic_stream: true,
        languages: vec!["en".into()],
        ..Default::default()
    }
}

fn plan(language: &str) -> SessionPlan {
    SessionPlan {
        language: language.into(),
        listener: ears(&["en", "es"]),
        voice: renderer(&["en", "es"]),
    }
}

fn brain() -> ScriptedCognitiveModel {
    ScriptedCognitiveModel::new(&["en", "es"])
        .answering(ASKED, ScriptedReply::saying(&[FIRST, SECOND]))
}

fn session(language: &str) -> SpeechSession<ScriptedCognitiveModel> {
    SpeechSession::new(plan(language), FloorPolicy::default(), brain()).expect("a workable session")
}

// -------------------------------------------------------------------- floor states

/// A free floor with a substantial answer waiting: take it, in full.
fn floor_is_free_with_a_real_answer() -> ConversationalFloorState {
    ConversationalFloorState {
        current_speaker: Speaker::Nobody,
        silence_scope: SilenceScope::EndOfTurn,
        thought_completion: Probability::new(0.9),
        response_utility: Probability::new(0.9),
        context_sufficiency: Probability::new(0.9),
        ..Default::default()
    }
}

/// The same, but only worth a sentence.
fn floor_is_free_and_worth_a_sentence() -> ConversationalFloorState {
    ConversationalFloorState {
        response_utility: Probability::new(0.5),
        ..floor_is_free_with_a_real_answer()
    }
}

/// They are mid-thought. The most common correct answer is silence.
fn they_are_still_talking() -> ConversationalFloorState {
    ConversationalFloorState {
        current_speaker: Speaker::Owner,
        speaker_hold: Probability::new(0.9),
        silence_ms: 0,
        ..Default::default()
    }
}

/// A long hold: silence would read as absence, so a listening noise, and nothing more.
fn a_long_hold_that_wants_a_listening_noise() -> ConversationalFloorState {
    ConversationalFloorState {
        current_speaker: Speaker::Owner,
        speaker_hold: Probability::new(0.9),
        silence_ms: 2_000,
        ..Default::default()
    }
}

/// Archie is talking and the person has started saying something of their own.
fn the_person_cuts_in() -> ConversationalFloorState {
    ConversationalFloorState {
        current_speaker: Speaker::Archie,
        overlap: OverlapState::UserOverArchie,
        acknowledgement_only: Probability::ZERO,
        ..Default::default()
    }
}

fn spoken(events: &[SessionEvent]) -> Vec<String> {
    events
        .iter()
        .filter_map(|event| match event {
            SessionEvent::Speak(SpokenSegment { text, .. }) => Some(text.clone()),
            _ => None,
        })
        .collect()
}

// -------------------------------------------------------------------- assembly

/// The decisive refusal, and the reason the two-plane design exists.
///
/// A model that writes its own replies is a fine conversational system. It is not one
/// that can say what Cerebra decided — and finding that out when the answer is ready
/// leaves a choice between silence and a voice improvising.
#[test]
fn a_voice_that_writes_its_own_replies_cannot_hold_an_archie_session() {
    let refused = SpeechSession::new(
        SessionPlan {
            language: "en".into(),
            listener: unified_duplex(),
            voice: unified_duplex(),
        },
        FloorPolicy::default(),
        brain(),
    );
    assert_eq!(refused.err(), Some(SessionError::VoiceCannotSpeakAnswers));
}

/// The same duplex model is fine as ears, paired with something that will pronounce.
#[test]
fn a_duplex_model_can_listen_while_a_renderer_speaks() {
    assert!(SpeechSession::new(
        SessionPlan {
            language: "en".into(),
            listener: unified_duplex(),
            voice: renderer(&["en"]),
        },
        FloorPolicy::default(),
        brain(),
    )
    .is_ok());
}

/// Nelson's language is the concrete case: an English-only stack must refuse a Spanish
/// session rather than pronounce Spanish it was never trained on.
#[test]
fn a_session_is_refused_in_a_language_some_part_of_it_does_not_speak() {
    let deaf_to_spanish = SpeechSession::new(
        SessionPlan {
            language: "es".into(),
            listener: ears(&["en"]),
            voice: renderer(&["en", "es"]),
        },
        FloorPolicy::default(),
        brain(),
    );
    assert_eq!(
        deaf_to_spanish.err(),
        Some(SessionError::ListenerDoesNotSpeak {
            language: "es".into()
        })
    );

    let mute_in_spanish = SpeechSession::new(
        SessionPlan {
            language: "es".into(),
            listener: ears(&["en", "es"]),
            voice: renderer(&["en"]),
        },
        FloorPolicy::default(),
        brain(),
    );
    assert_eq!(
        mute_in_spanish.err(),
        Some(SessionError::VoiceDoesNotSpeak {
            language: "es".into()
        })
    );

    let english_only_brain = SpeechSession::new(
        plan("es"),
        FloorPolicy::default(),
        ScriptedCognitiveModel::new(&["en"]),
    );
    assert_eq!(
        english_only_brain.err(),
        Some(SessionError::CognitiveDoesNotAnswer {
            language: "es".into()
        })
    );
}

/// A regional tag is served by its language, so a Chilean session runs on a Spanish stack.
#[test]
fn a_regional_conversation_runs_on_the_language_it_belongs_to() {
    assert!(SpeechSession::new(plan("es-CL"), FloorPolicy::default(), brain()).is_ok());
}

/// A backend claiming something impossible is refused before it can act on it.
#[test]
fn a_backend_that_declares_nonsense_is_refused_at_assembly() {
    let pretender = SpeechCapabilities {
        native_full_duplex: true,
        simultaneous_listen_speak: false,
        languages: vec!["en".into()],
        ..Default::default()
    };
    let refused = SpeechSession::new(
        SessionPlan {
            language: "en".into(),
            listener: pretender,
            voice: renderer(&["en"]),
        },
        FloorPolicy::default(),
        brain(),
    );
    assert!(matches!(
        refused.err(),
        Some(SessionError::ListenerIncoherent(_))
    ));
}

// -------------------------------------------------------------------- silence

/// Silence does not wake the cognitive plane.
///
/// The most common correct action in a conversation is to say nothing, and it should cost
/// nothing — no model run, and no paragraph left lying around for somebody to say.
#[test]
fn staying_silent_starts_no_thinking_at_all() {
    let mut session = session("en");
    let decision = session
        .heard("t1", ASKED, &they_are_still_talking())
        .unwrap();

    assert_eq!(decision.action, FloorAction::Silent);
    assert_eq!(session.cognitive().running_turn(), None);
    assert_eq!(session.pump(), Vec::new());
    assert_eq!(session.written_text(), "");
}

/// A listening noise is a floor decision with no content. It must not generate one.
#[test]
fn a_backchannel_does_not_wake_the_cognitive_plane() {
    let mut session = session("en");
    let decision = session
        .heard("t1", ASKED, &a_long_hold_that_wants_a_listening_noise())
        .unwrap();

    assert_eq!(decision.action, FloorAction::Backchannel);
    assert_eq!(decision.budget, SpeechBudget::Token);
    assert_eq!(
        session.cognitive().running_turn(),
        None,
        "\"mhm\" is not a question, and answering it costs a model run"
    );
}

// -------------------------------------------------------------------- answering

/// The ordinary case: the floor is free, the answer goes out whole.
#[test]
fn a_free_floor_and_a_real_answer_produce_the_whole_answer() {
    let mut session = session("en");
    let decision = session
        .heard("t1", ASKED, &floor_is_free_with_a_real_answer())
        .unwrap();
    assert_eq!(decision.action, FloorAction::TakeFloor);

    let events = session.pump();
    assert_eq!(spoken(&events), vec![FIRST.to_string(), SECOND.to_string()]);
    assert!(events.contains(&SessionEvent::Finished));
}

/// The voice renders verbatim, so what was cleared is what is heard — down to the string.
#[test]
fn the_words_released_are_the_words_that_were_written() {
    let mut session = session("en");
    session
        .heard("t1", ASKED, &floor_is_free_with_a_real_answer())
        .unwrap();
    let events = session.pump();

    assert_eq!(spoken(&events).join(" "), session.written_text());
    for event in &events {
        if let SessionEvent::Speak(segment) = event {
            assert_eq!(segment.source, SpokenContentSource::CognitiveModel);
            assert!(
                segment.source.is_substantive_answer(),
                "an answer path must not carry unreasoned words"
            );
        }
    }
}

/// A one-sentence budget releases one sentence, and holds the rest rather than deleting
/// it: the floor may open again, and then the remainder is what resumes.
#[test]
fn a_sentence_budget_releases_one_sentence_and_holds_the_rest() {
    let mut session = session("en");
    let decision = session
        .heard("t1", ASKED, &floor_is_free_and_worth_a_sentence())
        .unwrap();
    assert_eq!(decision.action, FloorAction::ShortResponse);

    let events = session.pump();
    assert_eq!(spoken(&events), vec![FIRST.to_string()]);
    assert!(events
        .iter()
        .any(|event| matches!(event, SessionEvent::Withheld { sequence: 1, .. })));

    assert_eq!(
        session.written_text(),
        format!("{FIRST} {SECOND}"),
        "the held sentence was still written, and is still on record"
    );
}

/// The held remainder is what resumes when the floor opens.
#[test]
fn what_the_budget_held_back_is_what_resumes() {
    let mut session = session("en");
    session
        .heard("t1", ASKED, &floor_is_free_and_worth_a_sentence())
        .unwrap();
    session.pump();
    session.spoke(0).unwrap();

    assert_eq!(
        session.resume(&floor_is_free_with_a_real_answer()),
        Some(SECOND.to_string())
    );
}

/// A tool proposal reaches the rest of the system as a proposal, correlated by the turn.
#[test]
fn a_tool_proposal_passes_through_correlated_by_its_turn() {
    let mut session = SpeechSession::new(
        plan("en"),
        FloorPolicy::default(),
        ScriptedCognitiveModel::new(&["en"]).answering(
            ASKED,
            ScriptedReply::saying(&["Checking."]).proposing("smart_report", "{\"disk\":\"sda\"}"),
        ),
    )
    .unwrap();
    session
        .heard("t1", ASKED, &floor_is_free_with_a_real_answer())
        .unwrap();

    let proposal = session
        .pump()
        .into_iter()
        .find_map(|event| match event {
            SessionEvent::ToolProposal(intent) => Some(intent),
            _ => None,
        })
        .expect("the script proposed a tool");
    assert_eq!(proposal.correlation_id, "t1");
    assert_eq!(proposal.tool_name, "smart_report");
}

/// A failure is an alert, never an answer.
#[test]
fn a_failed_turn_produces_no_words_at_all() {
    let mut session = session("en");
    session
        .heard(
            "t1",
            "nothing scripted",
            &floor_is_free_with_a_real_answer(),
        )
        .unwrap();

    let events = session.pump();
    assert!(spoken(&events).is_empty());
    assert!(matches!(events.as_slice(), [SessionEvent::Alert(_)]));
    assert_eq!(session.heard_text(), "");
}

// -------------------------------------------------------------------- interruption

/// Being cut into stops Archie, and the transcript is what was heard.
#[test]
fn being_interrupted_stops_the_answer_and_records_only_what_was_said() {
    let mut session = session("en");
    session
        .heard("t1", ASKED, &floor_is_free_with_a_real_answer())
        .unwrap();
    session.pump();
    session.spoke(0).unwrap();

    let decision = session.floor_moved(&the_person_cuts_in());
    assert_eq!(decision.action, FloorAction::Yield);

    assert_eq!(session.heard_text(), FIRST);
    assert_ne!(
        session.heard_text(),
        session.written_text(),
        "the second sentence was written and never heard"
    );
}

/// After yielding, the rest is still there to say — if the floor agrees it is still valid.
#[test]
fn an_interrupted_answer_resumes_from_where_it_stopped() {
    let mut session = session("en");
    session
        .heard("t1", ASKED, &floor_is_free_with_a_real_answer())
        .unwrap();
    session.pump();
    session.spoke(0).unwrap();
    session.floor_moved(&the_person_cuts_in());

    assert_eq!(
        session.resume(&floor_is_free_with_a_real_answer()),
        Some(SECOND.to_string()),
        "it continues, and does not repeat the sentence just heard"
    );
}

/// The interruption was a correction, so the half-finished answer is stale.
///
/// Saying the rest of an answer to a question that no longer exists sounds like Archie was
/// not listening, which is worse than saying nothing.
#[test]
fn an_answer_the_floor_judges_stale_is_not_resumed() {
    let mut session = session("en");
    session
        .heard("t1", ASKED, &floor_is_free_with_a_real_answer())
        .unwrap();
    session.pump();
    session.spoke(0).unwrap();
    session.floor_moved(&the_person_cuts_in());

    let they_changed_the_question = ConversationalFloorState {
        response_still_valid: Probability::new(0.1),
        ..floor_is_free_with_a_real_answer()
    };
    assert_eq!(session.resume(&they_changed_the_question), None);
}

/// A turn stopped for any reason other than an interruption does not resume.
#[test]
fn a_superseded_turn_does_not_resume_however_valid_the_floor_thinks_it_is() {
    for reason in [
        CancelReason::Superseded,
        CancelReason::PolicyRefused,
        CancelReason::Shutdown,
    ] {
        let mut session = session("en");
        session
            .heard("t1", ASKED, &floor_is_free_with_a_real_answer())
            .unwrap();
        session.pump();
        session.spoke(0).unwrap();
        session.cancel(reason);

        assert_eq!(
            session.resume(&floor_is_free_with_a_real_answer()),
            None,
            "{reason:?}"
        );
    }
}

/// An acknowledgement over Archie is not an interruption.
///
/// Stopping for "mhm" makes Archie unusable to anybody who makes listening noises, which
/// is most people.
#[test]
fn a_listening_noise_over_archie_does_not_stop_it() {
    let mut session = session("en");
    session
        .heard("t1", ASKED, &floor_is_free_with_a_real_answer())
        .unwrap();
    session.pump();

    let mhm = ConversationalFloorState {
        acknowledgement_only: Probability::new(0.9),
        ..the_person_cuts_in()
    };
    let decision = session.floor_moved(&mhm);
    assert_eq!(decision.action, FloorAction::ContinueSpeaking);

    session.spoke(0).unwrap();
    session.spoke(1).unwrap();
    assert_eq!(session.heard_text(), format!("{FIRST} {SECOND}"));
}

/// The floor keeps moving after a yield, and the turn is already stopped.
///
/// Cancelling it again on every estimate would tell the backend to abandon the same turn
/// several times a second, and would make the cancellation record — the evidence of why
/// Archie stopped — a list of repetitions instead of a reason.
#[test]
fn a_turn_already_yielded_is_not_cancelled_again() {
    let mut session = session("en");
    session
        .heard("t1", ASKED, &floor_is_free_with_a_real_answer())
        .unwrap();
    session.pump();
    session.spoke(0).unwrap();

    session.floor_moved(&the_person_cuts_in());
    session.floor_moved(&the_person_cuts_in());
    session.cancel(CancelReason::Shutdown);

    assert_eq!(
        session.cognitive().cancellations(),
        [("t1".to_string(), CancelReason::Interrupted)],
        "one turn, stopped once, for one reason"
    );
}

// -------------------------------------------------------------------- the record

/// The voice cannot claim to have said words it was never given.
#[test]
fn a_withheld_sentence_cannot_be_reported_as_spoken() {
    let mut session = session("en");
    session
        .heard("t1", ASKED, &floor_is_free_and_worth_a_sentence())
        .unwrap();
    session.pump();

    assert_eq!(
        session.spoke(1),
        Err(SessionError::NotReleased { sequence: 1 }),
        "sentence one was held back; nobody heard it"
    );
    session.spoke(0).unwrap();
    assert_eq!(session.heard_text(), FIRST);
}

/// Words released but not yet finished have not been heard.
#[test]
fn released_is_not_the_same_as_heard() {
    let mut session = session("en");
    session
        .heard("t1", ASKED, &floor_is_free_with_a_real_answer())
        .unwrap();
    session.pump();

    assert_eq!(
        session.heard_text(),
        "",
        "the voice was handed the words and has not finished saying them"
    );
    assert_eq!(session.written_text(), format!("{FIRST} {SECOND}"));
}

/// Reporting a segment spoken with no turn running is a bug in the caller, not a no-op.
#[test]
fn there_is_nothing_to_speak_before_a_turn_starts() {
    let mut session = session("en");
    assert_eq!(session.spoke(0), Err(SessionError::NoTurn));
}

/// Stopping does not un-send what is already on the wire.
///
/// A cancel tells a backend to stop; it does not reach into a socket and remove the chunks
/// already in flight. So a real adapter delivers a few more segments of a turn that is over
/// — and attributing them to whatever turn is running now would splice one answer into
/// another.
mod leaky {
    use archie_speech::adapter::SpokenContentSource;
    use archie_speech::cognitive::{
        validate_request, CancelReason, CognitiveError, CognitiveEvent, CognitiveModelAdapter,
        CognitiveRequest, ResponseSegment,
    };
    use std::collections::VecDeque;

    /// A backend that stops accepting new work when cancelled and keeps delivering what it
    /// had already produced. Deliberately worse behaved than the scripted model.
    #[derive(Debug, Default)]
    pub struct LeakyBackend {
        languages: Vec<String>,
        pending: VecDeque<CognitiveEvent>,
        running: Option<String>,
    }

    impl LeakyBackend {
        pub fn new() -> Self {
            Self {
                languages: vec!["en".into()],
                ..Default::default()
            }
        }
    }

    impl CognitiveModelAdapter for LeakyBackend {
        fn languages(&self) -> &[String] {
            &self.languages
        }

        fn begin(&mut self, request: CognitiveRequest) -> Result<(), CognitiveError> {
            if let Some(running) = &self.running {
                return Err(CognitiveError::TurnAlreadyRunning {
                    running: running.clone(),
                    requested: request.turn_id,
                });
            }
            validate_request(&request, &self.languages)?;
            self.running = Some(request.turn_id.clone());
            for sequence in 0..2 {
                self.pending
                    .push_back(CognitiveEvent::Segment(ResponseSegment {
                        turn_id: request.turn_id.clone(),
                        sequence,
                        text: format!("{} part {sequence}.", request.turn_id),
                        source: SpokenContentSource::CognitiveModel,
                        ends_sentence: true,
                    }));
            }
            self.pending.push_back(CognitiveEvent::Completed {
                turn_id: request.turn_id,
            });
            Ok(())
        }

        fn poll(&mut self) -> Option<CognitiveEvent> {
            self.pending.pop_front()
        }

        /// Frees the backend for new work and abandons nothing already produced.
        fn cancel(&mut self, turn_id: &str, _reason: CancelReason) {
            if self.running.as_deref() == Some(turn_id) {
                self.running = None;
            }
        }

        fn running_turn(&self) -> Option<&str> {
            self.running.as_deref()
        }
    }
}

/// Words still arriving from a turn that was stopped are not spliced into the next one.
#[test]
fn leftovers_from_a_stopped_turn_do_not_become_part_of_the_next_answer() {
    let mut session = SpeechSession::new(
        plan("en"),
        FloorPolicy::default(),
        leaky::LeakyBackend::new(),
    )
    .unwrap();

    session
        .heard("t1", ASKED, &floor_is_free_with_a_real_answer())
        .unwrap();
    session.cancel(CancelReason::Superseded);

    session
        .heard("t2", ASKED, &floor_is_free_with_a_real_answer())
        .unwrap();
    let events = session.pump();

    for line in spoken(&events) {
        assert!(
            line.starts_with("t2"),
            "{line:?} belongs to a turn that is over"
        );
    }
    assert_eq!(session.written_text(), "t2 part 0. t2 part 1.");
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, SessionEvent::Alert(_))),
        "the leftovers are not this turn's problem to report: {events:?}"
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, SessionEvent::Finished))
            .count(),
        1,
        "the old turn's completion must not end the new one"
    );
}

/// A listening noise over Archie changes nothing at the backend.
///
/// Cancelling on a decision that was not a yield would abandon the answer mid-sentence
/// because somebody said "mhm".
#[test]
fn a_decision_that_is_not_a_yield_cancels_nothing() {
    let mut session = session("en");
    session
        .heard("t1", ASKED, &floor_is_free_with_a_real_answer())
        .unwrap();
    session.pump();

    let mhm = ConversationalFloorState {
        acknowledgement_only: Probability::new(0.9),
        ..the_person_cuts_in()
    };
    assert_eq!(
        session.floor_moved(&mhm).action,
        FloorAction::ContinueSpeaking
    );
    assert!(
        session.cognitive().cancellations().is_empty(),
        "nothing was abandoned"
    );

    assert_eq!(
        session
            .floor_moved(&floor_is_free_with_a_real_answer())
            .action,
        FloorAction::TakeFloor
    );
    assert!(session.cognitive().cancellations().is_empty());
}
