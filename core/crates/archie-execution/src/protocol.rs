//! The typed protocol between `archied` and `archie-executord`.
//!
//! RFC-007 §7, step 3. This is deliberately *not* `SchedulingAction`.
//!
//! # Why the executor does not speak scheduler
//!
//! It would be shorter to hand `SchedulingAction` straight to the executor. It would also
//! give the smallest, most privileged process in the system a reason to know about queue
//! classes, preemption policy and node placement — none of which it may act on, and all
//! of which would grow.
//!
//! The executor's whole defensibility rests on being readable in an afternoon. So it
//! receives commands in the imperative — start this, stop that, ask this one to
//! checkpoint — and the translation from "the scheduler decided to preempt a deep-idle
//! job" into those commands stays on the authority side, where the policy lives.
//!
//! # Every command names a task, and the executor is the one who knows
//!
//! Commands are idempotent by task id. A control plane that restarted and is resending
//! is behaving correctly; the executor is the only component positioned to know whether
//! the first attempt landed, so it answers rather than complaining.

use archie_types::TaskId;
use serde::{Deserialize, Serialize};

use crate::lifecycle::ExecutionState;
use crate::spec::TaskExecutionSpec;

/// What `archied` asks the executor to do.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum Command {
    /// Run this task. Idempotent: resending for a task already running is answered with
    /// its current state, not an error.
    Start(Box<TaskExecutionSpec>),

    /// Ask a task to stop gracefully, then force it after `grace_ms`.
    ///
    /// The grace period is carried in the command rather than configured in the
    /// executor, because how long a shutdown may take is a property of the workload and
    /// the control plane is the one that knows it.
    Stop { task_id: TaskId, grace_ms: u64 },

    /// Free this task's resources for something more urgent.
    ///
    /// Not "freeze". Preemption is a goal, and how it is reached depends on what the
    /// worker can do: a worker that checkpoints is asked and given until the deadline; a
    /// worker that cannot is stopped or frozen according to policy. Naming the command
    /// after the mechanism would put that decision in the wrong process.
    Preempt {
        task_id: TaskId,
        /// How long to wait for a cooperative checkpoint before falling back.
        deadline_ms: u64,
        /// Whether the fallback may freeze the task rather than stopping it. Freezing
        /// keeps the work alive but resident; stopping loses it. That is a policy call.
        allow_freeze: bool,
    },

    /// Bring a checkpointed or frozen task back.
    Resume { task_id: TaskId },

    /// Change a running task's resource budget without restarting it. The resource
    /// governor's lever.
    Rebudget {
        task_id: TaskId,
        budget: crate::spec::ResourceBudget,
    },

    /// Report everything the executor knows. Used by `archied` on restart to rebuild
    /// scheduler state, rather than assuming its tasks died with it.
    Inventory,

    /// Collect everything that has happened since the last time anyone asked.
    ///
    /// Pull rather than push. A pushed event needs a connection the executor holds open
    /// and writes to unprompted, and this executor answers one caller at a time on
    /// purpose — a held subscription would block every other command. Pulling also means
    /// an archied that is restarting cannot miss anything: the events are still queued
    /// when it comes back.
    Drain,
}

impl Command {
    /// The task this command is about, if it is about one.
    pub fn task_id(&self) -> Option<TaskId> {
        match self {
            Self::Start(spec) => Some(spec.task_id),
            Self::Stop { task_id, .. }
            | Self::Preempt { task_id, .. }
            | Self::Resume { task_id }
            | Self::Rebudget { task_id, .. } => Some(*task_id),
            Self::Inventory | Self::Drain => None,
        }
    }

    /// Whether this command can change what is running.
    ///
    /// `Inventory` cannot, which is why it is the one command safe to issue during
    /// reconciliation before any policy decision has been made.
    pub fn is_mutating(&self) -> bool {
        // `Drain` empties the queue, which is a change to the executor's own bookkeeping
        // and not to anything running. It is grouped with the read-only commands because
        // the question this answers is "could this alter a worker".
        !matches!(self, Self::Inventory | Self::Drain)
    }
}

/// One task as the executor sees it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskStatus {
    pub task_id: TaskId,
    pub state: ExecutionState,
    pub pid: Option<u32>,
    /// Set once the task has ended.
    pub exit_code: Option<i32>,
    /// Set when it was ended by a signal rather than its own choice.
    pub exit_signal: Option<i32>,
    pub failure_reason: Option<String>,
}

/// What the executor answers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "response", rename_all = "snake_case")]
pub enum Response {
    /// The command was applied. Carries the resulting state so the caller never has to
    /// guess or immediately ask again.
    Accepted(TaskStatus),

    /// The command was a no-op because the task was already in that condition. Not an
    /// error: a control plane retrying after a timeout is behaving correctly.
    AlreadySo(TaskStatus),

    /// Refused, with a reason the caller can act on.
    Refused {
        task_id: Option<TaskId>,
        reason: RefusalReason,
        detail: String,
    },

