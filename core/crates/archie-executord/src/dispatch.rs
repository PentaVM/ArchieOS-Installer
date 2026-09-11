//! Applying a control-plane command to the supervisor.
//!
//! RFC-007 §7, step 3. This is where the protocol stops being types and starts changing
//! what is running on the machine.
//!
//! # Preemption is a goal, and this is where the goal is turned into a mechanism
//!
//! `archied` asks for a task's resources back and says how long it may wait and whether
//! freezing is permitted. It does not say how. That decision lives here because it
//! depends on a fact only the executor holds — whether this particular worker implements
//! the checkpoint protocol:
//!
//! ```text
//! worker checkpoints      → ask, and let the deadline run
//! worker does not         → do not ask at all
//! ```
//!
//! Asking a worker that cannot answer, and then waiting out the full deadline before
//! doing anything, would add the deadline to every preemption of every non-checkpointing
//! task, for nothing. The scheduler's user request would wait five seconds to be told
//! what was knowable immediately.

use archie_execution::lifecycle::ExecutionState;
use archie_execution::protocol::{Command, Event, RefusalReason, Response, TaskStatus};
use archie_execution::spec::ResourceBudget;
use archie_observability::kernel::cgroup::CgroupPath;
use archie_types::{TaskId, Timestamp};
use std::collections::BTreeMap;

use crate::cgroups::{CgroupRefusal, TaskCgroups};
use crate::events::EventQueue;
use crate::journal::{self, Journal};
use crate::launcher::{Exit, Launcher};
use crate::preempt::{self, CheckpointChannel, Fallback, Permissions};
use crate::recovery;
use crate::supervisor::{StartRefusal, Supervisor, TaskRecord};

/// A checkpoint channel for workers that have none.
///
/// Not a stand-in for the real thing: it is the correct implementation for every worker
/// that never advertised the protocol, and `plan_preemption` guarantees such a worker is
/// never asked in the first place. Having it means a dispatcher can be built without one
/// and still behave correctly, rather than the absence being a special case threaded
/// through the preemption path.
#[derive(Debug, Default)]
pub struct NoCheckpointChannel;

impl CheckpointChannel for NoCheckpointChannel {
    fn request(
        &self,
        _task_id: TaskId,
        _deadline: Timestamp,
    ) -> Option<archie_execution::checkpoint::CheckpointReply> {
        None
    }
}

/// Render a record as the protocol's view of it.
fn status_of(record: &TaskRecord) -> TaskStatus {
    let (exit_code, exit_signal) = match &record.exit {
        Some(Exit::Code(code)) => (Some(*code), None),
        Some(Exit::Signal(sig)) => (None, Some(*sig)),
        None => (None, None),
    };
    TaskStatus {
        task_id: record.task_id,
        state: record.state,
        pid: record.child.map(|c| c.pid),
        exit_code,
        exit_signal,
        failure_reason: record.failure_reason.clone(),
    }
}

fn refusal_reason(refusal: &StartRefusal) -> RefusalReason {
    match refusal {
        StartRefusal::AlreadyRunning => RefusalReason::NotApplicable,
        StartRefusal::AlreadyFinished(_) => RefusalReason::AlreadyFinished,
        StartRefusal::InvalidSpec(_) => RefusalReason::InvalidSpec,
        StartRefusal::NotAdmissible(_) => RefusalReason::NotAdmissible,
        StartRefusal::LaunchFailed(_) => RefusalReason::LaunchFailed,
    }
}

/// How a preemption should be attempted for a given worker.
///
/// Separated from the acting so the decision can be tested on its own — it is the part
/// with the judgement in it, and the part a future policy change will touch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreemptionPlan {
    /// Ask the worker to checkpoint and let the deadline run.
    RequestCheckpoint { deadline_ms: u64 },
    /// The worker cannot checkpoint and freezing is permitted, so keep it alive and
    /// resident rather than losing its work.
    FreezeImmediately,
    /// The worker cannot checkpoint and freezing is not permitted. Its work is lost
    /// either way, so stop it now rather than after a pointless wait.
    StopImmediately,
}

/// Decide how to preempt.
///
/// The rule that matters: a worker with no checkpoint protocol is never asked. Asking
/// costs the full deadline and can only ever end the same way, and the request that
/// triggered the preemption is waiting on it.
pub fn plan_preemption(
    supports_checkpoint: bool,
    deadline_ms: u64,
    allow_freeze: bool,
) -> PreemptionPlan {
    if supports_checkpoint {
        return PreemptionPlan::RequestCheckpoint { deadline_ms };
    }
    if allow_freeze {
        PreemptionPlan::FreezeImmediately
    } else {
        PreemptionPlan::StopImmediately
    }
}

/// Applies commands to a supervisor.
///
/// Owns the OS resources around a task — its cgroup, its control channel — while the
/// supervisor owns the process lifecycle. Keeping them apart means the supervisor's
/// idempotency and reconciliation logic never has to know whether this node has cgroup
/// delegation, which is the part most likely to differ between machines.
pub struct Dispatcher<L: Launcher, C: CheckpointChannel = NoCheckpointChannel> {
    supervisor: Supervisor<L>,
    cgroups: TaskCgroups,
    channel: C,
    /// Each task's cgroup, for the ones that have one.
    ///
    /// Held here rather than on the record because a node without delegation has tasks
    /// with no cgroup at all, and an `Option` on every record would invite code that
    /// forgets to check it.
    task_cgroups: BTreeMap<TaskId, CgroupPath>,
    /// What has happened since anyone last asked.
    events: EventQueue,
    /// Durable task records, when the node has somewhere to keep them.
    ///
    /// `None` on a node with no state directory — a development host, or one where the
    /// directory could not be created. Not fatal: the executor works, and only loses its
    /// memory across a restart. The cost of that is real and is logged, rather than being
    /// hidden behind a silently absent file.
    journal: Option<Journal>,
}

