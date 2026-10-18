//! The IRIS pillar's producer: bounded episodic action receipts from visual
//! interaction sessions.
//!
//! Raw frames and pixel crops never enter the vault or [`PillarContext`]. What
//! IRIS records here is the bounded semantic summary of one episode: the
//! session identifier, the instruction, the skill name, how many steps ran,
//! whether the postcondition verified, and the source timestamp.
//!
//! Each episode is sealed at [`PrivacyGrade::Restricted`] under
//! `<domain>/iris/episodes/<content_id>` as [`EntryClass::PrivateMemory`], and
//! included in [`PillarContext`] as [`Pillar::Iris`] (plus
//! [`Pillar::VerifiedOutcome`] when `verified` is `true`).

use crate::pillars::{Pillar, PillarContext, PillarError};
use archie_types::PrivacyGrade;
use archie_vault::{ContextDomain, EntryClass, Vault, VaultError, VaultRef};
use serde::{Deserialize, Serialize};

/// Format marker of a sealed IRIS episode summary.
pub const EPISODE_FORMAT: &str = "archie-iris-episode-v1";
const ACTOR: &str = "iris-bridge";
const MAX_FIELD_CHARS: usize = 256;
const MAX_SUMMARY_CHARS: usize = 1024;

/// One IRIS episode summary to seal into the vault.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IrisEpisodeInput {
    pub session_id: String,
    pub instruction: String,
    pub skill: String,
    pub step_count: u32,
    pub verified: bool,
    pub at_ms: u64,
    pub summary: String,
}

/// Sealed JSON representation stored in the vault.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IrisEpisodeContent {
    pub format: String,
    pub session_id: String,
    pub instruction: String,
    pub skill: String,
    pub step_count: u32,
    pub verified: bool,
    pub at_ms: u64,
    pub summary: String,
}

impl IrisEpisodeContent {
    /// Focus weight (0..=100): verified outcomes weigh more than unverified episodes.
    pub fn weight(&self) -> u8 {
        if self.verified {
            75
        } else {
            45
        }
    }
}

/// A recorded IRIS episode in the vault.
#[derive(Debug, Clone)]
pub struct IrisEpisodeRecord {
    pub reference: VaultRef,
    pub at_ms: i64,
    pub verified: bool,
    pub weight: u8,
    pub context: PillarContext,
}

#[derive(Debug, thiserror::Error)]
pub enum IrisBridgeError {
    #[error("invalid IRIS episode field: {0}")]
    InvalidField(&'static str),
    #[error("the domain cannot hold this episode's reference")]
    Domain,
    #[error("another record already sits at {reference:?}")]
    ReferenceCollision { reference: VaultRef },
    #[error(transparent)]
    Pillar(#[from] PillarError),
    #[error(transparent)]
    Vault(#[from] VaultError),
}

fn bounded(text: &str, max_chars: usize) -> bool {
    let trimmed = text.trim();
    !trimmed.is_empty()
        && trimmed.chars().count() <= max_chars
        && !trimmed.chars().any(char::is_control)
}

fn content_id(bytes: &[u8]) -> String {
    const fn fnv1a(bytes: &[u8], basis: u64) -> u64 {
        let mut hash = basis;
        let mut index = 0;
        while index < bytes.len() {
            hash ^= bytes[index] as u64;
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
            index += 1;
        }
        hash
    }
    let first = fnv1a(bytes, 0xcbf2_9ce4_8422_2325);
    let second = fnv1a(bytes, 0x9e37_79b9_7f4a_7c15);
    format!("{first:016x}{second:016x}")
}

/// Validate, seal (restricted, at the episode's own timestamp) and reference an
/// IRIS episode under `<domain>/iris/episodes/<content_id>`.
pub fn record_episode(
    vault: &mut Vault,
    domain: &ContextDomain,
    task_id: &str,
    episode: &IrisEpisodeInput,
) -> Result<IrisEpisodeRecord, IrisBridgeError> {
    if !bounded(&episode.session_id, MAX_FIELD_CHARS) {
        return Err(IrisBridgeError::InvalidField("session_id"));
    }
    if !bounded(&episode.instruction, MAX_FIELD_CHARS) {
        return Err(IrisBridgeError::InvalidField("instruction"));
    }
    if !bounded(&episode.skill, MAX_FIELD_CHARS) {
        return Err(IrisBridgeError::InvalidField("skill"));
    }
    if episode.step_count == 0 || episode.step_count > 10_000 {
        return Err(IrisBridgeError::InvalidField("step_count"));
    }
    if !bounded(&episode.summary, MAX_SUMMARY_CHARS) {
        return Err(IrisBridgeError::InvalidField("summary"));
    }
    let at_ms = i64::try_from(episode.at_ms).map_err(|_| IrisBridgeError::InvalidField("at_ms"))?;
    let content = IrisEpisodeContent {
        format: EPISODE_FORMAT.into(),
        session_id: episode.session_id.trim().into(),
        instruction: episode.instruction.trim().into(),
        skill: episode.skill.trim().into(),
        step_count: episode.step_count,
        verified: episode.verified,
        at_ms: episode.at_ms,
        summary: episode.summary.trim().into(),
    };
    let weight = content.weight();
    let json =
        serde_json::to_string(&content).map_err(|_| IrisBridgeError::InvalidField("encoding"))?;
    let id = content_id(json.as_bytes());
    let reference = domain
        .reference(&format!("iris/episodes/{id}"))
        .map_err(|_| IrisBridgeError::Domain)?;
    if vault.contains(&reference) {
        let existing = vault
            .disclose(ACTOR, &reference, PrivacyGrade::Restricted, at_ms)
            .ok()
            .and_then(|d| d.text().map(str::to_owned));
        if existing.as_deref() != Some(json.as_str()) {
            return Err(IrisBridgeError::ReferenceCollision { reference });
        }
    } else {
        vault.store(
            ACTOR,
            reference.clone(),
            EntryClass::PrivateMemory,
            PrivacyGrade::Restricted,
            &json,
            at_ms,
            None,
        )?;
    }
    let mut context = PillarContext::new(domain.clone(), task_id)?;
    context.include(Pillar::Iris, reference.clone())?;
    if episode.verified {
        context.include(Pillar::VerifiedOutcome, reference.clone())?;
    }
    Ok(IrisEpisodeRecord {
        reference,
        at_ms,
        verified: episode.verified,
        weight,
        context,
    })
}
