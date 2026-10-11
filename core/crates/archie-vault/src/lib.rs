//! # archie-vault
//!
//! Where Archie keeps what is the operator's, and the one door between it and a model.
//!
//! # The distinction this crate exists to hold
//!
//! Every other privacy control in ArchieOS answers *where* data may be processed:
//! [`archie_types::PrivacyGrade`] against [`archie_types::Locality`], enforced as a
//! router disqualifier rather than a scoring term. That axis is the right one for a
//! document, a private fact, a conversation. It is the wrong one for a password.
//!
//! A password disclosed to a local model is still disclosed. It contributes nothing to
//! reasoning — no model answers better for knowing the literal bytes of an API key —
//! and the disclosure cannot be withdrawn once it is in a context window, a log line or
//! a provider's retention window. So credentials are graded
//! [`archie_types::PrivacyGrade::Secret`], which no locality permits, and they reach an
//! outbound request through [`Vault::broker_credential`] at the execution boundary
//! instead. What the model sees is `credential_ref: vault:github/main`.
//!
//! # Shape
//!
//! * [`VaultRef`] — the `vault:` references the rest of the codebase already writes.
//! * [`Vault`] — AEAD-sealed entries whose grade is authenticated, not merely labelled,
//!   plus an access log that records refusals as carefully as grants.
//! * [`ContextGateway`] — assembles a [`ContextCapsule`]: minimal, task-bound,
//!   expiring, pseudonymised where it leaves the machine, and unserialisable so that it
//!   cannot become a trace by accident.
//!
//! # What this crate does not do
//!
//! It does not invent cryptography: sealing is AES-256-GCM and derivation is
//! PBKDF2-HMAC-SHA256, both from `ring`, already in this workspace's dependency graph
//! by way of rustls. It does not persist anything on its own — [`Vault::sealed_entries`]
//! hands ciphertext to whichever store the caller already trusts. And it does not
//! decide policy; it enforces the grade each entry was stored with.

#![forbid(unsafe_code)]

pub mod domain;
pub mod gateway;
pub mod reference;
pub mod store;

pub use domain::{ContextDomain, DomainError};
pub use gateway::{
    max_grade_for, CapsuleEntry, CapsuleReceipt, ContextCapsule, ContextGateway, ContextRequest,
    DisclosedItem, Entity, EntityKind, GatewayError, PseudonymPolicy,
};
pub use reference::{EntryClass, RefError, VaultRef};
pub use store::{
    refusals, AuditAction, AuditEntry, Disclosure, SealedCredential, SealedEntry, Vault,
    VaultError, VaultKey,
};
