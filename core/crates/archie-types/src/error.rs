//! Errors raised while constructing or validating domain values.

use thiserror::Error;

/// A value rejected at the boundary of the type system.
///
/// Every variant is a refusal to construct an invalid domain object. Nothing in the
/// control plane should ever hold a `Capability`, `NodeId`, or `Task` that violates
/// its own invariants, so these errors are raised at parse/construct time rather than
/// checked repeatedly downstream.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum TypeError {
    #[error("invalid capability name {name:?}: {reason}")]
    InvalidCapability { name: String, reason: &'static str },

    #[error("invalid node id {value:?}: {reason}")]
    InvalidNodeId { value: String, reason: &'static str },

    #[error("invalid module id {value:?}: {reason}")]
    InvalidModuleId { value: String, reason: &'static str },

    #[error("invalid model id {value:?}: {reason}")]
    InvalidModelId { value: String, reason: &'static str },

    #[error("illegal task transition: {from} -> {to}")]
    IllegalTaskTransition {
        from: &'static str,
        to: &'static str,
    },

    #[error("invalid task: {0}")]
    InvalidTask(String),

    #[error("invalid module manifest: {0}")]
    InvalidManifest(String),
}

pub type Result<T> = std::result::Result<T, TypeError>;
