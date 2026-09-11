//! The task execution service — the whole chain, driven from one place.
//!
//! RFC-007 §7. The links exist: a [`TaskQueue`] holds what was submitted and derives what is
//! ready, [`run_pass`] plans and dispatches the ready set, [`TaskBridge`] carries a start to
//! the executor and keeps the uncertainty honest. This is the thing that ticks them, so the
//! control plane's `runtime` needs only to hold one and call [`TaskExecutionService::tick`] on
//! a timer.
//!
//! It is deliberately a small composition of tested parts rather than new logic. Everything
//! hard — dependency resolution, the plan, the never-run-twice rule, reconciliation — already
//! lives and is tested in the queue, the dispatcher and the bridge. This wires them so a
//! submitted task, once its dependencies clear, becomes a running process, and a finished
//! process unblocks whatever waited on it.
//!
//! # One tick
//!
//! 1. Drain the executor's events and feed each terminal one back into the queue, which
//!    unblocks dependents and cancels the impossible.
//! 2. Run a scheduling pass over the queue's ready set — reconcile, plan, dispatch.
//! 3. Mark the tasks the pass placed as running, so the next tick does not offer them again.
//!
//! The order matters: completions first, so a task freed this tick can have its dependent
//! placed in the same tick rather than the next.

use archie_execution::lifecycle::ExecutionState;
use archie_execution::protocol::Event;
use archie_registry::ResourceGraph;
use archie_scheduler::queue::{CompletionEffects, SubmitError, TaskQueue};
use archie_scheduler::Scheduler;
use archie_types::{NodeId, Task, TaskId, TaskStatus, Timestamp};

use crate::dispatch::{run_pass, PassReport, SpecSource};
use crate::execution::{ExecutorLink, TaskBridge};

/// What one tick of the service did.
#[derive(Debug, Default, Clone)]
pub struct ServiceReport {
    /// The scheduling pass's own report.
    pub pass: PassReport,
    /// Tasks that finished since the last tick, with the terminal state the executor gave.
    pub finished: Vec<(TaskId, ExecutionState)>,
    /// Dependents unblocked by those completions.
    pub newly_ready: Vec<TaskId>,
    /// Tasks cancelled because a dependency can no longer satisfy them.
    pub cancelled: Vec<TaskId>,
    /// True when the executor's event stream overflowed and the picture is incomplete until
    /// the next inventory. Surfaced, never swallowed.
    pub events_incomplete: bool,
}

impl ServiceReport {
    /// Whether a human should look at this tick.
    pub fn needs_attention(&self) -> bool {
        self.pass.needs_attention() || self.events_incomplete
    }
}

/// Holds the queue, the scheduler, the bridge and the spec source, and ticks them together.
pub struct TaskExecutionService<L: ExecutorLink, S: SpecSource> {
    queue: TaskQueue,
    scheduler: Scheduler,
    bridge: TaskBridge<L>,
    specs: S,
    node: NodeId,
}

impl<L: ExecutorLink, S: SpecSource> TaskExecutionService<L, S> {
    pub fn new(scheduler: Scheduler, bridge: TaskBridge<L>, specs: S, node: NodeId) -> Self {
        Self {
            queue: TaskQueue::new(),
            scheduler,
            bridge,
            specs,
            node,
        }
    }

    /// Submit a task. It runs once its dependencies clear; it may be ready at once.
    pub fn submit(&mut self, task: Task) -> Result<TaskStatus, SubmitError> {
        self.queue.submit(task)
    }

    pub fn queued(&self) -> usize {
        self.queue.len()
    }

    pub fn get(&self, task_id: &TaskId) -> Option<&Task> {
        self.queue.get(task_id)
    }

    /// Advance the whole chain one tick.
    pub fn tick(&mut self, graph: &ResourceGraph, now: Timestamp) -> ServiceReport {
        let mut report = ServiceReport::default();

        // 1. Completions first. A task freed this tick can have its dependent placed in the
        //    same tick, not the next.
        report.events_incomplete = self.absorb_events(&mut report);

        // 2. The scheduling pass over the ready set. `run_pass` reconciles, plans and
        //    dispatches; the queue's ready() is exactly the pending set it expects.
        let ready = self.queue.ready();
        report.pass = run_pass(
            &mut self.scheduler,
            &mut self.bridge,
            &self.specs,
            graph,
            &ready,
            self.node,
            now,
        );

        // 3. Anything the pass placed — started, already running, or uncertain — is moved out
        //    of the queue's ready set so it is not offered for a second placement. Uncertain
        //    counts here for the same reason it holds a scheduler slot: it may be running, and
        //    offering it again could start a second copy.
        let placed: Vec<TaskId> = report
            .pass
            .started
            .iter()
            .chain(&report.pass.already_running)
            .chain(&report.pass.uncertain)
            .copied()
            .collect();
        for task_id in placed {
            self.queue.mark_running(&task_id);
        }

        report
    }

