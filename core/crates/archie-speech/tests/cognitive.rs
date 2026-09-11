//! What Archie says, and what Archie was heard to say.
//!
//! These are the same string only when nobody interrupts. Most of these tests are about
//! the other case.

use archie_speech::adapter::SpokenContentSource;
use archie_speech::cognitive::{
    validate_request, CancelReason, CognitiveError, CognitiveEvent, CognitiveModelAdapter,
    CognitiveRequest, ResponseCommitLog, ResponseGate, ResponseSegment, ScriptedCognitiveModel,
    ScriptedReply,
};
use archie_speech::floor::SpeechBudget;

const WEATHER: &str = "what is the weather";

fn model() -> ScriptedCognitiveModel {
    ScriptedCognitiveModel::new(&["en", "es"]).answering(
        WEATHER,
        ScriptedReply::saying(&[
            "It is fourteen degrees in Santiago.",
            "There is rain forecast after six.",
        ]),
    )
}

fn ask(turn: &str, transcript: &str, budget: SpeechBudget) -> CognitiveRequest {
    CognitiveRequest {
        turn_id: turn.into(),
        transcript: transcript.into(),
        language: "en".into(),
        budget,
    }
}

fn drain(model: &mut ScriptedCognitiveModel) -> Vec<CognitiveEvent> {
    let mut events = Vec::new();
    while let Some(event) = model.poll() {
        events.push(event);
    }
    events
}

fn segment(turn: &str, sequence: u32, text: &str, ends_sentence: bool) -> ResponseSegment {
    ResponseSegment {
        turn_id: turn.into(),
        sequence,
        text: text.into(),
        source: SpokenContentSource::CognitiveModel,
        ends_sentence,
    }
}

// --------------------------------------------------------------- the stream

/// An answer arrives in pieces, in order, and ends by saying so.
#[test]
fn an_answer_arrives_as_ordered_clauses_and_then_completes() {
    let mut model = model();
    model.begin(ask("t1", WEATHER, SpeechBudget::Full)).unwrap();

    let events = drain(&mut model);
    let sequences: Vec<u32> = events
        .iter()
        .filter_map(|event| match event {
            CognitiveEvent::Segment(segment) => Some(segment.sequence),
            _ => None,
        })
        .collect();
    assert_eq!(
        sequences,
        vec![0, 1],
        "clauses arrive in order, without gaps"
    );
    assert!(matches!(
        events.last(),
        Some(CognitiveEvent::Completed { .. })
    ));
    assert_eq!(
        model.running_turn(),
        None,
        "a completed turn is not running"
    );
}

/// Every clause is attributed to the cognitive plane, so nothing downstream has to guess
/// whether a sentence was reasoned about or reflexive.
#[test]
fn every_clause_carries_its_provenance() {
    let mut model = model();
    model.begin(ask("t1", WEATHER, SpeechBudget::Full)).unwrap();
    for event in drain(&mut model) {
        if let CognitiveEvent::Segment(segment) = event {
            assert_eq!(segment.source, SpokenContentSource::CognitiveModel);
            assert!(segment.source.is_substantive_answer());
        }
    }
}

/// Deterministic means deterministic: the same script, twice, event for event.
#[test]
fn the_same_request_produces_the_same_events_twice() {
    let mut first = model();
    first.begin(ask("t1", WEATHER, SpeechBudget::Full)).unwrap();
    let mut second = model();
    second
        .begin(ask("t1", WEATHER, SpeechBudget::Full))
        .unwrap();
    assert_eq!(drain(&mut first), drain(&mut second));
}

/// A request with no script fails loudly.
///
/// The alternative — a plausible "I'm not sure about that" — is the failure a test cannot
/// see, because it looks exactly like a real answer.
#[test]
fn an_unscripted_request_fails_rather_than_inventing_an_answer() {
    let mut model = model();
    model
        .begin(ask("t1", "something nobody scripted", SpeechBudget::Full))
        .unwrap();

    let events = drain(&mut model);
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, CognitiveEvent::Segment(_))),
        "no words at all: there is no answer to say"
    );
    assert!(matches!(events.as_slice(), [CognitiveEvent::Failed { .. }]));
}

// --------------------------------------------------------------- refusals at the door