impl<L: Launcher> Dispatcher<L, NoCheckpointChannel> {
    /// A dispatcher with no cgroup delegation and no worker channel.
    ///
    /// The honest configuration for a development host: preemption falls back to
    /// stopping, and a task requiring confinement is refused by the supervisor before it
    /// reaches here.
    pub fn new(supervisor: Supervisor<L>) -> Self {
        Self::with_resources(
            supervisor,
            TaskCgroups::undelegated(Default::default()),
            NoCheckpointChannel,
        )
    }
}

impl<L: Launcher, C: CheckpointChannel> Dispatcher<L, C> {
    pub fn with_resources(supervisor: Supervisor<L>, cgroups: TaskCgroups, channel: C) -> Self {
        Self {
            supervisor,
            cgroups,
            channel,
            task_cgroups: BTreeMap::new(),
            events: EventQueue::default(),
            journal: None,
        }
    }

    /// Keep durable records in this journal.
    pub fn with_journal(mut self, journal: Journal) -> Self {
        self.journal = Some(journal);
        self
    }

    pub fn journal(&self) -> Option<&Journal> {
        self.journal.as_ref()
    }

    /// Write a task's current state to the journal, if there is one.
    ///
    /// Failures are reported, never propagated. A journal that cannot be written is a
    /// restart that will not remember — bad, and not a reason to refuse to run the task
    /// that is in front of us. The alternative would let a full disk stop a node from
    /// working at all.
    fn record(&self, task_id: &TaskId) -> Option<journal::JournalError> {
        let journal = self.journal.as_ref()?;
        let record = self.supervisor.record(task_id)?;

        journal
            .write(&journal::Record {
                task_id: *task_id,
                state: record.state,
                pid: record.child.map(|c| c.pid),
                cgroup: self
                    .task_cgroups
                    .get(task_id)
                    .map(|p| p.as_str().to_string()),
                updated_at: Timestamp::now(),
            })
            .err()
    }

    /// The event queue, for the reaping loop to post to and for tests to inspect.
    pub fn events_mut(&mut self) -> &mut EventQueue {
        &mut self.events
    }

    pub fn events(&self) -> &EventQueue {
        &self.events
    }

    /// Compare what this executor believes against what the machine holds.
    ///
    /// Run at startup and after any suspected loss. Every finding is queued as an event
    /// rather than acted on: reconciliation reports, and policy decides. Concluding
    /// anything here would mean the process least able to judge — one that has just
    /// started and knows nothing — making the call.
    pub fn reconcile(&mut self, now: Timestamp) -> Result<recovery::Summary, CgroupRefusal> {
        let _ = now;
        let discovered = if self.cgroups.is_delegated() {
            self.cgroups.discover()?
        } else {
            // No delegation means no cgroups to find, so records are the only evidence
            // there is. Comparing against an empty list is still worth doing: it reports
            // records that expect a process where none can be confirmed.
            Vec::new()
        };

        // Live records first; they are the authority while this executor is running. The
        // journal fills in what a restart lost, which at startup is everything — and that
        // is exactly when reconciliation has to have something to compare against, or
        // every cgroup on the node reads as untracked.
        let mut records: Vec<recovery::RecordedTask> = self
            .supervisor
            .all_records()
            .map(|r| recovery::RecordedTask {
                task_id: r.task_id,
                state: r.state,
                pid: r.child.map(|c| c.pid),
            })
            .collect();

        if let Some(journal) = &self.journal {
            match journal.load() {
                Ok(loaded) => {
                    for unreadable in &loaded.unreadable {
                        // A record that cannot be read is a task that may still be
                        // running. Saying so is the point; the file is the only evidence
                        // left of what it was.
                        tracing::warn!(
                            path = %unreadable.path,
                            detail = %unreadable.detail,
                            "a task record could not be read; whatever it described may \
                             still be running"
                        );
                    }
                    let live: std::collections::BTreeSet<TaskId> =
                        records.iter().map(|r| r.task_id).collect();
                    for record in loaded.records {
                        if !live.contains(&record.task_id) {
                            records.push(recovery::RecordedTask {
                                task_id: record.task_id,
                                state: record.state,
                                pid: record.pid,
                            });
                        }
                    }
                }
                Err(e) => tracing::warn!(error = %e, "could not read the task journal"),
            }
        }

        let findings = recovery::reconcile(&records, &discovered);
        let summary = recovery::Summary::of(&findings);

        for finding in findings {
            if !finding.needs_decision() {
                continue;
            }
            // Only the disagreements are reported. A consistent record is not news, and
            // an event per task on every startup would bury the one that matters.
            self.events.push(Event::Discrepancy {
                task_id: finding.task_id(),
                recorded: finding
                    .implied_state()
                    .unwrap_or(ExecutionState::Recovering),
                detail: format!("{finding:?}"),
            });
        }

        Ok(summary)
    }

    /// Collect finished workers and queue what happened.
    ///
    /// Wraps the supervisor's reap so a completion cannot be noticed without being
    /// recorded: the two happening in different places is how an event gets dropped on
    /// the floor by a caller who forgot the second half.
    pub fn reap(&mut self, now: Timestamp) -> usize {
        let finished = self.supervisor.reap(now);
        let count = finished.len();
        for outcome in finished {
            self.events.push(Event::Finished(TaskStatus {
                task_id: outcome.task_id,
                state: outcome.state,
                pid: self
                    .supervisor
                    .record(&outcome.task_id)
                    .and_then(|r| r.child.map(|c| c.pid)),
                exit_code: match &outcome.exit {
                    Some(Exit::Code(code)) => Some(*code),
                    _ => None,
                },
                exit_signal: match &outcome.exit {
                    Some(Exit::Signal(sig)) => Some(*sig),
                    _ => None,
                },
                failure_reason: outcome.failure_reason,
            }));

            // The record is written, not removed. It is dropped only once the control
            // plane has actually collected the completion — a record removed before
            // archied hears about it is a completion nobody ever learns, and the slot
            // stays occupied in the scheduler's view forever.
            if let Some(e) = self.record(&outcome.task_id) {
                tracing::warn!(error = %e, "task finished but could not be recorded");
            }
        }
        count
    }

