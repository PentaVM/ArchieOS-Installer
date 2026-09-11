//! Reclaiming a slot: ask first, and let the kernel finish the sentence.
//!
//! RFC-007 §7, steps 5 and 6. The cooperative path and its fallback, in one place because
//! the interesting behaviour is entirely in how they hand over to each other.
//!
//! ```text
//! 1. ask the worker to checkpoint        cooperative, with a deadline
//! 2. on deadline, freeze                 the kernel stops it mid-instruction
//! 3. the user's work starts
//! 4. thaw, or kill and resume from the last checkpoint
//! ```
//!
//! Freezing is never the first step. A checkpoint is worth waiting a second for, because
//! idle research that resumes is worth vastly more than idle research that restarts. The
//! deadline is what stops that courtesy becoming an unbounded wait on a user's question.

use archie_execution::checkpoint::{classify, CheckpointOutcome, CheckpointReply};
use archie_observability::kernel::cgroup::CgroupPath;
use archie_types::{TaskId, Timestamp};

use crate::cgroups::{CgroupRefusal, TaskCgroups};

/// The worker control channel.
///
/// Behind a trait for the same reason as the launcher: the real one is a Unix socket the
/// worker inherited, and the sequencing around it — deadlines, late replies, the handover
/// to the freezer — is the part that must be right on every platform.
pub trait CheckpointChannel {
    /// Ask a worker to checkpoint, and wait until `deadline`.
    ///
    /// Returns `None` if nothing arrived in time. Implementations must not block past the
    /// deadline: a user request is waiting on this.
    fn request(&self, task_id: TaskId, deadline: Timestamp) -> Option<CheckpointReply>;

    /// Collect a reply that arrived after we stopped waiting.
    ///
    /// Called once after the fallback has fired. A worker that answered at deadline + 1 ms
    /// produced a real artifact, and the only thing our timing changes is whether we
    /// noticed.
    fn collect_late(&self, task_id: TaskId) -> Option<CheckpointReply> {
        let _ = task_id;
        None
    }
}

/// What the fallback did after the deadline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fallback {
    /// Not needed — the worker answered in time.
    NotNeeded,
    /// The task's cgroup was frozen. Its memory is still resident and its slot is still
    /// held, but it is consuming no CPU.
    Froze,
    /// Freezing was not permitted or not possible, so the task was stopped and its work
    /// since the last checkpoint is gone.
    Stopped,
    /// Neither could be done, and the reason.
    Failed(String),
}

/// The result of one preemption attempt.
#[derive(Debug, Clone, PartialEq)]
pub struct Preemption {
    pub task_id: TaskId,
    pub outcome: CheckpointOutcome,
    pub fallback: Fallback,
}

impl Preemption {
    /// Whether the slot is actually free now.
    ///
    /// A frozen task has *not* released its slot: its memory is resident and its cgroup
    /// exists. Reporting the slot free here would let the scheduler overcommit the node
    /// with a task that is still occupying it.
    pub fn slot_released(&self) -> bool {
        matches!(self.fallback, Fallback::Stopped)
            || (self.fallback == Fallback::NotNeeded && self.outcome.work_is_durable())
    }

    /// Whether the task can be killed now without losing work.
    ///
    /// This is what the late-reply handling buys: a frozen task with a durable checkpoint
    /// can be killed for nothing, and one without loses everything since its last.
    pub fn safe_to_kill(&self) -> bool {
        self.outcome.work_is_durable()
    }

    /// A line an operator can act on.
    pub fn explain(&self) -> String {
        match (&self.outcome, &self.fallback) {
            (CheckpointOutcome::Durable { .. }, _) => {
                "worker checkpointed on request; its work is durable".into()
            }
            (CheckpointOutcome::DurableButLate { .. }, _) => {
                "worker checkpointed after the deadline, once the kernel had already \
                 stopped it; the artifact is real and the task can now be killed for \
                 nothing"
                    .into()
            }
            (CheckpointOutcome::Refused { reason }, Fallback::Froze) => {
                format!(
                    "worker could not checkpoint ({reason}); frozen instead, so its \
                         work survives only while the process does"
                )
            }
            (CheckpointOutcome::Refused { reason }, Fallback::Stopped) => {
                format!(
                    "worker could not checkpoint ({reason}) and freezing was not \
                         permitted; work since its last checkpoint is lost"
                )
            }
            (CheckpointOutcome::TimedOut, Fallback::Froze) => {
                "worker did not answer in time; frozen by the kernel with its state still \
                 in memory"
                    .into()
            }
            (CheckpointOutcome::TimedOut, Fallback::Stopped) => {
                "worker did not answer in time and freezing was not permitted; work since \
                 its last checkpoint is lost"
                    .into()
            }
            (_, Fallback::Failed(detail)) => {
                format!("could not reclaim the slot: {detail}")
            }
            (_, Fallback::NotNeeded) => "no fallback was needed".into(),
        }
    }
}

