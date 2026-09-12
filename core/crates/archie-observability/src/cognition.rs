//! The cognitive trace: one pass through the loop, recorded without its contents.
//!
//! # Why this lives here and not beside the subsystems
//!
//! Every stage of the loop already keeps its own record. The router returns a
//! `RoutingRecord`, the vault issues a capsule receipt, the MCP gate produces a call
//! receipt, memory admission returns a decision. Each is good, and none of them joins
//! up: there is no answer to "what happened to this request", which is the question an
//! operator asks and the one a HUD has to render.
//!
//! This module is that join, and it lives in `archie-observability` because the
//! dependency direction has to run *towards* it. `archie-displayd` already depends on
//! this crate; if the trace lived beside the router, the display would have to depend on
//! the router in order to draw a panel. So the trace holds plain summaries and each
//! subsystem projects its own receipt into one. That projection is the internal
//! protocol: stages agree on this vocabulary rather than on each other's types.
//!
//! # The rule: shape, never content
//!
//! A trace records that a model was selected, which one, under which privacy grade, and
//! why in a stable category. It does not record the prompt, the reasoning, the tool
//! arguments, the retrieved context or the answer. That is not a storage decision — a
//! trace is the artefact most likely to be shipped to a dashboard, pasted into an issue
//! or read over a shoulder while debugging, and it is the last place private material
//! should survive.
//!
//! [`Subject`] exists for the same reason. A free-form `String` field is where a prompt
//! eventually ends up; a typed identifier cannot hold one by accident.

use archie_types::{PrivacyGrade, TaskId, TraceId};
use serde::{Deserialize, Serialize};

/// Bounds. Anything over these is a caller putting prose where an identifier goes.
pub mod limits {
    pub const MAX_GOAL_LABEL: usize = 120;
    pub const MAX_IDENTIFIER: usize = 128;
    pub const MAX_REASON: usize = 64;
    pub const MAX_STAGES: usize = 256;
    pub const MAX_CONTEXT_SOURCES: usize = 64;
}

/// Where in the loop a record was made.
///
/// These are the stages of the cognitive loop in the order they normally occur. They are
/// not a state machine: a pass may skip stages, repeat them, or stop early, and several
/// end at [`StageOutcome::Silent`], which is the most common correct ending.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Stage {
    /// Something arrived: an utterance, a sensor reading, a timer, an event.
    Observe,
    /// Context was assembled for it.
    Contextualize,
    /// What the observation was taken to mean.
    Interpret,
    /// Deliberation, including which model did it.
    Reason,
    /// A plan was formed.
    Plan,
    /// A capability was chosen to carry out a step.
    SelectCapability,
    /// Policy and authority were consulted.
    Authorize,
    /// Something was done.
    Act,
    /// The result was checked against what was expected.
    Verify,
    /// World or task state was updated.
    UpdateState,
    /// Memory was offered the result, and accepted or refused it.
    Remember,
}

impl Stage {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Observe => "observe",
            Self::Contextualize => "contextualize",
            Self::Interpret => "interpret",
            Self::Reason => "reason",
            Self::Plan => "plan",
            Self::SelectCapability => "select-capability",
            Self::Authorize => "authorize",
            Self::Act => "act",
            Self::Verify => "verify",
            Self::UpdateState => "update-state",
            Self::Remember => "remember",
        }
    }
}

impl std::fmt::Display for Stage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// How a stage ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "kebab-case")]
pub enum StageOutcome {
    /// It did what it set out to do.
    Proceeded,
    /// A rule refused it. The reason is a stable code, not an explanation.
    Refused { reason: String },
    /// Legitimate, and waiting on a person.
    AwaitingApproval { reason: String },
    /// Deliberately did nothing, and that was the right answer.
    ///
    /// Distinct from `Refused`: nothing was blocked. Most passes through an always-on
    /// loop should end here, and a system that cannot record a chosen silence cannot be
    /// told apart from one that broke quietly.
    Silent { reason: String },
    /// It tried and failed for a reason that was not a policy.
    Failed { reason: String },
}

impl StageOutcome {
    pub fn reason(&self) -> Option<&str> {
        match self {
            Self::Proceeded => None,
            Self::Refused { reason }
            | Self::AwaitingApproval { reason }
            | Self::Silent { reason }
            | Self::Failed { reason } => Some(reason),
        }
    }

