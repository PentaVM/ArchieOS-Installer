//! What an installer decides, separated from how it asks.
//!
//! The installer is a wizard: it prompts, it writes files, it needs a disk and a
//! terminal. None of that can be tested, and none of it is where the mistakes are. The
//! mistakes are in the *decisions* — whether this install starts a new fabric or joins
//! an existing one, whether it holds an owner key, what it ends up being able to talk
//! to. Those are pure, and they are here.
//!
//! # Why the guest case is the default shape rather than a special one
//!
//! The requirement is that somebody who installs ArchieOS does not end up on someone
//! else's fabric. The temptation is to add a "guest mode" switch, but a switch is a
//! thing that can be left in the wrong position, and the person it would harm is exactly
//! the person least likely to check it.
//!
//! So the plan has no guest flag. Every install generates its own owner key unless the
//! person installing it presents someone else's — which they can only do by copying it
//! off a machine they control. Isolation is what happens when nobody does anything
//! special, and joining is the step that takes deliberate action.

use crate::identity::NodeIdentity;
use crate::membership::{fingerprint, Enrolment, Membership, MembershipError, OwnerIdentity};
use std::path::PathBuf;

/// What the person running the installer told it about the fabric.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FabricChoice {
    /// Start a fresh fabric owned by this install.
    ///
    /// The default, and what a guest gets without asking for it.
    StartNew,

    /// Join a fabric whose owner key the installer was given.
    ///
    /// The hex private owner key, which only somebody with access to the owning machine
    /// can supply. Deliberately the *private* key rather than a fingerprint: admitting a
    /// node requires issuing a certificate, and only the owner key can do that. A design
    /// where a fingerprint sufficed would be a design where knowing a public value let
    /// you join.
    JoinWithOwnerKey { owner_private_key_hex: String },
}

impl Default for FabricChoice {
    fn default() -> Self {
        Self::StartNew
    }
}

/// Where the installed system keeps what the plan produces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnrolmentPaths {
    pub node_key: PathBuf,
    pub membership: PathBuf,
    /// Only written when this install owns its fabric.
    pub owner_key: PathBuf,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PlanError {
    #[error("the owner key provided is not usable: {0}")]
    UnusableOwnerKey(String),

    #[error(transparent)]
    Membership(#[from] MembershipError),
}

/// What the installer is about to do, decided before anything is written.
///
/// Returned so it can be *shown to the person* before they commit. An installer that
/// silently decides which fabric a machine joins is an installer whose most important
/// decision is invisible.
#[derive(Debug)]
pub struct EnrolmentPlan {
    /// The enrolment the installed node will hold.
    pub enrolment: Enrolment,
    /// The certificate to write out.
    pub membership: Membership,
    /// Present when this install owns the fabric, absent when it is joining one.
    pub owner: Option<OwnerIdentity>,
    /// Whether the fabric was created by this install.
    pub started_new_fabric: bool,
}

impl EnrolmentPlan {
    /// The fabric this install will be part of, in the form shown to a person.
    pub fn fabric_fingerprint(&self) -> String {
        self.membership.fabric_fingerprint()
    }

    /// One line an installer can print before it writes anything.
    ///
    /// Says which of the two things is happening in words rather than leaving the person
    /// to infer it from an absence.
    pub fn summary(&self) -> String {
        if self.started_new_fabric {
            format!(
                "This machine will start its own fabric ({}). It will not talk to any \
                 other ArchieOS install, including ones on this network, until you add \
                 them to it.",
                self.fabric_fingerprint()
            )
        } else {
            format!(
                "This machine will join the existing fabric {}. It will talk to the other \
                 nodes of that fabric and to nothing else.",
                self.fabric_fingerprint()
            )
        }
    }
}

/// Decide the enrolment for an install.
///
/// Pure: it generates keys and certificates and returns them, and writes nothing. The
/// wizard does the writing, having shown the person [`EnrolmentPlan::summary`] first.
pub fn plan_enrolment(
    node: &NodeIdentity,
    choice: &FabricChoice,
    issued_at_ms: i64,
) -> Result<EnrolmentPlan, PlanError> {
    let (owner, started_new_fabric) = match choice {
        FabricChoice::StartNew => (OwnerIdentity::generate(), true),
        FabricChoice::JoinWithOwnerKey {
            owner_private_key_hex,
        } => {
            let decoded = hex::decode(owner_private_key_hex.trim())
                .map_err(|e| PlanError::UnusableOwnerKey(format!("not hex: {e}")))?;
            let bytes: [u8; 32] = decoded.try_into().map_err(|_| {
                PlanError::UnusableOwnerKey("an owner key is exactly 32 bytes".into())
            })?;
            (OwnerIdentity::from_private_bytes(&bytes), false)
        }
    };

    let membership = owner.admit(&node.public_key_hex(), issued_at_ms);
    let enrolment = Enrolment::new(&node.public_key_hex(), membership.clone())?;

    Ok(EnrolmentPlan {
        enrolment,
        membership,
        // The owner key is only this install's to keep when this install made it.
        // Somebody joining an existing fabric was lent the key to issue one certificate;
        // writing it to their disk would quietly hand them the ability to admit anyone
        // else to a fabric that is not theirs.
        owner: started_new_fabric.then_some(owner),
        started_new_fabric,
    })
}

/// The fingerprint of a fabric an owner key belongs to, for showing someone which one
/// they are about to join before they commit to it.
pub fn fingerprint_of_owner_key(owner_private_key_hex: &str) -> Result<String, PlanError> {
    let decoded = hex::decode(owner_private_key_hex.trim())
        .map_err(|e| PlanError::UnusableOwnerKey(format!("not hex: {e}")))?;
    let bytes: [u8; 32] = decoded
        .try_into()
        .map_err(|_| PlanError::UnusableOwnerKey("an owner key is exactly 32 bytes".into()))?;
    Ok(fingerprint(
        &OwnerIdentity::from_private_bytes(&bytes).public_key_hex(),
    ))
}