    /// Everything the executor knows.
    Inventory { tasks: Vec<TaskStatus> },

    /// Everything that happened since the last drain.
    Events {
        events: Vec<Event>,
        /// Whether the queue overflowed and events were lost.
        ///
        /// The queue is bounded, so an archied that stops asking for long enough will
        /// miss things. Saying so is the whole point: a consumer that believes it has
        /// every event will keep a stale picture forever, whereas one that is told the
        /// stream has a hole in it can fall back to `Inventory`, which is authoritative.
        overflowed: bool,
    },
}

/// Why the executor refused.
///
/// An enum rather than a string because each of these calls for a different response
/// from the control plane, and only one of them is worth retrying.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefusalReason {
    /// The spec was malformed. Retrying sends the same bytes and gets the same answer.
    InvalidSpec,
    /// This node cannot enforce what the task requires. The scheduler should place it
    /// elsewhere — and this is the refusal that must never be worked around locally.
    NotAdmissible,
    /// The program is missing, unreadable, or the OS refused. Worth retrying only after
    /// something on the node changes.
    LaunchFailed,
    /// The task already ran to completion. Retrying would repeat its side effects.
    AlreadyFinished,
    /// The executor holds no record of this task.
    UnknownTask,
    /// The command does not apply in the task's current state.
    NotApplicable,
}

impl RefusalReason {
    /// Whether resending the identical command could succeed later.
    ///
    /// Only `LaunchFailed` can: the others describe the request or the node, and neither
    /// changes by asking again. A control plane that retries everything turns a missing
    /// binary into a permanent log flood.
    pub fn is_retryable(&self) -> bool {
        matches!(self, Self::LaunchFailed)
    }

    /// Whether this means the task should be placed on a different node.
    pub fn suggests_replacement(&self) -> bool {
        matches!(self, Self::NotAdmissible)
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::InvalidSpec => "invalid_spec",
            Self::NotAdmissible => "not_admissible",
            Self::LaunchFailed => "launch_failed",
            Self::AlreadyFinished => "already_finished",
            Self::UnknownTask => "unknown_task",
            Self::NotApplicable => "not_applicable",
        }
    }
}

/// Something the executor reports without being asked.
///
/// A worker finishing is not a reply to anything — nobody asked it to finish now — so it
/// arrives as an event. Modelling completion as a response to a poll would make the
/// control plane's view of the world depend on how often it remembered to ask.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    /// A task reached a terminal state.
    Finished(TaskStatus),
    /// A task changed state without finishing — checkpointed, frozen, resumed.
    StateChanged(TaskStatus),
    /// Reconciliation found a record and reality disagreeing. Always reported, never
    /// resolved silently.
    Discrepancy {
        task_id: TaskId,
        recorded: ExecutionState,
        detail: String,
    },
}

impl Event {
    pub fn task_id(&self) -> TaskId {
        match self {
            Self::Finished(status) | Self::StateChanged(status) => status.task_id,
            Self::Discrepancy { task_id, .. } => *task_id,
        }
    }

