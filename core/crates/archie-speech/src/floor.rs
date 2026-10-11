//! Who has the floor, and what Archie should do about it.
//!
//! A conversation is not a sequence of turns separated by silence. People pause mid
//! thought, say "mhm" without wanting the floor, interrupt to correct a premise, and
//! finish each other's sentences. A system that models turn-taking as *silence longer
//! than N milliseconds* gets all four of those wrong, and gets them wrong in the way that
//! makes somebody stop talking to it.
//!
//! So there is no `LISTEN` mode here and no silence timer. There is a continuously
//! maintained estimate of the floor, and a policy that reads it.
//!
//! # The three questions
//!
//! Every decision answers them in order, and they are genuinely separate:
//!
//! 1. **Can** Archie speak — is the floor available, or would this be an interruption?
//! 2. **Should** Archie speak — is there anything worth saying?
//! 3. **How much** — a sentence, an acknowledgement, or the whole answer?
//!
//! Having a response is not a reason to speak. The most common correct action in a
//! conversation is silence, and a system that answers every recognised utterance is
//! exhausting in a way that is hard to name and easy to feel.

use serde::{Deserialize, Serialize};

/// Who is talking.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Speaker {
    /// The person Archie belongs to.
    Owner,
    Archie,
    /// Somebody else in the room. Present because a second voice changes what an
    /// interruption means: talking over the owner and talking over a guest are different
    /// social acts.
    Other,
    /// Audio is arriving and nobody has been identified yet.
    Unidentified,
    /// Nobody is speaking.
    Nobody,
}

/// How the two streams currently overlap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OverlapState {
    None,
    /// The user started while Archie was already speaking.
    UserOverArchie,
    /// Archie started while the user was already speaking.
    ArchieOverUser,
}

/// Who most recently cut across whom.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InterruptionDirection {
    None,
    UserInterruptedArchie,
    ArchieInterruptedUser,
}

/// What a silence is likely to mean.
///
/// The same 400 milliseconds is a breath in the middle of a clause, a beat between two
/// clauses, or the end of a turn. Treating all three as "they stopped" is the single
/// most common way a voice assistant talks over somebody.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SilenceScope {
    /// Mid-clause. They are still saying this sentence.
    WithinClause,
    /// Between clauses. They may continue or may be done.
    BetweenClauses,
    /// The utterance sounds finished.
    EndOfTurn,
    Unknown,
}

/// A probability in `[0, 1]`.
///
/// A newtype rather than a bare `f32` because every field in the state below is one, and
/// a state built from an out-of-range or NaN estimate would make the policy's comparisons
/// meaningless in a way that produces no error, just strange behaviour.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct Probability(f32);

impl<'de> Deserialize<'de> for Probability {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = f32::deserialize(deserializer)?;
        if !value.is_finite() || !(0.0..=1.0).contains(&value) {
            return Err(serde::de::Error::custom(
                "probability must be finite and in [0, 1]",
            ));
        }
        Ok(Self(value))
    }
}

impl Probability {
    pub const ZERO: Self = Self(0.0);
    pub const ONE: Self = Self(1.0);

    /// Clamps, and treats NaN as zero.
    ///
    /// NaN is the dangerous input: every comparison against it is false, so a NaN
    /// `response_utility` would silently mean "never speak" and a NaN
    /// `speaker_hold_probability` would mean "always take the floor". Folding it to zero
    /// makes the failure a quiet conservative one rather than a quiet erratic one.
    pub fn new(value: f32) -> Self {
        if value.is_nan() {
            return Self(0.0);
        }
        Self(value.clamp(0.0, 1.0))
    }

    pub fn get(self) -> f32 {
        self.0
    }
}

impl Default for Probability {
    fn default() -> Self {
        Self::ZERO
    }
}

