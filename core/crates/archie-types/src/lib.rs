//! # archie-types
//!
//! The shared domain vocabulary of the ArchieOS control plane.
//!
//! This crate deliberately holds **no** I/O, no async runtime, and no dependency on any
//! other Archie crate. Everything else in the workspace depends on it, so keeping it
//! pure is what stops the dependency graph from becoming a cycle.
//!
//! The types here encode the invariants the rest of the system is allowed to assume:
//!
//! * A [`ids::NodeId`] is *derived* from a public key, so it cannot be spoofed.
//! * A [`capability::Capability`] is a validated dotted name, so it is safe to use in
//!   paths and topics.
//! * A [`task::Task`] can only move between states its lifecycle permits.
//! * A [`model::ModelDescriptor`] cannot claim to be local while pointing at a cloud
//!   provider.
//!
//! Validation happens at construction. Downstream code should not re-check invariants
//! that a type already guarantees.

#![forbid(unsafe_code)]
#![warn(missing_debug_implementations)]

pub mod capability;
pub mod error;
pub mod ids;
pub mod model;
pub mod model_role;
pub mod module;
pub mod node;
pub mod task;
pub mod time;

pub use capability::{Capability, CapabilitySet};
pub use error::{Result, TypeError};
pub use ids::{AgentId, JobId, LeaseId, ModelId, ModuleId, NodeId, TaskId, TraceId};
pub use model::{
    Availability, DataRetention, Locality, ModelDescriptor, PrivacyGrade, ProviderKind, TokenCost,
};
pub use model_role::{CompatibilityStatus, ModelRole};
pub use module::{
    DeviceAdapterClass, DeviceAdapterManifest, DeviceAdapterOperation, DeviceAdapterOrigin,
    DeviceAdapterResourceBudget, HealthCheck, ModuleManifest, ModulePermission, ModuleState,
    WorkerImpl,
};
pub use node::{
    Accelerator, AcceleratorKind, CpuInfo, LinkClass, MemoryInfo, NetworkInterface, NodeDescriptor,
    NodeHealth, NodeLoad, NodeProfile, StorageVolume,
};
pub use task::{
    DependencyCondition, DependencyVerdict, QueueClass, Task, TaskDependency, TaskError, TaskGraph,
    TaskStatus,
};
pub use time::Timestamp;

/// The ArchieOS release this build belongs to.
///
/// Sourced from the workspace version so that the CLI, the daemon, and every node
/// announcement report exactly one number.
pub const ARCHIEOS_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Version of the fabric wire protocol.
///
/// Bumped independently of the release version: two nodes on different ArchieOS
/// releases can still talk if they agree here.
pub const FABRIC_PROTOCOL_VERSION: u16 = 2;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_constants_are_populated() {
        assert!(!ARCHIEOS_VERSION.is_empty());
        assert_eq!(FABRIC_PROTOCOL_VERSION, 2);
    }
}
