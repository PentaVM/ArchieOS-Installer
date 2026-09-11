//! One scheduling pass: decide, dispatch, and account for what happened.
//!
//! RFC-007 §7. `Scheduler::plan` produces decisions and `TaskBridge` carries them to the
//! executor. This is the loop body that joins them, kept out of `runtime.rs` so it can be
//! tested against a scheduler under pressure and an executor that times out — neither of
//! which is reachable on demand from a live daemon.
//!
//! # The pass is not allowed to lose track of a slot
//!
//! Every decision the scheduler makes has to come back as an outcome the scheduler is told
//! about. A `Start` that succeeded and was not registered leaves a running process the
//! scheduler will place work on top of; a `Start` that failed and *was* registered leaves a
//! slot occupied by nothing, forever. Both are silent, and both are why this returns a
//! report rather than a bool.
//!
//! # Uncertainty occupies the slot
//!
//! A start whose outcome is unknown is registered as running. That is deliberate and it is
//! the conservative direction: an over-counted slot costs throughput until reconciliation
//! resolves it, an under-counted one puts a second copy of a job on the machine.

use archie_registry::ResourceGraph;
use archie_scheduler::{RunningJob, Scheduler, SchedulingAction};
use archie_types::{JobId, NodeId, Task, TaskId, Timestamp};

use crate::execution::{ExecutorLink, Outcome, Resolution, TaskBridge};

/// How a task becomes an execution spec.
///
/// A trait rather than a function because resolving a task means consulting the module
/// registry, and a pass that could not be tested without one would not be tested.
///
/// Returning `None` is not an error: a task whose module is not installed on this node is a
/// task this node cannot run, and the scheduler is told so rather than the pass panicking.
pub trait SpecSource {
    fn spec_for(&self, task: &Task) -> Option<archie_execution::spec::TaskExecutionSpec>;
}

/// What one pass did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PassReport {
    pub started: Vec<TaskId>,
    pub already_running: Vec<TaskId>,
    pub preempted: Vec<TaskId>,
    pub deferred: Vec<TaskId>,
    /// Tasks the executor or this node refused, with the reason.
    pub refused: Vec<(TaskId, String)>,
    /// Tasks whose start may or may not have taken effect.
    pub uncertain: Vec<TaskId>,
    /// Tasks with no spec on this node.
    pub unresolvable: Vec<TaskId>,
    /// What reconciliation settled at the start of the pass.
    pub resolutions: Vec<Resolution>,
}

impl PassReport {
    /// Whether a human should look at this pass.
    pub fn needs_attention(&self) -> bool {
        !self.uncertain.is_empty() || !self.unresolvable.is_empty()
    }

    pub fn describe(&self) -> String {
        format!(
            "{} started, {} already running, {} preempted, {} deferred, {} refused, \
             {} uncertain, {} unresolvable",
            self.started.len(),
            self.already_running.len(),
            self.preempted.len(),
            self.deferred.len(),
            self.refused.len(),
            self.uncertain.len(),
            self.unresolvable.len(),
        )
    }
}