/// What is believed about the floor right now.
///
/// Every field is an estimate from the `FloorEstimator`, and every one carries the
/// provenance of the signals it came from — because a decision to interrupt somebody
/// should be explicable afterwards, not merely correct.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConversationalFloorState {
    pub current_speaker: Speaker,

    /// How likely the current speaker is to keep going.
    ///
    /// High during "Move Postgres to the server, but…" even though there is a pause,
    /// because "but" is a promise of more.
    pub speaker_hold: Probability,
    /// How likely the utterance is a complete thought rather than a fragment.
    pub thought_completion: Probability,

    pub silence_scope: SilenceScope,
    /// How long the current silence has run.
    pub silence_ms: u32,

    /// What Archie would gain by cutting in now, against what it costs socially.
    pub interruption_utility: Probability,
    /// Whether anything Archie could say is worth saying.
    pub response_utility: Probability,
    /// How much this cannot wait.
    pub urgency: Probability,
    /// Whether Archie understands enough to answer at all.
    pub context_sufficiency: Probability,

    /// The user's utterance is an acknowledgement and nothing more — "okay", "yeah",
    /// "mhm". It carries no request and asks for no floor.
    pub acknowledgement_only: Probability,

    pub overlap: OverlapState,
    pub recent_interruption: InterruptionDirection,

    /// Whether a response Archie had begun is still valid.
    ///
    /// Falls when the user adds a constraint that invalidates it, which is why resuming
    /// after an interruption is a decision rather than a rewind.
    pub response_still_valid: Probability,

    /// What the estimate was built from. Free-form, and meant to be read by a person
    /// looking at why Archie interrupted them.
    #[serde(default)]
    pub evidence: Vec<String>,
}

impl Default for ConversationalFloorState {
    /// Silence, knowing nothing.
    ///
    /// Deliberately the state in which the policy says nothing: an observatory that
    /// starts talking because it has not measured anything yet is worse than one that
    /// waits.
    fn default() -> Self {
        Self {
            current_speaker: Speaker::Nobody,
            speaker_hold: Probability::ZERO,
            thought_completion: Probability::ZERO,
            silence_scope: SilenceScope::Unknown,
            silence_ms: 0,
            interruption_utility: Probability::ZERO,
            response_utility: Probability::ZERO,
            urgency: Probability::ZERO,
            context_sufficiency: Probability::ZERO,
            acknowledgement_only: Probability::ZERO,
            overlap: OverlapState::None,
            recent_interruption: InterruptionDirection::None,
            response_still_valid: Probability::ONE,
            evidence: Vec::new(),
        }
    }
}

/// What to do about the floor.
///
/// Actions, not modes. Archie is not "in listening state"; it decided to be silent this
/// moment and will decide again on the next estimate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FloorAction {
    /// Say nothing and keep listening. The most common correct answer.
    Silent,
    /// "mhm" — signal presence without taking the floor.
    Backchannel,
    /// A sentence, not an essay.
    ShortResponse,
    /// Stop speaking because the user has something to say. Keeps the response for
    /// possible resumption.
    Yield,
    /// Cut in gently: worth saying, not an emergency.
    SoftInterrupt,
    /// Cut in now. Reserved for when letting them continue costs real work.
    StrongInterrupt,
    /// The floor is free and there is something to say.
    TakeFloor,
    /// Keep speaking through a harmless overlap.
    ContinueSpeaking,
}

impl FloorAction {
    /// Whether this action puts audio into the room.
    pub const fn produces_speech(self) -> bool {
        matches!(
            self,
            Self::Backchannel
                | Self::ShortResponse
                | Self::SoftInterrupt
                | Self::StrongInterrupt
                | Self::TakeFloor
                | Self::ContinueSpeaking
        )
    }

    /// Whether this action speaks over somebody who is already speaking.
    pub const fn is_interruption(self) -> bool {
        matches!(self, Self::SoftInterrupt | Self::StrongInterrupt)
    }
}

/// How much Archie may say.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpeechBudget {
    /// Nothing.
    None,
    /// A backchannel token.
    Token,
    /// One sentence.
    Sentence,
    /// As much as the answer needs.
    Full,
}

/// A decision, with the reason it was made.
///
/// The reason is not decoration. An operator who was interrupted deserves to know what
/// Archie thought was worth interrupting for, and a benchmark harness needs to compare
/// decisions rather than only outcomes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FloorDecision {
    pub action: FloorAction,
    pub budget: SpeechBudget,
    pub reason: String,
}

