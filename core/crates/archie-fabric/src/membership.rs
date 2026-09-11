//! Fabric membership: which nodes belong together, and who says so.
//!
//! # The gap this closes
//!
//! [`crate::identity`] proves *who sent a message*. A signed announcement cannot be
//! forged, and the node id is derived from the key rather than chosen, so there is no
//! impersonating a peer. That is authentication, and it was the whole of the admission
//! check until now.
//!
//! Authentication is not authorisation. Every rejection reason the fabric had —
//! `Unauthenticated`, `DescriptorSenderMismatch`, `Malformed`, `ProtocolMismatch` — asks
//! whether a message is *well-formed and genuine*. None of them asks whether the sender
//! was ever *invited*. Any machine that generates a keypair and announces on the
//! multicast group was admitted as a peer, and the shipped configuration puts every
//! ArchieOS install on the same group. Two people installing the ISO on one network
//! formed one fabric, and neither chose to.
//!
//! # The shape of the fix
//!
//! A fabric is rooted in an **owner key**. The owner's public key *is* the fabric's
//! identity; a different owner is a different fabric, and that is a cryptographic fact
//! rather than a setting somebody could flip. The owner's private half signs a
//! [`Membership`] admitting one node's public key, and every message carries the
//! certificate that admitted its sender.
//!
//! A node accepts a peer only when the peer presents a membership signed by *the same
//! owner key this node is enrolled under*. So:
//!
//! * a guest install generates its own owner key at install time, which makes its
//!   fabric a different fabric — it cannot join anyone else's and nobody can join it,
//!   without either side doing anything;
//! * a node holding no membership at all talks to nobody, which is the safe direction
//!   for the state a machine is in before someone has decided what it belongs to.
//!
//! The private owner key is not needed to *run* a node, only to *admit* one. It can live
//! offline, or on one machine, and its absence costs nothing until the fabric grows.

use crate::identity::{public_key_from_hex, IdentityError};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use std::path::Path;
use thiserror::Error;

/// Domain separation. A signature made for one purpose must never verify for another,
/// and the version means a future format cannot be replayed as this one.
const MEMBERSHIP_SIGNING_TAG: &str = "archie-fabric-membership:v1";

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum MembershipError {
    #[error("this node has not been enrolled in any fabric")]
    Unenrolled,

    #[error("membership was issued by a different owner: {presented}")]
    ForeignFabric { presented: String },

    #[error("membership admits {admits}, but the message was signed by {sender}")]
    NotForSender { admits: String, sender: String },

    #[error("membership signature is not the owner's")]
    BadSignature,

    #[error("malformed membership: {0}")]
    Malformed(String),
}

impl From<IdentityError> for MembershipError {
    fn from(error: IdentityError) -> Self {
        Self::Malformed(error.to_string())
    }
}

/// A certificate: the owner of a fabric admitting one node into it.
///
/// Self-authenticating, so it can travel on every message and be checked without asking
/// anything else. It deliberately carries the owner's public key rather than only a
/// fingerprint: a receiver needs to compare it against the owner it is enrolled under,
/// and a comparison that required a lookup would be a lookup on the packet path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Membership {
    /// Hex Ed25519 public key of the fabric's owner. This is the fabric's identity.
    pub owner_public_key: String,
    /// Hex Ed25519 public key of the node being admitted.
    pub node_public_key: String,
    /// When the owner issued it.
    pub issued_at_ms: i64,
    /// Hex Ed25519 signature by the owner over the canonical bytes below.
    pub signature: String,
}

impl Membership {
    /// The exact bytes an owner signs. Anything not in here is not attested.
    fn signing_bytes(owner_public_key: &str, node_public_key: &str, issued_at_ms: i64) -> Vec<u8> {
        format!("{MEMBERSHIP_SIGNING_TAG}\n{owner_public_key}\n{node_public_key}\n{issued_at_ms}")
            .into_bytes()
    }

    /// Check this certificate against the fabric we belong to and the node that sent it.
    ///
    /// The order is deliberate. Comparing owner keys is a string comparison and
    /// comparing node keys is another; verifying a signature is elliptic-curve work
    /// roughly four orders of magnitude more expensive. A neighbour's fabric announcing
    /// on the same multicast group is the ordinary case, not an attack, and it must cost
    /// us a comparison rather than a verification — otherwise being next door to another
    /// ArchieOS household is a way to spend our CPU.
    pub fn verify_for(
        &self,
        fabric_owner_public_key: &str,
        sender_public_key: &str,
    ) -> Result<(), MembershipError> {
        if self.owner_public_key != fabric_owner_public_key {
            return Err(MembershipError::ForeignFabric {
                presented: self.owner_public_key.clone(),
            });
        }
        self.verify_admits(sender_public_key)
    }

