//! The context gateway: the only path from private storage to a model.
//!
//! # Why a capsule rather than a string
//!
//! Assembling context as a plain `String` loses three things at once: what it was
//! assembled *for*, when it stops being allowed to exist, and which real values the
//! aliases inside it stand for. Each of those omissions is recoverable by convention
//! and lost by accident. A capsule carries them as data, so that rendering context for
//! a task it was not built for, or after it expired, is a type-level refusal rather
//! than a code review note.
//!
//! # What deliberately does not exist here
//!
//! [`ContextCapsule`] has no `Serialize`. A capsule holds private text and the alias
//! table that reverses it, and the whole point of the design is that neither reaches a
//! trace, a log or a peer. [`CapsuleReceipt`] is the serialisable shadow: identifiers,
//! counts, grades and provenance, with nothing that was said.

use crate::reference::{EntryClass, VaultRef};
use crate::store::{Disclosure, Vault, VaultError};
use archie_types::{Locality, PrivacyGrade};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;

/// The most sensitive grade that may be disclosed to `locality`.
pub fn max_grade_for(locality: Locality) -> PrivacyGrade {
    match locality {
        Locality::Cloud => PrivacyGrade::Internal,
        Locality::Fabric => PrivacyGrade::Sensitive,
        Locality::Local => PrivacyGrade::Restricted,
    }
}

/// A sensitive thing that has a local name and an external stand-in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entity {
    pub kind: EntityKind,
    /// What it is really called. Never leaves this machine.
    pub value: String,
}