impl FloorDecision {
    fn new(action: FloorAction, budget: SpeechBudget, reason: &str) -> Self {
        Self {
            action,
            budget,
            reason: reason.to_string(),
        }
    }
}

/// The thresholds a decision turns on.
///
/// Data rather than constants in the code, so they can be calibrated from replayed
/// conversations instead of argued about.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct FloorPolicy {
    /// Above this, the speaker is judged to be continuing and the floor is not free.
    pub speaker_hold_threshold: f32,
    /// Below this, there is nothing worth saying.
    pub response_utility_threshold: f32,
    /// Interrupting needs to clear a higher bar than answering.
    pub soft_interrupt_threshold: f32,
    /// And cutting somebody off mid-sentence needs a higher one still.
    pub strong_interrupt_threshold: f32,
    /// Urgency required before a strong interruption is available at all.
    pub strong_interrupt_urgency: f32,
    /// Above this, an overlapping utterance is treated as a mere acknowledgement and
    /// does not stop Archie.
    pub acknowledgement_threshold: f32,
    /// Below this, Archie does not know enough to answer and keeps listening.
    pub context_sufficiency_threshold: f32,
    /// Below this, a half-finished response is stale and must not be resumed.
    pub resume_validity_threshold: f32,
}

impl Default for FloorPolicy {
    fn default() -> Self {
        Self {
            speaker_hold_threshold: 0.5,
            response_utility_threshold: 0.35,
            // Interrupting is expensive, so it needs more evidence than answering.
            soft_interrupt_threshold: 0.7,
            strong_interrupt_threshold: 0.85,
            strong_interrupt_urgency: 0.8,
            acknowledgement_threshold: 0.7,
            context_sufficiency_threshold: 0.4,
            resume_validity_threshold: 0.5,
        }
    }
}

impl FloorPolicy {
    /// Decide what to do about the floor.
    ///
    /// The order of the checks is the design. Archie's own speech is considered first,
    /// because the question "should I stop talking" has to be answered before "should I
    /// start"; then interruption, which is the only way to speak over somebody; then the
    /// ordinary case of a free floor.
    pub fn decide(&self, state: &ConversationalFloorState) -> FloorDecision {
        if !self.valid() {
            return FloorDecision::new(
                FloorAction::Silent,
                SpeechBudget::None,
                "invalid floor thresholds",
            );
        }
        match state.current_speaker {
            Speaker::Archie => self.while_archie_speaks(state),
            Speaker::Owner | Speaker::Other | Speaker::Unidentified => {
                self.while_someone_else_speaks(state)
            }
            Speaker::Nobody => self.while_nobody_speaks(state),
        }
    }

    /// Archie has the floor and somebody may be speaking over it.
    fn while_archie_speaks(&self, state: &ConversationalFloorState) -> FloorDecision {
        if state.overlap != OverlapState::UserOverArchie {
            return FloorDecision::new(
                FloorAction::ContinueSpeaking,
                SpeechBudget::Full,
                "nobody is speaking over this",
            );
        }

        // "yeah", "mhm", "right" while Archie finishes a sentence. Stopping would make
        // Archie unusable to anybody who makes listening noises, which is most people.
        if state.acknowledgement_only.get() >= self.acknowledgement_threshold {
            return FloorDecision::new(
                FloorAction::ContinueSpeaking,
                SpeechBudget::Full,
                "the overlap is an acknowledgement, not a bid for the floor",
            );
        }

        FloorDecision::new(
            FloorAction::Yield,
            SpeechBudget::None,
            "the user began saying something of their own",
        )
    }

