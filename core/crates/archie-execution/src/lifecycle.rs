//! The durable lifecycle of one task's execution.
//!
//! RFC-007 §7. This is the record that survives a restart of either daemon, so its states
//! have to describe not only what a task is doing but what we are *unsure* of.
//!
//! # Checkpointed and Frozen are not the same state
//!
//! It is tempting to collapse them: both mean "the task has stopped and its resources are
//! free". They are entirely different facts.
//!
//! A **checkpointed** task cooperated. It wrote its state somewhere and can be restarted
//! from that point, on this node or another one, after a reboot, tomorrow.
//!
//! A **frozen** task was stopped mid-instruction by the kernel because it did not answer
//! in time. Its state lives only in the memory of a process that is still there. It can
//! be thawed, and it can be killed — and killing it loses everything since its last real
//! checkpoint.
//!
//! Recording both as "paused" would make a week of idle research look recoverable when it
//! is one `SIGKILL` from gone.
//!
//! # Not-knowing is a state
//!
//! After a crash the executor has task records and a cgroup tree, and they can disagree.
//! [`ExecutionState::Lost`] and [`ExecutionState::Orphaned`] name the two ways they
//! disagree, because reconciliation that has to guess will eventually guess a duplicate
//! into existence.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Where one task's execution has got to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionState {
    /// Accepted by the executor, no process yet.
    Queued,
    /// The cgroup exists and the runner is being spawned. Brief, and the state a crash
    /// is most likely to be caught in — which is why it is distinct from Running.
    Starting,
    Running,
    /// A checkpoint has been asked for and the deadline is ticking.
    CheckpointRequested,
    /// The worker answered. Its state is durable and it can be restarted from here.
    Checkpointed,
    /// The kernel stopped it mid-instruction because the deadline expired. Its state is
    /// only in memory.
    Frozen,
    /// Being brought back from Checkpointed or Frozen.
    Resuming,

    // ---- terminal ----------------------------------------------------------
    Completed,
    Failed,
    /// Stopped on request, gracefully.
    Cancelled,
    /// Stopped with SIGKILL, either because it ignored SIGTERM or because policy said to.
    Killed,

    // ---- reconciliation ----------------------------------------------------
    /// We hold a record saying it was running, and there is no process. It died while we
    /// were not watching, and we do not know how.
    Lost,
    /// There is a process in a task cgroup we hold no live record for. The reverse
    /// disagreement, and the more dangerous one: starting the task again would run it
    /// twice.
    Orphaned,
    /// Reconciliation is in progress. Not a resting state; a task here has a decision
    /// pending.
    Recovering,
}

impl ExecutionState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Starting => "starting",
            Self::Running => "running",
            Self::CheckpointRequested => "checkpoint_requested",
            Self::Checkpointed => "checkpointed",
            Self::Frozen => "frozen",
            Self::Resuming => "resuming",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Killed => "killed",
            Self::Lost => "lost",
            Self::Orphaned => "orphaned",
            Self::Recovering => "recovering",
        }
    }

    /// Terminal states never transition again.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Failed | Self::Cancelled | Self::Killed
        )
    }

    /// Whether a process should exist for a task in this state.
    ///
    /// Used by reconciliation: a state where a process should exist and none does is
    /// [`Self::Lost`], and the check has to be explicit rather than a guess about which
    /// states "look running".
    pub fn expects_process(&self) -> bool {
        matches!(
            self,
            Self::Starting
                | Self::Running
                | Self::CheckpointRequested
                | Self::Frozen
                | Self::Resuming
        )
    }

    /// Whether the task currently holds a scheduler slot.
    ///
    /// A frozen task still holds one: its memory is resident and its cgroup exists. This
    /// is the distinction that stops a preemption from appearing to free capacity it has
    /// not actually freed.
    pub fn holds_slot(&self) -> bool {
        self.expects_process()
    }

    /// Whether work done so far survives being killed from this state.
    ///
    /// The whole reason Checkpointed and Frozen are separate.
    pub fn survives_kill(&self) -> bool {
        matches!(self, Self::Checkpointed | Self::Completed)
    }

    /// Whether this state needs an operator or a policy decision rather than a timer.
    pub fn needs_decision(&self) -> bool {
        matches!(self, Self::Orphaned | Self::Lost | Self::Recovering)
    }
}

