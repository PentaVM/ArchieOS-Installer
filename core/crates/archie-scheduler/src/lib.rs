//! # archie-scheduler
//!
//! Placement and preemption.
//!
//! # Idle compute is productive compute
//!
//! Three queue classes share the same hardware:
//!
//! | Class | Work | Preempted by |
//! |---|---|---|
//! | `Active` | Direct user tasks, live engineering programs | nothing |
//! | `Background` | Literature monitoring, ingestion, citation crawling | `Active` |
//! | `DeepIdle` | Verification, contradiction hunting, reproduction, hypotheses | `Active`, `Background` |
//!
//! A user request must be able to take a machine back *cleanly*. That word does the
//! work: eviction is not a kill. Background and deep-idle jobs are required to
//! checkpoint before releasing resources, so a week of idle verification is not thrown
//! away because someone asked a question. [`SchedulingAction::Preempt`] carries
//! `must_checkpoint`, and the runtime is expected to honour it before the slot is
//! reused.
//!
//! # Determinism
//!
//! Given the same graph and the same pending tasks, [`Scheduler::plan`] returns the
//! same actions in the same order. A scheduler that shuffles under load makes a
//! performance problem impossible to reproduce.

#![forbid(unsafe_code)]

pub mod queue;

use archie_observability::kernel::psi::{PressureThresholds, PressureVerdict, SystemPressure};
use archie_registry::ResourceGraph;
use archie_types::{Capability, JobId, NodeId, QueueClass, Task, TaskId, Timestamp};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Scheduler tuning.
///
/// `PartialEq` but not `Eq`: the pressure thresholds are floats, and float equality is
/// not an equivalence relation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SchedulerConfig {
    /// Concurrent jobs allowed per node.
    pub slots_per_node: u32,
    /// Slots kept free for `Active` work even when idle queues are saturated.
    ///
    /// Without this, deep-idle work fills every slot and a user request waits for a
    /// checkpoint before anything starts. Reserving capacity trades a little
    /// throughput for a responsive system.
    pub reserved_active_slots: u32,
    /// When the machine is considered too contended to start idle work.
    ///
    /// Slots answer "is there room"; pressure answers "is the machine coping". They are
    /// different questions: a node can have three free slots while thrashing memory, and
    /// filling them makes everything worse.
    pub pressure: PressureThresholds,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            slots_per_node: 4,
            reserved_active_slots: 1,
            pressure: PressureThresholds::default(),
        }
    }
}

impl SchedulerConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.slots_per_node == 0 {
            return Err("slots_per_node must be at least 1".into());
        }
        if self.reserved_active_slots >= self.slots_per_node {
            return Err(
                "reserved_active_slots must be less than slots_per_node, or no idle work \
                 could ever run"
                    .into(),
            );
        }
        Ok(())
    }
}

/// A job currently occupying a slot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunningJob {
    pub job_id: JobId,
    pub task_id: TaskId,
    pub node: NodeId,
    pub queue_class: QueueClass,
    pub started_at: Timestamp,
}

/// Why a task could not be placed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum DeferReason {
    /// No schedulable node provides the required capabilities.
    NoCapableNode { missing: Vec<Capability> },
    /// A capable node exists but every slot is taken by work of equal or higher
    /// priority.
    NoFreeSlot { node: NodeId },
    /// Capable nodes exist but none is currently schedulable.
    NoSchedulableNode,
    /// The machine is under too much pressure to admit work of this class.
    ///
    /// Distinct from having no free slot: there may be slots, but starting idle work on
    /// a contended machine only guarantees an eviction shortly after, and an eviction
    /// costs a checkpoint.
    UnderPressure {
        verdict: PressureVerdict,
        resource: String,
    },
}

