//! The plane that decides what Archie says.
//!
//! The speech plane owns the floor: who is talking, when to stop, how much may be said.
//! It does not own the words. Those come from here, and the separation is the whole point
//! — a voice model that answers on its own is a second assistant with its own opinions,
//! and Archie's behaviour becomes whatever that checkpoint happens to do this month.
//!
//! Two facts about real conversation shape this contract.
//!
//! The first is that answers arrive over time and speech starts before thinking finishes.
//! So a response is a stream of clauses, not a string, and each clause is independently
//! releasable — the clause is the unit because it is the smallest thing a voice can
//! actually say and the smallest thing a person can actually have heard.
//!
//! The second is that people interrupt. When they do, the answer that was written and the
//! answer that was heard stop being the same answer, and everything downstream — the
//! transcript, the memory, the resume — has to use the one that was heard.

use std::collections::{BTreeMap, VecDeque};

use serde::{Deserialize, Serialize};

use crate::adapter::{SpeechToolIntentCandidate, SpokenContentSource};
use crate::floor::SpeechBudget;

/// A request for an answer.
///
/// It carries the budget the floor granted, because "what to say" and "how much may be
/// said" are not separable questions: an answer written for a paragraph and then cut to
/// one sentence is a different, usually worse, answer than one written for one sentence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CognitiveRequest {
    /// Identifies the turn, and correlates tool intents that both planes propose.
    pub turn_id: String,
    /// What the person said, as the speech plane heard it.
    pub transcript: String,
    /// The language of the conversation, as a tag.
    pub language: String,
    /// How much the floor is willing to let Archie say.
    pub budget: SpeechBudget,
}

/// A releasable piece of an answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResponseSegment {
    pub turn_id: String,
    /// Position within the turn, from zero, without gaps.
    pub sequence: u32,
    /// A clause or sentence, spaced for speech. Never a sub-word token: a fragment that
    /// cannot be spoken alone cannot be committed alone either.
    pub text: String,
    /// Where these words came from. The cognitive plane may legitimately emit
    /// [`SpokenContentSource::ToolResult`] as well as its own words.
    pub source: SpokenContentSource,
    /// Whether the segment completes a sentence, and so is a safe place to stop.
    pub ends_sentence: bool,
}

/// What the cognitive plane emits while working on a turn.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum CognitiveEvent {
    /// Words, ready to be released to the voice.
    Segment(ResponseSegment),
    /// A tool the model believes was requested. A proposal: unvalidated, unauthorised,
    /// unexecuted. Cerebra and Policy decide, exactly as with the speech plane's.
    ToolIntent(SpeechToolIntentCandidate),
    /// The answer is finished. Nothing further will arrive for this turn.
    Completed { turn_id: String },
    /// The turn failed. The reason exists to be logged and, if useful, spoken as a
    /// [`SpokenContentSource::SystemAlert`] — never as the answer, because there isn't one.
    Failed { turn_id: String, reason: String },
}

impl CognitiveEvent {
    pub fn turn_id(&self) -> &str {
        match self {
            Self::Segment(segment) => &segment.turn_id,
            Self::ToolIntent(intent) => &intent.correlation_id,
            Self::Completed { turn_id } | Self::Failed { turn_id, .. } => turn_id,
        }
    }
}

/// Why a turn was stopped.
///
/// It decides whether the unspoken remainder is still worth saying, so it is not a log
/// string. Being interrupted leaves an answer that was still true; being superseded
/// leaves one that answers a question nobody is asking any more.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CancelReason {
    /// The person took the floor. What was being said may still be worth finishing.
    Interrupted,
    /// A newer turn replaced this one. The remainder answers a stale question.
    Superseded,
    /// Policy refused the content. The remainder must not be spoken.
    PolicyRefused,
    /// The process is going away.
    Shutdown,
}

impl CancelReason {
    /// Whether the unspoken remainder may be offered again.
    ///
    /// Only an interruption leaves the answer intact; every other reason means the words
    /// were wrong, stale, or forbidden, and reoffering them is the failure mode where a
    /// system finishes a sentence nobody wanted after the subject already changed.
    pub const fn permits_resume(self) -> bool {
        matches!(self, Self::Interrupted)
    }
}

