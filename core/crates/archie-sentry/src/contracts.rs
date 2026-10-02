use archie_types::{Timestamp, TraceId};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct ContractError(pub &'static str);

pub fn validate_id(id: &str) -> Result<(), ContractError> {
    if id.is_empty()
        || id.len() > 128
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
    {
        return Err(ContractError("invalid stable identifier"));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandKind {
    MediaPause,
    MediaPlay,
    SwitchSet,
    LightSetBrightness,
    ClimateSetTemperature,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    MediaPause,
    MediaPlay,
    SwitchSet { on: bool },
    LightSetBrightness { percent: u8 },
    ClimateSetTemperature { milli_celsius: i32 },
}
impl Command {
    pub fn capability(&self) -> CommandKind {
        match self {
            Self::MediaPause => CommandKind::MediaPause,
            Self::MediaPlay => CommandKind::MediaPlay,
            Self::SwitchSet { .. } => CommandKind::SwitchSet,
            Self::LightSetBrightness { .. } => CommandKind::LightSetBrightness,
            Self::ClimateSetTemperature { .. } => CommandKind::ClimateSetTemperature,
        }
    }
    pub fn validate(&self) -> Result<(), ContractError> {
        match self {
            Self::LightSetBrightness { percent } if *percent > 100 => {
                Err(ContractError("brightness outside 0..=100 percent"))
            }
            Self::ClimateSetTemperature { milli_celsius }
                if !(5000..=35000).contains(milli_celsius) =>
            {
                Err(ContractError(
                    "temperature outside 5000..=35000 milli-Celsius",
                ))
            }
            _ => Ok(()),
        }
    }
    pub fn matches(&self, state: &ResourceState) -> bool {
        match (self, state) {
            (Self::MediaPause, ResourceState::Media { playing: false })
            | (Self::MediaPlay, ResourceState::Media { playing: true }) => true,
            (Self::SwitchSet { on }, ResourceState::Switch { on: actual }) => on == actual,
            (
                Self::LightSetBrightness { percent },
                ResourceState::Light {
                    on,
                    brightness_percent,
                },
            ) => *percent == *brightness_percent && *on == (*percent > 0),
            (
                Self::ClimateSetTemperature { milli_celsius },
                ResourceState::Climate {
                    target_milli_celsius,
                },
            ) => milli_celsius == target_milli_celsius,
            _ => false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ResourceState {
    Media { playing: bool },
    Switch { on: bool },
    Light { on: bool, brightness_percent: u8 },
    Climate { target_milli_celsius: i32 },
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Intent {
    pub schema_version: u16,
    pub intent_id: String,
    pub operation_id: u64,
    pub trace_id: TraceId,
    pub resource_id: String,
    pub command: Command,
    pub created_at: Timestamp,
    pub deadline_at: Timestamp,
    pub max_observation_age_ms: u32,
    pub verify: bool,
}
impl Intent {
    pub fn validate(&self) -> Result<(), ContractError> {
        if self.schema_version != 1 {
            return Err(ContractError("unsupported intent schema"));
        }
        validate_id(&self.intent_id)?;
        validate_id(&self.resource_id)?;
        if self.operation_id == 0 || self.operation_id > i64::MAX as u64 {
            return Err(ContractError("operation identifier outside durable range"));
        }
        let duration = self
            .deadline_at
            .0
            .checked_sub(self.created_at.0)
            .ok_or(ContractError("invalid deadline"))?;
        if !(1..=30000).contains(&duration) {
            return Err(ContractError("deadline outside 1..=30000 ms"));
        }
        if !(1..=30000).contains(&self.max_observation_age_ms) {
            return Err(ContractError("freshness outside 1..=30000 ms"));
        }
        if !self.verify {
            return Err(ContractError(
                "mutation requires postcondition verification",
            ));
        }
        self.command.validate()
    }
    pub fn digest(&self) -> Result<String, ContractError> {
        self.validate()?;
        let bytes =
            serde_json::to_vec(self).map_err(|_| ContractError("intent encoding failed"))?;
        Ok(format!("{:x}", Sha256::digest(bytes)))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservationOrigin {
    Measurement,
    ModelHypothesis,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Observation {
    pub resource_id: String,
    pub adapter_id: String,
    pub evidence_id: String,
    pub observed_at: Timestamp,
    pub origin: ObservationOrigin,
    pub state: ResourceState,
}
impl Observation {
    pub fn validate_after(
        &self,
        intent: &Intent,
        adapter: &str,
        since: Timestamp,
        now: Timestamp,
    ) -> Result<(), ContractError> {
        validate_id(&self.evidence_id)?;
        if self.resource_id != intent.resource_id || self.adapter_id != adapter {
            return Err(ContractError("observation identity mismatch"));
        }
        if self.origin != ObservationOrigin::Measurement {
            return Err(ContractError(
                "hypothesis cannot verify a physical postcondition",
            ));
        }
        if self.observed_at < since || self.observed_at > now || now >= intent.deadline_at {
            return Err(ContractError("observation outside execution interval"));
        }
        let age = now
            .0
            .checked_sub(self.observed_at.0)
            .ok_or(ContractError("invalid observation time"))?;
        if age > i64::from(intent.max_observation_age_ms) {
            return Err(ContractError("stale observation"));
        }
        match self.state {
            ResourceState::Light {
                brightness_percent, ..
            } if brightness_percent > 100 => Err(ContractError("invalid measured brightness")),
            ResourceState::Climate {
                target_milli_celsius,
            } if !(5000..=35000).contains(&target_milli_celsius) => {
                Err(ContractError("invalid measured temperature"))
            }
            _ => Ok(()),
        }
    }
}
