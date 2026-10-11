//! Bounded reference-only context for the existing cognitive loop.
//!
//! The trusted runtime supplies the domain and producer attribution. Neither a pillar
//! tag nor a context reference verifies evidence, grants a lease or admits training.
use crate::{Observation, Trigger};
use archie_types::PrivacyGrade;
use archie_vault::{ContextDomain, VaultRef};
use std::collections::{BTreeMap, BTreeSet};

pub const MAX_CONTEXT_REFERENCES: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Pillar {
    Sentry,
    Wanda,
    Vault,
    Iris,
    Forge,
    VerifiedOutcome,
}

#[derive(Debug, Clone)]
pub struct PillarContext {
    domain: ContextDomain,
    task_id: String,
    references: BTreeMap<VaultRef, BTreeSet<Pillar>>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PillarError {
    #[error("pillar context requires a bounded task identifier")]
    InvalidTask,
    #[error("pillar evidence belongs to another context domain")]
    DomainMismatch,
    #[error("pillar context exceeded its reference budget")]
    ReferenceBudget,
}

impl PillarContext {
    pub fn new(domain: ContextDomain, task_id: impl Into<String>) -> Result<Self, PillarError> {
        let task_id = task_id.into();
        if task_id.is_empty()
            || task_id.len() > 128
            || !task_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        {
            return Err(PillarError::InvalidTask);
        }
        Ok(Self {
            domain,
            task_id,
            references: BTreeMap::new(),
        })
    }

    pub fn domain(&self) -> &ContextDomain {
        &self.domain
    }
    pub fn task_id(&self) -> &str {
        &self.task_id
    }
    pub fn references(&self) -> &BTreeMap<VaultRef, BTreeSet<Pillar>> {
        &self.references
    }

    pub fn include(&mut self, pillar: Pillar, reference: VaultRef) -> Result<(), PillarError> {
        if !self.domain.contains(&reference) {
            return Err(PillarError::DomainMismatch);
        }
        if !self.references.contains_key(&reference)
            && self.references.len() >= MAX_CONTEXT_REFERENCES
        {
            return Err(PillarError::ReferenceBudget);
        }
        self.references.entry(reference).or_default().insert(pillar);
        Ok(())
    }

    pub fn observation(&self, trigger: Trigger, goal_label: &str, at_ms: i64) -> Observation {
        let mut observation = Observation::new(trigger, &self.task_id, goal_label, at_ms)
            .in_domain(self.domain.clone())
            .graded(PrivacyGrade::Restricted);
        observation.context_refs = self.references.keys().cloned().collect();
        observation
    }
}