impl DeferReason {
    pub fn explain(&self) -> String {
        match self {
            Self::NoCapableNode { missing } => format!(
                "no node provides: {}",
                missing
                    .iter()
                    .map(|c| c.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Self::NoFreeSlot { node } => {
                format!("node {} has no slot this task may take", node.short())
            }
            Self::NoSchedulableNode => {
                "capable nodes exist but none is reachable, healthy and trusted".into()
            }
            Self::UnderPressure { verdict, resource } => format!(
                "machine is {} ({resource} pressure); idle work is not admitted",
                verdict.as_str()
            ),
        }
    }
}

/// What the scheduler wants done.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SchedulingAction {
    Start {
        job_id: JobId,
        task_id: TaskId,
        node: NodeId,
        queue_class: QueueClass,
    },
    /// Evict a running job to make room.
    Preempt {
        job_id: JobId,
        task_id: TaskId,
        node: NodeId,
        /// The class that displaced it.
        displaced_by: QueueClass,
        /// Whether the worker must checkpoint before the slot is reused.
        must_checkpoint: bool,
    },
    Defer {
        task_id: TaskId,
        reason: DeferReason,
    },
}

/// Placement and preemption decisions.
#[derive(Debug, Clone, Default)]
pub struct Scheduler {
    config: SchedulerConfig,
    running: BTreeMap<JobId, RunningJob>,
}

impl Scheduler {
    pub fn new(config: SchedulerConfig) -> Self {
        Self {
            config,
            running: BTreeMap::new(),
        }
    }

    pub fn config(&self) -> SchedulerConfig {
        self.config
    }

    /// Record that a job started.
    pub fn register_running(&mut self, job: RunningJob) {
        self.running.insert(job.job_id, job);
    }

    /// Record that a job finished or was evicted.
    pub fn release(&mut self, job_id: &JobId) -> Option<RunningJob> {
        self.running.remove(job_id)
    }

    pub fn running(&self) -> impl Iterator<Item = &RunningJob> {
        self.running.values()
    }

    pub fn running_count(&self) -> usize {
        self.running.len()
    }

    pub fn running_on(&self, node: &NodeId) -> Vec<&RunningJob> {
        let mut jobs: Vec<&RunningJob> =
            self.running.values().filter(|j| j.node == *node).collect();
        jobs.sort_by_key(|j| (j.started_at.as_millis(), j.job_id.to_string()));
        jobs
    }

    /// Slots available to a task of `class` on `node`.
    ///
    /// Non-active work sees fewer slots than exist, because some are reserved so a user
    /// request never has to wait for a checkpoint before anything can start.
    fn usable_slots(&self, class: QueueClass) -> u32 {
        match class {
            QueueClass::Active => self.config.slots_per_node,
            _ => self
                .config
                .slots_per_node
                .saturating_sub(self.config.reserved_active_slots),
        }
    }

    /// Plan what to do with `pending`, given the current fabric.
    ///
    /// `pending` should be the runnable set from the task graph. Tasks are considered
    /// in priority order, and the plan is deterministic for identical inputs.
    pub fn plan(&self, graph: &ResourceGraph, pending: &[&Task]) -> Vec<SchedulingAction> {
        self.plan_under_pressure(graph, pending, None)
    }

