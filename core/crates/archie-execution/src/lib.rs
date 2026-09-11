//! Execution contracts: what may be run, and what state it is in.
//!
//! RFC-007 §7 splits execution into three processes with different privileges:
//!
//! ```text
//! archied            authority   decides WHAT should run
//! archie-executord   mechanism   materializes that decision as a process
//! archie-task-runner sandbox     applies the irreversible boundary, then execs
//! worker             untrusted   the actual workload
//! ```
//!
//! This crate is the contract between them, and deliberately contains no I/O: no process
//! spawning, no cgroup writes, no sockets. Every rule here is a rule about what may be
//! *expressed*, and rules that need a running kernel to check are rules that go unchecked
//! on the machine where they are being written.
//!
//! Two of those rules carry most of the weight:
//!
//! * [`spec::TaskExecutionSpec`] has nowhere to put a shell command. The executable comes
//!   from the trusted module registry through [`spec::ResolvedExecutable`], whose fields
//!   are private, so a path cannot be supplied by a caller or invented by a model.
//! * [`lifecycle::ExecutionState`] keeps `Checkpointed` and `Frozen` apart. Both mean the
//!   task stopped; only one means the work survives being killed.

pub mod checkpoint;
pub mod client;
pub mod lifecycle;
pub mod protocol;
pub mod spec;

pub use checkpoint::{CheckpointOutcome, CheckpointReply, CheckpointRequest};
pub use client::{ClientError, DEFAULT_TIMEOUT};
pub use lifecycle::{
    reconcile, state_after_reconcile, Cause, ExecutionState, Reconciliation, Transition,
    TransitionRefusal,
};
pub use protocol::{Command, Event, RefusalReason, Response, TaskStatus};
pub use spec::{
    FilesystemGrant, NetworkGrant, RequiredEnforcement, ResolvedExecutable, ResourceBudget,
    RestartPolicy, SandboxStrength, SpecError, TaskExecutionSpec,
};
