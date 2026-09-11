//! The model registry.
//!
//! Stores what models exist and where. It deliberately does **not** choose between them
//! — that is `archie-router`'s job. Keeping storage and selection apart means the
//! routing policy can be replaced without touching how models are registered.

use archie_types::{Capability, ModelDescriptor, ModelId, NodeId, PrivacyGrade};
use std::collections::BTreeMap;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ModelRegistryError {
    #[error("model {0} is already registered")]
    Duplicate(ModelId),

    #[error("model {id} declares locality {locality:?}, which contradicts provider {provider}")]
    InconsistentLocality {
        id: ModelId,
        locality: archie_types::Locality,
        provider: String,
    },

    #[error("model {0} declares a zero context window")]
    ZeroContext(ModelId),
}

/// Registered models, keyed by id.
#[derive(Debug, Clone, Default)]
pub struct ModelRegistry {
    models: BTreeMap<ModelId, ModelDescriptor>,
}

impl ModelRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a model after validating what can be checked statically.
    ///
    /// The locality check is a security control, not a tidiness one: a cloud model
    /// registered as `local` would pass every downstream privacy test while still
    /// sending data to a third party.
    pub fn register(&mut self, model: ModelDescriptor) -> Result<(), ModelRegistryError> {
        if self.models.contains_key(&model.id) {
            return Err(ModelRegistryError::Duplicate(model.id.clone()));
        }
        if !model.locality_is_consistent() {
            return Err(ModelRegistryError::InconsistentLocality {
                id: model.id.clone(),
                locality: model.locality,
                provider: model.provider.to_string(),
            });
        }
        if model.context_window == 0 {
            return Err(ModelRegistryError::ZeroContext(model.id.clone()));
        }
        self.models.insert(model.id.clone(), model);
        Ok(())
    }

    /// Replace an existing registration, e.g. when availability changes.
    pub fn upsert(&mut self, model: ModelDescriptor) -> Result<(), ModelRegistryError> {
        self.models.remove(&model.id);
        self.register(model)
    }

    pub fn get(&self, id: &ModelId) -> Option<&ModelDescriptor> {
        self.models.get(id)
    }

    pub fn remove(&mut self, id: &ModelId) -> Option<ModelDescriptor> {
        self.models.remove(id)
    }

    pub fn iter(&self) -> impl Iterator<Item = &ModelDescriptor> {
        self.models.values()
    }

    pub fn len(&self) -> usize {
        self.models.len()
    }

    pub fn is_empty(&self) -> bool {
        self.models.is_empty()
    }

    /// Models currently usable.
    pub fn usable(&self) -> Vec<&ModelDescriptor> {
        self.models
            .values()
            .filter(|m| m.availability.usable())
            .collect()
    }

    /// Models served by a particular node.
    pub fn on_node(&self, node: &NodeId) -> Vec<&ModelDescriptor> {
        self.models
            .values()
            .filter(|m| m.node.as_ref() == Some(node))
            .collect()
    }

    /// Models providing a capability.
    pub fn providing(&self, capability: &Capability) -> Vec<&ModelDescriptor> {
        self.models
            .values()
            .filter(|m| m.provides(capability))
            .collect()
    }

    /// Whether any registered model can legally handle data of `grade`.
    ///
    /// Used at boot to warn an operator who has configured only cloud models but
    /// intends to work on restricted data — better to say so than to fail the first
    /// time it matters.
    pub fn can_serve_privacy_grade(&self, grade: PrivacyGrade) -> bool {
        self.models
            .values()
            .any(|m| m.availability.usable() && m.permits_privacy(grade))
    }

    /// Whether at least one model runs on this machine.
    ///
    /// A fabric with no local model is entirely dependent on the network to think,
    /// which the operator should know about.
    pub fn has_local_model(&self) -> bool {
        self.models
            .values()
            .any(|m| m.locality == archie_types::Locality::Local && m.availability.usable())
    }
}