    /// Plan with knowledge of what the kernel says about contention.
    ///
    /// `None` means pressure could not be read — a kernel without `CONFIG_PSI`, or a
    /// non-Linux host. In that case the scheduler falls back to slot accounting alone,
    /// which is what it did before PSI existed. It does **not** assume the machine is
    /// idle: an unknown reading must never be more permissive than a known one.
    pub fn plan_under_pressure(
        &self,
        graph: &ResourceGraph,
        pending: &[&Task],
        pressure: Option<&SystemPressure>,
    ) -> Vec<SchedulingAction> {
        let mut actions = Vec::new();

        // A local view of occupancy, updated as we place tasks within this one plan so
        // that two tasks in the same pass cannot be given the same slot.
        let mut projected: BTreeMap<NodeId, Vec<(JobId, QueueClass, Timestamp)>> = BTreeMap::new();
        for job in self.running.values() {
            projected.entry(job.node).or_default().push((
                job.job_id,
                job.queue_class,
                job.started_at,
            ));
        }

        // Job to task, for everything in `projected`: jobs already running *and* jobs
        // this pass places. Today only the first kind can be a victim, because tasks are
        // ordered most-urgent-first and `preempts` is strict — a task considered later
        // can never displace one placed earlier. That is a property of the sort, not of
        // the preemption code, so the map covers both and the lookup stays total if the
        // ordering ever changes.
        let mut placed_tasks: BTreeMap<JobId, TaskId> = self
            .running
            .values()
            .map(|job| (job.job_id, job.task_id))
            .collect();

        let mut ordered: Vec<&Task> = pending.to_vec();
        ordered.sort_by(|a, b| {
            a.queue_class
                .cmp(&b.queue_class)
                .then(a.created_at.cmp(&b.created_at))
                .then(a.task_id.0.cmp(&b.task_id.0))
        });

        for task in ordered {
            // Pressure gate, before placement. Slots answer "is there room"; this answers
            // "is the machine coping". A node can have free slots while thrashing.
            if let Some(pressure) = pressure {
                let verdict = pressure.verdict(&self.config.pressure);
                let admitted = match task.queue_class {
                    QueueClass::Active => true,
                    QueueClass::Background => verdict.permits_background(),
                    QueueClass::DeepIdle => verdict.permits_deep_idle(),
                };
                if !admitted {
                    actions.push(SchedulingAction::Defer {
                        task_id: task.task_id,
                        reason: DeferReason::UnderPressure {
                            verdict,
                            resource: pressure.worst_resource(&self.config.pressure).to_string(),
                        },
                    });
                    continue;
                }
            }

            let candidates = graph.rank_nodes(&task.required_capabilities, None);
            if candidates.is_empty() {
                actions.push(Self::defer_reason_for(graph, task));
                continue;
            }

            let limit = self.usable_slots(task.queue_class);

            // Walk candidates best-first and take the first node with genuine room.
            // Checking occupancy here rather than only on the top-ranked node is what
            // makes work spread across the fabric instead of queueing on one machine.
            let placed_on = candidates.iter().find(|candidate| {
                let occupants = projected.entry(candidate.node_id()).or_default();
                let same_or_higher = occupants
                    .iter()
                    .filter(|(_, class, _)| *class <= task.queue_class)
                    .count() as u32;
                same_or_higher < limit && (occupants.len() as u32) < self.config.slots_per_node
            });

            if let Some(candidate) = placed_on {
                let node_id = candidate.node_id();
                let job_id = JobId::new();
                projected.entry(node_id).or_default().push((
                    job_id,
                    task.queue_class,
                    Timestamp::now(),
                ));
                placed_tasks.insert(job_id, task.task_id);
                actions.push(SchedulingAction::Start {
                    job_id,
                    task_id: task.task_id,
                    node: node_id,
                    queue_class: task.queue_class,
                });
                continue;
            }

            // Everything is full. Fall back to the preferred node and consider evicting
            // something less urgent there.
            let node_id = candidates[0].node_id();
            let occupants = projected.entry(node_id).or_default();

            // Full. Look for the least urgent, most recently started victim this task
            // is actually allowed to displace. Newest-first so that a long-running idle
            // job that has made real progress is preserved over a fresh one.
            let victim = occupants
                .iter()
                .filter(|(_, class, _)| task.queue_class.preempts(*class))
                .max_by_key(|(job_id, class, started)| {
                    (*class, started.as_millis(), job_id.to_string())
                })
                .copied();

            // Resolve the victim's real task id. A Preempt naming a task that does not
            // exist is worse than not preempting: whoever handles it would checkpoint
            // nothing and report success. Deferring is the conservative answer if the id
            // is ever unknown — better than the fabricated one this used to invent.
            let victim = victim.and_then(|(victim_id, victim_class, _)| {
                placed_tasks
                    .get(&victim_id)
                    .map(|task_id| (victim_id, victim_class, *task_id))
            });

            match victim {
                Some((victim_id, victim_class, victim_task)) => {
                    occupants.retain(|(id, _, _)| *id != victim_id);

                    actions.push(SchedulingAction::Preempt {
                        job_id: victim_id,
                        task_id: victim_task,
                        node: node_id,
                        displaced_by: task.queue_class,
                        must_checkpoint: victim_class.must_checkpoint_before_yield(),
                    });

                    let job_id = JobId::new();
                    occupants.push((job_id, task.queue_class, Timestamp::now()));
                    placed_tasks.insert(job_id, task.task_id);
                    actions.push(SchedulingAction::Start {
                        job_id,
                        task_id: task.task_id,
                        node: node_id,
                        queue_class: task.queue_class,
                    });
                }
                None => actions.push(SchedulingAction::Defer {
                    task_id: task.task_id,
                    reason: DeferReason::NoFreeSlot { node: node_id },
                }),
            }
        }
        actions
    }

