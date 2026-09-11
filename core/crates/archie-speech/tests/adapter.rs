//! Capabilities have to be honest, because everything downstream trusts them.
//!
//! The fixtures here describe real backends as their documentation describes them. They
//! are not predictions about how those models perform — they are the claims a manifest
//! would carry, and the point is that the architecture behaves correctly given each.

use archie_speech::adapter::{
    deduplicate_tool_intents, SpeechCapabilities, SpeechToolIntentCandidate, SpokenContentSource,
};

/// A unified full-duplex model that writes its own replies.
///
/// This is the shape NVIDIA's VoiceChat card describes: one network that hears and
/// speaks, with its own language backbone producing the words. Whether it can be made to
/// pronounce text from elsewhere is *not documented*, so it is declared false — the
/// conservative reading, and the one that routes around the gap instead of into it.
fn unified_duplex_english() -> SpeechCapabilities {
    SpeechCapabilities {
        native_full_duplex: true,
        simultaneous_listen_speak: true,
        barge_in: true,
        native_turn_taking: true,
        generates_semantic_response: true,
        accepts_external_response_text: false,
        renders_external_response_verbatim: false,
        tool_intent_output: true,
        partial_transcript: true,
        semantic_stream: true,
        voice_conditioning: false,
        languages: vec!["en".into()],
        input_sample_rates: vec![16_000],
        output_sample_rates: vec![22_050],
    }
}

/// A plain text-to-speech backend: says what it is given, decides nothing.
fn renderer_spanish() -> SpeechCapabilities {
    SpeechCapabilities {
        accepts_external_response_text: true,
        renders_external_response_verbatim: true,
        languages: vec!["es".into(), "en".into()],
        output_sample_rates: vec![24_000],
        ..Default::default()
    }
}

// ------------------------------------------------------------------ the decisive question

/// The question the whole two-plane design turns on.
///
/// If the speech model writes its own replies and will not pronounce anybody else's, then
/// the cognitive model's answer cannot go out through it. A system that assumed otherwise
/// would let the voice say something the thinking half never produced.
#[test]
fn a_model_that_writes_its_own_replies_cannot_voice_someone_elses_answer() {
    assert!(
        !unified_duplex_english().can_voice_an_external_answer(),
        "a backend that generates its own words must not be handed Cerebra's answer"
    );
    assert!(renderer_spanish().can_voice_an_external_answer());
}

/// Accepting text and speaking it unchanged are two different promises.
///
/// A backend may take external text as a *prompt* and paraphrase it. For an answer Policy
/// approved, paraphrase is not acceptable: the approved sentence and the spoken sentence
/// have to be the same sentence.
#[test]
fn accepting_text_is_not_the_same_as_speaking_it_unchanged() {
    let paraphraser = SpeechCapabilities {
        accepts_external_response_text: true,
        renders_external_response_verbatim: false,
        languages: vec!["en".into()],
        ..Default::default()
    };

    assert!(
        !paraphraser.can_voice_an_external_answer(),
        "an approved sentence that gets reworded on the way out was not the approved sentence"
    );
    // And it is a coherent thing to claim, unlike the reverse.
    assert!(paraphraser.is_coherent());
}

// --------------------------------------------------------------------- coherence

/// Claiming full duplex while unable to listen while speaking is claiming the words
/// rather than the property.
#[test]
fn full_duplex_that_cannot_listen_while_speaking_is_refused() {
    let pretender = SpeechCapabilities {
        native_full_duplex: true,
        simultaneous_listen_speak: false,
        barge_in: false,
        languages: vec!["en".into()],
        ..Default::default()
    };

    let problems = pretender.incoherences();
    assert!(!pretender.is_coherent());
    assert!(
        problems.iter().any(|p| p.contains("listen while speaking")),
        "{problems:?}"
    );
    assert!(
        problems.iter().any(|p| p.contains("spoken over")),
        "{problems:?}"
    );
}

/// Speaking external text verbatim requires being able to receive it at all.
#[test]
fn verbatim_rendering_without_accepting_text_is_refused() {
    let impossible = SpeechCapabilities {
        accepts_external_response_text: false,
        renders_external_response_verbatim: true,
        languages: vec!["en".into()],
        ..Default::default()
    };
    assert!(impossible
        .incoherences()
        .iter()
        .any(|p| p.contains("verbatim")));
}

/// A backend that proposes tools with no stream to derive them from is guessing.
#[test]
fn tool_intents_need_something_to_derive_them_from() {
    let guessing = SpeechCapabilities {
        tool_intent_output: true,
        semantic_stream: false,
        partial_transcript: false,
        languages: vec!["en".into()],
        ..Default::default()
    };
    assert!(guessing
        .incoherences()
        .iter()
        .any(|p| p.contains("tool intents")));
}

/// Declaring no languages is not declaring all of them.
///
/// The specific failure: an English-only checkpoint handed a Spanish session, producing
/// confident nonsense rather than an error.
#[test]
fn a_backend_that_names_no_language_is_not_assumed_to_speak_every_one() {
    let silent_about_it = SpeechCapabilities {
        accepts_external_response_text: true,
        renders_external_response_verbatim: true,
        ..Default::default()
    };
    assert!(!silent_about_it.is_coherent());
    assert!(!silent_about_it.speaks("es"));
    assert!(!silent_about_it.speaks("en"));
}

