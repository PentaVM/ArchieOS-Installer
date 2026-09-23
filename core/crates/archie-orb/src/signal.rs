use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const TFOG_SCHEMA: u32 = 1;
pub const TOKEN_DIMENSIONS: usize = 256;
pub const MAX_SIGNAL_BYTES: usize = 16 * 1024;
pub const MAX_FOCUS_NODES: usize = 128;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntityKind {
    File,
    Directory,
    Process,
    Service,
    Device,
    Module,
    Library,
    Window,
    App,
    Control,
    Memory,
    Tool,
    Model,
    Node,
    System,
    Abstract,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StageStatus {
    Predicted,
    Active,
    Verified,
    Error,
}

/// Sanitized, transient observations used only to update the orb.
///
/// Speech carries a feature vector, never transcript/audio. Stage carries an opaque
/// operation reference and target identity, never the command or typed text. The
/// event codec rejects unknown fields so raw payloads cannot be smuggled through.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Signal {
    pub schema: u32,
    pub at_ms: i64,
    #[serde(flatten)]
    pub kind: SignalKind,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "signal", rename_all = "snake_case", deny_unknown_fields)]
pub enum SignalKind {
    EntityUpsert {
        entity: OrbEntity,
    },
    EntityRemove {
        id: String,
    },
    Listen {
        active: bool,
        audio_envelope: f32,
    },
    Think {
        active: bool,
        trace_id: String,
        working_set: BTreeMap<String, f32>,
    },
    Speak {
        active: bool,
        turn_id: String,
        semantic_features: Vec<f32>,
        audio_envelope: f32,
    },
    Focus {
        focus_id: String,
        scores: BTreeMap<String, f32>,
    },
    Stage {
        operation_id: String,
        app_id: String,
        surface: u32,
        pid: Option<u32>,
        status: StageStatus,
    },
    AccessibilitySelection {
        app_id: String,
        surface: u32,
        pid: Option<u32>,
        control_id: String,
    },
}

/// An entity feature record supplied by the source that owns the entity.
///
/// `token` is a normalized 256-channel vector. Content-bearing sources must derive it
/// locally and discard their raw content before publishing. Class probabilities are
/// multi-label; `id` is identity only and never used as a coordinate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OrbEntity {
    pub id: String,
    pub parent_id: Option<String>,
    /// Bounded authoritative relationships such as declared module dependencies.
    pub related_ids: Vec<String>,
    pub kind: EntityKind,
    pub token: Vec<f32>,
    pub class_probs: BTreeMap<String, f32>,
    pub size_bytes: u64,
    pub activity: f32,
    pub importance: f32,
    pub uncertainty: f32,
    pub runtime: Vec<f32>,
    pub label: String,
}

#[derive(Debug, thiserror::Error)]
pub enum SignalError {
    #[error("TFOG signal is malformed or exceeds its bound")]
    Invalid,
    #[error("TFOG signal uses a schema this runtime does not understand")]
    Schema,
    #[error("TFOG signal encoding failed")]
    Encoding,
    #[error("TFOG signal transport is unavailable")]
    Unavailable,
}

impl Signal {
    pub fn validate(&self) -> Result<(), SignalError> {
        if self.schema != TFOG_SCHEMA || self.at_ms <= 0 {
            return Err(SignalError::Schema);
        }
        match &self.kind {
            SignalKind::EntityUpsert { entity } => validate_entity(entity),
            SignalKind::EntityRemove { id } => valid_ref(id),
            SignalKind::Listen { audio_envelope, .. }
            | SignalKind::Speak { audio_envelope, .. } => {
                if finite_unit(*audio_envelope) {
                    if let SignalKind::Speak {
                        turn_id,
                        semantic_features,
                        ..
                    } = &self.kind
                    {
                        valid_ref(turn_id)?;
                        if semantic_features.len() > TOKEN_DIMENSIONS
                            || semantic_features.iter().any(|v| !v.is_finite())
                        {
                            return Err(SignalError::Invalid);
                        }
                    }
                    Ok(())
                } else {
                    Err(SignalError::Invalid)
                }
            }
            SignalKind::Think {
                trace_id,
                working_set,
                ..
            } => {
                valid_ref(trace_id)?;
                validate_scores(working_set)
            }
            SignalKind::Focus { focus_id, scores } => {
                valid_ref(focus_id)?;
                validate_scores(scores)
            }
            SignalKind::Stage {
                operation_id,
                app_id,
                ..
            } => {
                valid_ref(operation_id)?;
                valid_ref(app_id)
            }
            SignalKind::AccessibilitySelection {
                app_id, control_id, ..
            } => {
                valid_ref(app_id)?;
                valid_ref(control_id)
            }
        }
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, SignalError> {
        if bytes.is_empty() || bytes.len() > MAX_SIGNAL_BYTES {
            return Err(SignalError::Invalid);
        }
        let signal: Self = serde_json::from_slice(bytes).map_err(|_| SignalError::Invalid)?;
        signal.validate()?;
        Ok(signal)
    }

    pub fn encode(&self) -> Result<Vec<u8>, SignalError> {
        self.validate()?;
        let bytes = serde_json::to_vec(self).map_err(|_| SignalError::Encoding)?;
        if bytes.len() > MAX_SIGNAL_BYTES {
            return Err(SignalError::Invalid);
        }
        Ok(bytes)
    }
}

fn validate_entity(entity: &OrbEntity) -> Result<(), SignalError> {
    valid_ref(&entity.id)?;
    if entity
        .parent_id
        .as_ref()
        .is_some_and(|id| valid_ref(id).is_err())
        || entity.token.len() != TOKEN_DIMENSIONS
        || entity.token.iter().any(|v| !v.is_finite())
        || entity.runtime.len() > 64
        || entity.runtime.iter().any(|v| !v.is_finite())
        || entity.related_ids.len() > 64
        || entity.related_ids.iter().any(|id| valid_ref(id).is_err())
        || !finite_unit(entity.activity)
        || !finite_unit(entity.importance)
        || !finite_unit(entity.uncertainty)
        || entity.class_probs.len() > 256
        || entity
            .class_probs
            .iter()
            .any(|(name, p)| name.is_empty() || !finite_unit(*p))
        || entity.label.len() > 256
    {
        return Err(SignalError::Invalid);
    }
    Ok(())
}

fn validate_scores(scores: &BTreeMap<String, f32>) -> Result<(), SignalError> {
    if scores.len() > MAX_FOCUS_NODES
        || scores
            .iter()
            .any(|(id, score)| valid_ref(id).is_err() || !finite_unit(*score))
    {
        return Err(SignalError::Invalid);
    }
    Ok(())
}

fn valid_ref(value: &str) -> Result<(), SignalError> {
    if value.is_empty() || value.len() > 192 || value.chars().any(char::is_control) {
        return Err(SignalError::Invalid);
    }
    Ok(())
}

fn finite_unit(value: f32) -> bool {
    value.is_finite() && (0.0..=1.0).contains(&value)
}