/// Why a turn could not start, or a commit could not be recorded.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CognitiveError {
    #[error("a turn is already running ({running}); cancel it before starting {requested}")]
    TurnAlreadyRunning { running: String, requested: String },
    #[error("nothing was said; there is no question to answer")]
    EmptyTranscript,
    #[error("the backend does not speak {language}")]
    UnsupportedLanguage { language: String },
    #[error("the floor granted {budget:?}, which is not enough to say an answer")]
    BudgetForbidsAnswering { budget: SpeechBudget },
    #[error("segment {sequence} belongs to turn {segment_turn}, not {log_turn}")]
    ForeignSegment {
        sequence: u32,
        segment_turn: String,
        log_turn: String,
    },
    #[error("segment {got} arrived out of order; expected {expected}")]
    OutOfOrder { expected: u32, got: u32 },
    #[error("segment {sequence} was never generated, so it cannot have been spoken")]
    UnknownSegment { sequence: u32 },
    #[error("segment {sequence} was already passed; speech does not run backwards")]
    SpokenBackwards { sequence: u32 },
    #[error("{0}")]
    Backend(String),
}

/// A backend that turns what was heard into what Archie says.
///
/// Pull-based and synchronous on purpose: the speech fabric already has a clock, and a
/// contract that needs its own runtime cannot be driven deterministically from a test.
pub trait CognitiveModelAdapter {
    /// Which languages this backend answers in, as tags.
    fn languages(&self) -> &[String];

    /// Start a turn. Fails rather than replacing a turn already running, because
    /// silently dropping one loses the record of what was said before it stopped.
    fn begin(&mut self, request: CognitiveRequest) -> Result<(), CognitiveError>;

    /// Take the next event, if there is one. `None` means "not yet", never "finished" —
    /// finished is [`CognitiveEvent::Completed`].
    fn poll(&mut self) -> Option<CognitiveEvent>;

    /// Stop a turn. Emits nothing further for it, including `Completed`: a turn that was
    /// cut short did not complete, and recording otherwise would make the transcript claim
    /// Archie finished a sentence it never said.
    fn cancel(&mut self, turn_id: &str, reason: CancelReason);

    /// Whether a turn is in flight.
    fn running_turn(&self) -> Option<&str>;
}

/// Validates a request against a backend's declared languages.
///
/// Shared so every adapter refuses the same things: an empty transcript, a language the
/// backend does not speak, and a budget that does not permit an answer at all.
pub fn validate_request(
    request: &CognitiveRequest,
    languages: &[String],
) -> Result<(), CognitiveError> {
    if request.transcript.trim().is_empty() {
        return Err(CognitiveError::EmptyTranscript);
    }
    if !matches!(request.budget, SpeechBudget::Sentence | SpeechBudget::Full) {
        return Err(CognitiveError::BudgetForbidsAnswering {
            budget: request.budget,
        });
    }
    let spoken = languages.iter().any(|declared| {
        request.language == *declared
            || request
                .language
                .strip_prefix(declared.as_str())
                .is_some_and(|rest| rest.starts_with('-'))
    });
    if !spoken {
        return Err(CognitiveError::UnsupportedLanguage {
            language: request.language.clone(),
        });
    }
    Ok(())
}

/// Decides how much of a streaming answer the floor's budget allows out.
///
/// Truncation happens at a sentence boundary and nowhere else. Cutting mid-clause does
/// not produce a shorter answer; it produces a sentence that means something different
/// from the one that was written, which is worse than saying nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResponseGate {
    budget: SpeechBudget,
    closed: bool,
}

impl ResponseGate {
    pub fn new(budget: SpeechBudget) -> Self {
        Self {
            budget,
            closed: matches!(budget, SpeechBudget::None | SpeechBudget::Token),
        }
    }

    /// Whether this segment may be spoken. Call once per segment, in order.
    pub fn admit(&mut self, segment: &ResponseSegment) -> bool {
        if self.closed {
            return false;
        }
        if matches!(self.budget, SpeechBudget::Sentence) && segment.ends_sentence {
            // Admitted, and the last: one sentence means this one, complete.
            self.closed = true;
        }
        true
    }

