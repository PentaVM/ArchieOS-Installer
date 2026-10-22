//! # archie-state
//!
//! Durable local state and the filesystem layout it lives in.
//!
//! Two responsibilities, kept together because they answer the same question — *what
//! survives a reboot*:
//!
//! * [`paths::StatePaths`] decides **where** state lives.
//! * [`store::StateStore`] decides **what** is stored and guarantees it is crash-safe.
//!
//! Nothing here knows about the fabric, the scheduler, or models. It stores what it is
//! given and enforces the invariants of storage itself.

#![forbid(unsafe_code)]

mod adapters;
pub mod cognitive;
pub mod inference;
pub mod paths;
mod query;
pub mod snapshots;
pub mod store;
pub mod timeseries;

pub use paths::StatePaths;
pub use store::{
    CapabilityDelta, CortexCorrectionTrajectoryRecord, CortexCorrectionTrajectoryWrite,
    DeviceAdapterProposalRecord, DeviceClass, DeviceOperationRecord, DeviceOperationState,
    DevicePairingState, DeviceRecord, Durability, PeerRecord, Result, StateError, StateStore,
    TrustState, SCHEMA_VERSION,
};