/// Silence is not a question.
#[test]
fn an_empty_transcript_is_refused() {
    let mut model = model();
    assert_eq!(
        model.begin(ask("t1", "   ", SpeechBudget::Full)),
        Err(CognitiveError::EmptyTranscript)
    );
    assert_eq!(model.running_turn(), None);
}

/// Asking for an answer the floor will not let out is a bug, not a request.
///
/// A backchannel budget means Archie may make a listening noise. Generating a paragraph
/// to throw away costs a model run and, worse, invites somebody to speak it anyway.
#[test]
fn a_budget_that_forbids_speaking_forbids_answering() {
    let mut model = model();
    for budget in [SpeechBudget::None, SpeechBudget::Token] {
        assert_eq!(
            model.begin(ask("t1", WEATHER, budget)),
            Err(CognitiveError::BudgetForbidsAnswering { budget })
        );
    }
    assert!(model
        .begin(ask("t1", WEATHER, SpeechBudget::Sentence))
        .is_ok());
}

/// A language the backend does not speak is refused, and a region is served by its
/// language — the same rule the speech adapters use, so a session cannot be accepted by
/// one plane and refused by the other.
#[test]
fn a_language_the_backend_does_not_answer_in_is_refused() {
    let languages = vec!["es".to_string()];
    let mut request = ask("t1", WEATHER, SpeechBudget::Full);

    request.language = "es-CL".into();
    assert!(validate_request(&request, &languages).is_ok());

    request.language = "en".into();
    assert_eq!(
        validate_request(&request, &languages),
        Err(CognitiveError::UnsupportedLanguage {
            language: "en".into()
        })
    );
}

/// Starting a second turn over a live one is refused rather than silently winning.
///
/// Overwriting would discard the record of what the first turn had already said, and that
/// record is the only evidence of what the person heard.
#[test]
fn a_second_turn_cannot_quietly_replace_a_running_one() {
    let mut model = model();
    model.begin(ask("t1", WEATHER, SpeechBudget::Full)).unwrap();

    assert_eq!(
        model.begin(ask("t2", WEATHER, SpeechBudget::Full)),
        Err(CognitiveError::TurnAlreadyRunning {
            running: "t1".into(),
            requested: "t2".into(),
        })
    );
    assert_eq!(model.running_turn(), Some("t1"));

    model.cancel("t1", CancelReason::Superseded);
    assert!(model.begin(ask("t2", WEATHER, SpeechBudget::Full)).is_ok());
}

// --------------------------------------------------------------- interruption

/// A cancelled turn stops, and does not report completing.
#[test]
fn a_cancelled_turn_emits_nothing_further_and_never_completes() {
    let mut model = model();
    model.begin(ask("t1", WEATHER, SpeechBudget::Full)).unwrap();
    let first = model.poll().expect("one clause got out");
    assert!(matches!(first, CognitiveEvent::Segment(_)));

    model.cancel("t1", CancelReason::Interrupted);

    assert_eq!(
        drain(&mut model),
        Vec::new(),
        "a turn that was cut short did not finish, and must not say it did"
    );
    assert_eq!(
        model.cancellations(),
        [("t1".to_string(), CancelReason::Interrupted)]
    );
}

/// The decisive one: an interrupted answer is recorded as what was heard.
///
/// The model wrote two sentences. The person heard one and then started talking. A
/// transcript claiming both were said is a system that will later refer back to something
/// it never told anybody.
#[test]
fn an_interrupted_answer_is_recorded_as_what_was_heard_not_what_was_written() {
    let mut log = ResponseCommitLog::new("t1");
    log.record_generated(segment("t1", 0, "It is fourteen degrees.", true))
        .unwrap();
    log.record_generated(segment("t1", 1, "Rain is forecast after six.", true))
        .unwrap();
    log.mark_spoken(0).unwrap();

    assert_eq!(log.spoken_text(), "It is fourteen degrees.");
    assert_eq!(log.unspoken_text(), "Rain is forecast after six.");
    assert_ne!(
        log.spoken_text(),
        log.generated_text(),
        "the written answer and the heard answer are different answers here"
    );
}

