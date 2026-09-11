//! The supervisor: one task, one process, one durable record.
//!
//! RFC-007 §7, step 2. This is the piece that turns `SchedulingAction::Start` into
//! something that actually runs.
//!
//! # Starting twice is the failure to design against
//!
//! Everything else here is recoverable. A worker that fails reports a reason; one that is
//! lost gets reconciled. A task that runs *twice* has done its side effects twice, and no
//! amount of later bookkeeping undoes that.
//!
//! So [`Supervisor::start`] is idempotent on task id, and the interesting case is not the
//! obvious one. Rejecting a second start while the first is running is easy. The case
//! that matters is a second start arriving after `archied` restarted and lost its memory
//! of the first — which is exactly the situation in which a naive supervisor, asked
//! politely, runs the task again.

use archie_execution::lifecycle::{self, Cause, ExecutionState, Reconciliation, Transition};
use archie_execution::{SandboxStrength, SpecError, TaskExecutionSpec};
use archie_types::{TaskId, Timestamp};
use std::collections::BTreeMap;

use crate::launcher::{Child, Exit, LaunchError, Launcher};

/// Everything the executor knows about one task.
#[derive(Debug, Clone)]
pub struct TaskRecord {
    pub task_id: TaskId,
    pub state: ExecutionState,
    pub child: Option<Child>,
    pub started_at: Option<Timestamp>,
    pub ended_at: Option<Timestamp>,
    pub exit: Option<Exit>,
    /// Why it ended, in words an operator can act on.
    pub failure_reason: Option<String>,
    /// Every transition, for the audit trail.
    pub history: Vec<(ExecutionState, Cause, Timestamp)>,
    supports_checkpoint: bool,
}

impl TaskRecord {
    /// Whether this worker advertised the checkpoint protocol.
    ///
    /// Exposed because preemption planning depends on it, and that decision lives in the
    /// dispatcher: asking a worker that cannot answer costs the whole deadline for
    /// nothing.
    pub fn supports_checkpoint(&self) -> bool {
        self.supports_checkpoint
    }

    fn new(spec: &TaskExecutionSpec, now: Timestamp) -> Self {
        Self {
            task_id: spec.task_id,
            state: ExecutionState::Queued,
            child: None,
            started_at: None,
            ended_at: None,
            exit: None,
            failure_reason: None,
            history: vec![(ExecutionState::Queued, Cause::Scheduler, now)],
            supports_checkpoint: spec.supports_checkpoint,
        }
    }

    /// Apply a transition, refusing it if the lifecycle does not allow it.
    fn transition(
        &mut self,
        to: ExecutionState,
        cause: Cause,
        now: Timestamp,
    ) -> Result<(), lifecycle::TransitionRefusal> {
        let proposed = Transition {
            from: self.state,
            to,
            cause,
            supports_checkpoint: self.supports_checkpoint,
        };
        if let Some(refusal) = lifecycle::refuse(&proposed) {
            return Err(refusal);
        }
        self.state = to;
        self.history.push((to, cause, now));
        Ok(())
    }
}

/// Why a start was refused.
#[derive(Debug, Clone, PartialEq)]
pub enum StartRefusal {
    /// This task is already running here. The start is a no-op, not an error — a
    /// scheduler that retries after a timeout is behaving correctly, and the safe answer
    /// to "start this" when it is already started is "it is started".
    AlreadyRunning,
    /// It ran and finished. Running it again would repeat its side effects.
    AlreadyFinished(ExecutionState),
    /// The spec is malformed.
    InvalidSpec(SpecError),
    /// This node cannot provide the enforcement the task requires.
    NotAdmissible(SpecError),
    /// The OS refused.
    LaunchFailed(LaunchError),
}