    /// The cgroup this task is confined to, if any.
    /// The cgroup manager, for tests that need to plant a fixture the dispatcher will
    /// later discover.
    #[cfg(test)]
    pub(crate) fn cgroups_for_test(&self) -> &TaskCgroups {
        &self.cgroups
    }

    pub fn cgroup_for(&self, task_id: &TaskId) -> Option<&CgroupPath> {
        self.task_cgroups.get(task_id)
    }

    pub fn supervisor(&self) -> &Supervisor<L> {
        &self.supervisor
    }

    pub fn supervisor_mut(&mut self) -> &mut Supervisor<L> {
        &mut self.supervisor
    }

    /// Apply one command.
    pub fn apply(&mut self, command: Command, now: Timestamp) -> Response {
        match command {
            Command::Start(spec) => self.start(*spec, now),
            Command::Stop { task_id, grace_ms } => self.stop(task_id, grace_ms, now),
            Command::Preempt {
                task_id,
                deadline_ms,
                allow_freeze,
            } => self.preempt(task_id, deadline_ms, allow_freeze, now),
            Command::Resume { task_id } => self.resume(task_id),
            Command::Rebudget { task_id, budget } => self.rebudget(task_id, budget),
            Command::Inventory => self.inventory(),
            Command::Drain => self.drain(),
        }
    }

    fn start(&mut self, spec: archie_execution::TaskExecutionSpec, now: Timestamp) -> Response {
        let task_id = spec.task_id;

        // The cgroup and its budget come first, so the limit is in force before the
        // worker's first instruction rather than a moment after it. A node without
        // delegation simply has no cgroup to make, and the supervisor has already
        // refused anything that required one.
        if self.cgroups.is_delegated() && !self.task_cgroups.contains_key(&task_id) {
            match self.cgroups.create(&task_id, &spec.budget) {
                Ok(path) => {
                    self.task_cgroups.insert(task_id, path);
                }
                Err(e) => {
                    return Response::Refused {
                        task_id: Some(task_id),
                        reason: RefusalReason::LaunchFailed,
                        detail: format!("could not prepare the task's cgroup: {e}"),
                    }
                }
            }
        }

        // The cgroup the task was just given, so the launcher can hand it to the
        // runner. Looked up rather than remembered, because `start` is idempotent and a
        // retry must reach the same cgroup as the first attempt.
        let cgroup_procs = self
            .task_cgroups
            .get(&task_id)
            .map(|path| format!("/sys/fs/cgroup/{}/cgroup.procs", path.as_str()));

        match self.supervisor.start(&spec, cgroup_procs.as_deref(), now) {
            Ok(_) => {
                // Recorded after the launch rather than before. A record written first
                // would claim a running task on a node where the spawn then failed, and a
                // restart would go looking for a process that never existed.
                if let Some(e) = self.record(&task_id) {
                    tracing::warn!(error = %e, "task started but could not be recorded");
                }
                let record = self.supervisor.record(&task_id).expect("just started");
                Response::Accepted(status_of(record))
            }
            // Already running is the retry case, and it is answered rather than
            // complained about: the control plane that resent this is behaving
            // correctly, and only the executor knows the first attempt landed.
            Err(StartRefusal::AlreadyRunning) => {
                let record = self.supervisor.record(&task_id).expect("known");
                Response::AlreadySo(status_of(record))
            }
            Err(refusal) => Response::Refused {
                task_id: Some(task_id),
                reason: refusal_reason(&refusal),
                detail: refusal.to_string(),
            },
        }
    }

    fn stop(&mut self, task_id: TaskId, _grace_ms: u64, now: Timestamp) -> Response {
        let Some(record) = self.supervisor.record(&task_id) else {
            return Response::Refused {
                task_id: Some(task_id),
                reason: RefusalReason::UnknownTask,
                detail: "the executor holds no record of this task".into(),
            };
        };
        if record.state.is_terminal() {
            return Response::AlreadySo(status_of(record));
        }

        match self.supervisor.stop(&task_id, now) {
            Ok(()) => {
                let record = self.supervisor.record(&task_id).expect("known");
                Response::Accepted(status_of(record))
            }
            Err(refusal) => Response::Refused {
                task_id: Some(task_id),
                reason: refusal_reason(&refusal),
                detail: refusal.to_string(),
            },
        }
    }

    fn preempt(
        &mut self,
        task_id: TaskId,
        deadline_ms: u64,
        allow_freeze: bool,
        now: Timestamp,
    ) -> Response {
        let Some(record) = self.supervisor.record(&task_id) else {
            return Response::Refused {
                task_id: Some(task_id),
                reason: RefusalReason::UnknownTask,
                detail: "the executor holds no record of this task".into(),
            };
        };
        if !record.state.holds_slot() {
            // Nothing to reclaim. Reported as already-so rather than refused, since the
            // scheduler's goal — this task is not consuming the slot — already holds.
            return Response::AlreadySo(status_of(record));
        }

        let supports_checkpoint = record.supports_checkpoint();
        let cgroup = self.task_cgroups.get(&task_id).cloned();

        // Stopping belongs to the supervisor, so it is passed as a closure rather than
        // the preemption module reaching into it: that module needs one capability, not
        // the whole supervisor.
        let supervisor = &mut self.supervisor;
        let outcome = preempt::preempt(
            preempt::Request {
                task_id,
                supports_checkpoint,
                cgroup: cgroup.as_ref(),
                permissions: Permissions { allow_freeze },
                deadline: now.plus_millis(deadline_ms as i64),
            },
            &self.cgroups,
            &self.channel,
            Timestamp::now,
            || {
                supervisor
                    .stop(&task_id, now)
                    .map_err(|refusal| refusal.to_string())
            },
        );

        // Record the freeze on the task itself. Only Cause::Kernel may write Frozen, and
        // this is the one place where that is literally what happened.
        if outcome.fallback == Fallback::Froze {
            self.supervisor.mark_frozen(&task_id, now);
        }

        match &outcome.fallback {
            Fallback::Failed(detail) => Response::Refused {
                task_id: Some(task_id),
                reason: RefusalReason::NotApplicable,
                detail: format!("{}; {detail}", outcome.explain()),
            },
            _ => {
                let record = self.supervisor.record(&task_id).expect("known");
                let mut status = status_of(record);
                // The explanation travels with the answer. Whether the slot was really
                // released and whether the work survives a kill are the two things the
                // scheduler decides on next, and both are in that sentence.
                status.failure_reason = Some(outcome.explain());
                Response::Accepted(status)
            }
        }
    }