/// Nothing spoken means nothing spoken, not "probably the first bit".
#[test]
fn an_answer_cut_off_before_it_started_was_not_heard_at_all() {
    let mut log = ResponseCommitLog::new("t1");
    log.record_generated(segment("t1", 0, "It is fourteen degrees.", true))
        .unwrap();

    assert_eq!(log.spoken_text(), "");
    assert_eq!(log.unspoken_text(), "It is fourteen degrees.");
}

/// Resuming says the rest, never the part already heard.
#[test]
fn resuming_continues_and_does_not_repeat_what_was_already_said() {
    let mut log = ResponseCommitLog::new("t1");
    log.record_generated(segment("t1", 0, "It is fourteen degrees.", true))
        .unwrap();
    log.record_generated(segment("t1", 1, "Rain after six.", true))
        .unwrap();
    log.mark_spoken(0).unwrap();

    assert_eq!(
        log.resume_text(CancelReason::Interrupted),
        "Rain after six."
    );
}

/// Only an interruption leaves an answer worth finishing.
#[test]
fn a_superseded_or_refused_answer_is_not_resumed() {
    let mut log = ResponseCommitLog::new("t1");
    log.record_generated(segment("t1", 0, "The old answer.", true))
        .unwrap();

    assert!(CancelReason::Interrupted.permits_resume());
    for reason in [
        CancelReason::Superseded,
        CancelReason::PolicyRefused,
        CancelReason::Shutdown,
    ] {
        assert!(!reason.permits_resume(), "{reason:?}");
        assert_eq!(
            log.resume_text(reason),
            "",
            "{reason:?} means those words are stale, wrong or forbidden"
        );
    }
}

// --------------------------------------------------------------- the log's own honesty

/// A segment from another turn cannot be filed under this one.
#[test]
fn a_segment_from_another_turn_is_refused() {
    let mut log = ResponseCommitLog::new("t1");
    assert_eq!(
        log.record_generated(segment("t2", 0, "Someone else's answer.", true)),
        Err(CognitiveError::ForeignSegment {
            sequence: 0,
            segment_turn: "t2".into(),
            log_turn: "t1".into(),
        })
    );
}

/// A gap in the sequence means a clause went missing, and the text either side of the gap
/// no longer joins into the sentence that was written.
#[test]
fn a_missing_clause_is_refused_rather_than_stitched_over() {
    let mut log = ResponseCommitLog::new("t1");
    log.record_generated(segment("t1", 0, "First.", true))
        .unwrap();
    assert_eq!(
        log.record_generated(segment("t1", 2, "Third.", true)),
        Err(CognitiveError::OutOfOrder {
            expected: 1,
            got: 2
        })
    );
}

/// A segment cannot have been spoken before it existed.
#[test]
fn a_segment_that_was_never_generated_cannot_have_been_heard() {
    let mut log = ResponseCommitLog::new("t1");
    log.record_generated(segment("t1", 0, "First.", true))
        .unwrap();
    assert_eq!(
        log.mark_spoken(1),
        Err(CognitiveError::UnknownSegment { sequence: 1 })
    );
}

/// Speech does not run backwards, so the spoken mark does not either.
///
/// Accepting a lower sequence would shrink what the person is recorded as having heard,
/// which is how a system convinces itself it may repeat something.
#[test]
fn the_spoken_mark_only_moves_forward() {
    let mut log = ResponseCommitLog::new("t1");
    log.record_generated(segment("t1", 0, "First.", true))
        .unwrap();
    log.record_generated(segment("t1", 1, "Second.", true))
        .unwrap();
    log.mark_spoken(1).unwrap();

    assert_eq!(
        log.mark_spoken(0),
        Err(CognitiveError::SpokenBackwards { sequence: 0 })
    );
    assert_eq!(log.spoken_text(), "First. Second.");
}

/// The log survives the fabric, because the turn and the transcript are rarely written by
/// the same process.
#[test]
fn the_commit_log_round_trips_through_json() {
    let mut log = ResponseCommitLog::new("t1");
    log.record_generated(segment("t1", 0, "Heard.", true))
        .unwrap();
    log.record_generated(segment("t1", 1, "Unheard.", true))
        .unwrap();
    log.mark_spoken(0).unwrap();

    let text = serde_json::to_string(&log).expect("serialisable");
    let back: ResponseCommitLog = serde_json::from_str(&text).expect("deserialisable");
    assert_eq!(back, log);
    assert_eq!(back.spoken_text(), "Heard.");
}