impl std::fmt::Display for StartRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AlreadyRunning => f.write_str("task is already running on this node"),
            Self::AlreadyFinished(state) => {
                write!(
                    f,
                    "task already finished as {state}; starting it again would \
                          repeat its side effects"
                )
            }
            Self::InvalidSpec(e) => write!(f, "invalid execution spec: {e}"),
            Self::NotAdmissible(e) => write!(f, "{e}"),
            Self::LaunchFailed(e) => write!(f, "could not start worker: {e}"),
        }
    }
}

impl std::error::Error for StartRefusal {}

/// What a completed task produced.
#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    pub task_id: TaskId,
    pub state: ExecutionState,
    pub exit: Option<Exit>,
    pub failure_reason: Option<String>,
}

/// Supervises tasks on one node.
pub struct Supervisor<L: Launcher> {
    launcher: L,
    strength: SandboxStrength,
    tasks: BTreeMap<TaskId, TaskRecord>,
}

impl<L: Launcher> Supervisor<L> {
    pub fn new(launcher: L, strength: SandboxStrength) -> Self {
        Self {
            launcher,
            strength,
            tasks: BTreeMap::new(),
        }
    }

    /// What this node can actually enforce.
    pub fn strength(&self) -> &SandboxStrength {
        &self.strength
    }

    pub fn record(&self, task_id: &TaskId) -> Option<&TaskRecord> {
        self.tasks.get(task_id)
    }

    /// Every task the executor knows about, running or finished.
    pub fn all_records(&self) -> impl Iterator<Item = &TaskRecord> {
        self.tasks.values()
    }

    /// The launcher, for tests in sibling modules that need to script its answers.
    #[cfg(test)]
    pub(crate) fn launcher_for_test(&self) -> &L {
        &self.launcher
    }

    /// Tasks currently holding a slot. What `archied` rebuilds its scheduler state from
    /// after a restart, rather than assuming everything died with it.
    pub fn running(&self) -> Vec<&TaskRecord> {
        self.tasks
            .values()
            .filter(|r| r.state.holds_slot())
            .collect()
    }

    /// Start a task.
    ///
    /// Idempotent on task id: a second start for something already running returns
    /// `AlreadyRunning` and changes nothing. That is not defensive tidiness — a scheduler
    /// retrying after a timeout is behaving correctly, and the supervisor is the only
    /// component positioned to know the first attempt succeeded.
    pub fn start(
        &mut self,
        spec: &TaskExecutionSpec,
        cgroup_procs: Option<&str>,
        now: Timestamp,
    ) -> Result<Child, StartRefusal> {
        if let Some(existing) = self.tasks.get(&spec.task_id) {
            if existing.state.is_terminal() {
                return Err(StartRefusal::AlreadyFinished(existing.state));
            }
            if existing.state.holds_slot() {
                return Err(StartRefusal::AlreadyRunning);
            }
        }

        // Validity before admissibility, so a malformed spec is reported as malformed
        // rather than as a node capability problem.
        if let Some(problem) = spec.problems().into_iter().next() {
            return Err(StartRefusal::InvalidSpec(problem));
        }
        if let Some(shortfall) = self.strength.shortfall(&spec.required_enforcement) {
            return Err(StartRefusal::NotAdmissible(shortfall));
        }

        let record = self
            .tasks
            .entry(spec.task_id)
            .or_insert_with(|| TaskRecord::new(spec, now));

        // Starting is its own state. A crash between here and Running leaves a record
        // saying so, which is what lets reconciliation tell "never started" from "started
        // and vanished" — and those call for opposite responses.
        let _ = record.transition(ExecutionState::Starting, Cause::Scheduler, now);

        match self.launcher.launch(spec, cgroup_procs) {
            Ok(child) => {
                let record = self.tasks.get_mut(&spec.task_id).expect("just inserted");
                record.child = Some(child);
                record.started_at = Some(now);
                let _ = record.transition(ExecutionState::Running, Cause::Worker, now);
                Ok(child)
            }
            Err(error) => {
                let record = self.tasks.get_mut(&spec.task_id).expect("just inserted");
                record.failure_reason = Some(error.to_string());
                record.ended_at = Some(now);
                let _ = record.transition(ExecutionState::Failed, Cause::Kernel, now);
                Err(StartRefusal::LaunchFailed(error))
            }
        }
    }

