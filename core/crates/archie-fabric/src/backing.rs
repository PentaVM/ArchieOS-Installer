//! Where a node's private key actually lives, and why a peer is entitled to care.
//!
//! RFC-007 §3.5 and §7 step 10.
//!
//! # The property a file cannot have
//!
//! Today a node's identity is an Ed25519 key in a `0600` file. It works, and it has one
//! irreducible weakness: **copy the file, become the node, forever.** Root on the box,
//! a stray backup, a disk pulled from a decommissioned machine — any of those yields a
//! key that signs indistinguishably from the original, on hardware the original never
//! touched.
//!
//! A key sealed in a TPM cannot be copied. It signs on request and never leaves. That is
//! not a stronger version of the same guarantee; it is a different guarantee, and the
//! whole value of this module is refusing to let the two be confused.
//!
//! # Trust-on-first-use pins a key, and a key is not a machine
//!
//! The fabric already pins a peer's key the first time it sees it, and refuses a later
//! change. That defends against substitution and not at all against duplication: two
//! machines holding the same copied software key both pass the pin, because from the
//! outside they *are* the same identity.
//!
//! Hardware backing is what makes the pin mean "this machine" rather than "whoever holds
//! this file". So the backing travels with the descriptor, a peer may require it, and a
//! node that cannot prove it says so rather than staying quiet.
//!
//! # Claiming is not proving
//!
//! A descriptor field saying "hardware-backed" is a claim by the very party whose honesty
//! is in question. It is useful — an operator can see which of their own nodes are
//! sealed — and it is not evidence. Evidence is a TPM attestation, which this module
//! models as a separate, absent-by-default thing precisely so the difference is visible
//! in the type rather than assumed by the reader.

use serde::{Deserialize, Serialize};

/// Where the signing key lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyBacking {
    /// A file on disk. Copyable, and therefore an identity that can exist in two places.
    SoftwareHeld,
    /// Sealed in a TPM. The key signs on request and cannot be extracted.
    HardwareBacked,
}

impl KeyBacking {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::SoftwareHeld => "software_held",
            Self::HardwareBacked => "hardware_backed",
        }
    }

    /// Whether this backing makes the identity non-duplicable.
    pub fn is_unclonable(&self) -> bool {
        matches!(self, Self::HardwareBacked)
    }

    /// A sentence for an operator looking at a node list.
    pub fn explain(&self) -> &'static str {
        match self {
            Self::SoftwareHeld => "key is a file on disk; anyone who can read it can be this node",
            Self::HardwareBacked => "key is sealed in a TPM and cannot be extracted",
        }
    }
}

/// What a node says about its own key, and what it can show for it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackingClaim {
    pub backing: KeyBacking,
    /// A TPM quote or equivalent, when one exists.
    ///
    /// `None` is the ordinary case and is not suspicious on its own. What it means is
    /// that the claim above is a claim: useful for an operator surveying their own
    /// fleet, and not evidence to a peer deciding whether to trust a stranger.
    #[serde(default)]
    pub attestation: Option<Attestation>,
}

/// Evidence from the TPM itself.
///
/// Deliberately opaque here. Verifying a quote needs the manufacturer's endorsement
/// chain and a policy about which PCR values are acceptable, and neither belongs in the
/// type that merely carries it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attestation {
    /// The quote, as the TPM produced it.
    pub quote: Vec<u8>,
    /// Which PCRs it covers.
    pub pcrs: Vec<u32>,
    /// Whether this node has verified the endorsement chain behind the quote.
    ///
    /// A quote nobody checked is a blob. Carrying the flag rather than assuming
    /// verification keeps an unverified attestation from being counted as evidence
    /// simply because it is present and well formed.
    pub endorsement_verified: bool,
}

impl BackingClaim {
    /// A software-held identity — the honest default.
    pub fn software() -> Self {
        Self {
            backing: KeyBacking::SoftwareHeld,
            attestation: None,
        }
    }

