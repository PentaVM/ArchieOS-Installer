//! Where the two planes actually touch.
//!
//! The floor decides whether Archie may speak and how much. The cognitive plane decides
//! what the words are. The speech backends listen and pronounce. Each of those is a
//! separate contract, and this is the only place they meet — which makes it the only
//! place their disagreements can be resolved rather than discovered later in a room with
//! somebody in it.
//!
//! Three of those disagreements are worth naming, because they are what this module is
//! for.
//!
//! A session is refused at construction if the voice cannot pronounce an answer written
//! elsewhere. A unified speech model that writes its own replies is a perfectly good
//! conversational system; it is just not one that can say what Cerebra decided, and
//! discovering that when the answer is ready means either silence or a voice improvising.
//!
//! A backchannel does not wake the cognitive plane. "Mhm" is a floor decision with no
//! content, and generating a paragraph to accompany it costs a model run and leaves a
//! paragraph lying around that somebody downstream may decide to say.
//!
//! And words that were written but never released are held, not discarded. A one-sentence
//! budget does not delete the rest of the answer — it defers it, and if the floor opens
//! again the remainder is what resumes.

use crate::adapter::{SpeechCapabilities, SpeechToolIntentCandidate, SpokenContentSource};
use crate::cognitive::{
    CancelReason, CognitiveError, CognitiveEvent, CognitiveModelAdapter, CognitiveRequest,
    ResponseCommitLog, ResponseGate,
};
use crate::floor::{
    ConversationalFloorState, FloorAction, FloorDecision, FloorPolicy, SpeechBudget,
};

/// Which backends serve this conversation, and in what language.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionPlan {
    /// The language of the conversation, as a tag.
    pub language: String,
    /// The backend that hears.
    pub listener: SpeechCapabilities,
    /// The backend that pronounces what Cerebra decided. May be the same one, when it is
    /// willing to say somebody else's words.
    pub voice: SpeechCapabilities,
}

/// Why a session could not be assembled, or a claim about it refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SessionError {
    #[error("the listening backend declares something impossible: {0:?}")]
    ListenerIncoherent(Vec<String>),
    #[error("the voice backend declares something impossible: {0:?}")]
    VoiceIncoherent(Vec<String>),
    #[error(
        "the voice writes its own replies and will not pronounce anybody else's, so \
         Cerebra's answer has nowhere to go; pair it with a renderer"
    )]
    VoiceCannotSpeakAnswers,
    #[error("the listening backend does not hear {language}")]
    ListenerDoesNotSpeak { language: String },
    #[error("the voice backend does not speak {language}")]
    VoiceDoesNotSpeak { language: String },
    #[error("the cognitive backend does not answer in {language}")]
    CognitiveDoesNotAnswer { language: String },
    #[error("segment {sequence} was never released to the voice, so it was not spoken")]
    NotReleased { sequence: u32 },
    #[error("no turn is in progress")]
    NoTurn,
    #[error(transparent)]
    Cognitive(#[from] CognitiveError),
}

/// A piece of an answer, cleared to be said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpokenSegment {
    pub turn_id: String,
    pub sequence: u32,
    /// The exact words. The voice renders verbatim, so what is here is what is heard.
    pub text: String,
    pub source: SpokenContentSource,
}

/// What the session tells the rest of the system.
#[derive(Debug, Clone, PartialEq)]
pub enum SessionEvent {
    /// Say this.
    Speak(SpokenSegment),
    /// These words were written and are not being said, for the stated reason. They stay
    /// in the commit log, because the floor may open again.
    Withheld { sequence: u32, reason: String },
    /// A tool the cognitive plane believes was requested. Unvalidated and unauthorised.
    ToolProposal(SpeechToolIntentCandidate),
    /// The answer is complete.
    Finished,
    /// Something went wrong. May be reported to the person as a system alert; it is
    /// never an answer, because there wasn't one.
    Alert(String),
}

#[derive(Debug, Clone)]
struct ActiveTurn {
    id: String,
    log: ResponseCommitLog,
    gate: ResponseGate,
    /// Sequences actually handed to the voice. The voice cannot report having said
    /// anything else.
    released: Vec<u32>,
    finished: bool,
    cancelled: Option<CancelReason>,
}