impl Entity {
    pub fn new(kind: EntityKind, value: impl Into<String>) -> Self {
        Self {
            kind,
            value: value.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntityKind {
    Person,
    Company,
    Contract,
    Location,
    Account,
    Project,
    Other(String),
}

impl EntityKind {
    /// The prefix an alias of this kind carries, e.g. `PERSON`.
    fn prefix(&self) -> String {
        match self {
            Self::Person => "PERSON".into(),
            Self::Company => "COMPANY".into(),
            Self::Contract => "CONTRACT".into(),
            Self::Location => "LOCATION".into(),
            Self::Account => "ACCOUNT".into(),
            Self::Project => "PROJECT".into(),
            Self::Other(name) => name
                .chars()
                .filter(|c| c.is_ascii_alphanumeric())
                .collect::<String>()
                .to_uppercase(),
        }
    }
}

/// When real values should be swapped for stand-ins.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PseudonymPolicy {
    /// Substitute only when the context leaves the operator's control.
    ///
    /// Pseudonymising local inference costs accuracy and buys nothing: the machine
    /// holding the alias table is the machine running the model.
    #[default]
    WhenDisclosedExternally,
    /// Substitute regardless of destination.
    Always,
    /// Never substitute. Legitimate when the task *is* the identity — "how do you
    /// spell my landlord's surname" survives no pseudonym.
    Never,
}

/// What a caller wants context for.
#[derive(Debug, Clone, PartialEq)]
pub struct ContextRequest {
    /// Trusted runtime's retrieval boundary. None preserves historical consumers.
    pub domain: Option<crate::ContextDomain>,
    pub task_id: String,
    pub trace_id: String,
    /// Why this context is being assembled, for the audit record.
    pub purpose: String,
    /// Exactly the entries the task needs. The gateway retrieves no more than this.
    pub references: Vec<VaultRef>,
    /// Where the assembled context is going.
    pub destination: Locality,
    /// How long the capsule may exist.
    pub ttl_ms: i64,
    pub pseudonymize: PseudonymPolicy,
}

impl ContextRequest {
    pub fn new(
        task_id: impl Into<String>,
        trace_id: impl Into<String>,
        purpose: impl Into<String>,
        destination: Locality,
        ttl_ms: i64,
    ) -> Self {
        Self {
            domain: None,
            task_id: task_id.into(),
            trace_id: trace_id.into(),
            purpose: purpose.into(),
            references: Vec::new(),
            destination,
            ttl_ms,
            pseudonymize: PseudonymPolicy::default(),
        }
    }

    pub fn including(mut self, reference: VaultRef) -> Self {
        self.references.push(reference);
        self
    }

    pub fn in_domain(mut self, domain: crate::ContextDomain) -> Self {
        self.domain = Some(domain);
        self
    }

    /// Check the whole set before ranking or decrypting any entry.
    pub fn validate_domain(&self) -> Result<(), GatewayError> {
        if let Some(domain) = &self.domain {
            if self
                .references
                .iter()
                .any(|reference| !domain.contains(reference))
            {
                return Err(GatewayError::DomainMismatch);
            }
        } else if self
            .references
            .iter()
            .any(|reference| matches!(reference.scope(), "personal" | "work"))
        {
            return Err(GatewayError::DomainRequired);
        }
        Ok(())
    }
}

/// One disclosed item inside a capsule.
#[derive(Debug, Clone, PartialEq)]
pub struct CapsuleEntry {
    pub reference: VaultRef,
    pub class: EntryClass,
    pub grade: PrivacyGrade,
    /// Already pseudonymised where policy required it.
    pub text: String,
}

/// Assembles capsules. Holds the entity book that makes aliases possible.
#[derive(Debug, Default, Clone)]
pub struct ContextGateway {
    entities: Vec<Entity>,
}

impl ContextGateway {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register something whose real value must not leave this machine verbatim.
    pub fn knowing(mut self, entity: Entity) -> Self {
        self.entities.push(entity);
        self
    }

    pub fn entities(&self) -> &[Entity] {
        &self.entities
    }

    /// Retrieve, gate, pseudonymise and seal a capsule.
    ///
    /// Fails closed: one reference that may not travel to this destination refuses the
    /// whole capsule rather than quietly shipping a context with a hole in it, because
    /// a model reasoning over silently-truncated context reports confidence it has not
    /// earned. Secrets are the designed exception — they downgrade to a credential
    /// reference, which is what a tool call needs in order to name one.
    pub fn assemble(
        &self,
        vault: &mut Vault,
        request: &ContextRequest,
        now_ms: i64,
    ) -> Result<ContextCapsule, GatewayError> {
        if request.ttl_ms <= 0 {
            return Err(GatewayError::InvalidTtl);
        }
        request.validate_domain()?;
        let permitted = max_grade_for(request.destination);
        let mut entries = Vec::new();
        let mut credential_refs = Vec::new();
        let mut aliases = AliasTable::default();
        let substitute = match request.pseudonymize {
            PseudonymPolicy::Always => true,
            PseudonymPolicy::Never => false,
            PseudonymPolicy::WhenDisclosedExternally => {
                request.destination.discloses_to_third_party()
            }
        };

        for reference in &request.references {
            match vault.disclose(&request.purpose, reference, permitted, now_ms) {
                Ok(Disclosure::Content {
                    reference,
                    class,
                    grade,
                    text,
                }) => {
                    let text = if substitute {
                        aliases.substitute(&self.entities, &text)
                    } else {
                        text
                    };
                    entries.push(CapsuleEntry {
                        reference,
                        class,
                        grade,
                        text,
                    });
                }
                Ok(Disclosure::CredentialReference { reference, .. }) => {
                    credential_refs.push(reference);
                }
                Err(VaultError::GradeExceedsPermitted { entry, .. }) => {
                    return Err(GatewayError::Undisclosable {
                        reference: reference.clone(),
                        grade: entry,
                        destination: request.destination,
                    });
                }
                Err(error) => {
                    return Err(GatewayError::Retrieval {
                        reference: reference.clone(),
                        error,
                    });
                }
            }
        }

        Ok(ContextCapsule {
            domain: request.domain.clone(),
            capsule_id: format!("capsule-{}-{now_ms}", request.trace_id),
            task_id: request.task_id.clone(),
            trace_id: request.trace_id.clone(),
            issued_at_ms: now_ms,
            expires_at_ms: now_ms.saturating_add(request.ttl_ms),
            destination: request.destination,
            permitted_grade: permitted,
            entries,
            credential_refs,
            aliases,
            disposed: false,
        })
    }
}

/// Scoped, expiring context. Not serialisable, by design.
pub struct ContextCapsule {
    domain: Option<crate::ContextDomain>,
    capsule_id: String,
    task_id: String,
    trace_id: String,
    issued_at_ms: i64,
    expires_at_ms: i64,
    destination: Locality,
    permitted_grade: PrivacyGrade,
    entries: Vec<CapsuleEntry>,
    credential_refs: Vec<VaultRef>,
    aliases: AliasTable,
    disposed: bool,
}

impl fmt::Debug for ContextCapsule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // No entry text, no alias table: `{:?}` on a capsule is a thing people do while
        // debugging, and it must not be the hole every other rule here is protecting.
        f.debug_struct("ContextCapsule")
            .field("capsule_id", &self.capsule_id)
            .field("task_id", &self.task_id)
            .field("entries", &self.entries.len())
            .field("credential_refs", &self.credential_refs.len())
            .field("expires_at_ms", &self.expires_at_ms)
            .field("disposed", &self.disposed)
            .finish_non_exhaustive()
    }
}