    pub fn proceeded(&self) -> bool {
        matches!(self, Self::Proceeded)
    }
}

/// What a stage acted on, as an identifier.
///
/// Typed rather than a `String` so that a caller cannot put the thing itself here. Each
/// payload is an identifier by contract, and is length-bounded by
/// [`StageRecord::validate`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Subject {
    Model { id: String },
    Capability { name: String },
    Tool { server: String, tool: String },
    Capsule { id: String },
    VaultEntry { reference: String },
    Memory { record: String },
    Device { id: String },
    Provider { id: String },
}

impl Subject {
    fn identifiers(&self) -> Vec<&str> {
        match self {
            Self::Model { id }
            | Self::Capsule { id }
            | Self::Device { id }
            | Self::Provider { id } => vec![id],
            Self::Capability { name } => vec![name],
            Self::Tool { server, tool } => vec![server, tool],
            Self::VaultEntry { reference } => vec![reference],
            Self::Memory { record } => vec![record],
        }
    }
}

/// What a stage cost. Numbers only.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StageCost {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u32>,
    /// Currency micro-units, matching `archie_types::TokenCost`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_micros: Option<u64>,
}

impl StageCost {
    fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// One thing that happened, in one stage.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StageRecord {
    pub stage: Stage,
    pub at_ms: i64,
    pub outcome: StageOutcome,
    /// The sensitivity this stage handled, where it handled data at all.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub privacy: Option<PrivacyGrade>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<Subject>,
    #[serde(default, skip_serializing_if = "StageCost::is_empty")]
    pub cost: StageCost,
}

impl StageRecord {
    pub fn new(stage: Stage, at_ms: i64, outcome: StageOutcome) -> Self {
        Self {
            stage,
            at_ms,
            outcome,
            privacy: None,
            subject: None,
            cost: StageCost::default(),
        }
    }

    pub fn about(mut self, subject: Subject) -> Self {
        self.subject = Some(subject);
        self
    }

    pub fn handling(mut self, privacy: PrivacyGrade) -> Self {
        self.privacy = Some(privacy);
        self
    }

    pub fn costing(mut self, cost: StageCost) -> Self {
        self.cost = cost;
        self
    }

    pub fn validate(&self) -> Result<(), TraceError> {
        if self.at_ms <= 0 {
            return Err(TraceError::Time);
        }
        if let Some(reason) = self.outcome.reason() {
            if reason.is_empty() || reason.len() > limits::MAX_REASON {
                return Err(TraceError::Reason);
            }
            // A reason is a code a UI can group by. Whitespace means a sentence, and a
            // sentence about a refusal tends to quote what was refused.
            if reason.contains(char::is_whitespace) {
                return Err(TraceError::ReasonIsProse);
            }
        }
        if let Some(subject) = &self.subject {
            for identifier in subject.identifiers() {
                if identifier.is_empty() || identifier.len() > limits::MAX_IDENTIFIER {
                    return Err(TraceError::Identifier);
                }
            }
        }
        Ok(())
    }
}

/// One pass through the cognitive loop.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CognitiveTrace {
    pub trace_id: TraceId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<TaskId>,
    /// A short label for what this pass was for.
    ///
    /// A label, not the request. "summarise-inbox" belongs here; the inbox does not.
    /// Bounded at [`limits::MAX_GOAL_LABEL`] so that a caller pasting an utterance in
    /// fails rather than succeeding quietly.
    pub goal_label: String,
    pub started_at_ms: i64,
    /// Where context came from, as references.
    #[serde(default)]
    pub context_sources: Vec<Subject>,
    #[serde(default)]
    pub stages: Vec<StageRecord>,
}

impl CognitiveTrace {
    pub fn new(trace_id: TraceId, goal_label: impl Into<String>, started_at_ms: i64) -> Self {
        Self {
            trace_id,
            task_id: None,
            goal_label: goal_label.into(),
            started_at_ms,
            context_sources: Vec::new(),
            stages: Vec::new(),
        }
    }

    pub fn for_task(mut self, task_id: TaskId) -> Self {
        self.task_id = Some(task_id);
        self
    }

    pub fn sourced_from(mut self, subject: Subject) -> Self {
        self.context_sources.push(subject);
        self
    }