impl fmt::Display for ExecutionState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What caused a transition. Recorded so an audit can distinguish a task that finished
/// from one that was stopped, and a stop that was asked for from one the kernel imposed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Cause {
    /// The scheduler decided.
    Scheduler,
    /// The worker itself — it exited, or answered a checkpoint request.
    Worker,
    /// The kernel — an OOM kill, a freeze, a signal.
    Kernel,
    /// A human.
    Operator,
    /// A deadline expired.
    Deadline,
    /// Reconciliation after a restart.
    Reconciliation,
}

/// Why a transition was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransitionRefusal {
    /// The task has already finished.
    AlreadyTerminal(ExecutionState),
    /// The target does not follow from the current state.
    NotReachable {
        from: ExecutionState,
        to: ExecutionState,
    },
    /// A checkpoint was claimed for a worker that never advertised support for one.
    CheckpointNotSupported,
    /// Something other than the kernel claimed a task was frozen.
    OnlyKernelFreezes,
}

impl fmt::Display for TransitionRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyTerminal(state) => write!(
                f,
                "task already finished as {state}; a terminal state is a fact, not a \
                 stage"
            ),
            Self::NotReachable { from, to } => {
                write!(f, "{to} does not follow from {from}")
            }
            Self::CheckpointNotSupported => f.write_str(
                "a checkpoint was recorded for a worker that never advertised the \
                 protocol; the artifact would not exist",
            ),
            Self::OnlyKernelFreezes => f.write_str(
                "only the kernel freezes a cgroup; recording a freeze from any other \
                 cause would claim durability the task does not have",
            ),
        }
    }
}

impl std::error::Error for TransitionRefusal {}

/// A proposed change of execution state.
#[derive(Debug, Clone, PartialEq)]
pub struct Transition {
    pub from: ExecutionState,
    pub to: ExecutionState,
    pub cause: Cause,
    /// Whether the worker advertised checkpoint support. Needed because a checkpoint
    /// recorded for a worker that cannot produce one leaves a reference to an artifact
    /// that does not exist.
    pub supports_checkpoint: bool,
}

/// Whether a transition is permitted, and if not, why.
pub fn refuse(t: &Transition) -> Option<TransitionRefusal> {
    use ExecutionState as S;

    if t.from.is_terminal() {
        return Some(TransitionRefusal::AlreadyTerminal(t.from));
    }

    // Only the kernel freezes. A freeze recorded from any other cause would be an
    // executor claiming a process is stopped when it is merely asked to stop, and the
    // distinction is exactly what the freezer exists to provide.
    if t.to == S::Frozen && t.cause != Cause::Kernel {
        return Some(TransitionRefusal::OnlyKernelFreezes);
    }

    // A checkpoint that cannot have happened must not be recorded as if it had.
    if matches!(t.to, S::CheckpointRequested | S::Checkpointed) && !t.supports_checkpoint {
        return Some(TransitionRefusal::CheckpointNotSupported);
    }

    let reachable = match t.from {
        // Anything live can fail, be cancelled, be killed, or be found to have vanished.
        _ if matches!(t.to, S::Failed | S::Cancelled | S::Killed | S::Lost) => true,

        S::Queued => matches!(t.to, S::Starting),
        S::Starting => matches!(t.to, S::Running),
        S::Running => matches!(t.to, S::CheckpointRequested | S::Frozen | S::Completed),
        // The deadline path: asked to checkpoint, and either it answers or the kernel
        // stops it.
        S::CheckpointRequested => matches!(t.to, S::Checkpointed | S::Frozen | S::Running),
        S::Checkpointed => matches!(t.to, S::Resuming | S::Completed),
        S::Frozen => matches!(t.to, S::Resuming | S::Recovering),
        S::Resuming => matches!(t.to, S::Running),

        // Reconciliation.
        S::Lost => matches!(t.to, S::Recovering),
        S::Orphaned => matches!(t.to, S::Recovering | S::Running),
        S::Recovering => matches!(t.to, S::Running | S::Queued | S::Checkpointed),

        S::Completed | S::Failed | S::Cancelled | S::Killed => false,
    };

    if reachable {
        None
    } else {
        Some(TransitionRefusal::NotReachable {
            from: t.from,
            to: t.to,
        })
    }
}