    /// A hardware claim with no evidence behind it.
    ///
    /// Legitimate: a node that seals its key in a TPM but cannot produce a quote a
    /// stranger would accept is still better off than one that does not. It just cannot
    /// prove it.
    pub fn hardware_unattested() -> Self {
        Self {
            backing: KeyBacking::HardwareBacked,
            attestation: None,
        }
    }

    pub fn hardware_attested(attestation: Attestation) -> Self {
        Self {
            backing: KeyBacking::HardwareBacked,
            attestation: Some(attestation),
        }
    }

    /// Whether this claim is backed by verified evidence rather than assertion.
    pub fn is_proven(&self) -> bool {
        self.backing == KeyBacking::HardwareBacked
            && self
                .attestation
                .as_ref()
                .is_some_and(|a| a.endorsement_verified)
    }

    /// Whether the claim is internally coherent.
    ///
    /// A software-held key with a TPM quote attached is not a stronger software key; it
    /// is a contradiction, and the most likely explanation is that a descriptor was
    /// assembled by copying fields from another node's.
    pub fn is_coherent(&self) -> bool {
        !(self.backing == KeyBacking::SoftwareHeld && self.attestation.is_some())
    }
}

impl Default for BackingClaim {
    fn default() -> Self {
        Self::software()
    }
}

/// What a node demands of its peers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackingRequirement {
    /// Anything. The default, and correct for a home fabric where every node is on the
    /// same desk.
    #[default]
    Any,
    /// The peer must claim hardware backing. Catches an unsealed node joining a fleet
    /// that is supposed to be sealed; does not catch a liar.
    ClaimedHardware,
    /// The peer must prove hardware backing with a verified attestation.
    ProvenHardware,
}

/// Why a peer was refused on identity-backing grounds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackingRefusal {
    /// The peer's key is a file and this fabric requires sealed keys.
    SoftwareHeldKey,
    /// The peer claims hardware backing but produced no verified attestation.
    UnprovenClaim,
    /// The claim contradicts itself.
    Incoherent,
}

impl std::fmt::Display for BackingRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SoftwareHeldKey => f.write_str(
                "this peer's key is a file on disk, so its identity can exist on more \
                 than one machine; this fabric requires keys that cannot be copied",
            ),
            Self::UnprovenClaim => f.write_str(
                "this peer claims a sealed key but produced no verified attestation; the \
                 claim comes from the party whose honesty is in question",
            ),
            Self::Incoherent => f.write_str(
                "this peer claims a software-held key and attached a TPM quote; the most \
                 likely explanation is a descriptor assembled from another node's fields",
            ),
        }
    }
}

impl std::error::Error for BackingRefusal {}

/// Whether a peer's claim satisfies what this node requires.
pub fn refuse(requirement: BackingRequirement, claim: &BackingClaim) -> Option<BackingRefusal> {
    // Coherence first, whatever the requirement. A contradictory claim is a signal about
    // the descriptor regardless of how permissive this node is willing to be.
    if !claim.is_coherent() {
        return Some(BackingRefusal::Incoherent);
    }

    match requirement {
        BackingRequirement::Any => None,
        BackingRequirement::ClaimedHardware => {
            if claim.backing.is_unclonable() {
                None
            } else {
                Some(BackingRefusal::SoftwareHeldKey)
            }
        }
        BackingRequirement::ProvenHardware => {
            if !claim.backing.is_unclonable() {
                Some(BackingRefusal::SoftwareHeldKey)
            } else if claim.is_proven() {
                None
            } else {
                Some(BackingRefusal::UnprovenClaim)
            }
        }
    }
}

/// What changed about a peer's backing between two sightings.
///
/// Trust-on-first-use pins a key. This pins the *story about* the key, because the
/// interesting attack is not a changed key — that is already refused — but the same key
/// arriving with a weaker claim behind it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackingChange {
    Unchanged,
    /// The peer gained hardware backing. Ordinary: a node was re-provisioned onto sealed
    /// hardware, and the key was re-created there.
    Strengthened,
    /// The same key now claims *less* backing than it did.
    ///
    /// This is the one worth alarming on. A key that was sealed in a TPM cannot be
    /// extracted, so it cannot legitimately reappear as a file — and a peer presenting
    /// the pinned key with a downgraded claim is either a different machine holding a
    /// copy, or the same machine telling a different story about itself. Both deserve a
    /// human.
    Weakened,
}

