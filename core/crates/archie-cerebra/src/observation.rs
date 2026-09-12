//! What arrives, and what is known about it before anything has been decided.

use archie_memory::floor::ConversationalFloorState;
use archie_types::PrivacyGrade;
use archie_vault::VaultRef;
use serde::{Deserialize, Serialize};

/// Why this pass is happening.
///
/// Archie is not prompt-driven. A timer, a device and a person are all first-class
/// reasons to think, and a loop shaped so that only the last one works is a chat
/// application with extra steps.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Trigger {
    /// A person said something.
    Utterance,
    /// A scheduled moment arrived.
    Timer { schedule: String },
    /// Something in the environment changed.
    Sensor { device: String },
    /// Another part of the system finished, failed, or noticed something.
    SystemEvent { source: String },
    /// A suspended task became runnable again.
    Resumed { task: String },
}

impl Trigger {
    /// Whether a person is present and waiting on a turn.
    ///
    /// Only an utterance implies turn-taking. A timer firing while somebody happens to
    /// be in the room is not a turn, and treating it as one is how an assistant becomes
    /// the thing that talks during films.
    pub fn is_conversational(&self) -> bool {
        matches!(self, Self::Utterance)
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Utterance => "utterance",
            Self::Timer { .. } => "timer",
            Self::Sensor { .. } => "sensor",
            Self::SystemEvent { .. } => "system-event",
            Self::Resumed { .. } => "resumed",
        }
    }
}

/// One thing to think about.
#[derive(Debug, Clone)]
pub struct Observation {
    pub trigger: Trigger,
    pub at_ms: i64,
    /// Which task this pass belongs to. Capsules are bound to it.
    pub task_id: String,
    /// A label for what this pass is for — not the request itself. Bounded by
    /// `archie_observability::cognition::limits::MAX_GOAL_LABEL`.
    pub goal_label: String,
    /// Exactly what the task needs from the vault, and no more.
    pub context_refs: Vec<VaultRef>,
    /// How sensitive the assembled context will be.
    pub privacy: PrivacyGrade,
    /// The conversational situation, when there is one. Absent for a timer or a sensor
    /// reading: those have no floor to take.
    pub floor: Option<ConversationalFloorState>,
    /// Something happened that overrides ordinary turn-taking — a Sentry alarm, not a
    /// notification. Supplied by trusted runtime policy rather than estimated, because
    /// a component that can declare its own interruption urgent will.
    pub critical_alert: bool,
    pub input_tokens: u32,
    pub max_output_tokens: u32,
}

impl Observation {
    pub fn new(
        trigger: Trigger,
        task_id: impl Into<String>,
        goal_label: impl Into<String>,
        at_ms: i64,
    ) -> Self {
        Self {
            trigger,
            at_ms,
            task_id: task_id.into(),
            goal_label: goal_label.into(),
            context_refs: Vec::new(),
            privacy: PrivacyGrade::Internal,
            floor: None,
            critical_alert: false,
            input_tokens: 1024,
            max_output_tokens: 512,
        }
    }

    pub fn needing(mut self, reference: VaultRef) -> Self {
        self.context_refs.push(reference);
        self
    }

    pub fn graded(mut self, privacy: PrivacyGrade) -> Self {
        self.privacy = privacy;
        self
    }

    pub fn with_floor(mut self, floor: ConversationalFloorState) -> Self {
        self.floor = Some(floor);
        self
    }

    /// Mark this pass as carrying a trusted critical alert.
    ///
    /// Deliberately a separate call rather than a field set during construction: an
    /// override of turn-taking should be visible at the call site that claims it.
    pub fn urgent(mut self) -> Self {
        self.critical_alert = true;
        self
    }

    pub fn sized(mut self, input_tokens: u32, max_output_tokens: u32) -> Self {
        self.input_tokens = input_tokens;
        self.max_output_tokens = max_output_tokens;
        self
    }
}