impl ContextCapsule {
    pub fn capsule_id(&self) -> &str {
        &self.capsule_id
    }

    pub fn task_id(&self) -> &str {
        &self.task_id
    }

    pub fn trace_id(&self) -> &str {
        &self.trace_id
    }

    pub fn destination(&self) -> Locality {
        self.destination
    }

    pub fn credential_references(&self) -> &[VaultRef] {
        &self.credential_refs
    }

    pub fn entries(&self) -> &[CapsuleEntry] {
        &self.entries
    }

    pub fn expired(&self, now_ms: i64) -> bool {
        now_ms >= self.expires_at_ms
    }

    pub fn is_disposed(&self) -> bool {
        self.disposed
    }

    /// The text to send, for a task and a moment.
    ///
    /// `task_id` is checked rather than assumed: a capsule assembled under one task's
    /// policy is not context for another, and the cheapest place to catch that is here.
    pub fn render(&self, task_id: &str, now_ms: i64) -> Result<String, GatewayError> {
        if self.disposed {
            return Err(GatewayError::Disposed);
        }
        if self.expired(now_ms) {
            return Err(GatewayError::Expired {
                expired_at_ms: self.expires_at_ms,
            });
        }
        if task_id != self.task_id {
            return Err(GatewayError::WrongTask {
                capsule_task: self.task_id.clone(),
                requested: task_id.to_string(),
            });
        }

        let mut out = String::new();
        for entry in &self.entries {
            out.push_str(&format!("[{} · {}]\n", entry.reference, entry.class));
            out.push_str(&entry.text);
            out.push('\n');
        }
        for reference in &self.credential_refs {
            // The model is told the credential exists and how to ask for it, never what
            // it is. The broker resolves this at the execution boundary.
            out.push_str(&format!("[credential_ref: {reference}]\n"));
        }
        Ok(out)
    }

    /// Turn aliases in a model's output back into the real values.
    pub fn resolve(&self, output: &str, now_ms: i64) -> Result<String, GatewayError> {
        if self.disposed {
            return Err(GatewayError::Disposed);
        }
        if self.expired(now_ms) {
            return Err(GatewayError::Expired {
                expired_at_ms: self.expires_at_ms,
            });
        }
        Ok(self.aliases.restore(output))
    }

    /// Destroy the contents. Idempotent.
    pub fn dispose(&mut self) {
        for entry in &mut self.entries {
            entry.text.clear();
        }
        self.entries.clear();
        self.aliases.clear();
        self.disposed = true;
    }

    /// What may be written down about this capsule.
    pub fn receipt(&self) -> CapsuleReceipt {
        CapsuleReceipt {
            domain: self.domain.clone(),
            capsule_id: self.capsule_id.clone(),
            task_id: self.task_id.clone(),
            trace_id: self.trace_id.clone(),
            issued_at_ms: self.issued_at_ms,
            expires_at_ms: self.expires_at_ms,
            destination: self.destination,
            permitted_grade: self.permitted_grade,
            disclosed: self
                .entries
                .iter()
                .map(|e| DisclosedItem {
                    reference: e.reference.clone(),
                    class: e.class,
                    grade: e.grade,
                })
                .collect(),
            credential_references: self.credential_refs.clone(),
            aliases_issued: self.aliases.len(),
        }
    }
}