    /// Collect any tasks that have finished since the last poll.
    ///
    /// Returns outcomes rather than mutating a scheduler directly: the executor's job is
    /// to know what happened, and deciding what that means is the control plane's.
    pub fn reap(&mut self, now: Timestamp) -> Vec<Outcome> {
        let mut finished = Vec::new();

        for record in self.tasks.values_mut() {
            if !record.state.expects_process() {
                continue;
            }
            let Some(child) = record.child else {
                continue;
            };
            let Some(exit) = self.launcher.try_wait(child) else {
                continue;
            };

            let (state, reason) = if exit.is_success() {
                (ExecutionState::Completed, None)
            } else {
                // A signal and a non-zero code are different failures, and the message
                // says which — a worker that was OOM-killed is the node's problem, and
                // one that returned 1 is the worker's.
                let reason = match exit {
                    Exit::Signal(sig) => format!(
                        "worker was killed by signal {sig}; this was done to it rather \
                         than chosen by it"
                    ),
                    Exit::Code(code) => format!("worker exited with status {code}"),
                };
                (ExecutionState::Failed, Some(reason))
            };

            record.exit = Some(exit.clone());
            record.ended_at = Some(now);
            record.failure_reason = reason.clone();
            let _ = record.transition(state, Cause::Worker, now);

            finished.push(Outcome {
                task_id: record.task_id,
                state,
                exit: Some(exit),
                failure_reason: reason,
            });
        }

        finished
    }

    /// Compare every record against the live system.
    ///
    /// Run on executor startup and after any suspected loss. Records what it found and
    /// makes no policy decision: a missing process becomes `Lost`, never `Failed`,
    /// because concluding failure from absence erases the difference between a task that
    /// failed and a task nobody was watching.
    pub fn reconcile(&mut self, now: Timestamp) -> Vec<(TaskId, Reconciliation)> {
        let mut findings = Vec::new();

        for record in self.tasks.values_mut() {
            let present = record
                .child
                .is_some_and(|child| self.launcher.is_alive(child));
            let finding = lifecycle::reconcile(record.state, present);

            if finding != Reconciliation::Consistent {
                let next = lifecycle::state_after_reconcile(record.state, finding);
                // Reconciliation is its own cause, so an audit can tell a task that was
                // marked lost by a comparison from one that reported its own failure.
                let _ = record.transition(next, Cause::Reconciliation, now);
            }
            findings.push((record.task_id, finding));
        }

        findings
    }

    /// Record that the kernel froze this task.
    ///
    /// Separate from `stop` because a frozen task has not stopped: its memory is
    /// resident, its cgroup exists, and it still holds its slot. Only `Cause::Kernel` may
    /// write this state, and the dispatcher calls it at exactly the moment that is
    /// literally what happened.
    pub fn mark_frozen(&mut self, task_id: &TaskId, now: Timestamp) {
        if let Some(record) = self.tasks.get_mut(task_id) {
            let _ = record.transition(ExecutionState::Frozen, Cause::Kernel, now);
        }
    }

    /// Record that a frozen task was thawed and is running again.
    ///
    /// Goes through `Resuming` rather than straight to `Running`, because the lifecycle
    /// refuses the shortcut: a task marked running while still frozen would have the
    /// scheduler waiting forever for output it cannot produce.
    pub fn mark_thawed(&mut self, task_id: &TaskId, now: Timestamp) {
        if let Some(record) = self.tasks.get_mut(task_id) {
            let _ = record.transition(ExecutionState::Resuming, Cause::Scheduler, now);
            let _ = record.transition(ExecutionState::Running, Cause::Worker, now);
        }
    }