    /// Drain the executor's events and apply every terminal one to the queue.
    ///
    /// Returns whether the event stream was reported incomplete. A `Discrepancy` or a
    /// non-terminal `StateChanged` is not a completion and does not touch the queue — the
    /// former is for a human, the latter is a checkpoint or freeze the task survives.
    fn absorb_events(&mut self, report: &mut ServiceReport) -> bool {
        let (events, overflowed) = match self.bridge.drain_events() {
            Ok(drained) => drained,
            // The executor could not be reached this tick. Not fatal: the pass below still
            // reconciles, and the next tick drains again. Reported as incomplete so a caller
            // does not believe it has seen every completion.
            Err(_) => return true,
        };

        for event in events {
            if let Event::Finished(status) = event {
                if status.state.is_terminal() {
                    let outcome = map_terminal(status.state);
                    report.finished.push((status.task_id, status.state));
                    let CompletionEffects {
                        newly_ready,
                        cancelled,
                    } = self.queue.on_completion(status.task_id, outcome);
                    report.newly_ready.extend(newly_ready);
                    report.cancelled.extend(cancelled);
                    // The task's own record can go once its completion has been propagated:
                    // its dependents have already read the final status they needed.
                    self.queue.forget(&status.task_id);
                }
            }
        }

        overflowed
    }
}

