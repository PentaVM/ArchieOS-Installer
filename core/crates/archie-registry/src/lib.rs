//! # archie-registry
//!
//! The registries Archie Core consults to answer "what exists, and what can run".
//!
//! * [`resource::ResourceGraph`] — nodes, their hardware, and which of them are
//!   actually usable right now.
//! * [`modules::ModuleRegistry`] — installed manifests, and the topology compiler that
//!   decides which of them load on *this* node and explains every exclusion.
//! * [`models::ModelRegistry`] — which models exist and where they live.
//! * [`providers::ProviderRegistry`] — which upstream data sources are declared, and
//!   which of them can actually run on this node.
//!
//! Storage and selection are kept apart on purpose: the registries record facts, while
//! `archie-scheduler` and `archie-router` make choices from them.

#![forbid(unsafe_code)]

pub mod models;
pub mod modules;
pub mod providers;
pub mod resource;

pub use models::{ModelRegistry, ModelRegistryError};
pub use modules::{
    DeviceAdapterRegistry, DeviceAdapterRegistryError, ModuleRegistry, ModuleRegistryError,
    PermissionPolicy, Topology, TopologyContext,
};
pub use providers::{
    ProviderContext, ProviderHealth, ProviderManifest, ProviderPlan, ProviderRegistry,
    ProviderRegistryError, ProviderStatus,
};
pub use resource::{NodeOrigin, NodeRecord, ResourceGraph, ResourceTotals};