/// One conversation, with one person, in one language.
#[derive(Debug, Clone)]
pub struct SpeechSession<C: CognitiveModelAdapter> {
    plan: SessionPlan,
    policy: FloorPolicy,
    cognitive: C,
    turn: Option<ActiveTurn>,
}

impl<C: CognitiveModelAdapter> SpeechSession<C> {
    /// Assemble a session, or explain why these backends cannot hold this conversation.
    ///
    /// Everything checked here is checked before anybody speaks, because each of these
    /// failures is silent at runtime: a voice that paraphrases the approved answer, a
    /// backend confidently pronouncing Spanish it was never trained on.
    pub fn new(plan: SessionPlan, policy: FloorPolicy, cognitive: C) -> Result<Self, SessionError> {
        let listener_problems = plan.listener.incoherences();
        if !listener_problems.is_empty() {
            return Err(SessionError::ListenerIncoherent(listener_problems));
        }
        let voice_problems = plan.voice.incoherences();
        if !voice_problems.is_empty() {
            return Err(SessionError::VoiceIncoherent(voice_problems));
        }
        if !plan.voice.can_voice_an_external_answer() {
            return Err(SessionError::VoiceCannotSpeakAnswers);
        }
        if !plan.listener.speaks(&plan.language) {
            return Err(SessionError::ListenerDoesNotSpeak {
                language: plan.language.clone(),
            });
        }
        if !plan.voice.speaks(&plan.language) {
            return Err(SessionError::VoiceDoesNotSpeak {
                language: plan.language.clone(),
            });
        }
        if !speaks(cognitive.languages(), &plan.language) {
            return Err(SessionError::CognitiveDoesNotAnswer {
                language: plan.language.clone(),
            });
        }

        Ok(Self {
            plan,
            policy,
            cognitive,
            turn: None,
        })
    }

    pub fn plan(&self) -> &SessionPlan {
        &self.plan
    }

    /// Inspect the configured floor without starting cognition or assembling context.
    pub fn floor_decision(&self, state: &ConversationalFloorState) -> FloorDecision {
        self.policy.decide(state)
    }

    /// Somebody said something. Decide what the floor allows, and start thinking only if
    /// the answer could actually be said.
    ///
    /// The decision is returned whether or not a turn began, because "Archie stayed
    /// silent, and here is why" is the most common outcome and the one worth being able
    /// to inspect.
    pub fn heard(
        &mut self,
        turn_id: &str,
        transcript: &str,
        state: &ConversationalFloorState,
    ) -> Result<FloorDecision, SessionError> {
        let decision = self.policy.decide(state);

        // A backchannel is a noise, not an answer, and silence is neither. Waking the
        // cognitive plane for either would generate words nobody asked for and nobody may
        // say. The budget is the whole test: the floor never grants one large enough for
        // an answer to an action that says nothing, which `speaking_actions_are_exactly_
        // the_ones_granted_words` holds it to.
        if !matches!(decision.budget, SpeechBudget::Sentence | SpeechBudget::Full) {
            return Ok(decision);
        }

        self.cognitive.begin(CognitiveRequest {
            turn_id: turn_id.to_string(),
            transcript: transcript.to_string(),
            language: self.plan.language.clone(),
            budget: decision.budget,
        })?;
        self.turn = Some(ActiveTurn {
            id: turn_id.to_string(),
            log: ResponseCommitLog::new(turn_id),
            gate: ResponseGate::new(decision.budget),
            released: Vec::new(),
            finished: false,
            cancelled: None,
        });
        Ok(decision)
    }

