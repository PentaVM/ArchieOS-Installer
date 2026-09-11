//! # archie-fabric
//!
//! Node identity, authenticated messaging, and peer discovery.
//!
//! The fabric is capability-based: a node advertises *what it can do*, and Archie Core
//! never needs machine-specific knowledge to use it. Adding hardware means a new node
//! announcing new capabilities, not an architectural change.
//!
//! ## Layering
//!
//! ```text
//! discovery::Fabric      who is out there, and what do they advertise
//!   envelope::SignedEnvelope   authentication of every byte
//!     identity::NodeIdentity   the Ed25519 keypair that IS the node
//!   transport::Transport       how bytes move (loopback / UDP / future Zenoh)
//! ```
//!
//! ## The one rule
//!
//! Authentication is not authorisation. This crate establishes *who* sent a message.
//! Whether that peer may be scheduled work, or receive sensitive data, is decided above
//! it against the pinned trust state in `archie-state`.

#![forbid(unsafe_code)]

pub mod backing;
pub mod discovery;
pub mod enrolment_plan;
pub mod envelope;
pub mod identity;
pub mod membership;
pub mod message;
pub mod transport;

pub use backing::{
    Attestation, BackingChange, BackingClaim, BackingRefusal, BackingRequirement, KeyBacking,
};
pub use discovery::{Fabric, FabricConfig, FabricEvent, PeerEntry, RejectReason};
pub use enrolment_plan::{
    fingerprint_of_owner_key, plan_enrolment, EnrolmentPaths, EnrolmentPlan, FabricChoice,
    PlanError,
};
pub use envelope::{EnvelopeError, SignedEnvelope, MAX_ENVELOPE_BYTES};
pub use identity::{IdentityError, NodeIdentity};
pub use membership::{fingerprint, Enrolment, Membership, MembershipError, OwnerIdentity};
pub use message::FabricMessage;
pub use transport::{
    Inbound, LoopbackBus, LoopbackTransport, PeerAddress, Transport, TransportError, UdpTransport,
};