    /// Somebody else has the floor.
    fn while_someone_else_speaks(&self, state: &ConversationalFloorState) -> FloorDecision {
        // Interruption is the only way to speak here, and it has to be worth it.
        if state.interruption_utility.get() >= self.strong_interrupt_threshold
            && state.urgency.get() >= self.strong_interrupt_urgency
        {
            return FloorDecision::new(
                FloorAction::StrongInterrupt,
                SpeechBudget::Sentence,
                "letting this continue would cost more than cutting in",
            );
        }
        if state.interruption_utility.get() >= self.soft_interrupt_threshold {
            return FloorDecision::new(
                FloorAction::SoftInterrupt,
                SpeechBudget::Sentence,
                "there is something worth saying before they go further",
            );
        }

        // A backchannel is not taking the floor; it is evidence of listening. Only during
        // a genuinely long hold, where silence would read as absence.
        if state.speaker_hold.get() >= self.speaker_hold_threshold && state.silence_ms >= 1_500 {
            return FloorDecision::new(
                FloorAction::Backchannel,
                SpeechBudget::Token,
                "they are still going and a long silence would read as absence",
            );
        }

        FloorDecision::new(
            FloorAction::Silent,
            SpeechBudget::None,
            "they have the floor",
        )
    }

    /// Nobody is speaking. The interesting case is *why* nobody is speaking.
    fn while_nobody_speaks(&self, state: &ConversationalFloorState) -> FloorDecision {
        // A pause inside a thought is not an invitation. This is the check that stops
        // Archie finishing somebody's sentence for them.
        if state.silence_scope == SilenceScope::WithinClause
            || state.speaker_hold.get() >= self.speaker_hold_threshold
        {
            return FloorDecision::new(
                FloorAction::Silent,
                SpeechBudget::None,
                "the pause is inside a thought that is still going",
            );
        }

        // An acknowledgement that ended the turn wants no answer. "Okay." after Archie
        // said it was already working on something is a full stop, not a question.
        if state.acknowledgement_only.get() >= self.acknowledgement_threshold {
            return FloorDecision::new(
                FloorAction::Silent,
                SpeechBudget::None,
                "that was an acknowledgement; answering it would be noise",
            );
        }

        if state.response_utility.get() < self.response_utility_threshold {
            return FloorDecision::new(
                FloorAction::Silent,
                SpeechBudget::None,
                "there is nothing worth saying",
            );
        }

        // Knowing there is something to say is not the same as knowing enough to say it.
        if state.context_sufficiency.get() < self.context_sufficiency_threshold {
            return FloorDecision::new(
                FloorAction::Silent,
                SpeechBudget::None,
                "not enough context to answer yet; still listening",
            );
        }

        if state.thought_completion.get() < self.speaker_hold_threshold {
            return FloorDecision::new(
                FloorAction::Silent,
                SpeechBudget::None,
                "the last utterance did not sound finished",
            );
        }

        let budget = if state.response_utility.get() >= 0.8 {
            SpeechBudget::Full
        } else {
            SpeechBudget::Sentence
        };
        let action = if budget == SpeechBudget::Full {
            FloorAction::TakeFloor
        } else {
            FloorAction::ShortResponse
        };
        FloorDecision::new(action, budget, "the floor is free and there is an answer")
    }

    /// Whether a response Archie stopped mid-way may be picked up again.
    ///
    /// Never automatic. Between yielding and resuming, the user said something — and the
    /// most common reason to interrupt is to change the question. Speaking the rest of an
    /// answer to a question that no longer exists is worse than saying nothing, because
    /// it sounds like Archie was not listening.
    pub fn may_resume(&self, state: &ConversationalFloorState) -> bool {
        self.valid()
            && state.current_speaker == Speaker::Nobody
            && state.silence_scope != SilenceScope::WithinClause
            && state.speaker_hold.get() < self.speaker_hold_threshold
            && state.response_still_valid.get() >= self.resume_validity_threshold
    }

    fn valid(&self) -> bool {
        [
            self.speaker_hold_threshold,
            self.response_utility_threshold,
            self.soft_interrupt_threshold,
            self.strong_interrupt_threshold,
            self.strong_interrupt_urgency,
            self.acknowledgement_threshold,
            self.context_sufficiency_threshold,
            self.resume_validity_threshold,
        ]
        .iter()
        .all(|value| value.is_finite() && (0.0..=1.0).contains(value))
    }
}