// --------------------------------------------------------------- budget

/// One sentence means one whole sentence.
#[test]
fn a_sentence_budget_stops_at_the_sentence_and_not_inside_it() {
    let mut gate = ResponseGate::new(SpeechBudget::Sentence);
    assert!(gate.admit(&segment("t1", 0, "It is fourteen degrees", false)));
    assert!(
        !gate.is_closed(),
        "the clause did not end the sentence, so the sentence is not over"
    );
    assert!(gate.admit(&segment("t1", 1, "in Santiago.", true)));
    assert!(gate.is_closed());
    assert!(!gate.admit(&segment("t1", 2, "Rain after six.", true)));
}

/// A full budget lets the whole answer out.
#[test]
fn a_full_budget_admits_everything() {
    let mut gate = ResponseGate::new(SpeechBudget::Full);
    for sequence in 0..5 {
        assert!(gate.admit(&segment("t1", sequence, "more", sequence % 2 == 1)));
    }
    assert!(!gate.is_closed());
}

/// A budget that permits no words permits no words.
#[test]
fn a_token_budget_admits_nothing() {
    for budget in [SpeechBudget::None, SpeechBudget::Token] {
        let mut gate = ResponseGate::new(budget);
        assert!(gate.is_closed());
        assert!(!gate.admit(&segment("t1", 0, "anything at all", true)));
    }
}

/// A clause that does not finish a sentence is not a place to stop.
///
/// The boundary is read from the clause, not from its position in the stream, because the
/// consequence of getting it wrong is audible: a sentence budget that closes after "It is
/// fourteen degrees" leaves the person without the half that carried the meaning.
#[test]
fn a_clause_that_does_not_end_a_sentence_is_not_a_boundary() {
    let mut model = ScriptedCognitiveModel::new(&["en"]).answering(
        WEATHER,
        ScriptedReply::saying(&["It is fourteen degrees", "in Santiago."]),
    );
    model.begin(ask("t1", WEATHER, SpeechBudget::Full)).unwrap();

    let boundaries: Vec<bool> = drain(&mut model)
        .iter()
        .filter_map(|event| match event {
            CognitiveEvent::Segment(segment) => Some(segment.ends_sentence),
            _ => None,
        })
        .collect();
    assert_eq!(boundaries, vec![false, true]);
}

/// Two complete sentences are two boundaries, and a one-sentence budget takes the first.
#[test]
fn a_sentence_budget_takes_the_first_whole_sentence_of_a_longer_answer() {
    let mut model = model();
    model
        .begin(ask("t1", WEATHER, SpeechBudget::Sentence))
        .unwrap();

    let mut gate = ResponseGate::new(SpeechBudget::Sentence);
    let spoken: Vec<String> = drain(&mut model)
        .iter()
        .filter_map(|event| match event {
            CognitiveEvent::Segment(segment) if gate.admit(segment) => Some(segment.text.clone()),
            _ => None,
        })
        .collect();

    assert_eq!(
        spoken,
        vec!["It is fourteen degrees in Santiago.".to_string()]
    );
}

// --------------------------------------------------------------- tools

/// A tool proposed by the cognitive plane is correlated by the turn, so the speech
/// plane's proposal for the same utterance collapses into it instead of running twice.
#[test]
fn a_cognitive_tool_intent_is_correlated_by_the_turn_it_came_from() {
    let mut model = ScriptedCognitiveModel::new(&["en"]).answering(
        "open the dashboard",
        ScriptedReply::saying(&["Opening it."]).proposing("open_dashboard", "{}"),
    );
    model
        .begin(ask("t1", "open the dashboard", SpeechBudget::Full))
        .unwrap();

    let intent = drain(&mut model)
        .into_iter()
        .find_map(|event| match event {
            CognitiveEvent::ToolIntent(intent) => Some(intent),
            _ => None,
        })
        .expect("the script proposed a tool");

    assert_eq!(intent.correlation_id, "t1");
    assert_eq!(intent.tool_name, "open_dashboard");
    assert_eq!(intent.source_adapter, "scripted-cognitive");
}
