//! `archie-task-runner` — the last program before the worker.
//!
//! RFC-007 §7 step 7, §3.4.
//!
//! This is a separate program from the executor for one reason, and it is not tidiness.
//! **Landlock is irreversible for a process tree.** Whatever applies a ruleset can never
//! remove it, and neither can anything it spawns afterwards. A long-lived supervisor that
//! confined itself to one task's grants could never start a second task.
//!
//! So the sequence is: the executor forks, the child becomes this program, this program
//! confines *itself* to exactly what one task was granted, and then `exec`s the worker
//! into the restrictions it just accepted.
//!
//! ```text
//! archie-executord   forks
//!   archie-task-runner
//!     validates the spec it was handed
//!     joins the task's cgroup
//!     applies Landlock, irreversibly
//!     drops what it does not need
//!     exec -> worker
//! ```
//!
//! Everything here runs before the worker exists, which means every mistake here is a
//! mistake made with the executor's privileges. The program is deliberately tiny.

pub mod abi;
pub mod handoff;
pub mod plan;
pub mod rights;
pub mod ruleset;

pub use abi::{probe, Probe, Unavailable};
pub use handoff::{Handoff, HandoffError, MAX_SPEC_BYTES};
pub use plan::{plan, Plan, PlanError};
pub use rights::{handled_access, mask_to_handled, rights_for};
pub use ruleset::{apply, rules_for, ApplyError, Rule};