pub fn permitted(t: &Transition) -> bool {
    refuse(t).is_none()
}

/// What reconciliation found when comparing a stored record against the live system.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reconciliation {
    /// Record and reality agree.
    Consistent,
    /// We expected a process and found none.
    ProcessMissing,
    /// We found a process we hold no live record for. Starting the task again would run
    /// it twice, so this is the case that must never be resolved by guessing.
    ProcessUnexpected,
    /// The record is terminal and a process is still there — it outlived its own record.
    SurvivedItsRecord,
}

/// Compare a stored state against what the system actually shows.
///
/// `process_present` is whatever the executor could determine: a live pid in the task's
/// cgroup, a non-empty `cgroup.procs`. The point of separating this from the state
/// machine is that reconciliation is a *comparison*, and a comparison that silently
/// picks a side is how a task gets started twice.
pub fn reconcile(recorded: ExecutionState, process_present: bool) -> Reconciliation {
    if recorded.is_terminal() {
        return if process_present {
            Reconciliation::SurvivedItsRecord
        } else {
            Reconciliation::Consistent
        };
    }
    match (recorded.expects_process(), process_present) {
        (true, true) => Reconciliation::Consistent,
        (true, false) => Reconciliation::ProcessMissing,
        (false, true) => Reconciliation::ProcessUnexpected,
        (false, false) => Reconciliation::Consistent,
    }
}

