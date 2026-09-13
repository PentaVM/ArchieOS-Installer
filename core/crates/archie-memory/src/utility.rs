//! Small integer context-utility learner. References/outcomes only, no content.
//! Weights are Restricted/local state, expire after seven days and must be encrypted
//! by the persistence adapter. Expiry does not renew on training or restore.

use crate::{
    admission::CuratedTrainingRecord,
    cognitive::{LearningSignal, TrainingEvent},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const RETENTION_MS: i64 = 7 * 24 * 60 * 60 * 1000;
const MAX_REFERENCES: usize = 128;
const MAX_UPDATES: usize = 1024;
const MIN_UPDATES: usize = 4;
const MAX_SNAPSHOT_BYTES: usize = 512 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum UtilityError {
    #[error("invalid utility event or parameters")]
    Invalid,
    #[error("utility observation is not bound to this admitted record")]
    NotAdmitted,
    #[error("process measurements do not attribute reference utility")]
    Unattributed,
    #[error("utility observation was already consumed")]
    Replay,
    #[error("bounded utility model is full")]
    Capacity,
    #[error("utility model or observation expired")]
    Expired,
}

/// Canonical event digest to be verified by the supervision producer before admission.
/// Changing a reward after admission changes the digest and cannot train the model.
pub fn event_digest(event: &TrainingEvent) -> Result<String, UtilityError> {
    event.validate().map_err(|_| UtilityError::Invalid)?;
    let bytes = serde_json::to_vec(event).map_err(|_| UtilityError::Invalid)?;
    Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
}

/// Not constructible or deserializable without a real admission receipt.
///
/// ```compile_fail
/// use archie_memory::{LearningCandidate, utility::ContextUtilityModel};
/// fn bypass(model: &mut ContextUtilityModel, raw: &LearningCandidate) {
///     model.train(raw, 1000).unwrap();
/// }
/// ```
pub struct AdmittedUtilitySample {
    reference: String,
    digest: [u8; 32],
    reward: i32,
    at_ms: i64,
}

impl AdmittedUtilitySample {
    pub fn bind(
        record: &CuratedTrainingRecord,
        event: &TrainingEvent,
    ) -> Result<Self, UtilityError> {
        if event_digest(event)? != record.evidence_sha256()
            || record.source_ref() != event.event_id
            || record.evidence_ref() != event.evidence_ref
            || record.observed_at_ms() != event.recorded_at_ms
        {
            return Err(UtilityError::NotAdmitted);
        }
        let base = match event.signal {
            LearningSignal::ContextUseful => 1000,
            LearningSignal::ContextIrrelevant => -1000,
            LearningSignal::MissingContext => 500,
            LearningSignal::TaskOutcome => {
                if event.observed.task_success == Some(true) {
                    1000
                } else {
                    -1000
                }
            }
            LearningSignal::ProcessSample => return Err(UtilityError::Unattributed),
        };
        let permille = |v: Option<f64>, default: i32| {
            v.map(|x| (x * 1000.0).round() as i32).unwrap_or(default)
        };
        // Missing privacy measurement is worst-case, never free disclosure.
        let reward = base
            + permille(event.observed.quality, 0) / 2
            + permille(event.observed.relevance, 0) / 2
            - 2 * permille(event.observed.privacy_exposure, 1000)
            - permille(event.observed.risk, 1000);
        Ok(Self {
            reference: event.evidence_ref.clone(),
            digest: Sha256::digest(record.evidence_sha256().as_bytes()).into(),
            reward,
            at_ms: event.recorded_at_ms,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Weight {
    reference_hash: [u8; 32],
    total: i64,
    count: u32,
}

/// Deterministic empirical utility, with no stochastic optimisation or GPU dependency.
/// The seed salts reference hashes; it never changes a privacy rule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ContextUtilityModel {
    version: u32,
    seed: u64,
    created_at_ms: i64,
    expires_at_ms: i64,
    weights: Vec<Weight>,
    consumed: Vec<[u8; 32]>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Snapshot {
    version: u32,
    seed: u64,
    created_at_ms: i64,
    expires_at_ms: i64,
    weights: Vec<Weight>,
    consumed: Vec<[u8; 32]>,
}

impl ContextUtilityModel {
    pub fn new(seed: u64, now_ms: i64) -> Result<Self, UtilityError> {
        if now_ms <= 0 || now_ms > i64::MAX - RETENTION_MS {
            return Err(UtilityError::Invalid);
        }
        Ok(Self {
            version: 1,
            seed,
            created_at_ms: now_ms,
            expires_at_ms: now_ms + RETENTION_MS,
            weights: Vec::new(),
            consumed: Vec::new(),
        })
    }
    pub fn expires_at_ms(&self) -> i64 {
        self.expires_at_ms
    }
    pub fn trained_samples(&self) -> usize {
        self.consumed.len()
    }
    fn hash(&self, reference: &str) -> [u8; 32] {
        let mut hash = Sha256::new();
        hash.update(self.seed.to_le_bytes());
        hash.update(reference.as_bytes());
        hash.finalize().into()
    }
    pub fn train(
        &mut self,
        sample: AdmittedUtilitySample,
        now_ms: i64,
    ) -> Result<(), UtilityError> {
        if now_ms < self.created_at_ms
            || now_ms >= self.expires_at_ms
            || sample.at_ms > now_ms
            || sample.at_ms < self.created_at_ms
        {
            return Err(UtilityError::Expired);
        }
        if self.consumed.contains(&sample.digest) {
            return Err(UtilityError::Replay);
        }
        let hash = self.hash(&sample.reference);
        let position = self.weights.iter().position(|w| w.reference_hash == hash);
        if self.consumed.len() >= MAX_UPDATES
            || (position.is_none() && self.weights.len() >= MAX_REFERENCES)
        {
            return Err(UtilityError::Capacity);
        }
        if let Some(i) = position {
            self.weights[i].total += i64::from(sample.reward);
            self.weights[i].count += 1;
        } else {
            self.weights.push(Weight {
                reference_hash: hash,
                total: i64::from(sample.reward),
                count: 1,
            });
        }
        self.weights.sort_by_key(|w| w.reference_hash);
        self.consumed.push(sample.digest);
        self.consumed.sort();
        Ok(())
    }
    /// Returns indices into the task's request, so this learner cannot name an extra
    /// context source. Unknown/undertrained references retain deterministic priority.
    pub fn select_indices(&self, requested: &[String], now_ms: i64) -> Vec<usize> {
        let mut indices: Vec<_> = (0..requested.len()).collect();
        if self.consumed.len() < MIN_UPDATES
            || now_ms < self.created_at_ms
            || now_ms >= self.expires_at_ms
        {
            return indices;
        }
        let scores: Vec<i64> = requested
            .iter()
            .map(|r| {
                self.weights
                    .iter()
                    .find(|w| w.reference_hash == self.hash(r) && w.count >= 2)
                    .map(|w| w.total / i64::from(w.count))
                    .unwrap_or(0)
            })
            .collect();
        indices.retain(|i| scores[*i] >= 0);
        indices.sort_by(|a, b| scores[*b].cmp(&scores[*a]).then(a.cmp(b)));
        indices
    }
    pub fn snapshot(&self) -> Vec<u8> {
        serde_json::to_vec(self).expect("integer-only bounded parameters")
    }
    pub fn restore(bytes: &[u8], now_ms: i64) -> Result<Self, UtilityError> {
        if bytes.len() > MAX_SNAPSHOT_BYTES {
            return Err(UtilityError::Capacity);
        }
        let s: Snapshot = serde_json::from_slice(bytes).map_err(|_| UtilityError::Invalid)?;
        if s.version != 1
            || s.created_at_ms <= 0
            || s.expires_at_ms.checked_sub(s.created_at_ms) != Some(RETENTION_MS)
            || s.weights.len() > MAX_REFERENCES
            || s.consumed.len() > MAX_UPDATES
            || s.weights
                .windows(2)
                .any(|w| w[0].reference_hash >= w[1].reference_hash)
            || s.consumed.windows(2).any(|w| w[0] >= w[1])
            || s.weights.iter().any(|w| {
                w.count == 0
                    || w.count as usize > MAX_UPDATES
                    || w.total < -4000 * i64::from(w.count)
                    || w.total > 2000 * i64::from(w.count)
            })
            || s.weights.iter().map(|w| u64::from(w.count)).sum::<u64>() != s.consumed.len() as u64
        {
            return Err(UtilityError::Invalid);
        }
        if now_ms < s.created_at_ms || now_ms >= s.expires_at_ms {
            return Err(UtilityError::Expired);
        }
        Ok(Self {
            version: s.version,
            seed: s.seed,
            created_at_ms: s.created_at_ms,
            expires_at_ms: s.expires_at_ms,
            weights: s.weights,
            consumed: s.consumed,
        })
    }
}