    /// Whether anything further will be admitted.
    pub fn is_closed(&self) -> bool {
        self.closed
    }
}

/// What was written, and what was actually heard.
///
/// After an interruption these differ, and every consumer downstream wants the second
/// one. A transcript that records the generated answer claims Archie said things nobody
/// heard; a memory built on it will later refer back to them as established.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResponseCommitLog {
    turn_id: String,
    segments: Vec<ResponseSegment>,
    /// Sequence of the last segment the voice finished. `None` means nothing was heard.
    spoken_through: Option<u32>,
}

impl ResponseCommitLog {
    pub fn new(turn_id: impl Into<String>) -> Self {
        Self {
            turn_id: turn_id.into(),
            segments: Vec::new(),
            spoken_through: None,
        }
    }

    pub fn turn_id(&self) -> &str {
        &self.turn_id
    }

    /// Record a segment the model produced. Not yet heard by anybody.
    pub fn record_generated(&mut self, segment: ResponseSegment) -> Result<(), CognitiveError> {
        if segment.turn_id != self.turn_id {
            return Err(CognitiveError::ForeignSegment {
                sequence: segment.sequence,
                segment_turn: segment.turn_id,
                log_turn: self.turn_id.clone(),
            });
        }
        let expected = self.segments.len() as u32;
        if segment.sequence != expected {
            return Err(CognitiveError::OutOfOrder {
                expected,
                got: segment.sequence,
            });
        }
        self.segments.push(segment);
        Ok(())
    }

    /// Record that the voice finished speaking a segment.
    pub fn mark_spoken(&mut self, sequence: u32) -> Result<(), CognitiveError> {
        if sequence as usize >= self.segments.len() {
            return Err(CognitiveError::UnknownSegment { sequence });
        }
        if self
            .spoken_through
            .is_some_and(|through| sequence <= through)
        {
            return Err(CognitiveError::SpokenBackwards { sequence });
        }
        self.spoken_through = Some(sequence);
        Ok(())
    }

    /// Everything the model wrote, spoken or not.
    pub fn generated_text(&self) -> String {
        join(self.segments.iter())
    }

    /// What the person actually heard. This is the transcript.
    pub fn spoken_text(&self) -> String {
        match self.spoken_through {
            None => String::new(),
            Some(through) => join(self.segments.iter().take(through as usize + 1)),
        }
    }

    /// What was written but never reached anybody.
    pub fn unspoken_text(&self) -> String {
        let from = self
            .spoken_through
            .map_or(0, |through| through as usize + 1);
        join(self.segments.iter().skip(from))
    }

    /// What to say on resuming, given why the turn stopped.
    ///
    /// Empty when the reason forbids resuming, and never includes what was already heard —
    /// repeating a sentence the person just listened to reads as a malfunction.
    pub fn resume_text(&self, reason: CancelReason) -> String {
        if reason.permits_resume() {
            self.unspoken_text()
        } else {
            String::new()
        }
    }

    pub fn segments(&self) -> &[ResponseSegment] {
        &self.segments
    }
}

fn join<'a>(segments: impl Iterator<Item = &'a ResponseSegment>) -> String {
    let mut out = String::new();
    for segment in segments {
        if !out.is_empty() && !segment.text.is_empty() {
            out.push(' ');
        }
        out.push_str(&segment.text);
    }
    out
}

// --------------------------------------------------------------------------- test backend

/// One prepared answer.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScriptedReply {
    /// Clauses, in order. The last is marked as ending a sentence.
    pub clauses: Vec<String>,
    /// Tool proposals, as `(name, raw arguments)`.
    pub tools: Vec<(String, String)>,
}

/// Whether a clause finishes a sentence, by looking at it.
///
/// A script is written as real clauses, so the boundary is read from the text rather than
/// assumed from the position: a two-clause answer whose first clause is already a complete
/// sentence has two boundaries, and under a one-sentence budget only the first goes out.
/// The final clause always closes, because an answer left trailing off unfinished is not
/// an answer the voice should be holding.
fn ends_a_sentence(clause: &str) -> bool {
    matches!(clause.trim_end().chars().last(), Some('.' | '?' | '!'))
}