/// What the caller may do to reclaim the slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Permissions {
    /// Whether the task may be frozen rather than stopped. A policy call: freezing keeps
    /// the work alive but resident, stopping loses it.
    pub allow_freeze: bool,
}

/// One preemption, described.
///
/// Bundled rather than passed as loose arguments because there are six of them and
/// several are booleans — a call site with `true, false, true` in it is one refactor away
/// from freezing the wrong thing.
pub struct Request<'a> {
    pub task_id: TaskId,
    /// Whether this worker implements the checkpoint protocol. A worker that does not is
    /// never asked.
    pub supports_checkpoint: bool,
    /// The task's own cgroup. `None` when it has none, in which case freezing is not
    /// available whatever the permissions say.
    pub cgroup: Option<&'a CgroupPath>,
    pub permissions: Permissions,
    /// When the cooperative window closes. Absolute, not a duration.
    pub deadline: Timestamp,
}

/// Run one preemption.
///
/// `stop` is supplied by the caller rather than taken as a trait, because stopping a task
/// is the supervisor's job and threading the whole supervisor in here would let this
/// module reach far more of it than it needs.
pub fn preempt<C, S>(
    request: Request<'_>,
    cgroups: &TaskCgroups,
    channel: &C,
    now: impl Fn() -> Timestamp,
    stop: S,
) -> Preemption
where
    C: CheckpointChannel,
    S: FnOnce() -> Result<(), String>,
{
    let Request {
        task_id,
        supports_checkpoint,
        cgroup,
        permissions,
        deadline,
    } = request;

    // A worker with no checkpoint protocol is never asked. Asking costs the whole
    // deadline, can only end one way, and the request that triggered this is waiting.
    let outcome = if supports_checkpoint {
        let reply = channel.request(task_id, deadline);
        classify(reply, deadline, now())
    } else {
        CheckpointOutcome::TimedOut
    };

    if outcome.work_is_durable() && !outcome.needed_fallback() {
        // The cooperative path completed. The worker has saved its state and stood down;
        // there is nothing for the kernel to do.
        return Preemption {
            task_id,
            outcome,
            fallback: Fallback::NotNeeded,
        };
    }

    // Cooperation did not arrive, or did not arrive in time. Freeze if permitted and
    // possible, otherwise stop.
    let fallback = if permissions.allow_freeze {
        match cgroup {
            Some(path) => match cgroups.freeze(path) {
                Ok(()) => Fallback::Froze,
                // A freeze that could not be applied must not be reported as one. The
                // scheduler would believe the slot was reclaimed and start work on a node
                // where the old task is still running flat out.
                Err(CgroupRefusal::NoDelegation) => match stop() {
                    Ok(()) => Fallback::Stopped,
                    Err(detail) => Fallback::Failed(detail),
                },
                Err(e) => Fallback::Failed(e.to_string()),
            },
            None => match stop() {
                Ok(()) => Fallback::Stopped,
                Err(detail) => Fallback::Failed(detail),
            },
        }
    } else {
        match stop() {
            Ok(()) => Fallback::Stopped,
            Err(detail) => Fallback::Failed(detail),
        }
    };

    // Now that the task is stopped one way or another, see whether the worker answered
    // while we were doing it. A reply that landed at deadline + 1 ms named an artifact
    // that exists on disk; our attention span is not what makes it real, and knowing it
    // exists turns "killing this loses a week" into "killing this loses nothing".
    let outcome =
        if matches!(fallback, Fallback::Froze | Fallback::Stopped) && !outcome.work_is_durable() {
            match channel.collect_late(task_id) {
                Some(late) => classify(Some(late), deadline, now()),
                None => outcome,
            }
        } else {
            outcome
        };

    Preemption {
        task_id,
        outcome,
        fallback,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use archie_execution::spec::ResourceBudget;
    use archie_observability::kernel::cgroup::CgroupController;
    use std::cell::Cell;
    use std::path::PathBuf;

    struct Silent;
    impl CheckpointChannel for Silent {
        fn request(&self, _: TaskId, _: Timestamp) -> Option<CheckpointReply> {
            None
        }
    }

    struct Cooperative {
        artifact: String,
    }
    impl CheckpointChannel for Cooperative {
        fn request(&self, task_id: TaskId, _: Timestamp) -> Option<CheckpointReply> {
            Some(CheckpointReply::Checkpointed {
                task_id,
                artifact: self.artifact.clone(),
            })
        }
    }

    struct Refusing;
    impl CheckpointChannel for Refusing {
        fn request(&self, task_id: TaskId, _: Timestamp) -> Option<CheckpointReply> {
            Some(CheckpointReply::Failed {
                task_id,
                reason: "checkpoint volume is full".into(),
            })
        }
    }

    /// Silent until the deadline, then answers.
    struct LateAnswer {
        artifact: String,
    }
    impl CheckpointChannel for LateAnswer {
        fn request(&self, _: TaskId, _: Timestamp) -> Option<CheckpointReply> {
            None
        }
        fn collect_late(&self, task_id: TaskId) -> Option<CheckpointReply> {
            Some(CheckpointReply::Checkpointed {
                task_id,
                artifact: self.artifact.clone(),
            })
        }
    }

    fn fixture() -> (PathBuf, TaskCgroups, CgroupPath) {
        let root = std::env::temp_dir().join(format!(
            "archie-preempt-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("cgroup.controllers"), "cpu io memory\n").unwrap();

        let delegated = CgroupPath::new("system.slice/archie-executord.service").unwrap();
        std::fs::create_dir_all(delegated.resolve(&root)).unwrap();
        let cgroups = TaskCgroups::new(CgroupController::new(&root), delegated);

        let task = TaskId::new();
        let path = cgroups.create(&task, &ResourceBudget::default()).unwrap();
        std::fs::write(path.resolve(&root).join("cgroup.freeze"), "0").unwrap();
        (root, cgroups, path)
    }

    fn at(ms: i64) -> Timestamp {
        Timestamp::from_millis(ms)
    }

    #[test]
    fn a_cooperative_worker_never_reaches_the_kernel() {
        let (root, cgroups, path) = fixture();
        let result = preempt(
            Request {
                task_id: TaskId::new(),
                supports_checkpoint: true,
                cgroup: Some(&path),
                permissions: Permissions { allow_freeze: true },
                deadline: at(1_000),
            },
            &cgroups,
            &Cooperative {
                artifact: "/ckpt/a".into(),
            },
            || at(500),
            || panic!("must not stop a worker that checkpointed"),
        );

        assert_eq!(result.fallback, Fallback::NotNeeded);
        assert!(result.outcome.work_is_durable());
        assert!(result.slot_released());
        assert!(result.safe_to_kill());
        assert!(result.explain().contains("durable"));

        // And nothing was frozen.
        assert_eq!(
            std::fs::read_to_string(path.resolve(&root).join("cgroup.freeze")).unwrap(),
            "0"
        );
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn silence_past_the_deadline_freezes_the_task() {
        let (root, cgroups, path) = fixture();
        let result = preempt(
            Request {
                task_id: TaskId::new(),
                supports_checkpoint: true,
                cgroup: Some(&path),
                permissions: Permissions { allow_freeze: true },
                deadline: at(1_000),
            },
            &cgroups,
            &Silent,
            || at(1_500),
            || panic!("freezing was permitted and possible"),
        );

        assert_eq!(result.outcome, CheckpointOutcome::TimedOut);
        assert_eq!(result.fallback, Fallback::Froze);
        assert_eq!(
            std::fs::read_to_string(path.resolve(&root).join("cgroup.freeze")).unwrap(),
            "1"
        );

        assert!(
            !result.slot_released(),
            "a frozen task's memory is still resident and its cgroup still exists; \
             reporting the slot free would let the scheduler overcommit the node"
        );
        assert!(
            !result.safe_to_kill(),
            "killing it now loses everything since its last real checkpoint"
        );
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_late_reply_turns_a_costly_kill_into_a_free_one() {
        // The case the whole late-collection path exists for. The worker answered at
        // deadline + 1 ms, after the freezer already stopped it. The artifact is on
        // disk; our attention span is not what makes it real.
        let (root, cgroups, path) = fixture();
        let result = preempt(
            Request {
                task_id: TaskId::new(),
                supports_checkpoint: true,
                cgroup: Some(&path),
                permissions: Permissions { allow_freeze: true },
                deadline: at(1_000),
            },
            &cgroups,
            &LateAnswer {
                artifact: "/ckpt/late".into(),
            },
            || at(1_001),
            || panic!("freezing was possible"),
        );

        assert_eq!(result.fallback, Fallback::Froze);
        assert_eq!(
            result.outcome,
            CheckpointOutcome::DurableButLate {
                artifact: "/ckpt/late".into()
            }
        );
        assert!(
            result.safe_to_kill(),
            "the artifact exists, so killing the frozen task now loses nothing"
        );
        assert!(!result.slot_released(), "it is frozen, not gone");
        assert!(result.explain().contains("for nothing"));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_worker_that_cannot_checkpoint_is_never_asked() {
        // Asking costs the whole deadline and can only end one way.
        let (root, cgroups, path) = fixture();
        let asked = Cell::new(false);
        struct Counting<'a>(&'a Cell<bool>);
        impl CheckpointChannel for Counting<'_> {
            fn request(&self, _: TaskId, _: Timestamp) -> Option<CheckpointReply> {
                self.0.set(true);
                None
            }
        }

        let result = preempt(
            Request {
                task_id: TaskId::new(),
                supports_checkpoint: false,
                cgroup: Some(&path),
                permissions: Permissions { allow_freeze: true },
                deadline: at(1_000),
            },
            &cgroups,
            &Counting(&asked),
            || at(1_000),
            || panic!("freezing was possible"),
        );

        assert!(
            !asked.get(),
            "a worker with no checkpoint protocol must not be sent a request"
        );
        assert_eq!(result.fallback, Fallback::Froze);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_refusal_ends_the_wait_and_still_reclaims_the_slot() {
        let (root, cgroups, path) = fixture();
        let result = preempt(
            Request {
                task_id: TaskId::new(),
                supports_checkpoint: true,
                cgroup: Some(&path),
                permissions: Permissions { allow_freeze: true },
                deadline: at(5_000),
            },
            &cgroups,
            &Refusing,
            || at(100),
            || panic!("freezing was possible"),
        );

        match &result.outcome {
            CheckpointOutcome::Refused { reason } => assert!(reason.contains("full")),
            other => panic!("unexpected {other:?}"),
        }
        assert_eq!(result.fallback, Fallback::Froze);
        assert!(result
            .explain()
            .contains("survives only while the process does"));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn when_freezing_is_not_permitted_the_task_is_stopped() {
        let (root, cgroups, path) = fixture();
        let stopped = Cell::new(false);

        let result = preempt(
            Request {
                task_id: TaskId::new(),
                supports_checkpoint: true,
                cgroup: Some(&path),
                permissions: Permissions {
                    allow_freeze: false,
                },
                deadline: at(1_000),
            },
            &cgroups,
            &Silent,
            || at(2_000),
            || {
                stopped.set(true);
                Ok(())
            },
        );

        assert!(stopped.get());
        assert_eq!(result.fallback, Fallback::Stopped);
        assert!(
            result.slot_released(),
            "a stopped task genuinely released its slot"
        );
        assert!(!result.safe_to_kill());
        assert!(result
            .explain()
            .contains("work since its last checkpoint is lost"));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_node_without_delegation_stops_rather_than_claiming_a_freeze() {
        // The dangerous failure: reporting a freeze that did not happen would have the
        // scheduler believe the slot was reclaimed and start work on a node where the old
        // task is still running flat out.
        let root = std::env::temp_dir().join(format!("archie-nodeleg-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let cgroups = TaskCgroups::undelegated(CgroupController::new(&root));
        let path = CgroupPath::new("system.slice/x/task-1").unwrap();
        let stopped = Cell::new(false);

        let result = preempt(
            Request {
                task_id: TaskId::new(),
                supports_checkpoint: true,
                cgroup: Some(&path),
                permissions: Permissions { allow_freeze: true },
                deadline: at(1_000),
            },
            &cgroups,
            &Silent,
            || at(2_000),
            || {
                stopped.set(true);
                Ok(())
            },
        );

        assert!(stopped.get(), "it must fall back to stopping, not give up");
        assert_eq!(result.fallback, Fallback::Stopped);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_task_with_no_cgroup_is_stopped() {
        let (root, cgroups, _) = fixture();
        let stopped = Cell::new(false);

        let result = preempt(
            Request {
                task_id: TaskId::new(),
                supports_checkpoint: true,
                cgroup: None,
                permissions: Permissions { allow_freeze: true },
                deadline: at(1_000),
            },
            &cgroups,
            &Silent,
            || at(2_000),
            || {
                stopped.set(true);
                Ok(())
            },
        );

        assert!(stopped.get());
        assert_eq!(result.fallback, Fallback::Stopped);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_failure_to_reclaim_is_reported_rather_than_assumed() {
        let (root, cgroups, path) = fixture();
        let result = preempt(
            Request {
                task_id: TaskId::new(),
                supports_checkpoint: true,
                cgroup: Some(&path),
                permissions: Permissions {
                    allow_freeze: false,
                },
                deadline: at(1_000),
            },
            &cgroups,
            &Silent,
            || at(2_000),
            || Err("the process is unkillable in state D".into()),
        );

        match &result.fallback {
            Fallback::Failed(detail) => assert!(detail.contains("state D")),
            other => panic!("unexpected {other:?}"),
        }
        assert!(
            !result.slot_released(),
            "nothing was reclaimed, and saying otherwise would overcommit the node"
        );
        assert!(result.explain().contains("could not reclaim"));
        std::fs::remove_dir_all(&root).ok();
    }
}
