//! Local baseline, not a validated turn-completion model. No labelled corpus exists.
//! Audio amplitude/timing cannot infer question semantics or task usefulness: those
//! inputs remain caller-owned. Online labels update only a fixed matrix of weights.
use archie_memory::floor::ConversationalFloorState;
use archie_types::Locality;
use serde::{Deserialize, Serialize};
use zeroize::Zeroize;

/// Exactly 20 ms of mono 16 kHz normalised PCM. No capture driver is implied.
pub const FRAME_SAMPLES: usize = 320;
pub const RETENTION_MS: i64 = 7 * 24 * 60 * 60 * 1000;

/// Ephemeral features are neither serializable, printable nor cloneable.
///
/// ```compile_fail
/// use archie_speech::estimator::AudioFeatures;
/// fn persist(f: &AudioFeatures) { let _ = serde_json::to_string(f); }
/// ```
/// ```compile_fail
/// use archie_speech::estimator::AudioFeatures;
/// fn log(f: &AudioFeatures) { println!("{f:?}"); }
/// ```
pub struct AudioFeatures {
    values: [i32; 4],
    valid: bool,
}

impl AudioFeatures {
    /// Consumes a bounded callback buffer, zeroing it before returning. The caller
    /// supplies elapsed silence from its local clock; no samples are stored here.
    pub fn extract(frame: &mut [f32; FRAME_SAMPLES], silence_ms: u32) -> Self {
        let mut sum = 0_i32;
        let mut crossings = 0_i32;
        let mut previous_positive = false;
        let mut valid = true;
        for (i, sample) in frame.iter().enumerate() {
            if !sample.is_finite() || !(-1.0..=1.0).contains(sample) {
                valid = false;
            } else {
                sum += (sample.abs() * 1000.0).round() as i32;
                let positive = *sample > 0.0;
                if i > 0 && positive != previous_positive {
                    crossings += 1;
                }
                previous_positive = positive;
            }
        }
        frame.zeroize();
        Self {
            values: [
                1000,
                sum / FRAME_SAMPLES as i32,
                (silence_ms.min(3000) / 3) as i32,
                crossings * 1000 / (FRAME_SAMPLES as i32 - 1),
            ],
            valid,
        }
    }
}
impl Drop for AudioFeatures {
    fn drop(&mut self) {
        self.values.zeroize();
        self.valid = false;
    }
}