    /// Ask a task to stop, gracefully.
    pub fn stop(&mut self, task_id: &TaskId, now: Timestamp) -> Result<(), StartRefusal> {
        let Some(record) = self.tasks.get_mut(task_id) else {
            return Ok(());
        };
        let Some(child) = record.child else {
            return Ok(());
        };
        if record.state.is_terminal() {
            return Ok(());
        }

        self.launcher
            .terminate(child)
            .map_err(StartRefusal::LaunchFailed)?;
        let _ = record.transition(ExecutionState::Cancelled, Cause::Operator, now);
        record.ended_at = Some(now);
        Ok(())
    }

    /// Stop a task whether it cooperates or not.
    pub fn kill(&mut self, task_id: &TaskId, now: Timestamp) -> Result<(), StartRefusal> {
        let Some(record) = self.tasks.get_mut(task_id) else {
            return Ok(());
        };
        let Some(child) = record.child else {
            return Ok(());
        };
        if record.state.is_terminal() {
            return Ok(());
        }

        // Recorded before the kill so a crash between the two leaves a record that says
        // a kill was intended, which reconciliation can act on. The reverse order would
        // leave a dead process and a record claiming it was running.
        let survives = record.state.survives_kill();
        self.launcher
            .kill(child)
            .map_err(StartRefusal::LaunchFailed)?;
        let _ = record.transition(ExecutionState::Killed, Cause::Operator, now);
        record.ended_at = Some(now);
        record.failure_reason = Some(if survives {
            "killed after checkpointing; work up to the checkpoint survives".into()
        } else {
            "killed without a checkpoint; work since the last one is lost".into()
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::launcher::testing::FakeLauncher;
    use archie_execution::{
        FilesystemGrant, RequiredEnforcement, ResolvedExecutable, ResourceBudget, RestartPolicy,
    };
    use archie_types::{ModuleId, QueueClass, TraceId};
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

    fn supervisor() -> Supervisor<FakeLauncher> {
        Supervisor::new(FakeLauncher::new(), SandboxStrength::none())
    }

    #[test]
    fn start_creates_a_child_and_records_it_running() {
        let mut sup = supervisor();
        let spec = spec();
        let now = Timestamp::now();

        let child = sup.start(&spec, None, now).unwrap();
        assert!(child.pid > 0);

        let record = sup.record(&spec.task_id).unwrap();
        assert_eq!(record.state, ExecutionState::Running);
        assert_eq!(record.child, Some(child));
        assert!(record.started_at.is_some());
        assert_eq!(sup.running().len(), 1);

        // The record passed through Starting, which is what lets reconciliation later
        // tell "never started" from "started and vanished".
        let states: Vec<_> = record.history.iter().map(|(s, _, _)| *s).collect();
        assert_eq!(
            states,
            vec![
                ExecutionState::Queued,
                ExecutionState::Starting,
                ExecutionState::Running
            ]
        );
    }

    #[test]
    fn starting_the_same_task_twice_does_not_run_it_twice() {
        // The failure everything else is designed around: a task that runs twice has
        // done its side effects twice, and no bookkeeping undoes that.
        let mut sup = supervisor();
        let spec = spec();
        let now = Timestamp::now();

        sup.start(&spec, None, now).unwrap();
        let second = sup.start(&spec, None, now);
        assert_eq!(second.unwrap_err(), StartRefusal::AlreadyRunning);
        assert_eq!(sup.launcher.launched.lock().unwrap().len(), 1);
    }

    #[test]
    fn a_finished_task_is_not_silently_restarted() {
        let mut sup = supervisor();
        let spec = spec();
        let now = Timestamp::now();

        let child = sup.start(&spec, None, now).unwrap();
        sup.launcher.finish(child, Exit::Code(0));
        sup.reap(now);

        let again = sup.start(&spec, None, now).unwrap_err();
        assert_eq!(
            again,
            StartRefusal::AlreadyFinished(ExecutionState::Completed)
        );
        assert!(again.to_string().contains("side effects"));
    }

    #[test]
    fn a_failed_launch_reports_the_exact_reason() {
        // "Failed to start" sends someone to read logs; "the program is not installed"
        // sends them to install it.
        let mut sup = supervisor();
        let spec = spec();
        sup.launcher.fail_next_launch(LaunchError::ProgramNotFound(
            "/usr/lib/archie/workers/crawler".into(),
        ));

        let err = sup.start(&spec, None, Timestamp::now()).unwrap_err();
        match &err {
            StartRefusal::LaunchFailed(LaunchError::ProgramNotFound(path)) => {
                assert!(path.contains("crawler"));
            }
            other => panic!("unexpected {other:?}"),
        }
        assert!(err.to_string().contains("not installed"));

        let record = sup.record(&spec.task_id).unwrap();
        assert_eq!(record.state, ExecutionState::Failed);
        assert!(record
            .failure_reason
            .as_ref()
            .unwrap()
            .contains("not installed"));
    }

    #[test]
    fn a_node_that_cannot_enforce_refuses_before_launching_anything() {
        let mut sup = supervisor();
        let mut spec = spec();
        spec.required_enforcement = RequiredEnforcement::untrusted();

        let err = sup.start(&spec, None, Timestamp::now()).unwrap_err();
        assert!(matches!(err, StartRefusal::NotAdmissible(_)));
        assert!(
            sup.launcher.launched.lock().unwrap().is_empty(),
            "nothing may be spawned before admissibility is settled"
        );
    }

    #[test]
    fn a_malformed_spec_is_reported_as_malformed_not_as_a_node_problem() {
        let mut sup = supervisor();
        let mut spec = spec();
        spec.working_directory = PathBuf::from("relative");

        let err = sup.start(&spec, None, Timestamp::now()).unwrap_err();
        assert!(
            matches!(err, StartRefusal::InvalidSpec(_)),
            "otherwise an operator goes looking at node capabilities for a typo"
        );
    }

    #[test]
    fn completion_updates_the_record_and_frees_the_slot() {
        let mut sup = supervisor();
        let spec = spec();
        let now = Timestamp::now();

        let child = sup.start(&spec, None, now).unwrap();
        assert!(sup.reap(now).is_empty(), "still running");

        sup.launcher.finish(child, Exit::Code(0));
        let outcomes = sup.reap(now);
        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0].state, ExecutionState::Completed);
        assert_eq!(outcomes[0].task_id, spec.task_id);
        assert!(outcomes[0].failure_reason.is_none());
        assert!(sup.running().is_empty());

        // Reaping again reports nothing: an outcome is delivered once.
        assert!(sup.reap(now).is_empty());
    }

    #[test]
    fn a_signal_and_a_nonzero_exit_are_different_failures() {
        // One was done to the worker, the other chosen by it, and only the first is the
        // node's problem.
        let mut sup = supervisor();

        let killed_spec = spec();
        let child = sup.start(&killed_spec, None, Timestamp::now()).unwrap();
        sup.launcher.finish(child, Exit::Signal(9));
        let outcomes = sup.reap(Timestamp::now());
        assert_eq!(outcomes[0].state, ExecutionState::Failed);
        let reason = outcomes[0].failure_reason.as_ref().unwrap();
        assert!(reason.contains("signal 9"));
        assert!(reason.contains("rather than chosen by it"));

        let failed_spec = spec();
        let child = sup.start(&failed_spec, None, Timestamp::now()).unwrap();
        sup.launcher.finish(child, Exit::Code(2));
        let outcomes = sup.reap(Timestamp::now());
        assert!(outcomes[0]
            .failure_reason
            .as_ref()
            .unwrap()
            .contains("status 2"));

        assert!(!Exit::Signal(0).is_success());
        assert!(Exit::Code(0).is_success());
    }

    #[test]
    fn a_worker_that_vanished_is_lost_not_failed() {
        // Concluding failure from absence erases the difference between a task that
        // failed and a task nobody was watching. The second is a bug in us.
        let mut sup = supervisor();
        let spec = spec();
        let now = Timestamp::now();

        let child = sup.start(&spec, None, now).unwrap();
        sup.launcher.vanish(child);

        let findings = sup.reconcile(now);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].1, Reconciliation::ProcessMissing);