    /// Move the turn forward: take whatever the cognitive plane has produced, record it,
    /// and release what the budget allows.
    pub fn pump(&mut self) -> Vec<SessionEvent> {
        let mut events = Vec::new();
        let Some(turn) = self.turn.as_mut() else {
            return events;
        };
        if turn.cancelled.is_some() {
            return events;
        }

        while let Some(event) = self.cognitive.poll() {
            if event.turn_id() != turn.id {
                // Left over from a turn that was cancelled. It belongs to nobody now.
                continue;
            }
            match event {
                CognitiveEvent::Segment(segment) => {
                    let sequence = segment.sequence;
                    let text = segment.text.clone();
                    let source = segment.source;
                    // Recorded before it is released: the log is the record of what was
                    // written, and withheld words were still written.
                    if let Err(error) = turn.log.record_generated(segment.clone()) {
                        events.push(SessionEvent::Alert(error.to_string()));
                        continue;
                    }
                    if turn.gate.admit(&segment) {
                        turn.released.push(sequence);
                        events.push(SessionEvent::Speak(SpokenSegment {
                            turn_id: turn.id.clone(),
                            sequence,
                            text,
                            source,
                        }));
                    } else {
                        events.push(SessionEvent::Withheld {
                            sequence,
                            reason: "the floor granted one sentence, and it has been said"
                                .to_string(),
                        });
                    }
                }
                CognitiveEvent::ToolIntent(intent) => {
                    events.push(SessionEvent::ToolProposal(intent));
                }
                CognitiveEvent::Completed { .. } => {
                    turn.finished = true;
                    events.push(SessionEvent::Finished);
                }
                CognitiveEvent::Failed { reason, .. } => {
                    turn.finished = true;
                    events.push(SessionEvent::Alert(reason));
                }
            }
        }
        events
    }

    /// The voice finished saying a segment.
    ///
    /// Refused for anything that was never released, because the commit log is the record
    /// of what a person heard and a voice cannot have said words it was not given.
    pub fn spoke(&mut self, sequence: u32) -> Result<(), SessionError> {
        let turn = self.turn.as_mut().ok_or(SessionError::NoTurn)?;
        if !turn.released.contains(&sequence) {
            return Err(SessionError::NotReleased { sequence });
        }
        turn.log.mark_spoken(sequence)?;
        Ok(())
    }

    /// The floor moved while Archie was speaking. Stop if it says to.
    ///
    /// Yielding cancels the turn as an interruption, which is the one cancellation that
    /// leaves the remainder worth finishing.
    pub fn floor_moved(&mut self, state: &ConversationalFloorState) -> FloorDecision {
        let decision = self.policy.decide(state);
        if matches!(decision.action, FloorAction::Yield) {
            self.cancel(CancelReason::Interrupted);
        }
        decision
    }

    /// Stop the current turn for a stated reason.
    pub fn cancel(&mut self, reason: CancelReason) {
        let Some(turn) = self.turn.as_mut() else {
            return;
        };
        if turn.cancelled.is_some() {
            return;
        }
        turn.cancelled = Some(reason);
        self.cognitive.cancel(&turn.id, reason);
    }

    /// What Archie would say to pick the answer back up, if it should.
    ///
    /// Two conditions, and both are real. The reason it stopped has to leave the words
    /// worth saying — a superseded answer does not become good again. And the floor has to
    /// judge the response still valid, because between yielding and resuming the person
    /// said something, and the most common reason to interrupt is to change the question.
    pub fn resume(&mut self, state: &ConversationalFloorState) -> Option<String> {
        let turn = self.turn.as_ref()?;
        if let Some(reason) = turn.cancelled {
            if !reason.permits_resume() {
                return None;
            }
        }
        if !self.policy.may_resume(state) {
            return None;
        }
        let remainder = turn.log.unspoken_text();
        (!remainder.is_empty()).then_some(remainder)
    }

    /// What the person actually heard this turn. This is the transcript.
    pub fn heard_text(&self) -> String {
        self.turn
            .as_ref()
            .map(|turn| turn.log.spoken_text())
            .unwrap_or_default()
    }

    /// Everything the model wrote this turn, said or not.
    pub fn written_text(&self) -> String {
        self.turn
            .as_ref()
            .map(|turn| turn.log.generated_text())
            .unwrap_or_default()
    }

    pub fn commit_log(&self) -> Option<&ResponseCommitLog> {
        self.turn.as_ref().map(|turn| &turn.log)
    }

    pub fn cognitive(&self) -> &C {
        &self.cognitive
    }

    /// The cognitive backend, for what the adapter contract does not cover — carrying a
    /// turn past the tools it asked for. What it produces for the running turn still
    /// passes through [`Self::pump`], the gate and the commit log.
    pub fn cognitive_mut(&mut self) -> &mut C {
        &mut self.cognitive
    }
}

/// Tag-prefix language match, the same rule the adapters and the router use.
fn speaks(declared: &[String], language: &str) -> bool {
    declared.iter().any(|tag| {
        language == tag
            || language
                .strip_prefix(tag.as_str())
                .is_some_and(|rest| rest.starts_with('-'))
    })
}