    fn resume(&mut self, task_id: TaskId) -> Response {
        let Some(record) = self.supervisor.record(&task_id) else {
            return Response::Refused {
                task_id: Some(task_id),
                reason: RefusalReason::UnknownTask,
                detail: "the executor holds no record of this task".into(),
            };
        };
        if record.state == ExecutionState::Running {
            return Response::AlreadySo(status_of(record));
        }
        if record.state != ExecutionState::Frozen {
            // A checkpointed task resumes by being started again from its artifact,
            // which is a Start with a different spec rather than a thaw. Saying so beats
            // a generic refusal, because the caller is holding the artifact.
            return Response::Refused {
                task_id: Some(task_id),
                reason: RefusalReason::NotApplicable,
                detail: format!(
                    "only a frozen task can be thawed; this one is {}. A checkpointed \
                     task resumes by being started again from its artifact",
                    record.state
                ),
            };
        }

        let Some(path) = self.task_cgroups.get(&task_id).cloned() else {
            return Response::Refused {
                task_id: Some(task_id),
                reason: RefusalReason::NotApplicable,
                detail: "the task is frozen but has no cgroup on record; thawing it \
                         would be guessing at which one"
                    .into(),
            };
        };

        match self.cgroups.thaw(&path) {
            Ok(()) => {
                self.supervisor.mark_thawed(&task_id, Timestamp::now());
                let record = self.supervisor.record(&task_id).expect("known");
                Response::Accepted(status_of(record))
            }
            Err(e) => Response::Refused {
                task_id: Some(task_id),
                reason: RefusalReason::NotApplicable,
                detail: format!("could not thaw the task: {e}"),
            },
        }
    }

    fn rebudget(&mut self, task_id: TaskId, budget: ResourceBudget) -> Response {
        let Some(record) = self.supervisor.record(&task_id) else {
            return Response::Refused {
                task_id: Some(task_id),
                reason: RefusalReason::UnknownTask,
                detail: "the executor holds no record of this task".into(),
            };
        };
        if record.state.is_terminal() {
            return Response::AlreadySo(status_of(record));
        }

        let Some(path) = self.task_cgroups.get(&task_id).cloned() else {
            // Without a cgroup the budget is a number nobody enforces, and accepting it
            // would have the resource governor believe it had throttled something.
            return Response::Refused {
                task_id: Some(task_id),
                reason: RefusalReason::NotApplicable,
                detail: "this task has no cgroup, so a budget change would be advisory; \
                         configure Delegate= on archie-executord.service"
                    .into(),
            };
        };

        match self.cgroups.apply_budget(&path, &budget) {
            Ok(()) => {
                let record = self.supervisor.record(&task_id).expect("known");
                Response::Accepted(status_of(record))
            }
            Err(e) => Response::Refused {
                task_id: Some(task_id),
                reason: RefusalReason::NotApplicable,
                detail: format!("could not write the new budget: {e}"),
            },
        }
    }

    fn drain(&mut self) -> Response {
        let (events, overflowed) = self.events.drain();

        // Now that the control plane has them, finished tasks can be forgotten. This is
        // the only safe moment: earlier and a completion is lost, later and the journal
        // grows without bound on a node that runs many short tasks.
        if let Some(journal) = &self.journal {
            for event in &events {
                if let Event::Finished(status) = event {
                    if status.state.is_terminal() {
                        if let Err(e) = journal.forget(&status.task_id) {
                            tracing::warn!(error = %e, "could not forget a finished task");
                        }
                    }
                }
            }
        }

        Response::Events { events, overflowed }
    }