impl ScriptedReply {
    pub fn saying(clauses: &[&str]) -> Self {
        Self {
            clauses: clauses.iter().map(|c| (*c).to_string()).collect(),
            tools: Vec::new(),
        }
    }

    pub fn proposing(mut self, tool: &str, arguments: &str) -> Self {
        self.tools.push((tool.to_string(), arguments.to_string()));
        self
    }
}

/// A cognitive backend with no model in it.
///
/// Everything downstream of this contract — floor integration, provenance, commit
/// buffering, tool correlation — needs to be testable without a GPU, a checkpoint, or a
/// network. So this backend answers from a script: the same request produces the same
/// events in the same order, every time.
///
/// A request it has no script for **fails** rather than inventing a polite non-answer,
/// because a plausible-sounding filler is exactly the thing a test would fail to notice.
#[derive(Debug, Clone, Default)]
pub struct ScriptedCognitiveModel {
    languages: Vec<String>,
    replies: BTreeMap<String, ScriptedReply>,
    pending: VecDeque<CognitiveEvent>,
    running: Option<String>,
    cancellations: Vec<(String, CancelReason)>,
}

impl ScriptedCognitiveModel {
    pub fn new(languages: &[&str]) -> Self {
        Self {
            languages: languages.iter().map(|l| (*l).to_string()).collect(),
            ..Default::default()
        }
    }

    /// Prepare the answer to a transcript, matched exactly.
    pub fn answering(mut self, transcript: &str, reply: ScriptedReply) -> Self {
        self.replies.insert(transcript.to_string(), reply);
        self
    }

    /// Every cancellation seen, in order — so a test can assert the fabric cancelled, and
    /// for which reason, rather than only that events stopped arriving.
    pub fn cancellations(&self) -> &[(String, CancelReason)] {
        &self.cancellations
    }
}

impl CognitiveModelAdapter for ScriptedCognitiveModel {
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

        let turn_id = request.turn_id.clone();
        self.running = Some(turn_id.clone());

        let Some(reply) = self.replies.get(&request.transcript).cloned() else {
            self.pending.push_back(CognitiveEvent::Failed {
                turn_id,
                reason: format!("no scripted reply for {:?}", request.transcript),
            });
            return Ok(());
        };

        let last = reply.clauses.len().saturating_sub(1);
        for (index, text) in reply.clauses.iter().enumerate() {
            self.pending
                .push_back(CognitiveEvent::Segment(ResponseSegment {
                    turn_id: turn_id.clone(),
                    sequence: index as u32,
                    text: text.clone(),
                    source: SpokenContentSource::CognitiveModel,
                    ends_sentence: index == last || ends_a_sentence(text),
                }));
        }
        for (tool, arguments) in reply.tools {
            self.pending
                .push_back(CognitiveEvent::ToolIntent(SpeechToolIntentCandidate {
                    // The turn is the correlation: it is what both planes heard.
                    correlation_id: turn_id.clone(),
                    tool_name: tool,
                    raw_arguments: arguments,
                    source_adapter: "scripted-cognitive".to_string(),
                    confidence: Some(0.9),
                }));
        }
        self.pending
            .push_back(CognitiveEvent::Completed { turn_id });
        Ok(())
    }

    fn poll(&mut self) -> Option<CognitiveEvent> {
        let event = self.pending.pop_front()?;
        if matches!(
            event,
            CognitiveEvent::Completed { .. } | CognitiveEvent::Failed { .. }
        ) {
            self.running = None;
        }
        Some(event)
    }

    fn cancel(&mut self, turn_id: &str, reason: CancelReason) {
        self.cancellations.push((turn_id.to_string(), reason));
        self.pending.retain(|event| event.turn_id() != turn_id);
        if self.running.as_deref() == Some(turn_id) {
            self.running = None;
        }
    }

    fn running_turn(&self) -> Option<&str> {
        self.running.as_deref()
    }
}
