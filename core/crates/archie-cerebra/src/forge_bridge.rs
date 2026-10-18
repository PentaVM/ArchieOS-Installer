//! The Forge pillar's producer: bounded build, test, patch and review receipts.
//!
//! Each Forge run is sealed at [`PrivacyGrade::Restricted`] under
//! `<domain>/forge/artifacts/<content_id>` as [`EntryClass::PrivateFact`], and
//! included in [`PillarContext`] as [`Pillar::Forge`] (plus
//! [`Pillar::VerifiedOutcome`] when a build or test run succeeded).

use crate::pillars::{Pillar, PillarContext, PillarError};
use archie_types::PrivacyGrade;
use archie_vault::{ContextDomain, EntryClass, Vault, VaultError, VaultRef};
use serde::{Deserialize, Serialize};

/// Format marker of a sealed Forge artifact receipt.
pub const ARTIFACT_FORMAT: &str = "archie-forge-artifact-v1";
const ACTOR: &str = "forge-bridge";
const MAX_FIELD_CHARS: usize = 256;
const MAX_SUMMARY_CHARS: usize = 1024;

/// What kind of Forge job produced the receipt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ForgeArtifactKind {
    Build,
    Test,
    Patch,
    Review,
}

impl ForgeArtifactKind {
    pub fn weight(self, succeeded: bool) -> u8 {
        match (self, succeeded) {
            (Self::Build | Self::Test, false) => 85,
            (Self::Build | Self::Test | Self::Patch, true) => 65,
            (Self::Patch, false) => 75,
            (Self::Review, _) => 50,
        }
    }
}

/// One Forge artifact receipt to seal into the vault.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForgeArtifactInput {
    pub job_id: String,
    pub kind: ForgeArtifactKind,
    pub succeeded: bool,
    pub at_ms: u64,
    pub summary: String,
}

/// Sealed JSON representation stored in the vault.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ForgeArtifactContent {
    pub format: String,
    pub job_id: String,
    pub kind: ForgeArtifactKind,
    pub succeeded: bool,
    pub at_ms: u64,
    pub summary: String,
}

impl ForgeArtifactContent {
    pub fn weight(&self) -> u8 {
        self.kind.weight(self.succeeded)
    }
}

/// A recorded Forge artifact in the vault.
#[derive(Debug, Clone)]
pub struct ForgeArtifactRecord {
    pub reference: VaultRef,
    pub at_ms: i64,
    pub kind: ForgeArtifactKind,
    pub succeeded: bool,
    pub weight: u8,
    pub context: PillarContext,
}

#[derive(Debug, thiserror::Error)]
pub enum ForgeBridgeError {
    #[error("invalid Forge artifact field: {0}")]
    InvalidField(&'static str),
    #[error("the domain cannot hold this artifact's reference")]
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

/// Validate, seal (restricted, at the job's own timestamp) and reference a
/// Forge artifact under `<domain>/forge/artifacts/<content_id>`.
pub fn record_artifact(
    vault: &mut Vault,
    domain: &ContextDomain,
    task_id: &str,
    artifact: &ForgeArtifactInput,
) -> Result<ForgeArtifactRecord, ForgeBridgeError> {
    if !bounded(&artifact.job_id, MAX_FIELD_CHARS) {
        return Err(ForgeBridgeError::InvalidField("job_id"));
    }
    if !bounded(&artifact.summary, MAX_SUMMARY_CHARS) {
        return Err(ForgeBridgeError::InvalidField("summary"));
    }
    let at_ms =
        i64::try_from(artifact.at_ms).map_err(|_| ForgeBridgeError::InvalidField("at_ms"))?;
    let content = ForgeArtifactContent {
        format: ARTIFACT_FORMAT.into(),
        job_id: artifact.job_id.trim().into(),
        kind: artifact.kind,
        succeeded: artifact.succeeded,
        at_ms: artifact.at_ms,
        summary: artifact.summary.trim().into(),
    };
    let weight = content.weight();
    let json =
        serde_json::to_string(&content).map_err(|_| ForgeBridgeError::InvalidField("encoding"))?;
    let id = content_id(json.as_bytes());
    let reference = domain
        .reference(&format!("forge/artifacts/{id}"))
        .map_err(|_| ForgeBridgeError::Domain)?;
    if vault.contains(&reference) {
        let existing = vault
            .disclose(ACTOR, &reference, PrivacyGrade::Restricted, at_ms)
            .ok()
            .and_then(|d| d.text().map(str::to_owned));
        if existing.as_deref() != Some(json.as_str()) {
            return Err(ForgeBridgeError::ReferenceCollision { reference });
        }
    } else {
        vault.store(
            ACTOR,
            reference.clone(),
            EntryClass::PrivateFact,
            PrivacyGrade::Restricted,
            &json,
            at_ms,
            None,
        )?;
    }
    let mut context = PillarContext::new(domain.clone(), task_id)?;
    context.include(Pillar::Forge, reference.clone())?;
    if artifact.succeeded
        && matches!(
            artifact.kind,
            ForgeArtifactKind::Build | ForgeArtifactKind::Test
        )
    {
        context.include(Pillar::VerifiedOutcome, reference.clone())?;
    }
    Ok(ForgeArtifactRecord {
        reference,
        at_ms,
        kind: artifact.kind,
        succeeded: artifact.succeeded,
        weight,
        context,
    })
}
