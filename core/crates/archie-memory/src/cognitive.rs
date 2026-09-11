//! Versioned, privacy-minimised cognitive experience. No authority or raw payloads.
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const COGNITIVE_SCHEMA_VERSION: u32 = 1;

pub fn reference(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 192
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b":._-/".contains(&b))
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FocusActivation {
    pub node_id: String,
    pub strength: f64,
    pub activated_at_ms: i64,
    pub pinned: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FocusRecord {
    pub focus_id: String,
    pub revision: u64,
    pub objective_ref: String,
    pub parent_focus_id: Option<String>,
    pub suspended: bool,
    pub half_life_ms: i64,
    pub updated_at_ms: i64,
    pub activations: Vec<FocusActivation>,
}

impl FocusRecord {
    pub fn validate(&self) -> Result<(), String> {
        if !reference(&self.focus_id)
            || !reference(&self.objective_ref)
            || self.revision == 0
            || self.revision > i64::MAX as u64
            || self.updated_at_ms <= 0
            || self.half_life_ms <= 0
            || self.activations.len() > 128
            || self
                .parent_focus_id
                .as_ref()
                .is_some_and(|id| !reference(id) || id == &self.focus_id)
        {
            return Err("invalid Focus identity, revision, time, parent or bound".into());
        }
        let mut seen = BTreeSet::new();
        for a in &self.activations {
            if !reference(&a.node_id)
                || !seen.insert(&a.node_id)
                || !a.strength.is_finite()
                || !(0.0..=1.0).contains(&a.strength)
                || a.activated_at_ms <= 0
                || a.activated_at_ms > self.updated_at_ms
            {
                return Err("invalid or duplicate Focus activation".into());
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LearningSignal {
    ContextUseful,
    ContextIrrelevant,
    MissingContext,
    TaskOutcome,
    ProcessSample,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UtilityObservation {
    pub quality: Option<f64>,
    pub relevance: Option<f64>,
    pub confidence: Option<f64>,
    pub latency_ms: Option<f64>,
    pub tokens: Option<f64>,
    pub cpu_ms: Option<f64>,
    pub memory_bytes: Option<f64>,
    pub energy_joules: Option<f64>,
    pub privacy_exposure: Option<f64>,
    pub risk: Option<f64>,
    pub task_success: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrainingEvent {
    pub event_id: String,
    pub subject_id: String,
    pub snapshot_id: String,
    pub focus_id: String,
    pub evidence_ref: String,
    pub signal: LearningSignal,
    pub observed: UtilityObservation,
    pub recorded_at_ms: i64,
}

impl TrainingEvent {
    pub fn validate(&self) -> Result<(), String> {
        if !reference(&self.event_id)
            || !reference(&self.subject_id)
            || !reference(&self.snapshot_id)
            || !reference(&self.evidence_ref)
            || (!self.focus_id.is_empty() && !reference(&self.focus_id))
            || self.recorded_at_ms <= 0
        {
            return Err("invalid training references or time".into());
        }
        let u = &self.observed;
        for (v, max) in [
            (u.quality, 1.0),
            (u.relevance, 1.0),
            (u.confidence, 1.0),
            (u.privacy_exposure, 1.0),
            (u.risk, 1.0),
            (u.latency_ms, 1e15),
            (u.tokens, 1e15),
            (u.cpu_ms, 1e15),
            (u.memory_bytes, 1e18),
            (u.energy_joules, 1e18),
        ] {
            if v.is_some_and(|v| !v.is_finite() || v < 0.0 || v > max) {
                return Err("invalid utility measurement".into());
            }
        }
        let valid = match self.signal {
            LearningSignal::TaskOutcome => u.task_success.is_some(),
            LearningSignal::ProcessSample => {
                u.latency_ms.is_some() || u.memory_bytes.is_some() || u.cpu_ms.is_some()
            }
            _ => u.task_success.is_none(),
        };
        if !valid {
            return Err(
                "signal has no corresponding measurement or asserts unverified success".into(),
            );
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum CognitivePayload {
    FocusUpdated(FocusRecord),
    Training(TrainingEvent),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CognitiveEvent {
    pub schema_version: u32,
    pub event_id: String,
    pub recorded_at_ms: i64,
    pub payload: CognitivePayload,
}

impl CognitiveEvent {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != COGNITIVE_SCHEMA_VERSION
            || !reference(&self.event_id)
            || self.recorded_at_ms <= 0
        {
            return Err("invalid cognitive event envelope".into());
        }
        match &self.payload {
            CognitivePayload::FocusUpdated(f) => {
                f.validate()?;
                if f.updated_at_ms != self.recorded_at_ms {
                    return Err("Focus time mismatch".into());
                }
            }
            CognitivePayload::Training(t) => {
                t.validate()?;
                if t.event_id != self.event_id || t.recorded_at_ms != self.recorded_at_ms {
                    return Err("training identity mismatch".into());
                }
            }
        }
        Ok(())
    }
}