    /// Check the signature and that the certificate admits this exact key, without
    /// asking which fabric it belongs to.
    ///
    /// Split out because a node adopting its own membership is answering a different
    /// question from a node judging a stranger's: it is not comparing against a fabric
    /// it already belongs to, it is reading which fabric it is being told to join.
    pub fn verify_admits(&self, node_public_key: &str) -> Result<(), MembershipError> {
        if self.node_public_key != node_public_key {
            return Err(MembershipError::NotForSender {
                admits: self.node_public_key.clone(),
                sender: node_public_key.to_string(),
            });
        }

        let owner_key = public_key_from_hex(&self.owner_public_key)?;
        let verifying_key = VerifyingKey::from_bytes(&owner_key)
            .map_err(|e| MembershipError::Malformed(e.to_string()))?;

        let raw = hex::decode(&self.signature)
            .map_err(|e| MembershipError::Malformed(format!("signature is not hex: {e}")))?;
        let bytes: [u8; 64] = raw
            .try_into()
            .map_err(|_| MembershipError::Malformed("signature must be 64 bytes".into()))?;

        let signing_bytes = Self::signing_bytes(
            &self.owner_public_key,
            &self.node_public_key,
            self.issued_at_ms,
        );
        verifying_key
            .verify(&signing_bytes, &Signature::from_bytes(&bytes))
            .map_err(|_| MembershipError::BadSignature)
    }

    /// A short, human-readable form of the fabric this membership belongs to.
    ///
    /// For showing an operator which fabric a node thinks it is in, so two installs that
    /// were meant to be separate can be seen to be separate.
    pub fn fabric_fingerprint(&self) -> String {
        fingerprint(&self.owner_public_key)
    }
}

/// Sixteen hex characters of the owner key. Enough to tell two fabrics apart by eye,
/// short enough to print next to a hostname.
pub fn fingerprint(owner_public_key: &str) -> String {
    owner_public_key.chars().take(16).collect()
}

/// The key that defines a fabric and admits nodes to it.
///
/// Holding the private half is what it means to own a fabric: it is the only thing that
/// can issue a [`Membership`]. It is not needed to run a node, so it can live offline —
/// on a stick in a drawer — and a fabric whose owner key is nowhere reachable keeps
/// working, it simply cannot admit anything new.
pub struct OwnerIdentity {
    signing_key: SigningKey,
}

impl std::fmt::Debug for OwnerIdentity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OwnerIdentity")
            .field("fabric", &self.fingerprint())
            .field("signing_key", &"<redacted>")
            .finish()
    }
}

impl OwnerIdentity {
    /// A brand-new fabric, from the OS entropy source.
    ///
    /// This is what the installer calls for a guest: a new owner key is a new fabric,
    /// which is the whole of the isolation. Nobody has to remember to turn anything off.
    pub fn generate() -> Self {
        Self {
            signing_key: SigningKey::generate(&mut rand::rngs::OsRng),
        }
    }

    pub fn from_private_bytes(bytes: &[u8; 32]) -> Self {
        Self {
            signing_key: SigningKey::from_bytes(bytes),
        }
    }

    pub fn public_key_hex(&self) -> String {
        hex::encode(self.signing_key.verifying_key().to_bytes())
    }

    pub fn fingerprint(&self) -> String {
        fingerprint(&self.public_key_hex())
    }

    /// Admit a node to this fabric.
    pub fn admit(&self, node_public_key: &str, issued_at_ms: i64) -> Membership {
        let owner_public_key = self.public_key_hex();
        let signing_bytes =
            Membership::signing_bytes(&owner_public_key, node_public_key, issued_at_ms);
        Membership {
            owner_public_key,
            node_public_key: node_public_key.to_string(),
            issued_at_ms,
            signature: hex::encode(self.signing_key.sign(&signing_bytes).to_bytes()),
        }
    }

    /// Load the owner key, generating and persisting one if absent.
    ///
    /// Deliberately the same shape as [`crate::identity::NodeIdentity::load_or_create`],
    /// including refusing a world-readable file: an owner key that anyone on the machine
    /// can read is an owner key anyone on the machine can use to admit themselves.
    pub fn load_or_create(key_path: impl AsRef<Path>) -> Result<Self, IdentityError> {
        let key_path = key_path.as_ref();
        if key_path.exists() {
            return Self::load(key_path);
        }
        if let Some(parent) = key_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let owner = Self::generate();
        crate::identity::write_private_key(key_path, &owner.signing_key.to_bytes())?;
        let _ = std::fs::write(key_path.with_extension("pub"), owner.public_key_hex());
        Ok(owner)
    }