    /// Whether this event needs a human or a policy decision rather than bookkeeping.
    pub fn needs_attention(&self) -> bool {
        match self {
            Self::Discrepancy { .. } => true,
            Self::Finished(status) | Self::StateChanged(status) => status.state.needs_decision(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::{
        FilesystemGrant, RequiredEnforcement, ResolvedExecutable, ResourceBudget, RestartPolicy,
    };
    use archie_types::{ModuleId, QueueClass, Timestamp, TraceId};
    use std::path::PathBuf;

    fn spec() -> TaskExecutionSpec {
        TaskExecutionSpec {
            task_id: TaskId::new(),
            trace_id: TraceId::new(),
            executable: ResolvedExecutable::from_registry(
                ModuleId::parse("research.crawler").unwrap(),
                "1.0.0",
                "/usr/lib/archie/workers/crawler",
                vec![],
            )
            .unwrap(),
            working_directory: PathBuf::from("/var/lib/archie/work"),
            environment_allowlist: vec![],
            environment_overrides: Default::default(),
            required_capabilities: vec![],
            filesystem_grants: vec![FilesystemGrant::read_only("/usr/lib/archie")],
            network: Default::default(),
            required_enforcement: RequiredEnforcement::none(),
            budget: ResourceBudget::default(),
            queue_class: QueueClass::Background,
            supports_checkpoint: false,
            checkpoint_deadline_ms: 5_000,
            lease_reference: None,
            artifact_directory: None,
            restart: RestartPolicy::Never,
            timeout_ms: 0,
            requested_by: "scheduler".into(),
            requested_at: Timestamp::now(),
        }
    }

    #[test]
    fn preempt_is_a_goal_not_a_mechanism() {
        // Naming the command "freeze" would decide in the wrong process how preemption
        // is achieved. The executor is told what to accomplish and what it is permitted
        // to use; how depends on what the worker can do.
        let command = Command::Preempt {
            task_id: TaskId::new(),
            deadline_ms: 5_000,
            allow_freeze: true,
        };
        assert!(command.is_mutating());
        assert!(command.task_id().is_some());
    }

    #[test]
    fn inventory_is_the_one_command_safe_before_a_decision() {
        // Reconciliation has to look before anything acts, and this is what it uses.
        assert!(!Command::Inventory.is_mutating());
        assert!(Command::Inventory.task_id().is_none());
    }

    #[test]
    fn every_task_command_names_its_task() {
        let id = TaskId::new();
        let commands = [
            Command::Start(Box::new(spec())),
            Command::Stop {
                task_id: id,
                grace_ms: 10_000,
            },
            Command::Preempt {
                task_id: id,
                deadline_ms: 5_000,
                allow_freeze: false,
            },
            Command::Resume { task_id: id },
            Command::Rebudget {
                task_id: id,
                budget: ResourceBudget::default(),
            },
        ];
        for command in commands {
            assert!(command.task_id().is_some(), "{command:?}");
            assert!(command.is_mutating());
        }
    }

    #[test]
    fn only_a_launch_failure_is_worth_resending() {
        // A control plane that retries everything turns a missing binary into a
        // permanent log flood, and turns an inadmissible task into one that is never
        // placed anywhere else.
        assert!(RefusalReason::LaunchFailed.is_retryable());
        for reason in [
            RefusalReason::InvalidSpec,
            RefusalReason::NotAdmissible,
            RefusalReason::AlreadyFinished,
            RefusalReason::UnknownTask,
            RefusalReason::NotApplicable,
        ] {
            assert!(!reason.is_retryable(), "{}", reason.as_str());
        }
    }

    #[test]
    fn inadmissibility_is_the_one_that_moves_the_task() {
        assert!(RefusalReason::NotAdmissible.suggests_replacement());
        assert!(!RefusalReason::LaunchFailed.suggests_replacement());
        assert!(!RefusalReason::InvalidSpec.suggests_replacement());
    }

    #[test]
    fn a_no_op_is_not_an_error() {
        // A control plane retrying after a timeout is behaving correctly, and the
        // executor is the only component that knows whether the first attempt landed.
        let status = TaskStatus {
            task_id: TaskId::new(),
            state: ExecutionState::Running,
            pid: Some(4242),
            exit_code: None,
            exit_signal: None,
            failure_reason: None,
        };
        let response = Response::AlreadySo(status.clone());
        match response {
            Response::AlreadySo(s) => assert_eq!(s.state, ExecutionState::Running),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn completion_is_an_event_not_a_reply() {
        // Nobody asked the worker to finish now. Modelling this as a response to a poll
        // would make the control plane's view depend on how often it remembered to ask.
        let status = TaskStatus {
            task_id: TaskId::new(),
            state: ExecutionState::Completed,
            pid: Some(4242),
            exit_code: Some(0),
            exit_signal: None,
            failure_reason: None,
        };
        let event = Event::Finished(status.clone());
        assert_eq!(event.task_id(), status.task_id);
        assert!(!event.needs_attention());
    }

    #[test]
    fn a_discrepancy_always_needs_attention() {
        let event = Event::Discrepancy {
            task_id: TaskId::new(),
            recorded: ExecutionState::Running,
            detail: "no process in the task cgroup".into(),
        };
        assert!(
            event.needs_attention(),
            "a record and reality disagreeing is never resolved silently"
        );

        // And so does a task that ended up in a state only policy can settle.
        let orphaned = Event::StateChanged(TaskStatus {
            task_id: TaskId::new(),
            state: ExecutionState::Orphaned,
            pid: Some(1),
            exit_code: None,
            exit_signal: None,
            failure_reason: None,
        });
        assert!(orphaned.needs_attention());
    }

    #[test]
    fn a_signal_exit_is_carried_apart_from_an_exit_code() {
        // A worker OOM-killed and one returning 137 are different events, and only one
        // is the node's fault. Collapsing them into a single integer loses that.
        let status = TaskStatus {
            task_id: TaskId::new(),
            state: ExecutionState::Failed,
            pid: Some(9),
            exit_code: None,
            exit_signal: Some(9),
            failure_reason: Some("killed by signal 9".into()),
        };
        assert!(status.exit_code.is_none());
        assert_eq!(status.exit_signal, Some(9));
    }

    #[test]
    fn commands_and_responses_survive_the_wire() {
        let command = Command::Start(Box::new(spec()));
        let json = serde_json::to_string(&command).unwrap();
        assert!(json.contains("\"command\":\"start\""));
        assert_eq!(serde_json::from_str::<Command>(&json).unwrap(), command);

        let response = Response::Refused {
            task_id: None,
            reason: RefusalReason::NotAdmissible,
            detail: "no delegated cgroup subtree".into(),
        };
        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("not_admissible"));
        assert_eq!(serde_json::from_str::<Response>(&json).unwrap(), response);
    }
}