/// The state a reconciliation result implies.
///
/// Never a terminal state: reconciliation records what it found and hands the decision to
/// policy. Concluding `Failed` from a missing process would erase the difference between
/// a task that failed and a task nobody was watching.
pub fn state_after_reconcile(recorded: ExecutionState, finding: Reconciliation) -> ExecutionState {
    match finding {
        Reconciliation::Consistent => recorded,
        Reconciliation::ProcessMissing => ExecutionState::Lost,
        Reconciliation::ProcessUnexpected | Reconciliation::SurvivedItsRecord => {
            ExecutionState::Orphaned
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ExecutionState as S;

    fn t(from: S, to: S, cause: Cause) -> Transition {
        Transition {
            from,
            to,
            cause,
            supports_checkpoint: true,
        }
    }

    #[test]
    fn the_happy_path_runs_start_to_finish() {
        assert!(permitted(&t(S::Queued, S::Starting, Cause::Scheduler)));
        assert!(permitted(&t(S::Starting, S::Running, Cause::Worker)));
        assert!(permitted(&t(S::Running, S::Completed, Cause::Worker)));
    }

    #[test]
    fn a_frozen_task_is_not_a_checkpointed_one() {
        // The distinction the whole module exists for. Both stop the task; only one
        // leaves work that survives being killed.
        assert!(S::Checkpointed.survives_kill());
        assert!(
            !S::Frozen.survives_kill(),
            "a frozen task's state lives only in the memory of a process that is one \
             SIGKILL from gone"
        );

        // And a frozen task still holds its slot, because its memory is still resident.
        assert!(S::Frozen.holds_slot());
        assert!(
            !S::Checkpointed.holds_slot(),
            "a checkpointed task released its resources; that is the point of asking"
        );
    }

    #[test]
    fn only_the_kernel_may_record_a_freeze() {
        for cause in [
            Cause::Scheduler,
            Cause::Worker,
            Cause::Operator,
            Cause::Deadline,
            Cause::Reconciliation,
        ] {
            let refusal = refuse(&t(S::Running, S::Frozen, cause)).unwrap();
            assert_eq!(refusal, TransitionRefusal::OnlyKernelFreezes);
        }
        assert!(permitted(&t(S::Running, S::Frozen, Cause::Kernel)));
    }

    #[test]
    fn a_checkpoint_cannot_be_recorded_for_a_worker_that_cannot_produce_one() {
        // Otherwise the record holds a reference to an artifact that does not exist, and
        // a resume from it fails much later for a reason nobody can trace.
        let mut transition = t(S::Running, S::CheckpointRequested, Cause::Scheduler);
        transition.supports_checkpoint = false;
        assert_eq!(
            refuse(&transition),
            Some(TransitionRefusal::CheckpointNotSupported)
        );

        let mut done = t(S::CheckpointRequested, S::Checkpointed, Cause::Worker);
        done.supports_checkpoint = false;
        assert_eq!(
            refuse(&done),
            Some(TransitionRefusal::CheckpointNotSupported)
        );
    }

    #[test]
    fn the_preemption_deadline_has_both_outcomes() {
        // Worker cooperates.
        assert!(permitted(&t(
            S::CheckpointRequested,
            S::Checkpointed,
            Cause::Worker
        )));
        // Worker does not, and the kernel stops it.
        assert!(permitted(&t(
            S::CheckpointRequested,
            S::Frozen,
            Cause::Kernel
        )));
        // Or the preemption is called off and it simply carries on.
        assert!(permitted(&t(
            S::CheckpointRequested,
            S::Running,
            Cause::Scheduler
        )));
    }

    #[test]
    fn a_finished_task_never_moves_again() {
        for terminal in [S::Completed, S::Failed, S::Cancelled, S::Killed] {
            assert!(terminal.is_terminal());
            let refusal = refuse(&t(terminal, S::Running, Cause::Operator)).unwrap();
            assert_eq!(refusal, TransitionRefusal::AlreadyTerminal(terminal));
        }
    }

    #[test]
    fn anything_live_can_be_killed_or_lost() {
        for live in [
            S::Queued,
            S::Starting,
            S::Running,
            S::CheckpointRequested,
            S::Checkpointed,
            S::Frozen,
            S::Resuming,
        ] {
            assert!(permitted(&t(live, S::Killed, Cause::Operator)), "{live}");
            assert!(permitted(&t(live, S::Cancelled, Cause::Operator)), "{live}");
            assert!(
                permitted(&t(live, S::Lost, Cause::Reconciliation)),
                "{live}"
            );
        }
    }

    #[test]
    fn a_frozen_task_cannot_go_straight_back_to_running() {
        // Thawing is its own step, and a task that skipped it would be marked running
        // while still frozen — the scheduler would then wait forever for output.
        assert_eq!(
            refuse(&t(S::Frozen, S::Running, Cause::Scheduler)),
            Some(TransitionRefusal::NotReachable {
                from: S::Frozen,
                to: S::Running
            })
        );
        assert!(permitted(&t(S::Frozen, S::Resuming, Cause::Scheduler)));
        assert!(permitted(&t(S::Resuming, S::Running, Cause::Worker)));
    }

    // ---- reconciliation --------------------------------------------------------

    #[test]
    fn a_record_that_expects_a_process_and_finds_none_is_lost_not_failed() {
        // Concluding Failed would erase the difference between a task that failed and a
        // task nobody was watching, and the second is a bug in us.
        let finding = reconcile(S::Running, false);
        assert_eq!(finding, Reconciliation::ProcessMissing);
        assert_eq!(state_after_reconcile(S::Running, finding), S::Lost);
        assert!(!S::Lost.is_terminal());
        assert!(S::Lost.needs_decision());
    }

    #[test]
    fn a_process_with_no_live_record_is_orphaned_and_must_not_be_restarted() {
        // The dangerous direction: starting the task again would run it twice.
        let finding = reconcile(S::Queued, true);
        assert_eq!(finding, Reconciliation::ProcessUnexpected);
        assert_eq!(state_after_reconcile(S::Queued, finding), S::Orphaned);
        assert!(S::Orphaned.needs_decision());
    }

    #[test]
    fn a_task_that_outlived_its_terminal_record_is_orphaned() {
        // The executor died between the worker exiting and the record being written, and
        // a new worker with the same identity started. Rare, and silently wrong if
        // treated as consistent.
        let finding = reconcile(S::Completed, true);
        assert_eq!(finding, Reconciliation::SurvivedItsRecord);
        assert_eq!(state_after_reconcile(S::Completed, finding), S::Orphaned);
    }

    #[test]
    fn agreement_in_either_direction_is_consistent() {
        assert_eq!(reconcile(S::Running, true), Reconciliation::Consistent);
        assert_eq!(reconcile(S::Queued, false), Reconciliation::Consistent);
        assert_eq!(reconcile(S::Completed, false), Reconciliation::Consistent);
        assert_eq!(
            state_after_reconcile(S::Running, Reconciliation::Consistent),
            S::Running
        );
    }

    #[test]
    fn a_frozen_task_surviving_a_restart_is_consistent() {
        // Freezing survives the executor dying: the cgroup and its frozen process are
        // still there. Treating that as Lost would kill recoverable work.
        assert!(S::Frozen.expects_process());
        assert_eq!(reconcile(S::Frozen, true), Reconciliation::Consistent);
    }

    #[test]
    fn reconciliation_never_concludes_a_terminal_state() {
        for recorded in [S::Queued, S::Starting, S::Running, S::Frozen, S::Resuming] {
            for present in [true, false] {
                let finding = reconcile(recorded, present);
                let next = state_after_reconcile(recorded, finding);
                assert!(
                    !next.is_terminal(),
                    "reconciliation records what it found and hands the decision to \
                     policy; concluding {next} from a comparison would be a guess"
                );
            }
        }
    }

    #[test]
    fn recovery_leads_back_to_a_live_state_not_out_of_the_machine() {
        assert!(permitted(&t(S::Lost, S::Recovering, Cause::Reconciliation)));
        assert!(permitted(&t(
            S::Orphaned,
            S::Recovering,
            Cause::Reconciliation
        )));
        assert!(permitted(&t(
            S::Recovering,
            S::Running,
            Cause::Reconciliation
        )));
        assert!(permitted(&t(S::Recovering, S::Queued, Cause::Scheduler)));
        // An orphan that turns out to be exactly the task we meant to run is adopted
        // rather than started again.
        assert!(permitted(&t(
            S::Orphaned,
            S::Running,
            Cause::Reconciliation
        )));
    }

    #[test]
    fn a_frozen_task_holds_its_slot_until_it_is_actually_released() {
        // A preemption that appears to free capacity it has not freed would let the
        // scheduler overcommit the node.
        for holding in [
            S::Starting,
            S::Running,
            S::CheckpointRequested,
            S::Frozen,
            S::Resuming,
        ] {
            assert!(holding.holds_slot(), "{holding}");
        }
        for released in [S::Queued, S::Checkpointed, S::Completed, S::Killed] {
            assert!(!released.holds_slot(), "{released}");
        }
    }

    #[test]
    fn states_survive_serialization_by_name() {
        for state in [S::Running, S::CheckpointRequested, S::Frozen, S::Orphaned] {
            let json = serde_json::to_string(&state).unwrap();
            assert_eq!(json, format!("\"{}\"", state.as_str()));
            assert_eq!(
                serde_json::from_str::<ExecutionState>(&json).unwrap(),
                state
            );
        }
    }
}