impl Drop for ContextCapsule {
    fn drop(&mut self) {
        if !self.disposed {
            self.dispose();
        }
    }
}

/// The serialisable record of a capsule: shape, never content.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CapsuleReceipt {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<crate::ContextDomain>,
    pub capsule_id: String,
    pub task_id: String,
    pub trace_id: String,
    pub issued_at_ms: i64,
    pub expires_at_ms: i64,
    pub destination: Locality,
    pub permitted_grade: PrivacyGrade,
    pub disclosed: Vec<DisclosedItem>,
    pub credential_references: Vec<VaultRef>,
    pub aliases_issued: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DisclosedItem {
    pub reference: VaultRef,
    pub class: EntryClass,
    pub grade: PrivacyGrade,
}

/// Real value ↔ stand-in, held locally for the life of one capsule.
#[derive(Debug, Default, Clone)]
struct AliasTable {
    /// alias → real value
    by_alias: BTreeMap<String, String>,
    /// real value → alias, so the same entity gets one alias within a capsule
    by_value: BTreeMap<String, String>,
    counters: BTreeMap<String, usize>,
}

impl AliasTable {
    fn len(&self) -> usize {
        self.by_alias.len()
    }

    fn clear(&mut self) {
        self.by_alias.clear();
        self.by_value.clear();
        self.counters.clear();
    }

    /// Replace every known entity in `text` with a stable stand-in.
    ///
    /// Longest value first: with "Acme" and "Acme Holdings" both registered, replacing
    /// the short one first leaves `COMPANY_1 Holdings`, which discloses the very word
    /// the substitution existed to remove.
    fn substitute(&mut self, entities: &[Entity], text: &str) -> String {
        let mut ordered: Vec<&Entity> = entities.iter().filter(|e| !e.value.is_empty()).collect();
        ordered.sort_by(|a, b| {
            b.value
                .len()
                .cmp(&a.value.len())
                .then(a.value.cmp(&b.value))
        });

        let mut out = text.to_string();
        for entity in ordered {
            if !out.contains(&entity.value) {
                continue;
            }
            let alias = self.alias_for(entity);
            out = out.replace(&entity.value, &alias);
        }
        out
    }

    fn alias_for(&mut self, entity: &Entity) -> String {
        if let Some(existing) = self.by_value.get(&entity.value) {
            return existing.clone();
        }
        let prefix = entity.kind.prefix();
        let counter = self.counters.entry(prefix.clone()).or_insert(0);
        *counter += 1;
        let alias = format!("{prefix}_{counter}");
        self.by_alias.insert(alias.clone(), entity.value.clone());
        self.by_value.insert(entity.value.clone(), alias.clone());
        alias
    }

    /// Put real values back. Aliases the model invented resolve to nothing and are left
    /// exactly as they arrived, rather than guessed at.
    fn restore(&self, text: &str) -> String {
        let mut ordered: Vec<(&String, &String)> = self.by_alias.iter().collect();
        ordered.sort_by(|a, b| b.0.len().cmp(&a.0.len()).then(a.0.cmp(b.0)));
        let mut out = text.to_string();
        for (alias, value) in ordered {
            out = out.replace(alias.as_str(), value.as_str());
        }
        out
    }
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum GatewayError {
    #[error("personal and work context require an explicit domain")]
    DomainRequired,
    #[error("context references cross the task's domain boundary")]
    DomainMismatch,
    #[error("capsule time-to-live must be positive")]
    InvalidTtl,
    #[error("{reference} is graded {grade} and cannot be disclosed to {destination:?}")]
    Undisclosable {
        reference: VaultRef,
        grade: PrivacyGrade,
        destination: Locality,
    },
    #[error("could not retrieve {reference}: {error}")]
    Retrieval {
        reference: VaultRef,
        error: VaultError,
    },
    #[error("capsule expired at {expired_at_ms}")]
    Expired { expired_at_ms: i64 },
    #[error("capsule has been disposed")]
    Disposed,
    #[error("capsule belongs to task {capsule_task}, not {requested}")]
    WrongTask {
        capsule_task: String,
        requested: String,
    },
}