    /// Append a record, refusing one that does not hold to the rules.
    ///
    /// Returns an error rather than dropping it: a trace that silently discards the
    /// record of a refusal is worse than no trace, because it looks complete.
    pub fn record(&mut self, record: StageRecord) -> Result<(), TraceError> {
        record.validate()?;
        if self.stages.len() >= limits::MAX_STAGES {
            return Err(TraceError::TooManyStages);
        }
        self.stages.push(record);
        Ok(())
    }

    pub fn validate(&self) -> Result<(), TraceError> {
        if self.goal_label.is_empty() || self.goal_label.len() > limits::MAX_GOAL_LABEL {
            return Err(TraceError::GoalLabel);
        }
        if self.started_at_ms <= 0 {
            return Err(TraceError::Time);
        }
        if self.context_sources.len() > limits::MAX_CONTEXT_SOURCES {
            return Err(TraceError::TooManySources);
        }
        for record in &self.stages {
            record.validate()?;
        }
        Ok(())
    }

    /// The last record for a stage, if the pass reached it.
    pub fn last(&self, stage: Stage) -> Option<&StageRecord> {
        self.stages.iter().rev().find(|r| r.stage == stage)
    }

    /// The most sensitive grade this pass handled anywhere.
    pub fn peak_privacy(&self) -> Option<PrivacyGrade> {
        self.stages.iter().filter_map(|r| r.privacy).max()
    }

    /// Total measured latency across the stages that reported one.
    pub fn total_latency_ms(&self) -> u32 {
        self.stages
            .iter()
            .filter_map(|r| r.cost.latency_ms)
            .fold(0, u32::saturating_add)
    }

    pub fn total_cost_micros(&self) -> u64 {
        self.stages
            .iter()
            .filter_map(|r| r.cost.cost_micros)
            .fold(0, u64::saturating_add)
    }

    /// Whether this pass ended without Archie doing anything outwardly.
    pub fn was_silent(&self) -> bool {
        !self
            .stages
            .iter()
            .any(|r| r.stage == Stage::Act && r.outcome.proceeded())
    }

    /// What a display may show.
    pub fn hud(&self) -> HudProjection {
        HudProjection {
            trace_id: self.trace_id,
            goal_label: self.goal_label.clone(),
            stage: self.stages.last().map(|r| r.stage),
            model: self.subject_of(Stage::Reason),
            capability: self.subject_of(Stage::SelectCapability),
            privacy: self.peak_privacy(),
            context_source_count: self.context_sources.len(),
            authorization: self.last(Stage::Authorize).map(|r| r.outcome.clone()),
            execution: self.last(Stage::Act).map(|r| r.outcome.clone()),
            verification: self.last(Stage::Verify).map(|r| r.outcome.clone()),
            memory: self.last(Stage::Remember).map(|r| r.outcome.clone()),
            silent: self.was_silent(),
            total_latency_ms: self.total_latency_ms(),
            total_cost_micros: self.total_cost_micros(),
        }
    }

    fn subject_of(&self, stage: Stage) -> Option<Subject> {
        self.last(stage).and_then(|r| r.subject.clone())
    }
}

/// What an interface is allowed to render.
///
/// Everything here is a shape: identifiers, grades, outcomes, counts and numbers. A HUD
/// built on this can show that Archie consulted four private sources under `restricted`,
/// chose a local model, and stayed silent — without showing any of the four sources, the
/// reasoning, or what it decided not to say.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HudProjection {
    pub trace_id: TraceId,
    pub goal_label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage: Option<Stage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<Subject>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capability: Option<Subject>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub privacy: Option<PrivacyGrade>,
    pub context_source_count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authorization: Option<StageOutcome>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution: Option<StageOutcome>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verification: Option<StageOutcome>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory: Option<StageOutcome>,
    /// True when nothing outward happened. Worth rendering: a silent assistant that
    /// looks identical to a stalled one gets restarted.
    pub silent: bool,
    pub total_latency_ms: u32,
    pub total_cost_micros: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum TraceError {
    #[error("goal label is empty or longer than a label should be")]
    GoalLabel,
    #[error("timestamps must be positive")]
    Time,
    #[error("reason is empty or over-long")]
    Reason,
    #[error("reason must be a stable code, not a sentence")]
    ReasonIsProse,
    #[error("subject identifier is empty or over-long")]
    Identifier,
    #[error("trace has more stages than a single pass can have")]
    TooManyStages,
    #[error("trace names more context sources than permitted")]
    TooManySources,
}
