//! `archie-executord` — mechanism, not authority.
//!
//! RFC-007 §7. `archied` decides what should run; this turns that decision into an
//! operating-system process, supervises it, and reports what happened. It deliberately
//! does none of the deciding:
//!
//! * it makes no model calls and holds no policy;
//! * it parses no provider data;
//! * it exposes no network interface;
//! * it owns exactly one delegated cgroup subtree and nothing above it.
//!
//! Keeping it this small is the point. The privilege a worker supervisor needs — creating
//! cgroups, spawning processes, applying sandboxes — is privilege `archied` should never
//! acquire, so it lives here instead, in a process whose entire job can be read in an
//! afternoon.
//!
//! The one failure everything is designed against is running a task twice. Everything
//! else recovers: a failed worker reports a reason, a lost one gets reconciled. A task
//! that ran twice has done its side effects twice, and no later bookkeeping undoes that.

pub mod cgroups;
pub mod dispatch;
pub mod events;
pub mod journal;
pub mod landlock;
pub mod launcher;
pub mod preempt;
pub mod process;
pub mod recovery;
pub mod server;
pub mod supervisor;

pub use cgroups::{CgroupRefusal, TaskCgroupState, TaskCgroups};
pub use dispatch::{plan_preemption, Dispatcher, PreemptionPlan};
pub use events::{EventQueue, MAX_QUEUED_EVENTS};
pub use journal::{Journal, JournalError, Record};
pub use landlock::{Enforcement, SandboxPlan, SandboxRefusal};
pub use launcher::{Child, Exit, LaunchError, Launcher};
pub use preempt::Request as PreemptRequest;
pub use preempt::{preempt, CheckpointChannel, Fallback, Permissions, Preemption};
pub use process::ProcessLauncher;
pub use recovery::{DiscoveredCgroup, Finding, RecordedTask, Summary};
pub use server::{decode, encode, FrameError, MAX_COMMAND_BYTES};
pub use supervisor::{Outcome, StartRefusal, Supervisor, TaskRecord};