/// The real fixtures are coherent, which is the point of writing them from the cards.
#[test]
fn the_documented_backends_declare_something_possible() {
    assert!(unified_duplex_english().is_coherent());
    assert!(renderer_spanish().is_coherent());
}

// --------------------------------------------------------------------- language

/// A regional tag is served by the language, and not the other way round.
#[test]
fn a_regional_variant_is_served_by_the_language_it_belongs_to() {
    let renderer = renderer_spanish();
    assert!(renderer.speaks("es"));
    assert!(renderer.speaks("es-CL"), "Chilean Spanish is Spanish");
    assert!(!renderer.speaks("pt"));
    // A prefix that is not a tag boundary is a different language.
    assert!(!renderer.speaks("esperanto"));

    let regional = SpeechCapabilities {
        languages: vec!["es-ES".into()],
        ..Default::default()
    };
    assert!(
        !regional.speaks("es"),
        "a backend that only does peninsular Spanish has not offered Spanish in general"
    );
}

/// The English-only duplex model must refuse a Spanish session rather than try.
///
/// This is the concrete consequence for Nelson: the target speech architecture is
/// English-only today, so a Spanish conversation routes to a different backend.
#[test]
fn an_english_only_duplex_model_refuses_spanish() {
    assert!(unified_duplex_english().speaks("en"));
    assert!(!unified_duplex_english().speaks("es"));
    assert!(renderer_spanish().speaks("es"));
}

// --------------------------------------------------------------------- provenance

/// A reflex is not an answer.
///
/// Filler keeps a conversation alive while something slow thinks. Letting it stand as the
/// answer is how a system confidently tells somebody something no part of it reasoned
/// about.
#[test]
fn only_reasoned_content_counts_as_an_answer() {
    assert!(SpokenContentSource::CognitiveModel.is_substantive_answer());
    assert!(SpokenContentSource::ToolResult.is_substantive_answer());

    for filler in [
        SpokenContentSource::SpeechLmReflex,
        SpokenContentSource::CerebraTemplate,
        SpokenContentSource::BackchannelPolicy,
        SpokenContentSource::SystemAlert,
    ] {
        assert!(
            !filler.is_substantive_answer(),
            "{filler:?} was never reasoned about and must not stand as the answer"
        );
    }
}

// --------------------------------------------------------------------- tool correlation

fn candidate(
    correlation: &str,
    tool: &str,
    adapter: &str,
    confidence: Option<f32>,
) -> SpeechToolIntentCandidate {
    SpeechToolIntentCandidate {
        correlation_id: correlation.into(),
        tool_name: tool.into(),
        raw_arguments: "{}".into(),
        source_adapter: adapter.into(),
        confidence,
    }
}

/// Both planes hear the same request and both propose it. It must happen once.
///
/// For anything that moves, opens or sends, executing twice is not a duplicate log line.
#[test]
fn one_utterance_proposed_by_two_planes_executes_once() {
    let both = vec![
        candidate("utterance-1", "open_dashboard", "speech", Some(0.6)),
        candidate("utterance-1", "open_dashboard", "cognitive", Some(0.9)),
    ];

    let kept = deduplicate_tool_intents(both);
    assert_eq!(kept.len(), 1);
    assert_eq!(
        kept[0].source_adapter, "cognitive",
        "the more confident reading survives"
    );
}

/// Asking twice on purpose is not a duplicate.
///
/// Correlation is by the utterance the intent came from, never by comparing arguments —
/// two identical calls may be one intent seen twice or somebody genuinely repeating
/// themselves, and only the correlation knows which.
#[test]
fn the_same_request_made_twice_runs_twice() {
    let asked_twice = vec![
        candidate("utterance-1", "open_dashboard", "speech", Some(0.9)),
        candidate("utterance-2", "open_dashboard", "speech", Some(0.9)),
    ];
    assert_eq!(deduplicate_tool_intents(asked_twice).len(), 2);
}

/// A backend that stated a confidence knows more than one that did not.
#[test]
fn a_stated_confidence_beats_an_absent_one() {
    let mixed = vec![
        candidate("u", "act", "silent", None),
        candidate("u", "act", "measured", Some(0.4)),
    ];
    let kept = deduplicate_tool_intents(mixed);
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0].source_adapter, "measured");
}

/// Different intents from one utterance are different intents.
#[test]
fn distinct_actions_are_not_collapsed() {
    let two_actions = vec![
        candidate("u1", "open_dashboard", "speech", Some(0.8)),
        candidate("u2", "close_window", "speech", Some(0.8)),
    ];
    assert_eq!(deduplicate_tool_intents(two_actions).len(), 2);
}

/// Nothing in, nothing out.
#[test]
fn no_candidates_produce_no_actions() {
    assert!(deduplicate_tool_intents(Vec::new()).is_empty());
}

/// Capabilities cross the fabric, because the backend and the router are rarely on the
/// same machine.
#[test]
fn capabilities_round_trip_through_json() {
    let original = unified_duplex_english();
    let text = serde_json::to_string(&original).expect("serialisable");
    let back: SpeechCapabilities = serde_json::from_str(&text).expect("deserialisable");
    assert_eq!(back, original);
    assert!(!back.can_voice_an_external_answer());
}