/// Map an executor terminal state to the queue's task status.
///
/// `Killed` maps to `Failed`, not `Cancelled`: a task the kernel killed did not complete its
/// work, so a dependent that needed it to succeed must be failed, while a compensation path
/// that runs on failure should fire. `Cancelled` is reserved for a task that was deliberately
/// not run.
fn map_terminal(state: ExecutionState) -> TaskStatus {
    match state {
        ExecutionState::Completed => TaskStatus::Succeeded,
        ExecutionState::Cancelled => TaskStatus::Cancelled,
        ExecutionState::Failed | ExecutionState::Killed => TaskStatus::Failed,
        // Not terminal; absorb_events only calls this for terminal states, and a
        // non-terminal one arriving here is a bug better surfaced as a failure than silently
        // mapped to success.
        _ => TaskStatus::Failed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use archie_execution::client::ClientError;
    use archie_execution::protocol::{Command, Response, TaskStatus as ExecStatus};
    use archie_execution::spec::{ResolvedExecutable, ResourceBudget, TaskExecutionSpec};
    use archie_registry::{NodeOrigin, NodeRecord, ResourceGraph};
    use archie_scheduler::SchedulerConfig;
    use archie_state::TrustState;
    use archie_types::{
        Capability, CpuInfo, DependencyCondition, LinkClass, MemoryInfo, ModuleId,
        NetworkInterface, NodeDescriptor, NodeHealth, NodeLoad, NodeProfile, QueueClass,
        TaskDependency, TraceId,
    };
    use std::cell::RefCell;
    use std::sync::Mutex;

    /// Scripts the executor: `starts` answer Start commands, `drains` are the event batches
    /// returned by successive Drain commands (the real path a completion takes back into the
    /// service). Inventory answers empty.
    struct ScriptedLink {
        starts: Mutex<RefCell<Vec<Result<Response, ClientError>>>>,
        drains: Mutex<RefCell<Vec<Vec<Event>>>>,
    }
    impl ScriptedLink {
        fn new(starts: Vec<Result<Response, ClientError>>, drains: Vec<Vec<Event>>) -> Self {
            Self {
                starts: Mutex::new(RefCell::new(starts)),
                drains: Mutex::new(RefCell::new(drains)),
            }
        }
    }
    impl ExecutorLink for ScriptedLink {
        fn send(&self, command: &Command) -> Result<Response, ClientError> {
            match command {
                Command::Drain => {
                    let drains = self.drains.lock().unwrap();
                    let mut drains = drains.borrow_mut();
                    let events = if drains.is_empty() {
                        Vec::new()
                    } else {
                        drains.remove(0)
                    };
                    Ok(Response::Events {
                        events,
                        overflowed: false,
                    })
                }
                Command::Inventory => Ok(Response::Inventory { tasks: Vec::new() }),
                _ => {
                    let starts = self.starts.lock().unwrap();
                    let mut starts = starts.borrow_mut();
                    if starts.is_empty() {
                        return Err(ClientError::ExecutorAbsent {
                            path: "exhausted".into(),
                        });
                    }
                    starts.remove(0)
                }
            }
        }
    }

    fn finished_event(id: TaskId, state: ExecutionState) -> Event {
        Event::Finished(ExecStatus {
            task_id: id,
            state,
            pid: Some(7),
            exit_code: Some(0),
            exit_signal: None,
            failure_reason: None,
        })
    }

    struct EverySpec;
    impl SpecSource for EverySpec {
        fn spec_for(&self, task: &Task) -> Option<TaskExecutionSpec> {
            Some(TaskExecutionSpec {
                task_id: task.task_id,
                trace_id: TraceId::new(),
                executable: ResolvedExecutable::from_registry(
                    ModuleId::parse("core.test").unwrap(),
                    "1.0.0",
                    "/usr/bin/true",
                    Vec::new(),
                )
                .unwrap(),
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
                requested_by: "test".into(),
                requested_at: Timestamp::now(),
            })
        }
    }

    fn node_record(seed: u8) -> NodeRecord {
        let key = [seed; 32];
        NodeRecord {
            descriptor: NodeDescriptor {
                node_id: NodeId::from_public_key_bytes(&key),
                public_key: hex::encode(key),
                hostname: "n".into(),
                profile: NodeProfile::Server,
                archieos_version: "0.1.0".into(),
                os: "Arch".into(),
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

    fn graph(record: &NodeRecord) -> ResourceGraph {
        let mut g = ResourceGraph::new();
        g.upsert(record.clone());
        g
    }

    fn service(
        record: &NodeRecord,
        starts: Vec<Result<Response, ClientError>>,
        drains: Vec<Vec<Event>>,
    ) -> TaskExecutionService<ScriptedLink, EverySpec> {
        let scheduler = Scheduler::new(SchedulerConfig {
            slots_per_node: 4,
            reserved_active_slots: 1,
            ..Default::default()
        });
        TaskExecutionService::new(
            scheduler,
            TaskBridge::new(ScriptedLink::new(starts, drains)),
            EverySpec,
            record.descriptor.node_id,
        )
    }

    fn task(node: NodeId) -> Task {
        let mut t = Task::new("run", TraceId::new(), QueueClass::Active);
        t.assigned_node = Some(node);
        t
    }

    fn running(id: TaskId) -> ExecStatus {
        ExecStatus {
            task_id: id,
            state: ExecutionState::Running,
            pid: Some(7),
            exit_code: None,
            exit_signal: None,
            failure_reason: None,
        }
    }

    #[test]
    fn a_submitted_ready_task_is_started_on_the_next_tick() {
        let record = node_record(1);
        let node = record.descriptor.node_id;
        let mut svc = service(
            &record,
            vec![Ok(Response::Accepted(running(TaskId::new())))],
            vec![],
        );

        let t = task(node);
        let id = t.task_id;
        assert_eq!(svc.submit(t).unwrap(), TaskStatus::Ready);

        let report = svc.tick(&graph(&record), Timestamp::now());
        assert_eq!(report.pass.started, vec![id]);
        assert_eq!(
            svc.get(&id).unwrap().status,
            TaskStatus::Running,
            "a placed task is marked running so the next tick does not offer it again"
        );
    }

    #[test]
    fn a_blocked_task_is_not_started_until_its_dependency_finishes() {
        let record = node_record(1);
        let node = record.descriptor.node_id;
        // First tick starts the dependency; second tick, after it finishes, starts the
        // dependent. Two Accepted answers, one per start.
        let dep = task(node);
        let dep_id = dep.task_id;
        let mut dependent = task(node);
        dependent.depends_on = vec![TaskDependency {
            task_id: dep_id,
            condition: DependencyCondition::Succeeded,
        }];
        let dependent_id = dependent.task_id;

        let mut svc = service(
            &record,
            vec![
                Ok(Response::Accepted(running(TaskId::new()))),
                Ok(Response::Accepted(running(TaskId::new()))),
            ],
            // Tick 1 drains nothing; tick 2 drains the dependency's completion.
            vec![
                vec![],
                vec![finished_event(dep_id, ExecutionState::Completed)],
            ],
        );

        svc.submit(dep).unwrap();
        assert_eq!(svc.submit(dependent).unwrap(), TaskStatus::Blocked);

        // Tick 1: only the dependency starts.
        let r1 = svc.tick(&graph(&record), Timestamp::now());
        assert_eq!(r1.pass.started, vec![dep_id]);

        // Tick 2: the drain carries the dependency's completion, which unblocks the
        // dependent, and the pass starts it in the same tick.
        let r2 = svc.tick(&graph(&record), Timestamp::now());
        assert!(
            r2.newly_ready.contains(&dependent_id),
            "the dependency finishing must unblock the dependent"
        );
        assert_eq!(r2.pass.started, vec![dependent_id]);
    }

    #[test]
    fn a_failed_dependency_cancels_the_dependent_rather_than_starting_it() {
        let record = node_record(1);
        let node = record.descriptor.node_id;
        let dep = task(node);
        let dep_id = dep.task_id;
        let mut dependent = task(node);
        dependent.depends_on = vec![TaskDependency::on_success(dep_id)];
        let dependent_id = dependent.task_id;

        let mut svc = service(
            &record,
            vec![Ok(Response::Accepted(running(TaskId::new())))],
            vec![vec![], vec![finished_event(dep_id, ExecutionState::Failed)]],
        );
        svc.submit(dep).unwrap();
        svc.submit(dependent).unwrap();

        svc.tick(&graph(&record), Timestamp::now()); // start the dependency
        let report = svc.tick(&graph(&record), Timestamp::now());

        assert!(report.cancelled.contains(&dependent_id));
        assert!(
            !report.pass.started.contains(&dependent_id),
            "a task whose dependency failed is cancelled, never started"
        );
    }
}