impl BackingChange {
    /// Compare a newly seen claim against the pinned one.
    pub fn between(pinned: &BackingClaim, seen: &BackingClaim) -> Self {
        match (pinned.backing, seen.backing) {
            (a, b) if a == b => Self::Unchanged,
            (KeyBacking::SoftwareHeld, KeyBacking::HardwareBacked) => Self::Strengthened,
            (KeyBacking::HardwareBacked, KeyBacking::SoftwareHeld) => Self::Weakened,
            _ => Self::Unchanged,
        }
    }

    pub fn needs_attention(&self) -> bool {
        matches!(self, Self::Weakened)
    }

    pub fn explain(&self) -> &'static str {
        match self {
            Self::Unchanged => "the peer's key backing is as previously seen",
            Self::Strengthened => {
                "the peer now seals its key in hardware; likely a re-provisioning"
            }
            Self::Weakened => {
                "the peer presents its pinned key with weaker backing than before; a key \
                 sealed in a TPM cannot reappear as a file, so this is either a second \
                 machine holding a copy or the same machine telling a different story"
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn verified() -> Attestation {
        Attestation {
            quote: vec![1, 2, 3],
            pcrs: vec![0, 7],
            endorsement_verified: true,
        }
    }

    fn unverified() -> Attestation {
        Attestation {
            endorsement_verified: false,
            ..verified()
        }
    }

    #[test]
    fn software_and_hardware_are_never_the_same_guarantee() {
        assert!(!KeyBacking::SoftwareHeld.is_unclonable());
        assert!(KeyBacking::HardwareBacked.is_unclonable());
        assert!(KeyBacking::SoftwareHeld
            .explain()
            .contains("anyone who can read it can be this node"));
    }

    #[test]
    fn claiming_is_not_proving() {
        // A descriptor field is an assertion by the party whose honesty is in question.
        assert!(!BackingClaim::hardware_unattested().is_proven());
        assert!(BackingClaim::hardware_attested(verified()).is_proven());
    }

    #[test]
    fn an_unverified_quote_is_a_blob_not_evidence() {
        // Carrying the flag rather than assuming verification stops a well-formed but
        // unchecked attestation from counting simply because it is present.
        let claim = BackingClaim::hardware_attested(unverified());
        assert!(!claim.is_proven());
        assert_eq!(
            refuse(BackingRequirement::ProvenHardware, &claim),
            Some(BackingRefusal::UnprovenClaim)
        );
    }

    #[test]
    fn a_software_key_with_a_tpm_quote_is_a_contradiction() {
        // The most likely explanation is a descriptor assembled by copying fields from
        // another node's, which is worth saying out loud.
        let incoherent = BackingClaim {
            backing: KeyBacking::SoftwareHeld,
            attestation: Some(verified()),
        };
        assert!(!incoherent.is_coherent());

        // Refused even by the most permissive requirement: it is a signal about the
        // descriptor, not about how strict this node feels.
        assert_eq!(
            refuse(BackingRequirement::Any, &incoherent),
            Some(BackingRefusal::Incoherent)
        );
    }

    #[test]
    fn the_default_fabric_accepts_a_software_key() {
        // Correct for a home fabric where every node is on the same desk. Requiring
        // sealed keys by default would make the honest state unusable.
        assert_eq!(BackingRequirement::default(), BackingRequirement::Any);
        assert!(refuse(BackingRequirement::Any, &BackingClaim::software()).is_none());
        assert_eq!(BackingClaim::default(), BackingClaim::software());
    }

    #[test]
    fn a_fabric_requiring_sealed_keys_refuses_a_file() {
        for requirement in [
            BackingRequirement::ClaimedHardware,
            BackingRequirement::ProvenHardware,
        ] {
            let refusal = refuse(requirement, &BackingClaim::software()).unwrap();
            assert_eq!(refusal, BackingRefusal::SoftwareHeldKey);
            assert!(refusal
                .to_string()
                .contains("can exist on more than one machine"));
        }
    }

    #[test]
    fn claimed_hardware_catches_an_unsealed_node_but_not_a_liar() {
        // Worth being explicit about what this level does and does not buy.
        assert!(refuse(
            BackingRequirement::ClaimedHardware,
            &BackingClaim::hardware_unattested()
        )
        .is_none());
        assert_eq!(
            refuse(
                BackingRequirement::ProvenHardware,
                &BackingClaim::hardware_unattested()
            ),
            Some(BackingRefusal::UnprovenClaim)
        );
    }

    #[test]
    fn proven_hardware_accepts_only_verified_evidence() {
        assert!(refuse(
            BackingRequirement::ProvenHardware,
            &BackingClaim::hardware_attested(verified())
        )
        .is_none());
    }

    #[test]
    fn a_pinned_key_arriving_with_weaker_backing_is_the_alarm() {
        // Trust-on-first-use already refuses a changed key. The interesting attack is
        // the *same* key with a downgraded story: a key sealed in a TPM cannot be
        // extracted, so it cannot legitimately reappear as a file.
        let change = BackingChange::between(
            &BackingClaim::hardware_attested(verified()),
            &BackingClaim::software(),
        );
        assert_eq!(change, BackingChange::Weakened);
        assert!(change.needs_attention());
        assert!(change.explain().contains("second machine holding a copy"));
    }

    #[test]
    fn gaining_hardware_backing_is_ordinary() {
        // A node re-provisioned onto sealed hardware re-creates its key there.
        let change = BackingChange::between(
            &BackingClaim::software(),
            &BackingClaim::hardware_attested(verified()),
        );
        assert_eq!(change, BackingChange::Strengthened);
        assert!(!change.needs_attention());
    }

    #[test]
    fn an_unchanged_backing_is_quiet() {
        for claim in [
            BackingClaim::software(),
            BackingClaim::hardware_unattested(),
            BackingClaim::hardware_attested(verified()),
        ] {
            let change = BackingChange::between(&claim, &claim);
            assert_eq!(change, BackingChange::Unchanged);
            assert!(!change.needs_attention());
        }
    }

    #[test]
    fn losing_only_the_attestation_is_not_a_downgrade_of_backing() {
        // A TPM that is still sealing the key but whose quote could not be produced this
        // time — a busy resource manager, a policy change — has not become a file. It is
        // a weaker *claim*, and the requirement level is what decides whether that is
        // acceptable, not the change detector.
        let change = BackingChange::between(
            &BackingClaim::hardware_attested(verified()),
            &BackingClaim::hardware_unattested(),
        );
        assert_eq!(change, BackingChange::Unchanged);
        assert_eq!(
            refuse(
                BackingRequirement::ProvenHardware,
                &BackingClaim::hardware_unattested()
            ),
            Some(BackingRefusal::UnprovenClaim),
            "the requirement is what refuses it, and it says exactly why"
        );
    }

    #[test]
    fn a_claim_survives_the_wire_and_names_itself() {
        for claim in [
            BackingClaim::software(),
            BackingClaim::hardware_unattested(),
            BackingClaim::hardware_attested(verified()),
        ] {
            let json = serde_json::to_string(&claim).unwrap();
            assert!(json.contains(claim.backing.as_str()));
            assert_eq!(serde_json::from_str::<BackingClaim>(&json).unwrap(), claim);
        }
    }

    #[test]
    fn a_descriptor_without_the_field_decodes_as_software_held() {
        // Older peers predate this field, and the honest reading of their silence is the
        // weaker of the two — never the stronger.
        let claim: BackingClaim = serde_json::from_str(r#"{"backing":"software_held"}"#).unwrap();
        assert_eq!(claim, BackingClaim::software());
        assert!(claim.attestation.is_none());
    }
}