    /// Write this key out, with the permissions an owner key must have.
    ///
    /// Exists so the installer can persist the *particular* key a plan was built around.
    /// Without it the only way to get an owner key onto a disk is `load_or_create`, which
    /// on an absent path invents a different one — so an installer would generate a
    /// fabric, plan an install around it, and then write a second unrelated fabric to the
    /// machine. The alternative, an accessor for the private half, is the thing this type
    /// exists to refuse: a type that will hand you a private key will hand it to a log
    /// line.
    ///
    /// Refuses to overwrite. Replacing an owner key silently is destroying a fabric.
    pub fn save(&self, key_path: impl AsRef<Path>) -> Result<(), IdentityError> {
        let key_path = key_path.as_ref();
        if key_path.exists() {
            return Err(IdentityError::MalformedKey(format!(
                "{} already holds an owner key; refusing to replace it",
                key_path.display()
            )));
        }
        if let Some(parent) = key_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        crate::identity::write_private_key(key_path, &self.signing_key.to_bytes())?;
        let _ = std::fs::write(key_path.with_extension("pub"), self.public_key_hex());
        Ok(())
    }

    pub fn load(key_path: impl AsRef<Path>) -> Result<Self, IdentityError> {
        let key_path = key_path.as_ref();
        crate::identity::check_permissions(key_path)?;
        let raw = std::fs::read_to_string(key_path)?;
        let decoded = hex::decode(raw.trim())
            .map_err(|e| IdentityError::MalformedKey(format!("owner key is not hex: {e}")))?;
        let bytes: [u8; 32] = decoded.try_into().map_err(|_| {
            IdentityError::MalformedKey("owner key must be exactly 32 bytes".into())
        })?;
        Ok(Self::from_private_bytes(&bytes))
    }
}

/// What a node knows about the fabric it belongs to.
///
/// `None` is a real and safe state, not a missing configuration: a machine that has been
/// installed but not yet told what it is part of should talk to nobody, and that is what
/// an absent enrolment produces.
#[derive(Debug, Clone, Default)]
pub struct Enrolment {
    inner: Option<EnrolledIn>,
}

#[derive(Debug, Clone)]
struct EnrolledIn {
    owner_public_key: String,
    membership: Membership,
}

impl Enrolment {
    /// A node that belongs to nothing yet.
    pub fn none() -> Self {
        Self { inner: None }
    }

    /// A node holding a membership. The certificate must admit this node's own key, or
    /// the enrolment is refused here rather than failing confusingly on every packet.
    pub fn new(node_public_key: &str, membership: Membership) -> Result<Self, MembershipError> {
        membership.verify_admits(node_public_key)?;
        Ok(Self {
            inner: Some(EnrolledIn {
                owner_public_key: membership.owner_public_key.clone(),
                membership,
            }),
        })
    }

    pub fn is_enrolled(&self) -> bool {
        self.inner.is_some()
    }

    /// The membership to attach to outgoing messages.
    pub fn membership(&self) -> Option<&Membership> {
        self.inner.as_ref().map(|e| &e.membership)
    }

    pub fn fabric_fingerprint(&self) -> Option<String> {
        self.inner
            .as_ref()
            .map(|e| fingerprint(&e.owner_public_key))
    }

    /// Read an enrolment from disk.
    ///
    /// An absent file is not an error. It is a node that has been installed and not yet
    /// told what it belongs to, and it produces [`Enrolment::none`] — which talks to
    /// nobody. A *present but wrong* file is an error, because somebody meant something
    /// by it and silently ignoring it would leave a machine isolated for a reason nobody
    /// could see.
    pub fn load(node_public_key: &str, path: impl AsRef<Path>) -> Result<Self, MembershipError> {
        let path = path.as_ref();
        if !path.exists() {
            return Ok(Self::none());
        }
        let raw = std::fs::read_to_string(path)
            .map_err(|e| MembershipError::Malformed(format!("cannot read membership: {e}")))?;
        let membership: Membership = serde_json::from_str(&raw)
            .map_err(|e| MembershipError::Malformed(format!("cannot decode membership: {e}")))?;
        Self::new(node_public_key, membership)
    }

    /// Write an enrolment out, so the node keeps it across reboots.
    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), MembershipError> {
        let Some(membership) = self.membership() else {
            return Err(MembershipError::Unenrolled);
        };
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                MembershipError::Malformed(format!("cannot create {parent:?}: {e}"))
            })?;
        }
        let encoded = serde_json::to_string_pretty(membership)
            .map_err(|e| MembershipError::Malformed(format!("cannot encode membership: {e}")))?;
        std::fs::write(path, encoded)
            .map_err(|e| MembershipError::Malformed(format!("cannot write membership: {e}")))
    }

    /// Decide whether an inbound sender may be admitted.
    pub fn admits(
        &self,
        sender_public_key: &str,
        presented: Option<&Membership>,
    ) -> Result<(), MembershipError> {
        let Some(enrolled) = self.inner.as_ref() else {
            // We belong to no fabric, so there is no one we are entitled to talk to.
            return Err(MembershipError::Unenrolled);
        };
        let Some(presented) = presented else {
            return Err(MembershipError::Unenrolled);
        };
        presented.verify_for(&enrolled.owner_public_key, sender_public_key)
    }
}
