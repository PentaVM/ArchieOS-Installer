//! What a speech backend can actually do, said honestly.
//!
//! Speech models differ in ways that are invisible until they matter. One hears and
//! speaks in a single network and can do both at once. One hears and speaks in turn and
//! calls that conversation. One will pronounce any text you hand it; another generates
//! its own words and cannot be made to say yours.
//!
//! That last difference decides the whole architecture. If the speech model writes its
//! own replies, then the cognitive model's answer has nowhere to go — and a system built
//! assuming otherwise produces a voice that confidently says something the thinking half
//! never said.
//!
//! So capabilities are declared, checked for coherence, and never assumed. An adapter
//! that claims something it cannot do is worse than one that admits a gap, because the
//! gap can be routed around.

use serde::{Deserialize, Serialize};

/// What a backend can do.
///
/// Every field is a claim that can be wrong, so [`SpeechCapabilities::incoherences`]
/// exists to catch the combinations that cannot all be true at once.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SpeechCapabilities {
    /// Hears and speaks in one network, at the same time.
    pub native_full_duplex: bool,
    /// Can receive audio while producing it.
    pub simultaneous_listen_speak: bool,
    /// Reacts to being spoken over.
    pub barge_in: bool,
    /// Has its own sense of when a turn ends.
    ///
    /// Useful evidence for the floor estimator and never the decision itself: the model
    /// knows about the sound, and Cerebra knows about the task.
    pub native_turn_taking: bool,

    /// Generates its own words.
    ///
    /// When true, anything it says is the *speech* model's answer and not Archie's, and
    /// must be marked as such before it reaches a person.
    pub generates_semantic_response: bool,
    /// Will accept text from elsewhere to speak.
    pub accepts_external_response_text: bool,
    /// And will speak that text as given, without rewriting it.
    ///
    /// Separate from accepting it: a model may take external text as a *prompt* and
    /// paraphrase. For an answer that Policy approved, paraphrase is not acceptable —
    /// the approved sentence and the spoken sentence must be the same sentence.
    pub renders_external_response_verbatim: bool,

    /// Can propose a tool call while the conversation continues.
    pub tool_intent_output: bool,
    pub partial_transcript: bool,
    pub semantic_stream: bool,
    /// Voice character can be conditioned.
    pub voice_conditioning: bool,

    /// Language tags the backend actually supports.
    ///
    /// Empty means unknown, which is not the same as "all". An English-only checkpoint
    /// that silently accepts Spanish produces confident nonsense in the wrong accent.
    pub languages: Vec<String>,

    pub input_sample_rates: Vec<u32>,
    pub output_sample_rates: Vec<u32>,
}

impl SpeechCapabilities {
    /// Claims that cannot all be true together.
    ///
    /// Returned rather than panicked on, because a backend advertising nonsense should be
    /// refused at registration with a reason, not crash the process that read its
    /// manifest.
    pub fn incoherences(&self) -> Vec<String> {
        let mut problems = Vec::new();

        if self.native_full_duplex && !self.simultaneous_listen_speak {
            problems.push(
                "claims native full duplex but cannot listen while speaking, which is what \
                 full duplex means"
                    .into(),
            );
        }
        if self.native_full_duplex && !self.barge_in {
            problems.push("claims native full duplex but cannot react to being spoken over".into());
        }
        // Speaking somebody else's words verbatim requires being able to accept them.
        if self.renders_external_response_verbatim && !self.accepts_external_response_text {
            problems
                .push("claims to speak external text verbatim while refusing external text".into());
        }
        if self.tool_intent_output && !self.semantic_stream && !self.partial_transcript {
            problems.push(
                "claims tool intents with no semantic or transcript stream to derive them from"
                    .into(),
            );
        }
        if self.languages.is_empty() {
            problems.push(
                "declares no languages; unknown is not the same as all, and a session must \
                 not be routed here on a guess"
                    .into(),
            );
        }
        problems
    }

    pub fn is_coherent(&self) -> bool {
        self.incoherences().is_empty()
    }