/// Run one pass.
///
/// Reconciliation happens first, on purpose. A task left uncertain by the previous pass may
/// have finished since, and resolving it before planning means the scheduler decides against
/// the machine as it is rather than as it was when something went wrong.
pub fn run_pass<L: ExecutorLink, S: SpecSource>(
    scheduler: &mut Scheduler,
    bridge: &mut TaskBridge<L>,
    specs: &S,
    graph: &ResourceGraph,
    pending: &[&Task],
    node: NodeId,
    now: Timestamp,
) -> PassReport {
    let mut report = PassReport::default();

    // Settle what was unknown before deciding anything new.
    for resolution in bridge.reconcile() {
        if let Resolution::ItNeverStarted(task_id) | Resolution::ItFinished(task_id, _) =
            &resolution
        {
            // The slot it was holding is free. Releasing it here rather than waiting for an
            // event is what lets a node recover from a lost answer without a restart.
            release_task(scheduler, *task_id);
        }
        report.resolutions.push(resolution);
    }

    for action in scheduler.plan(graph, pending) {
        match action {
            SchedulingAction::Defer { task_id, .. } => report.deferred.push(task_id),

            SchedulingAction::Preempt { task_id, .. } => {
                // The deadline and the freeze policy come from the task, not from here: how
                // long a checkpoint may take is a property of the workload.
                let (deadline_ms, allow_freeze) = pending
                    .iter()
                    .find(|t| t.task_id == task_id)
                    .and_then(|t| specs.spec_for(t))
                    .map(|s| (s.checkpoint_deadline_ms, s.supports_checkpoint))
                    .unwrap_or((5_000, false));

                match bridge.preempt(task_id, deadline_ms, allow_freeze) {
                    Outcome::Preempted(id) => {
                        release_task(scheduler, id);
                        report.preempted.push(id);
                    }
                    Outcome::Refused { task_id, detail }
                    | Outcome::FailedCleanly { task_id, detail } => {
                        // The task is still running. Leaving it registered is correct: the
                        // slot really is occupied, and forgetting it here would let the
                        // scheduler place work on top of a process that never stopped.
                        report.refused.push((task_id, detail));
                    }
                    other => report.refused.push((
                        other.task_id(),
                        format!("unexpected outcome from a preempt: {other:?}"),
                    )),
                }
            }

            SchedulingAction::Start {
                job_id,
                task_id,
                node: target,
                queue_class,
            } => {
                let Some(task) = pending.iter().find(|t| t.task_id == task_id) else {
                    // The scheduler named a task that is not in the pending set it was
                    // given. That is a bug rather than a condition, and it is reported
                    // rather than ignored so it cannot go unnoticed for a release.
                    report.refused.push((
                        task_id,
                        "the scheduler planned a task that was not in the pending set".to_string(),
                    ));
                    continue;
                };
                let Some(spec) = specs.spec_for(task) else {
                    report.unresolvable.push(task_id);
                    continue;
                };

                let outcome = bridge.start(spec);
                if outcome.holds_a_slot() {
                    scheduler.register_running(RunningJob {
                        job_id,
                        task_id,
                        node: target,
                        queue_class,
                        started_at: now,
                    });
                }
                match outcome {
                    Outcome::Started(id) => report.started.push(id),
                    Outcome::AlreadyRunning(id) => report.already_running.push(id),
                    Outcome::Uncertain { task_id, .. } => report.uncertain.push(task_id),
                    Outcome::Refused { task_id, detail }
                    | Outcome::FailedCleanly { task_id, detail } => {
                        report.refused.push((task_id, detail))
                    }
                    other => report.refused.push((
                        other.task_id(),
                        format!("unexpected outcome from a start: {other:?}"),
                    )),
                }
            }
        }
    }

    let _ = node;
    report
}