    fn inventory(&self) -> Response {
        Response::Inventory {
            tasks: self.supervisor.all_records().map(status_of).collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::launcher::testing::FakeLauncher;
    use crate::launcher::LaunchError;
    use archie_execution::{
        FilesystemGrant, RequiredEnforcement, ResolvedExecutable, ResourceBudget, RestartPolicy,
        SandboxStrength, TaskExecutionSpec,
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

    fn dispatcher() -> Dispatcher<FakeLauncher> {
        Dispatcher::new(Supervisor::new(
            FakeLauncher::new(),
            SandboxStrength::none(),
        ))
    }

    /// A dispatcher with a real (fixture-backed) delegated cgroup subtree.
    ///
    /// Returns the fixture root so the test can read what was actually written to the
    /// controller files — the only way to tell a freeze that happened from one that was
    /// merely reported.
    fn delegated_dispatcher(
        tag: &str,
    ) -> (
        std::path::PathBuf,
        Dispatcher<FakeLauncher, NoCheckpointChannel>,
    ) {
        use archie_observability::kernel::cgroup::{CgroupController, CgroupPath};

        let root = std::env::temp_dir().join(format!(
            "archie-dispatch-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("cgroup.controllers"), "cpu io memory pids\n").unwrap();

        let delegated = CgroupPath::new("system.slice/archie-executord.service").unwrap();
        std::fs::create_dir_all(delegated.resolve(&root)).unwrap();

        let cgroups = TaskCgroups::new(CgroupController::new(&root), delegated);
        let dispatcher = Dispatcher::with_resources(
            Supervisor::new(FakeLauncher::new(), SandboxStrength::none()),
            cgroups,
            NoCheckpointChannel,
        );
        (root, dispatcher)
    }

    #[test]
    fn starting_a_task_writes_its_budget_before_the_worker_runs() {
        // The limit has to be in force before the worker's first instruction, not a
        // moment after it: a worker that allocates hard on startup is exactly the one
        // whose budget matters.
        let (root, mut d) = delegated_dispatcher("budget");
        let mut spec = spec();
        spec.budget.cpu_cores = 0.5;
        let task_id = spec.task_id;

        d.apply(Command::Start(Box::new(spec)), Timestamp::now());

        let path = d
            .cgroup_for(&task_id)
            .expect("the task got a cgroup")
            .clone();
        assert_eq!(
            std::fs::read_to_string(path.resolve(&root).join("cpu.max")).unwrap(),
            "50000 100000"
        );
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_restart_recognises_its_own_task_instead_of_calling_it_an_orphan() {
        // The reason the journal exists. Without durable records a restarted executor has
        // nothing to compare against, so every cgroup on the node reads as untracked —
        // and it would raise an alarm about every task it had been running perfectly well
        // a second earlier.
        let (root, mut first) = delegated_dispatcher("restart");
        let journal_dir = root.join("journal");
        first = first.with_journal(Journal::new(&journal_dir));
        first.journal().unwrap().prepare().unwrap();

        let spec = spec();
        let task_id = spec.task_id;
        let now = Timestamp::now();
        first.apply(Command::Start(Box::new(spec)), now);

        let path = first.cgroup_for(&task_id).unwrap().clone();
        let dir = path.resolve(&root);
        std::fs::write(dir.join("cgroup.freeze"), "0").unwrap();
        std::fs::write(
            dir.join("cgroup.events"),
            "populated 1
frozen 0
",
        )
        .unwrap();

        // The executor dies. A new one starts with no memory but the same journal and the
        // same cgroups.
        drop(first);
        let mut second = Dispatcher::with_resources(
            Supervisor::new(FakeLauncher::new(), SandboxStrength::none()),
            TaskCgroups::new(
                archie_observability::kernel::cgroup::CgroupController::new(&root),
                archie_observability::kernel::cgroup::CgroupPath::new(
                    "system.slice/archie-executord.service",
                )
                .unwrap(),
            ),
            NoCheckpointChannel,
        )
        .with_journal(Journal::new(&journal_dir));

        let summary = second.reconcile(now).unwrap();
        assert_eq!(
            summary.untracked, 0,
            "the journal is what stops a healthy task reading as an orphan"
        );
        assert_eq!(summary.survived, 1, "it is adopted, not restarted");
        assert!(!summary.needs_attention());

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_completion_is_forgotten_only_once_the_control_plane_has_it() {
        // Earlier and the completion is lost forever; later and the journal grows without
        // bound on a node running many short tasks.
        let (root, mut d) = delegated_dispatcher("forget");
        let journal_dir = root.join("journal");
        d = d.with_journal(Journal::new(&journal_dir));
        d.journal().unwrap().prepare().unwrap();

        let spec = spec();
        let task_id = spec.task_id;
        let now = Timestamp::now();
        d.apply(Command::Start(Box::new(spec)), now);

        let child = d.supervisor().record(&task_id).unwrap().child.unwrap();
        d.supervisor_mut()
            .launcher_for_test()
            .finish(child, Exit::Code(0));
        d.reap(now);

        assert_eq!(
            d.journal().unwrap().load().unwrap().records.len(),
            1,
            "still recorded: archied has not been told yet"
        );

        d.apply(Command::Drain, now);
        assert!(
            d.journal().unwrap().load().unwrap().records.is_empty(),
            "forgotten only after the completion was collected"
        );

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_cgroup_left_by_a_dead_executor_is_reported_as_untracked() {
        // The one situation where starting a task "again" runs it twice. A fresh executor
        // has no records, so anything in its subtree is by definition something nothing
        // is supervising.
        let (root, mut d) = delegated_dispatcher("startup");

        // Stand in for a previous instance's leftovers.
        let orphan = archie_types::TaskId::new();
        let path = d
            .cgroups_for_test()
            .create(&orphan, &ResourceBudget::default())
            .unwrap();
        let dir = path.resolve(&root);
        std::fs::write(dir.join("cgroup.freeze"), "0").unwrap();
        std::fs::write(
            dir.join("cgroup.events"),
            "populated 1
frozen 0
",
        )
        .unwrap();

        let summary = d.reconcile(Timestamp::now()).unwrap();
        assert_eq!(summary.untracked, 1);
        assert!(summary.needs_attention());
        assert!(summary.describe().contains("must not be started again"));

        // And it is queued for the control plane rather than resolved here.
        match d.apply(Command::Drain, Timestamp::now()) {
            Response::Events { events, .. } => {
                assert_eq!(events.len(), 1);
                assert!(events[0].needs_attention());
                assert_eq!(events[0].task_id(), orphan);
            }
            other => panic!("unexpected {other:?}"),
        }

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_task_this_executor_started_reconciles_as_consistent() {
        // The ordinary case must be quiet, or the alarming one is impossible to spot.
        let (root, mut d) = delegated_dispatcher("consistent");
        let spec = spec();
        let task_id = spec.task_id;
        let now = Timestamp::now();

        d.apply(Command::Start(Box::new(spec)), now);
        let path = d.cgroup_for(&task_id).unwrap().clone();
        std::fs::write(path.resolve(&root).join("cgroup.freeze"), "0").unwrap();
        std::fs::write(
            path.resolve(&root).join("cgroup.events"),
            "populated 1
frozen 0
",
        )
        .unwrap();

        let summary = d.reconcile(now).unwrap();
        assert_eq!(summary.survived, 1);
        assert!(!summary.needs_attention());

        match d.apply(Command::Drain, now) {
            Response::Events { events, .. } => assert!(
                events.is_empty(),
                "a consistent record is not news; an event per task would bury the one                  that matters"
            ),
            other => panic!("unexpected {other:?}"),
        }

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn the_task_cgroup_reaches_the_launcher_rather_than_being_dropped() {
        // The dispatcher makes the cgroup and the launcher hands it to the runner. If the
        // path is lost between them the worker starts outside its own budget, which looks
        // exactly like a working system until something runs away with the machine.
        let (root, mut d) = delegated_dispatcher("threading");
        let spec = spec();
        let task_id = spec.task_id;

        d.apply(Command::Start(Box::new(spec)), Timestamp::now());

        let told = d
            .supervisor()
            .launcher_for_test()
            .cgroups
            .lock()
            .unwrap()
            .clone();
        assert_eq!(told.len(), 1);
        let procs = told[0]
            .as_deref()
            .expect("the launcher was told the cgroup");
        assert!(procs.ends_with("/cgroup.procs"));
        assert!(procs.contains(&task_id.as_uuid().to_string()));

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_node_without_delegation_tells_the_launcher_there_is_no_cgroup() {
        // Not an omission: the supervisor has already refused anything requiring enforced
        // limits, so a task reaching here without a cgroup never asked for one.
        let mut d = dispatcher();
        d.apply(Command::Start(Box::new(spec())), Timestamp::now());

        let told = d
            .supervisor()
            .launcher_for_test()
            .cgroups
            .lock()
            .unwrap()
            .clone();
        assert_eq!(told, vec![None]);
    }

    #[test]
    fn a_silent_worker_past_its_deadline_is_frozen_by_the_kernel() {
        // The full preemption sequence on a node that can actually do it: ask, wait,
        // and let the kernel finish the sentence.
        let (root, mut d) = delegated_dispatcher("freeze");
        let mut spec = spec();
        spec.supports_checkpoint = true;
        let task_id = spec.task_id;
        let now = Timestamp::now();

        d.apply(Command::Start(Box::new(spec)), now);
        let path = d.cgroup_for(&task_id).expect("cgroup").clone();
        std::fs::write(path.resolve(&root).join("cgroup.freeze"), "0").unwrap();

        let response = d.apply(
            Command::Preempt {
                task_id,
                deadline_ms: 0,
                allow_freeze: true,
            },
            now,
        );

        match response {
            Response::Accepted(status) => {
                assert_eq!(status.state, ExecutionState::Frozen);
                assert!(status
                    .failure_reason
                    .unwrap()
                    .contains("state still in memory"));
            }
            other => panic!("unexpected {other:?}"),
        }

        assert_eq!(
            std::fs::read_to_string(path.resolve(&root).join("cgroup.freeze")).unwrap(),
            "1",
            "the freezer must actually have been written"
        );

        // A frozen task has not released its slot: its memory is resident and its cgroup
        // exists. Saying otherwise would let the scheduler overcommit the node.
        let record = d.supervisor().record(&task_id).unwrap();
        assert!(record.state.holds_slot());
        assert!(!record.state.survives_kill());

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_frozen_task_can_be_thawed_and_runs_again() {
        let (root, mut d) = delegated_dispatcher("thaw");
        let mut spec = spec();
        spec.supports_checkpoint = true;
        let task_id = spec.task_id;
        let now = Timestamp::now();

        d.apply(Command::Start(Box::new(spec)), now);
        let path = d.cgroup_for(&task_id).expect("cgroup").clone();
        std::fs::write(path.resolve(&root).join("cgroup.freeze"), "0").unwrap();
        d.apply(
            Command::Preempt {
                task_id,
                deadline_ms: 0,
                allow_freeze: true,
            },
            now,
        );

        match d.apply(Command::Resume { task_id }, now) {
            Response::Accepted(status) => assert_eq!(status.state, ExecutionState::Running),
            other => panic!("unexpected {other:?}"),
        }
        assert_eq!(
            std::fs::read_to_string(path.resolve(&root).join("cgroup.freeze")).unwrap(),
            "0"
        );
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_running_tasks_budget_can_be_changed_without_restarting_it() {
        // The resource governor's lever.
        let (root, mut d) = delegated_dispatcher("rebudget");
        let spec = spec();
        let task_id = spec.task_id;
        let now = Timestamp::now();
        d.apply(Command::Start(Box::new(spec)), now);

        let tightened = ResourceBudget {
            cpu_cores: 0.25,
            memory_high_bytes: 64 * 1024 * 1024,
            memory_max_bytes: 128 * 1024 * 1024,
            io_weight: 10,
        };
        match d.apply(
            Command::Rebudget {
                task_id,
                budget: tightened,
            },
            now,
        ) {
            Response::Accepted(status) => {
                assert_eq!(status.state, ExecutionState::Running)
            }
            other => panic!("unexpected {other:?}"),
        }

        let path = d.cgroup_for(&task_id).unwrap().clone();
        assert_eq!(
            std::fs::read_to_string(path.resolve(&root).join("cpu.max")).unwrap(),
            "25000 100000"
        );
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_start_command_runs_the_task_and_answers_with_its_state() {
        let mut d = dispatcher();
        let spec = spec();
        let task_id = spec.task_id;

        match d.apply(Command::Start(Box::new(spec)), Timestamp::now()) {
            Response::Accepted(status) => {
                assert_eq!(status.task_id, task_id);
                assert_eq!(status.state, ExecutionState::Running);
                assert!(status.pid.is_some());
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn resending_a_start_is_answered_not_complained_about() {
        // The control plane that resent this is behaving correctly, and the executor is
        // the only component that knows the first attempt landed.
        let mut d = dispatcher();
        let spec = spec();
        let now = Timestamp::now();

        d.apply(Command::Start(Box::new(spec.clone())), now);
        match d.apply(Command::Start(Box::new(spec)), now) {
            Response::AlreadySo(status) => assert_eq!(status.state, ExecutionState::Running),
            other => panic!("expected a no-op answer, got {other:?}"),
        }
    }

    #[test]
    fn a_refusal_says_whether_resending_could_ever_help() {
        let mut d = dispatcher();
        let mut spec = spec();
        spec.required_enforcement = RequiredEnforcement::untrusted();

        match d.apply(Command::Start(Box::new(spec)), Timestamp::now()) {
            Response::Refused { reason, detail, .. } => {
                assert_eq!(reason, RefusalReason::NotAdmissible);
                assert!(!reason.is_retryable());
                assert!(
                    reason.suggests_replacement(),
                    "this is the refusal that should move the task to another node"
                );
                assert!(detail.contains("weaker form"));
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn a_launch_failure_is_the_one_worth_retrying() {
        let mut d = dispatcher();
        d.supervisor_mut()
            .launcher_for_test()
            .fail_next_launch(LaunchError::ProgramNotFound("/usr/lib/archie/x".into()));

        match d.apply(Command::Start(Box::new(spec())), Timestamp::now()) {
            Response::Refused { reason, .. } => {
                assert_eq!(reason, RefusalReason::LaunchFailed);
                assert!(reason.is_retryable());
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn a_command_about_an_unknown_task_says_so() {
        let mut d = dispatcher();
        let now = Timestamp::now();
        for command in [
            Command::Stop {
                task_id: TaskId::new(),
                grace_ms: 1000,
            },
            Command::Preempt {
                task_id: TaskId::new(),
                deadline_ms: 1000,
                allow_freeze: false,
            },
            Command::Resume {
                task_id: TaskId::new(),
            },
        ] {
            match d.apply(command, now) {
                Response::Refused { reason, .. } => {
                    assert_eq!(reason, RefusalReason::UnknownTask)
                }
                other => panic!("unexpected {other:?}"),
            }
        }
    }

    // ---- preemption planning ---------------------------------------------------

    #[test]
    fn a_worker_that_cannot_checkpoint_is_never_asked_to() {
        // Asking costs the full deadline and can only end the same way, and the user
        // request that triggered the preemption is waiting on it.
        assert_eq!(
            plan_preemption(false, 5_000, true),
            PreemptionPlan::FreezeImmediately
        );
        assert_eq!(
            plan_preemption(false, 5_000, false),
            PreemptionPlan::StopImmediately
        );
    }

    #[test]
    fn a_worker_that_can_checkpoint_is_asked_and_given_the_deadline() {
        assert_eq!(
            plan_preemption(true, 5_000, true),
            PreemptionPlan::RequestCheckpoint { deadline_ms: 5_000 }
        );
        // Checkpointing is preferred even when freezing is allowed: a checkpoint
        // survives being killed and a freeze does not.
        assert_eq!(
            plan_preemption(true, 5_000, false),
            PreemptionPlan::RequestCheckpoint { deadline_ms: 5_000 }
        );
    }

    #[test]
    fn preempting_a_non_checkpointing_worker_stops_it_now() {
        // The one preemption path that is fully real today.
        let mut d = dispatcher();
        let spec = spec();
        let task_id = spec.task_id;
        let now = Timestamp::now();

        d.apply(Command::Start(Box::new(spec)), now);
        match d.apply(
            Command::Preempt {
                task_id,
                deadline_ms: 5_000,
                allow_freeze: false,
            },
            now,
        ) {
            Response::Accepted(status) => {
                assert_eq!(status.state, ExecutionState::Cancelled)
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn a_checkpointing_worker_on_a_node_with_no_channel_is_stopped_not_stranded() {
        // A worker that advertises the protocol, on a node with no channel to reach it
        // and no cgroup to freeze it: the deadline passes with silence, freezing is
        // impossible, so it is stopped. The task must never be left sitting in
        // CheckpointRequested, holding its slot while appearing to cooperate with a
        // request nobody can answer.
        let mut d = dispatcher();
        let mut spec = spec();
        spec.supports_checkpoint = true;
        let task_id = spec.task_id;
        let now = Timestamp::now();

        d.apply(Command::Start(Box::new(spec)), now);
        match d.apply(
            Command::Preempt {
                task_id,
                deadline_ms: 5_000,
                allow_freeze: true,
            },
            now,
        ) {
            Response::Accepted(status) => {
                assert_eq!(status.state, ExecutionState::Cancelled);
                let explanation = status.failure_reason.unwrap();
                assert!(
                    explanation.contains("did not answer in time"),
                    "the answer carries why the slot was reclaimed: {explanation}"
                );
            }
            other => panic!("unexpected {other:?}"),
        }

        let state = d.supervisor().record(&task_id).unwrap().state;
        assert_ne!(
            state,
            ExecutionState::CheckpointRequested,
            "a task stranded mid-protocol holds its slot forever"
        );
        assert!(!state.holds_slot(), "the slot must actually be free");
    }

    #[test]
    fn a_budget_change_is_refused_when_nothing_would_enforce_it() {
        // Accepting it would have the resource governor believe it had throttled
        // something.
        let mut d = dispatcher();
        let spec = spec();
        let task_id = spec.task_id;
        let now = Timestamp::now();
        d.apply(Command::Start(Box::new(spec)), now);

        match d.apply(
            Command::Rebudget {
                task_id,
                budget: ResourceBudget::default(),
            },
            now,
        ) {
            Response::Refused { detail, .. } => {
                assert!(detail.contains("advisory"));
                assert!(detail.contains("Delegate="));
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn a_checkpointed_task_is_not_thawed_because_it_was_never_frozen() {
        // It resumes by being started again from its artifact, which is a different
        // command with a different spec. Saying so beats a generic refusal, because the
        // caller is holding the artifact.
        let mut d = dispatcher();
        let spec = spec();
        let task_id = spec.task_id;
        let now = Timestamp::now();
        d.apply(Command::Start(Box::new(spec)), now);

        match d.apply(Command::Resume { task_id }, now) {
            Response::AlreadySo(status) => {
                assert_eq!(status.state, ExecutionState::Running)
            }
            other => panic!("a running task is already resumed: {other:?}"),
        }
    }

    #[test]
    fn preempting_something_that_holds_no_slot_is_already_done() {
        let mut d = dispatcher();
        let spec = spec();
        let task_id = spec.task_id;
        let now = Timestamp::now();

        let child = d.supervisor_mut().start(&spec, None, now).unwrap();
        d.supervisor_mut()
            .launcher_for_test()
            .finish(child, Exit::Code(0));
        d.supervisor_mut().reap(now);

        match d.apply(
            Command::Preempt {
                task_id,
                deadline_ms: 5_000,
                allow_freeze: true,
            },
            now,
        ) {
            Response::AlreadySo(status) => {
                assert_eq!(status.state, ExecutionState::Completed)
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn a_finished_task_becomes_an_event_the_control_plane_can_collect() {
        // Reaping and recording happen together on purpose. The two in different places
        // is how a completion gets noticed and then dropped on the floor by a caller who
        // forgot the second half.
        let mut d = dispatcher();
        let spec = spec();
        let task_id = spec.task_id;
        let now = Timestamp::now();

        let child = d.supervisor_mut().start(&spec, None, now).unwrap();
        d.supervisor_mut()
            .launcher_for_test()
            .finish(child, Exit::Code(0));

        assert_eq!(d.reap(now), 1);

        match d.apply(Command::Drain, now) {
            Response::Events { events, overflowed } => {
                assert!(!overflowed);
                assert_eq!(events.len(), 1);
                assert_eq!(events[0].task_id(), task_id);
                match &events[0] {
                    Event::Finished(status) => {
                        assert_eq!(status.state, ExecutionState::Completed);
                        assert_eq!(status.exit_code, Some(0));
                    }
                    other => panic!("unexpected {other:?}"),
                }
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn draining_twice_does_not_deliver_the_same_completion_twice() {
        // A completion delivered twice would have the scheduler free a slot it already
        // freed, and then place work against capacity that does not exist.
        let mut d = dispatcher();
        let spec = spec();
        let now = Timestamp::now();

        let child = d.supervisor_mut().start(&spec, None, now).unwrap();
        d.supervisor_mut()
            .launcher_for_test()
            .finish(child, Exit::Code(0));
        d.reap(now);

        match d.apply(Command::Drain, now) {
            Response::Events { events, .. } => assert_eq!(events.len(), 1),
            other => panic!("unexpected {other:?}"),
        }
        match d.apply(Command::Drain, now) {
            Response::Events { events, overflowed } => {
                assert!(events.is_empty());
                assert!(!overflowed);
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn a_failure_carries_its_reason_all_the_way_to_the_event() {
        // The scheduler decides what to do next from this, and "it failed" is not enough
        // to decide anything with.
        let mut d = dispatcher();
        let spec = spec();
        let now = Timestamp::now();

        let child = d.supervisor_mut().start(&spec, None, now).unwrap();
        d.supervisor_mut()
            .launcher_for_test()
            .finish(child, Exit::Signal(9));
        d.reap(now);

        match d.apply(Command::Drain, now) {
            Response::Events { events, .. } => match &events[0] {
                Event::Finished(status) => {
                    assert_eq!(status.state, ExecutionState::Failed);
                    assert_eq!(status.exit_signal, Some(9));
                    assert!(status.exit_code.is_none());
                    assert!(status
                        .failure_reason
                        .as_ref()
                        .unwrap()
                        .contains("rather than chosen by it"));
                }
                other => panic!("unexpected {other:?}"),
            },
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn draining_changes_nothing_that_is_running() {
        // It empties the executor's own bookkeeping and touches no worker, which is why
        // it groups with the read-only commands.
        assert!(!Command::Drain.is_mutating());

        let mut d = dispatcher();
        let now = Timestamp::now();
        d.apply(Command::Start(Box::new(spec())), now);
        d.apply(Command::Drain, now);
        assert_eq!(d.supervisor().running().len(), 1);
    }

    #[test]
    fn inventory_reports_everything_and_changes_nothing() {
        // What archied uses on restart to rebuild scheduler state instead of assuming
        // its tasks died with it.
        let mut d = dispatcher();
        let now = Timestamp::now();
        let a = spec();
        let b = spec();
        d.apply(Command::Start(Box::new(a.clone())), now);
        d.apply(Command::Start(Box::new(b.clone())), now);

        match d.apply(Command::Inventory, now) {
            Response::Inventory { tasks } => {
                assert_eq!(tasks.len(), 2);
                assert!(tasks.iter().all(|t| t.state == ExecutionState::Running));
                assert!(tasks.iter().any(|t| t.task_id == a.task_id));
                assert!(tasks.iter().any(|t| t.task_id == b.task_id));
            }
            other => panic!("unexpected {other:?}"),
        }
        assert_eq!(d.supervisor().running().len(), 2);
    }

    #[test]
    fn a_finished_task_reports_how_it_finished() {
        let mut d = dispatcher();
        let spec = spec();
        let task_id = spec.task_id;
        let now = Timestamp::now();

        let child = d.supervisor_mut().start(&spec, None, now).unwrap();
        d.supervisor_mut()
            .launcher_for_test()
            .finish(child, Exit::Signal(9));
        d.supervisor_mut().reap(now);

        match d.apply(Command::Inventory, now) {
            Response::Inventory { tasks } => {
                let status = tasks.iter().find(|t| t.task_id == task_id).unwrap();
                assert_eq!(status.state, ExecutionState::Failed);
                assert_eq!(status.exit_signal, Some(9));
                assert!(
                    status.exit_code.is_none(),
                    "a signal is not an exit code; collapsing them loses whose fault it was"
                );
            }
            other => panic!("unexpected {other:?}"),
        }
    }
}