    /// Work out *why* nothing could take this task, for a useful defer reason.
    fn defer_reason_for(graph: &ResourceGraph, task: &Task) -> SchedulingAction {
        // Distinguish "no such hardware anywhere" from "the hardware exists but is
        // untrusted or offline" — they need entirely different operator responses.
        let capable_but_unschedulable = task
            .required_capabilities
            .iter()
            .all(|cap| !graph.all_providers_of(cap).is_empty())
            && !task.required_capabilities.is_empty();

        if capable_but_unschedulable {
            return SchedulingAction::Defer {
                task_id: task.task_id,
                reason: DeferReason::NoSchedulableNode,
            };
        }

        let available = graph.available_capabilities();
        let missing = available.missing(task.required_capabilities.iter());
        SchedulingAction::Defer {
            task_id: task.task_id,
            reason: DeferReason::NoCapableNode {
                missing: if missing.is_empty() {
                    task.required_capabilities.clone()
                } else {
                    missing
                },
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use archie_registry::{NodeOrigin, NodeRecord};
    use archie_state::TrustState;
    use archie_types::{
        CpuInfo, LinkClass, MemoryInfo, NetworkInterface, NodeDescriptor, NodeHealth, NodeLoad,
        NodeProfile, TaskStatus, TraceId,
    };

    fn node(seed: u8, capabilities: &[&str]) -> NodeRecord {
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
                capabilities: capabilities
                    .iter()
                    .map(|c| Capability::parse(*c).unwrap())
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

    fn one_node_graph() -> ResourceGraph {
        let mut graph = ResourceGraph::new();
        graph.upsert(node(1, &["compute.cpu"]));
        graph
    }

    fn task(title: &str, class: QueueClass, caps: &[&str]) -> Task {
        let mut t = Task::new(title, TraceId::new(), class);
        t.required_capabilities = caps
            .iter()
            .map(|c| Capability::parse(*c).unwrap())
            .collect();
        t.transition_to(TaskStatus::Ready).unwrap();
        t
    }

    fn starts(actions: &[SchedulingAction]) -> Vec<TaskId> {
        actions
            .iter()
            .filter_map(|a| match a {
                SchedulingAction::Start { task_id, .. } => Some(*task_id),
                _ => None,
            })
            .collect()
    }

    fn preempts(actions: &[SchedulingAction]) -> Vec<&SchedulingAction> {
        actions
            .iter()
            .filter(|a| matches!(a, SchedulingAction::Preempt { .. }))
            .collect()
    }

    #[test]
    fn default_config_is_valid() {
        assert!(SchedulerConfig::default().validate().is_ok());
    }

    #[test]
    fn a_config_reserving_every_slot_is_rejected() {
        // Reserving all slots would starve idle work permanently.
        let config = SchedulerConfig {
            slots_per_node: 2,
            reserved_active_slots: 2,
            pressure: PressureThresholds::default(),
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn a_ready_task_is_placed_on_a_capable_node() {
        let scheduler = Scheduler::default();
        let graph = one_node_graph();
        let t = task("work", QueueClass::Active, &["compute.cpu"]);
        let actions = scheduler.plan(&graph, &[&t]);
        assert_eq!(starts(&actions), vec![t.task_id]);
    }

    #[test]
    fn a_task_needing_absent_hardware_is_deferred_with_the_missing_capability() {
        let scheduler = Scheduler::default();
        let graph = one_node_graph();
        let t = task("simulate", QueueClass::Active, &["simulation.cfd"]);
        let actions = scheduler.plan(&graph, &[&t]);

        match &actions[0] {
            SchedulingAction::Defer { reason, .. } => {
                assert!(reason.explain().contains("simulation.cfd"));
            }
            other => panic!("expected a defer, got {other:?}"),
        }
    }

    #[test]
    fn an_untrusted_capable_node_defers_differently_from_absent_hardware() {
        // The operator's fix differs: trust the node, versus buy the hardware.
        let mut graph = ResourceGraph::new();
        let mut gpu = node(2, &["compute.cpu", "compute.cuda"]);
        gpu.trust = TrustState::Unverified;
        graph.upsert(gpu);

        let scheduler = Scheduler::default();
        let t = task("train", QueueClass::Active, &["compute.cuda"]);
        let actions = scheduler.plan(&graph, &[&t]);
        assert!(matches!(
            &actions[0],
            SchedulingAction::Defer {
                reason: DeferReason::NoSchedulableNode,
                ..
            }
        ));
    }

    #[test]
    fn tasks_are_planned_in_priority_order() {
        let scheduler = Scheduler::new(SchedulerConfig {
            slots_per_node: 8,
            reserved_active_slots: 1,
            pressure: PressureThresholds::default(),
        });
        let graph = one_node_graph();
        let idle = task("idle", QueueClass::DeepIdle, &["compute.cpu"]);
        let background = task("bg", QueueClass::Background, &["compute.cpu"]);
        let active = task("user", QueueClass::Active, &["compute.cpu"]);

        // Submit in the least helpful order.
        let actions = scheduler.plan(&graph, &[&idle, &background, &active]);
        assert_eq!(
            starts(&actions),
            vec![active.task_id, background.task_id, idle.task_id]
        );
    }

    #[test]
    fn planning_is_deterministic() {
        let scheduler = Scheduler::default();
        let graph = one_node_graph();
        let a = task("a", QueueClass::Background, &["compute.cpu"]);
        let b = task("b", QueueClass::Background, &["compute.cpu"]);
        let first = starts(&scheduler.plan(&graph, &[&a, &b]));
        for _ in 0..5 {
            assert_eq!(starts(&scheduler.plan(&graph, &[&a, &b])), first);
        }
    }

    #[test]
    fn a_user_task_preempts_deep_idle_work() {
        let mut scheduler = Scheduler::new(SchedulerConfig {
            slots_per_node: 1,
            reserved_active_slots: 0,
            pressure: PressureThresholds::default(),
        });
        let graph = one_node_graph();
        let node_id = graph.local().unwrap().node_id();

        let idle_job = RunningJob {
            job_id: JobId::new(),
            task_id: TaskId::new(),
            node: node_id,
            queue_class: QueueClass::DeepIdle,
            started_at: Timestamp::now(),
        };
        scheduler.register_running(idle_job.clone());

        let user = task("user asked", QueueClass::Active, &["compute.cpu"]);
        let actions = scheduler.plan(&graph, &[&user]);

        let evictions = preempts(&actions);
        assert_eq!(evictions.len(), 1);
        match evictions[0] {
            SchedulingAction::Preempt {
                job_id,
                must_checkpoint,
                displaced_by,
                ..
            } => {
                assert_eq!(*job_id, idle_job.job_id);
                assert_eq!(*displaced_by, QueueClass::Active);
                assert!(
                    *must_checkpoint,
                    "idle work must checkpoint, or a week of verification is lost"
                );
            }
            other => panic!("unexpected {other:?}"),
        }
        assert_eq!(starts(&actions), vec![user.task_id]);
    }

    #[test]
    fn every_preempt_names_a_task_the_scheduler_actually_knows() {
        // A Preempt carries the task id whoever handles it will checkpoint. If that id
        // named nothing, the handler would checkpoint nothing and report success — the
        // week of verification the checkpoint exists to protect would be lost silently.
        let mut scheduler = Scheduler::new(SchedulerConfig {
            slots_per_node: 2,
            reserved_active_slots: 0,
            pressure: PressureThresholds::default(),
        });
        let graph = one_node_graph();
        let node_id = graph.local().unwrap().node_id();

        let running: Vec<RunningJob> = [QueueClass::DeepIdle, QueueClass::Background]
            .into_iter()
            .map(|queue_class| RunningJob {
                job_id: JobId::new(),
                task_id: TaskId::new(),
                node: node_id,
                queue_class,
                started_at: Timestamp::now(),
            })
            .collect();
        for job in &running {
            scheduler.register_running(job.clone());
        }

        let known: Vec<TaskId> = running.iter().map(|job| job.task_id).collect();
        let first = task("user asked", QueueClass::Active, &["compute.cpu"]);
        let second = task("user asked again", QueueClass::Active, &["compute.cpu"]);
        let actions = scheduler.plan(&graph, &[&first, &second]);

        let evictions = preempts(&actions);
        assert!(
            !evictions.is_empty(),
            "two user tasks should displace idle work"
        );
        for eviction in evictions {
            match eviction {
                SchedulingAction::Preempt {
                    task_id, job_id, ..
                } => {
                    assert!(
                        known.contains(task_id),
                        "Preempt named {task_id:?}, which belongs to no running task"
                    );
                    let expected = running
                        .iter()
                        .find(|job| job.job_id == *job_id)
                        .expect("preempted a job that was never running");
                    assert_eq!(*task_id, expected.task_id, "task id must match its job");
                }
                other => panic!("unexpected {other:?}"),
            }
        }
    }

    #[test]
    fn background_preempts_deep_idle_but_not_active() {
        let mut scheduler = Scheduler::new(SchedulerConfig {
            slots_per_node: 1,
            reserved_active_slots: 0,
            pressure: PressureThresholds::default(),
        });
        let graph = one_node_graph();
        let node_id = graph.local().unwrap().node_id();

        scheduler.register_running(RunningJob {
            job_id: JobId::new(),
            task_id: TaskId::new(),
            node: node_id,
            queue_class: QueueClass::Active,
            started_at: Timestamp::now(),
        });

        let background = task("crawl", QueueClass::Background, &["compute.cpu"]);
        let actions = scheduler.plan(&graph, &[&background]);
        assert!(
            preempts(&actions).is_empty(),
            "background work must never evict a user task"
        );
        assert!(matches!(
            &actions[0],
            SchedulingAction::Defer {
                reason: DeferReason::NoFreeSlot { .. },
                ..
            }
        ));
    }

    #[test]
    fn a_task_never_preempts_its_own_class() {
        let mut scheduler = Scheduler::new(SchedulerConfig {
            slots_per_node: 1,
            reserved_active_slots: 0,
            pressure: PressureThresholds::default(),
        });
        let graph = one_node_graph();
        let node_id = graph.local().unwrap().node_id();
        scheduler.register_running(RunningJob {
            job_id: JobId::new(),
            task_id: TaskId::new(),
            node: node_id,
            queue_class: QueueClass::Active,
            started_at: Timestamp::now(),
        });

        let another = task("also urgent", QueueClass::Active, &["compute.cpu"]);
        let actions = scheduler.plan(&graph, &[&another]);
        assert!(preempts(&actions).is_empty());
    }

    #[test]
    fn preempting_active_work_does_not_demand_a_checkpoint() {
        // Only background/idle classes carry the checkpoint obligation.
        assert!(!QueueClass::Active.must_checkpoint_before_yield());
        assert!(QueueClass::Background.must_checkpoint_before_yield());
        assert!(QueueClass::DeepIdle.must_checkpoint_before_yield());
    }

    #[test]
    fn reserved_slots_keep_capacity_free_for_user_work() {
        let scheduler = Scheduler::new(SchedulerConfig {
            slots_per_node: 2,
            reserved_active_slots: 1,
            pressure: PressureThresholds::default(),
        });
        let graph = one_node_graph();
        let a = task("idle-a", QueueClass::DeepIdle, &["compute.cpu"]);
        let b = task("idle-b", QueueClass::DeepIdle, &["compute.cpu"]);

        let actions = scheduler.plan(&graph, &[&a, &b]);
        assert_eq!(
            starts(&actions).len(),
            1,
            "idle work may use only the unreserved slot"
        );
    }

    #[test]
    fn active_work_may_use_every_slot_including_the_reserve() {
        let scheduler = Scheduler::new(SchedulerConfig {
            slots_per_node: 2,
            reserved_active_slots: 1,
            pressure: PressureThresholds::default(),
        });
        let graph = one_node_graph();
        let a = task("user-a", QueueClass::Active, &["compute.cpu"]);
        let b = task("user-b", QueueClass::Active, &["compute.cpu"]);
        assert_eq!(starts(&scheduler.plan(&graph, &[&a, &b])).len(), 2);
    }

    #[test]
    fn two_tasks_in_one_pass_do_not_get_the_same_slot() {
        let scheduler = Scheduler::new(SchedulerConfig {
            slots_per_node: 1,
            reserved_active_slots: 0,
            pressure: PressureThresholds::default(),
        });
        let graph = one_node_graph();
        let a = task("a", QueueClass::Active, &["compute.cpu"]);
        let b = task("b", QueueClass::Active, &["compute.cpu"]);

        let actions = scheduler.plan(&graph, &[&a, &b]);
        assert_eq!(
            starts(&actions).len(),
            1,
            "the second task must see the slot the first just took"
        );
    }

    #[test]
    fn work_spreads_across_nodes() {
        let mut graph = ResourceGraph::new();
        let mut local = node(1, &["compute.cpu"]);
        local.origin = NodeOrigin::Local;
        let mut peer = node(2, &["compute.cpu"]);
        peer.origin = NodeOrigin::Peer;
        graph.upsert(local);
        graph.upsert(peer);

        let scheduler = Scheduler::new(SchedulerConfig {
            slots_per_node: 1,
            reserved_active_slots: 0,
            pressure: PressureThresholds::default(),
        });
        let a = task("a", QueueClass::Active, &["compute.cpu"]);
        let b = task("b", QueueClass::Active, &["compute.cpu"]);
        let actions = scheduler.plan(&graph, &[&a, &b]);

        // Both should start; the second lands on the peer once local is full.
        assert_eq!(starts(&actions).len(), 2);
        let nodes: std::collections::HashSet<NodeId> = actions
            .iter()
            .filter_map(|a| match a {
                SchedulingAction::Start { node, .. } => Some(*node),
                _ => None,
            })
            .collect();
        assert_eq!(
            nodes.len(),
            2,
            "work must spread rather than queue on one node"
        );
    }

    #[test]
    fn releasing_a_job_frees_its_slot() {
        let mut scheduler = Scheduler::new(SchedulerConfig {
            slots_per_node: 1,
            reserved_active_slots: 0,
            pressure: PressureThresholds::default(),
        });
        let graph = one_node_graph();
        let node_id = graph.local().unwrap().node_id();
        let job = RunningJob {
            job_id: JobId::new(),
            task_id: TaskId::new(),
            node: node_id,
            queue_class: QueueClass::Active,
            started_at: Timestamp::now(),
        };
        scheduler.register_running(job.clone());

        let t = task("next", QueueClass::Active, &["compute.cpu"]);
        assert!(starts(&scheduler.plan(&graph, &[&t])).is_empty());

        scheduler.release(&job.job_id);
        assert_eq!(scheduler.running_count(), 0);
        assert_eq!(starts(&scheduler.plan(&graph, &[&t])).len(), 1);
    }

    // ─────────────────────────────────────────────────────────── pressure gate

    fn pressure_at(cpu_some: f32, mem_full: f32) -> SystemPressure {
        use archie_observability::kernel::psi::{PressureWindow, ResourcePressure};
        SystemPressure {
            cpu: ResourcePressure {
                some: PressureWindow {
                    avg10: cpu_some,
                    ..Default::default()
                },
                full: None,
            },
            memory: ResourcePressure {
                some: PressureWindow {
                    avg10: mem_full,
                    ..Default::default()
                },
                full: Some(PressureWindow {
                    avg10: mem_full,
                    ..Default::default()
                }),
            },
            io: ResourcePressure::default(),
        }
    }

    fn defers(actions: &[SchedulingAction]) -> Vec<&DeferReason> {
        actions
            .iter()
            .filter_map(|a| match a {
                SchedulingAction::Defer { reason, .. } => Some(reason),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn an_idle_machine_admits_deep_idle_work() {
        let scheduler = Scheduler::default();
        let graph = one_node_graph();
        let t = task("verify", QueueClass::DeepIdle, &["compute.cpu"]);
        let idle = pressure_at(0.0, 0.0);
        let actions = scheduler.plan_under_pressure(&graph, &[&t], Some(&idle));
        assert_eq!(starts(&actions), vec![t.task_id]);
    }

    #[test]
    fn a_contended_machine_refuses_new_deep_idle_work() {
        // There may be free slots. Starting idle work anyway only guarantees an eviction
        // shortly after, and an eviction costs a checkpoint.
        let scheduler = Scheduler::default();
        let graph = one_node_graph();
        let t = task("verify", QueueClass::DeepIdle, &["compute.cpu"]);
        let busy = pressure_at(35.0, 0.0);

        let actions = scheduler.plan_under_pressure(&graph, &[&t], Some(&busy));
        assert!(starts(&actions).is_empty());
        match defers(&actions)[0] {
            DeferReason::UnderPressure { verdict, resource } => {
                assert_eq!(*verdict, PressureVerdict::Contended);
                assert_eq!(resource, "cpu");
            }
            other => panic!("expected a pressure defer, got {other:?}"),
        }
    }

    #[test]
    fn a_user_task_runs_however_bad_the_pressure_is() {
        // The whole point of the priority system. A saturated machine is exactly when a
        // user is most likely to be asking for something.
        let scheduler = Scheduler::default();
        let graph = one_node_graph();
        let t = task("user asked", QueueClass::Active, &["compute.cpu"]);
        let saturated = pressure_at(90.0, 60.0);
        let actions = scheduler.plan_under_pressure(&graph, &[&t], Some(&saturated));
        assert_eq!(starts(&actions), vec![t.task_id]);
    }

    #[test]
    fn light_pressure_admits_background_but_not_deep_idle() {
        let scheduler = Scheduler::new(SchedulerConfig {
            slots_per_node: 8,
            reserved_active_slots: 1,
            pressure: PressureThresholds::default(),
        });
        let graph = one_node_graph();
        let background = task("crawl", QueueClass::Background, &["compute.cpu"]);
        let deep = task("verify", QueueClass::DeepIdle, &["compute.cpu"]);
        let light = pressure_at(5.0, 0.0);

        let actions = scheduler.plan_under_pressure(&graph, &[&background, &deep], Some(&light));
        assert_eq!(starts(&actions), vec![background.task_id]);
        assert!(matches!(
            defers(&actions)[0],
            DeferReason::UnderPressure { .. }
        ));
    }

    #[test]
    fn a_thrashing_machine_is_caught_even_though_cpu_looks_quiet() {
        // The case a CPU-percentage heuristic gets wrong: 2% CPU while memory is stalling
        // is the least responsive the machine will ever be.
        let scheduler = Scheduler::default();
        let graph = one_node_graph();
        let t = task("crawl", QueueClass::Background, &["compute.cpu"]);
        let thrashing = pressure_at(2.0, 30.0);

        let actions = scheduler.plan_under_pressure(&graph, &[&t], Some(&thrashing));
        assert!(starts(&actions).is_empty());
        match defers(&actions)[0] {
            DeferReason::UnderPressure { verdict, resource } => {
                assert_eq!(*verdict, PressureVerdict::Saturated);
                assert_eq!(resource, "memory", "the stalling resource must be named");
            }
            other => panic!("expected a pressure defer, got {other:?}"),
        }
    }

    #[test]
    fn unknown_pressure_is_never_more_permissive_than_known_pressure() {
        // A kernel without CONFIG_PSI must fall back to slot accounting, not to an
        // assumption that the machine is idle.
        let scheduler = Scheduler::default();
        let graph = one_node_graph();
        let t = task("verify", QueueClass::DeepIdle, &["compute.cpu"]);

        let without = scheduler.plan_under_pressure(&graph, &[&t], None);
        let with_idle = scheduler.plan_under_pressure(&graph, &[&t], Some(&pressure_at(0.0, 0.0)));
        assert_eq!(
            starts(&without).len(),
            starts(&with_idle).len(),
            "no pressure reading should behave like the old slot-only scheduler"
        );
    }

    #[test]
    fn plan_without_pressure_matches_the_pressure_aware_call() {
        let scheduler = Scheduler::default();
        let graph = one_node_graph();
        let t = task("t", QueueClass::Background, &["compute.cpu"]);
        assert_eq!(
            starts(&scheduler.plan(&graph, &[&t])),
            starts(&scheduler.plan_under_pressure(&graph, &[&t], None))
        );
    }

    #[test]
    fn the_pressure_defer_explains_itself() {
        let reason = DeferReason::UnderPressure {
            verdict: PressureVerdict::Saturated,
            resource: "io".into(),
        };
        let explanation = reason.explain();
        assert!(explanation.contains("saturated"), "{explanation}");
        assert!(explanation.contains("io"), "{explanation}");
    }

    #[test]
    fn an_empty_pending_set_produces_no_actions() {
        let scheduler = Scheduler::default();
        assert!(scheduler.plan(&one_node_graph(), &[]).is_empty());
    }

    #[test]
    fn an_empty_fabric_defers_everything() {
        let scheduler = Scheduler::default();
        let graph = ResourceGraph::new();
        let t = task("orphan", QueueClass::Active, &["compute.cpu"]);
        let actions = scheduler.plan(&graph, &[&t]);
        assert!(matches!(&actions[0], SchedulingAction::Defer { .. }));
    }
}