/// Free whatever job is holding this task's slot.
///
/// The scheduler is keyed by job, and everything above knows the task. Looking it up rather
/// than threading the job id through means a task that finished can be released from an
/// event that only carries its own identifier — which is all the executor sends.
fn release_task(scheduler: &mut Scheduler, task_id: TaskId) -> Option<JobId> {
    let job_id = scheduler
        .running()
        .find(|job| job.task_id == task_id)
        .map(|job| job.job_id)?;
    scheduler.release(&job_id);
    Some(job_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::execution::ExecutorLink;
    use archie_execution::client::ClientError;
    use archie_execution::lifecycle::ExecutionState;
    use archie_execution::protocol::{Command, Response, TaskStatus as ExecStatus};
    use archie_execution::spec::{ResolvedExecutable, ResourceBudget, TaskExecutionSpec};
    use archie_registry::{NodeOrigin, NodeRecord};
    use archie_scheduler::SchedulerConfig;
    use archie_state::TrustState;
    use archie_types::{
        Capability, CpuInfo, LinkClass, MemoryInfo, ModuleId, NetworkInterface, NodeDescriptor,
        NodeHealth, NodeLoad, NodeProfile, QueueClass, TraceId,
    };
    use std::cell::RefCell;
    use std::sync::Mutex;

    struct ScriptedLink {
        answers: Mutex<RefCell<Vec<Result<Response, ClientError>>>>,
        sent: Mutex<RefCell<Vec<Command>>>,
    }

    impl ScriptedLink {
        fn new(answers: Vec<Result<Response, ClientError>>) -> Self {
            Self {
                answers: Mutex::new(RefCell::new(answers)),
                sent: Mutex::new(RefCell::new(Vec::new())),
            }
        }
        fn sent_count(&self) -> usize {
            self.sent.lock().unwrap().borrow().len()
        }
    }

    impl ExecutorLink for ScriptedLink {
        fn send(&self, command: &Command) -> Result<Response, ClientError> {
            self.sent.lock().unwrap().borrow_mut().push(command.clone());
            let answers = self.answers.lock().unwrap();
            let mut answers = answers.borrow_mut();
            if answers.is_empty() {
                return Err(ClientError::ExecutorAbsent {
                    path: "script exhausted".into(),
                });
            }
            answers.remove(0)
        }
    }

    /// Every task resolves to the same trivial spec.
    struct EverySpec;
    impl SpecSource for EverySpec {
        fn spec_for(&self, task: &Task) -> Option<TaskExecutionSpec> {
            Some(spec_for_id(task.task_id))
        }
    }

    /// No task resolves — the module is not installed here.
    struct NoSpec;
    impl SpecSource for NoSpec {
        fn spec_for(&self, _task: &Task) -> Option<TaskExecutionSpec> {
            None
        }
    }

    fn spec_for_id(task_id: TaskId) -> TaskExecutionSpec {
        TaskExecutionSpec {
            task_id,
            trace_id: TraceId::new(),
            executable: ResolvedExecutable::from_registry(
                ModuleId::parse("core.test").expect("a valid module id"),
                "1.0.0",
                "/usr/bin/true",
                Vec::new(),
            )
            .expect("an absolute path is a valid registry entry"),
            working_directory: "/var/lib/archie".into(),
            environment_allowlist: Vec::new(),
            environment_overrides: Default::default(),
            required_capabilities: Vec::new(),
            filesystem_grants: Vec::new(),
            network: Default::default(),
            required_enforcement: Default::default(),
            budget: ResourceBudget::default(),
            queue_class: QueueClass::Active,
            supports_checkpoint: false,
            checkpoint_deadline_ms: 5_000,
            lease_reference: None,
            artifact_directory: None,
            restart: Default::default(),
            timeout_ms: 0,
            requested_by: "test".to_string(),
            requested_at: Timestamp::now(),
        }
    }

    fn exec_status(task_id: TaskId, state: ExecutionState) -> ExecStatus {
        ExecStatus {
            task_id,
            state,
            pid: Some(7),
            exit_code: None,
            exit_signal: None,
            failure_reason: None,
        }
    }

    fn scheduler() -> Scheduler {
        Scheduler::new(SchedulerConfig {
            slots_per_node: 4,
            reserved_active_slots: 1,
            ..Default::default()
        })
    }

    /// One healthy, trusted, local node with nothing running on it.
    fn node_record(seed: u8) -> NodeRecord {
        let key = [seed; 32];
        NodeRecord {
            descriptor: NodeDescriptor {
                node_id: NodeId::from_public_key_bytes(&key),
                public_key: hex::encode(key),
                hostname: format!("n{seed}"),
                profile: NodeProfile::Server,
                archieos_version: "0.1.0".into(),
                os: "Arch Linux".into(),
                kernel: "6.10".into(),
                architecture: "x86_64".into(),
                cpu: CpuInfo {
                    model: "T".into(),
                    architecture: "x86_64".into(),
                    physical_cores: 4,
                    logical_cores: 8,
                    base_frequency_mhz: 3000,
                },
                memory: MemoryInfo {
                    total_bytes: 16 << 30,
                    available_bytes: 8 << 30,
                    swap_total_bytes: 0,
                },
                accelerators: vec![],
                storage: vec![],
                networks: vec![NetworkInterface {
                    name: "eth0".into(),
                    addresses: vec![],
                    mac: None,
                    speed_mbps: Some(1000),
                    link_class: LinkClass::Lan,
                }],
                capabilities: [Capability::parse("compute.cpu").unwrap()]
                    .into_iter()
                    .collect(),
                loaded_models: vec![],
                modules: vec![],
                health: NodeHealth::Ok,
                load: NodeLoad::default(),
                announced_at: Timestamp::now(),
            },
            origin: NodeOrigin::Local,
            trust: TrustState::Trusted,
            reachable: true,
            last_seen: Timestamp::now(),
        }
    }

    fn idle_graph(record: &NodeRecord) -> ResourceGraph {
        let mut graph = ResourceGraph::new();
        graph.upsert(record.clone());
        graph
    }

    fn task(node: NodeId) -> Task {
        let mut t = Task::new("run", TraceId::new(), QueueClass::Active);
        t.assigned_node = Some(node);
        t
    }

    #[test]
    fn a_started_task_occupies_a_slot() {
        let record = node_record(1);
        let node = record.descriptor.node_id;
        let mut sched = scheduler();
        let t = task(node);
        let id = t.task_id;

        let link = ScriptedLink::new(vec![Ok(Response::Accepted(exec_status(
            id,
            ExecutionState::Running,
        )))]);
        let mut bridge = TaskBridge::new(link);

        let report = run_pass(
            &mut sched,
            &mut bridge,
            &EverySpec,
            &idle_graph(&record),
            &[&t],
            node,
            Timestamp::now(),
        );

        assert_eq!(report.started, vec![id]);
        assert_eq!(
            sched.running_count(),
            1,
            "a start the executor accepted must occupy a slot, or the scheduler will place \
             work on top of a running process"
        );
    }

    #[test]
    fn a_failed_start_does_not_occupy_a_slot() {
        // The other direction of the same mistake: a slot held by nothing, forever.
        let record = node_record(1);
        let node = record.descriptor.node_id;
        let mut sched = scheduler();
        let t = task(node);
        let id = t.task_id;

        let link = ScriptedLink::new(vec![Err(ClientError::ExecutorAbsent {
            path: "/run/x".into(),
        })]);
        let mut bridge = TaskBridge::new(link);

        let report = run_pass(
            &mut sched,
            &mut bridge,
            &EverySpec,
            &idle_graph(&record),
            &[&t],
            node,
            Timestamp::now(),
        );

        assert!(report.started.is_empty());
        assert_eq!(report.refused.len(), 1);
        assert_eq!(report.refused[0].0, id);
        assert_eq!(sched.running_count(), 0);
    }

    #[test]
    fn an_uncertain_start_occupies_the_slot_until_it_is_resolved() {
        // The conservative direction, and the one that matters: an over-counted slot costs
        // throughput, an under-counted one puts a second copy of a job on the machine.
        let record = node_record(1);
        let node = record.descriptor.node_id;
        let mut sched = scheduler();
        let t = task(node);
        let id = t.task_id;

        let link = ScriptedLink::new(vec![Err(ClientError::Timeout {
            after: std::time::Duration::from_secs(5),
        })]);
        let mut bridge = TaskBridge::new(link);

        let report = run_pass(
            &mut sched,
            &mut bridge,
            &EverySpec,
            &idle_graph(&record),
            &[&t],
            node,
            Timestamp::now(),
        );

        assert_eq!(report.uncertain, vec![id]);
        assert!(report.needs_attention());
        assert_eq!(sched.running_count(), 1);
    }

    #[test]
    fn reconciliation_frees_the_slot_of_a_task_that_never_started() {
        // Without this a lost answer costs a slot until the daemon restarts.
        let record = node_record(1);
        let node = record.descriptor.node_id;
        let mut sched = scheduler();
        let t = task(node);
        let id = t.task_id;

        let link = ScriptedLink::new(vec![
            Err(ClientError::Timeout {
                after: std::time::Duration::from_secs(5),
            }),
            // Second pass: the executor has never heard of it.
            Ok(Response::Inventory { tasks: Vec::new() }),
            Ok(Response::Accepted(exec_status(id, ExecutionState::Running))),
        ]);
        let mut bridge = TaskBridge::new(link);

        run_pass(
            &mut sched,
            &mut bridge,
            &EverySpec,
            &idle_graph(&record),
            &[&t],
            node,
            Timestamp::now(),
        );
        assert_eq!(sched.running_count(), 1, "uncertain holds the slot");

        let second = run_pass(
            &mut sched,
            &mut bridge,
            &EverySpec,
            &idle_graph(&record),
            &[&t],
            node,
            Timestamp::now(),
        );
        assert_eq!(second.resolutions, vec![Resolution::ItNeverStarted(id)]);
        assert_eq!(
            second.started,
            vec![id],
            "once it is known not to have started, the task may be placed again"
        );
        assert_eq!(sched.running_count(), 1);
    }

    #[test]
    fn a_task_with_no_spec_on_this_node_is_reported_not_dispatched() {
        let record = node_record(1);
        let node = record.descriptor.node_id;
        let mut sched = scheduler();
        let t = task(node);
        let id = t.task_id;

        let link = ScriptedLink::new(Vec::new());
        let mut bridge = TaskBridge::new(link);

        let report = run_pass(
            &mut sched,
            &mut bridge,
            &NoSpec,
            &idle_graph(&record),
            &[&t],
            node,
            Timestamp::now(),
        );

        assert_eq!(report.unresolvable, vec![id]);
        assert!(report.needs_attention());
        assert_eq!(
            bridge.link().sent_count(),
            0,
            "a task this node cannot resolve must not reach the socket"
        );
        assert_eq!(sched.running_count(), 0);
    }

    #[test]
    fn a_pass_with_nothing_pending_touches_nothing() {
        let record = node_record(1);
        let node = record.descriptor.node_id;
        let mut sched = scheduler();
        let link = ScriptedLink::new(Vec::new());
        let mut bridge = TaskBridge::new(link);

        let report = run_pass(
            &mut sched,
            &mut bridge,
            &EverySpec,
            &idle_graph(&record),
            &[],
            node,
            Timestamp::now(),
        );

        assert_eq!(report, PassReport::default());
        assert!(!report.needs_attention());
        assert_eq!(bridge.link().sent_count(), 0);
    }
}