    /// Whether this backend supports a language, by tag prefix.
    ///
    /// `es-CL` is served by a backend declaring `es`, and a backend declaring `es-ES` does
    /// not serve a request for plain `es` — the specific satisfies the general and not the
    /// other way round, which is the same rule capabilities use.
    pub fn speaks(&self, language: &str) -> bool {
        self.languages.iter().any(|declared| {
            language == declared
                || language
                    .strip_prefix(declared.as_str())
                    .is_some_and(|rest| rest.starts_with('-'))
        })
    }

    /// Whether an answer produced elsewhere can be spoken by this backend unchanged.
    ///
    /// The question the two-plane design turns on. A unified speech model that writes its
    /// own replies fails this, and a system that assumed otherwise would let the voice say
    /// something the cognitive model never produced and Policy never saw.
    pub fn can_voice_an_external_answer(&self) -> bool {
        self.accepts_external_response_text && self.renders_external_response_verbatim
    }
}

/// Where a spoken segment came from.
///
/// Carried with every piece of audio because "Archie said it" is not specific enough to
/// debug or to trust. A reflex from the speech model and an answer from the cognitive
/// model are different claims about the world, and only one of them went past Policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpokenContentSource {
    /// The cognitive plane produced these words.
    CognitiveModel,
    /// Cerebra's own wording — "I'm checking that" while a large model thinks.
    CerebraTemplate,
    /// A backchannel from the floor policy. Carries no content.
    BackchannelPolicy,
    /// A tool's result, rendered for speech.
    ToolResult,
    /// The speech model's own words, generated without the cognitive plane.
    ///
    /// Fast and useful for filler, and never a substantive answer: nothing checked it.
    SpeechLmReflex,
    /// A system alert.
    SystemAlert,
}

impl SpokenContentSource {
    /// Whether these words may be presented as Archie's actual answer.
    ///
    /// A reflex is a noise that keeps a conversation alive. Letting it stand as the answer
    /// is how a system ends up confidently telling somebody something no part of it
    /// actually reasoned about.
    pub const fn is_substantive_answer(self) -> bool {
        matches!(self, Self::CognitiveModel | Self::ToolResult)
    }
}

/// A tool the speech model thinks the user asked for.
///
/// A *candidate*. It has not been validated, authorised or executed, and the speech model
/// has no way to do any of those. It travels to Cerebra like any other proposal.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpeechToolIntentCandidate {
    /// Correlates this candidate with the same intent arriving from elsewhere.
    pub correlation_id: String,
    pub tool_name: String,
    /// Unparsed. Validating the shape is Cerebra's job, not the transport's.
    pub raw_arguments: String,
    /// Which adapter proposed it.
    pub source_adapter: String,
    pub confidence: Option<f32>,
}

/// Both model planes may propose the same action, and it must happen once.
///
/// The speech model proposes fast and shallowly; the cognitive model proposes slowly and
/// with context. They frequently mean the same thing, and executing both would run it
/// twice — which for anything that moves, opens or sends is not a duplicate log line.
///
/// Correlation is by `correlation_id`, assigned upstream from the utterance both derive
/// from, and never by comparing arguments: two calls with identical arguments may be a
/// duplicate or may be somebody asking twice on purpose.
pub fn deduplicate_tool_intents(
    candidates: Vec<SpeechToolIntentCandidate>,
) -> Vec<SpeechToolIntentCandidate> {
    let mut kept: Vec<SpeechToolIntentCandidate> = Vec::new();
    for candidate in candidates {
        match kept
            .iter_mut()
            .find(|existing| existing.correlation_id == candidate.correlation_id)
        {
            Some(existing) => {
                // Same intent from two planes. Keep the more confident reading; an
                // absent confidence loses to a stated one, because a backend that
                // measured something knows more than one that did not say.
                if candidate.confidence.unwrap_or(0.0) > existing.confidence.unwrap_or(0.0) {
                    *existing = candidate;
                }
            }
            None => kept.push(candidate),
        }
    }
    kept
}