        let record = sup.record(&spec.task_id).unwrap();
        assert_eq!(record.state, ExecutionState::Lost);
        assert!(!record.state.is_terminal());

        // And the audit says it was a comparison that concluded this, not the worker.
        let (_, cause, _) = record.history.last().unwrap();
        assert_eq!(*cause, Cause::Reconciliation);
    }

    #[test]
    fn a_surviving_worker_is_recognised_rather_than_restarted() {
        // Executor restart with the worker still alive. Treating this as Lost and
        // starting again is precisely the duplicate-execution bug.
        let mut sup = supervisor();
        let spec = spec();
        let now = Timestamp::now();

        sup.start(&spec, None, now).unwrap();
        let findings = sup.reconcile(now);
        assert_eq!(findings[0].1, Reconciliation::Consistent);
        assert_eq!(
            sup.record(&spec.task_id).unwrap().state,
            ExecutionState::Running
        );
        assert_eq!(sup.running().len(), 1);
    }

    #[test]
    fn stopping_a_task_terminates_it_and_records_cancellation() {
        let mut sup = supervisor();
        let spec = spec();
        let now = Timestamp::now();

        let child = sup.start(&spec, None, now).unwrap();
        sup.stop(&spec.task_id, now).unwrap();

        assert_eq!(*sup.launcher.terminated.lock().unwrap(), vec![child.pid]);
        assert_eq!(
            sup.record(&spec.task_id).unwrap().state,
            ExecutionState::Cancelled
        );
    }

    #[test]
    fn killing_says_whether_work_survived() {
        // The operator needs to know this at the moment they do it, not afterwards.
        let mut sup = supervisor();
        let spec = spec();
        let now = Timestamp::now();

        sup.start(&spec, None, now).unwrap();
        sup.kill(&spec.task_id, now).unwrap();

        let record = sup.record(&spec.task_id).unwrap();
        assert_eq!(record.state, ExecutionState::Killed);
        assert!(record
            .failure_reason
            .as_ref()
            .unwrap()
            .contains("work since the last one is lost"));
    }

    #[test]
    fn stopping_something_unknown_or_finished_is_harmless() {
        let mut sup = supervisor();
        let spec = spec();
        let now = Timestamp::now();

        // Never started.
        assert!(sup.stop(&spec.task_id, now).is_ok());
        assert!(sup.kill(&spec.task_id, now).is_ok());

        // Already finished.
        let child = sup.start(&spec, None, now).unwrap();
        sup.launcher.finish(child, Exit::Code(0));
        sup.reap(now);
        assert!(sup.stop(&spec.task_id, now).is_ok());
        assert_eq!(
            sup.record(&spec.task_id).unwrap().state,
            ExecutionState::Completed,
            "a stop must not move a task out of a terminal state"
        );
    }

    #[test]
    fn several_tasks_are_supervised_independently() {
        let mut sup = supervisor();
        let now = Timestamp::now();

        let a = spec();
        let b = spec();
        let child_a = sup.start(&a, None, now).unwrap();
        sup.start(&b, None, now).unwrap();
        assert_eq!(sup.running().len(), 2);

        sup.launcher.finish(child_a, Exit::Code(0));
        let outcomes = sup.reap(now);
        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0].task_id, a.task_id);
        assert_eq!(sup.running().len(), 1);
        assert_eq!(sup.running()[0].task_id, b.task_id);
    }
}