/// Explicit non-acoustic inputs from the local task/session. No strings or content.
#[derive(Default)]
pub struct FloorContext {
    pub response_utility: f64,
    pub context_sufficiency: f64,
    pub urgency: f64,
    pub overlap: bool,
    pub archie_speaking: bool,
    pub explicit_question: bool,
    pub requested_floor: bool,
    pub acknowledgement_only: bool,
    pub listen_preference: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum EstimatorError {
    #[error("floor estimation is available only on this device")]
    NonLocal,
    #[error("invalid floor features, labels or parameters")]
    Invalid,
    #[error("floor parameters expired")]
    Expired,
}

/// Human/verified online targets: hold, completion, resume, horizon/3ms,
/// interruption utility, in permille. Never retained by the estimator.
///
/// ```compile_fail
/// use archie_speech::estimator::FloorTargets;
/// fn retain(t: &FloorTargets) { let _ = serde_json::to_vec(t); }
/// ```
pub struct FloorTargets([i32; 5]);
impl FloorTargets {
    pub fn new(values: [i32; 5]) -> Result<Self, EstimatorError> {
        if values[..4].iter().any(|v| !(0..=1000).contains(v))
            || !(-1000..=1000).contains(&values[4])
        {
            return Err(EstimatorError::Invalid);
        }
        Ok(Self(values))
    }
}
impl Drop for FloorTargets {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

/// Five bounded linear heads. Restricted local parameters, seven-day lifetime;
/// only `snapshot` is persistable. No examples, identities, transcripts or embeddings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FloorEstimator {
    parameters: Parameters,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Parameters {
    version: u32,
    created_at_ms: i64,
    expires_at_ms: i64,
    updates: u32,
    weights: [[i32; 4]; 5],
}

impl FloorEstimator {
    pub fn new(locality: Locality, now_ms: i64) -> Result<Self, EstimatorError> {
        if locality != Locality::Local {
            return Err(EstimatorError::NonLocal);
        }
        if now_ms <= 0 || now_ms > i64::MAX - RETENTION_MS {
            return Err(EstimatorError::Invalid);
        }
        Ok(Self {
            parameters: Parameters {
                version: 1,
                created_at_ms: now_ms,
                expires_at_ms: now_ms + RETENTION_MS,
                updates: 0,
                weights: [
                    [900, 200, -800, 0],
                    [100, -100, 700, 0],
                    [800, 100, -700, 0],
                    [0, -100, 1000, 0],
                    [-1000, -100, 200, 0],
                ],
            },
        })
    }
    pub const fn locality(&self) -> Locality {
        Locality::Local
    }
    pub fn expires_at_ms(&self) -> i64 {
        self.parameters.expires_at_ms
    }
    pub fn updates(&self) -> u32 {
        self.parameters.updates
    }
    fn valid_at(&self, now_ms: i64) -> bool {
        now_ms >= self.parameters.created_at_ms && now_ms < self.parameters.expires_at_ms
    }
    fn predict(&self, features: &AudioFeatures) -> [i32; 5] {
        std::array::from_fn(|i| {
            let value: i64 = self.parameters.weights[i]
                .iter()
                .zip(features.values)
                .map(|(w, x)| i64::from(*w) * i64::from(x))
                .sum();
            (value / 1000).clamp(if i == 4 { -1000 } else { 0 }, 1000) as i32
        })
    }
    /// Produces estimates only; `archie_memory::floor::decide_floor` is untouched.
    /// Invalid audio, task numbers or expired weights produce valid conservative
    /// estimates (hold=1, completion=0, response=-1), hence silence in the kernel.
    pub fn estimate(
        &self,
        features: &AudioFeatures,
        context: &FloorContext,
        now_ms: i64,
    ) -> ConversationalFloorState {
        let task_valid = context.response_utility.is_finite()
            && (-1.0..=1.0).contains(&context.response_utility)
            && context.context_sufficiency.is_finite()
            && (0.0..=1.0).contains(&context.context_sufficiency)
            && context.urgency.is_finite()
            && (0.0..=1.0).contains(&context.urgency);
        let valid = features.valid && task_valid && self.valid_at(now_ms);
        let p = if valid {
            self.predict(features)
        } else {
            [1000, 0, 1000, 0, -1000]
        };
        ConversationalFloorState {
            observed_at_ms: now_ms.max(1),
            current_speaker_ref: None,
            speaker_hold_probability: f64::from(p[0]) / 1000.0,
            thought_completion_probability: f64::from(p[1]) / 1000.0,
            expected_silence_horizon_ms: p[3] as u32 * 3,
            silence_scope_ref: None,
            interruption_utility: f64::from(p[4]) / 1000.0,
            response_utility: if valid {
                context.response_utility
            } else {
                -1.0
            },
            urgency: if valid { context.urgency } else { 0.0 },
            context_sufficiency: if valid {
                context.context_sufficiency
            } else {
                0.0
            },
            predicted_intent_ref: None,
            intent_confidence: 0.0,
            resume_probability: f64::from(p[2]) / 1000.0,
            overlap: context.overlap,
            archie_speaking: context.archie_speaking,
            explicit_question: context.explicit_question,
            requested_floor: context.requested_floor,
            acknowledgement_only: context.acknowledgement_only,
            listen_preference: context.listen_preference,
        }
    }
    /// Online supervised update; consumes features and labels and retains only weights.
    /// The caller must provide actual verified labels, never the head's own guesses.
    pub fn learn(
        &mut self,
        features: AudioFeatures,
        labels: FloorTargets,
        now_ms: i64,
    ) -> Result<(), EstimatorError> {
        if !self.valid_at(now_ms) {
            return Err(EstimatorError::Expired);
        }
        if !features.valid {
            return Err(EstimatorError::Invalid);
        }
        let prediction = self.predict(&features);
        for (i, row) in self.parameters.weights.iter_mut().enumerate() {
            let error = labels.0[i] - prediction[i];
            for (w, x) in row.iter_mut().zip(features.values) {
                *w = (*w + error * x / 16000).clamp(-2000, 2000);
            }
        }
        self.parameters.updates = self.parameters.updates.saturating_add(1);
        Ok(())
    }
    pub fn snapshot(&self) -> Vec<u8> {
        serde_json::to_vec(&self.parameters).expect("fixed integer parameters")
    }
    pub fn restore(bytes: &[u8], locality: Locality, now_ms: i64) -> Result<Self, EstimatorError> {
        if locality != Locality::Local {
            return Err(EstimatorError::NonLocal);
        }
        if bytes.len() > 2048 {
            return Err(EstimatorError::Invalid);
        }
        let p: Parameters = serde_json::from_slice(bytes).map_err(|_| EstimatorError::Invalid)?;
        if p.version != 1
            || p.created_at_ms <= 0
            || p.expires_at_ms.checked_sub(p.created_at_ms) != Some(RETENTION_MS)
            || p.weights
                .iter()
                .flatten()
                .any(|w| !(-2000..=2000).contains(w))
        {
            return Err(EstimatorError::Invalid);
        }
        let estimator = Self { parameters: p };
        if !estimator.valid_at(now_ms) {
            return Err(EstimatorError::Expired);
        }
        Ok(estimator)
    }
}
