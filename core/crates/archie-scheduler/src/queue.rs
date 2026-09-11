//! The pending-task queue — what has been submitted, what is ready to run, and what is
//! blocked on what.
//!
//! RFC-007 §7. `Scheduler::plan` decides which of the *ready* tasks to place, and the bridge
//! carries a start to the executor. Nothing held the tasks in between, or worked out which
//! were ready — a task graph is a graph, and a submitted task with an unmet dependency must
//! wait rather than run. This is that missing middle: submit lands here, readiness is derived
//! from the dependency graph, and `ready()` is the set `plan` is given.
//!
//! # Blocked, ready, and never
//!
//! A task is ready when every dependency is in a state that satisfies it. It is blocked when a
//! dependency is not yet terminal. It is *cancelled* when a dependency can never satisfy it —
//! a task that needed another to succeed, and that other failed. Distinguishing "not yet" from
//! "never" is what lets a failed step fail its dependents promptly instead of leaving a
//! subtree blocked forever, and it is the difference between a queue and a leak.
//!
//! # Completion propagates
//!
//! When a task reaches a terminal state, its dependents are re-evaluated: some become ready,
//! some become impossible. The propagation is transitive — a cancelled task cancels the tasks
//! that needed it to succeed, which cancels theirs — so a failure at the root of a subtree
//! does not leave orphaned work waiting on a result that will never come.

use std::collections::{BTreeMap, VecDeque};

use archie_types::{Task, TaskId, TaskStatus};

/// Why a submission was rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubmitError {
    /// A task with this id is already in the queue. Resubmitting is not how a task is
    /// retried, and silently replacing it would discard whatever state it had.
    AlreadyQueued(TaskId),
    /// The task depends on a task the queue has never seen. A dependency on an unknown task
    /// can never be evaluated, so it is refused at the door rather than left blocked forever.
    UnknownDependency { task: TaskId, depends_on: TaskId },
    /// The task depends on itself, directly. A self-dependency can never be satisfied.
    SelfDependency(TaskId),
}

/// What changed when a completion was applied, so a caller can act without re-scanning.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CompletionEffects {
    /// Tasks that became ready to run as a result.
    pub newly_ready: Vec<TaskId>,
    /// Tasks that became impossible — a dependency they needed can never satisfy them — and
    /// were cancelled.
    pub cancelled: Vec<TaskId>,
}

/// The tasks the node has been asked to run.
#[derive(Debug, Default)]
pub struct TaskQueue {
    tasks: BTreeMap<TaskId, Task>,
}

impl TaskQueue {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.tasks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tasks.is_empty()
    }

    pub fn get(&self, task_id: &TaskId) -> Option<&Task> {
        self.tasks.get(task_id)
    }

    /// Accept a task, deriving its initial status from its dependencies.
    ///
    /// Every dependency must already be in the queue: a task graph is submitted leaves-first,
    /// or at least dependency-first, and a forward reference to a task nobody has submitted is
    /// a mistake worth catching now rather than a block that never clears.
    pub fn submit(&mut self, mut task: Task) -> Result<TaskStatus, SubmitError> {
        let id = task.task_id;
        if self.tasks.contains_key(&id) {
            return Err(SubmitError::AlreadyQueued(id));
        }
        for dependency in &task.depends_on {
            if dependency.task_id == id {
                return Err(SubmitError::SelfDependency(id));
            }
            if !self.tasks.contains_key(&dependency.task_id) {
                return Err(SubmitError::UnknownDependency {
                    task: id,
                    depends_on: dependency.task_id,
                });
            }
        }

        task.status = self.evaluate(&task);
        let status = task.status;
        self.tasks.insert(id, task);
        Ok(status)
    }

    /// The tasks that may be placed now: ready, and not already running or terminal.
    ///
    /// This is the set `Scheduler::plan` is given. A task that is running is not offered again
    /// — offering it would have the scheduler try to start a second copy, the failure the
    /// whole execution layer is built against.
    pub fn ready(&self) -> Vec<&Task> {
        self.tasks
            .values()
            .filter(|task| task.status == TaskStatus::Ready)
            .collect()
    }

    /// Mark a task as claimed by the scheduler, so `ready()` stops offering it.
    ///
    /// The scheduler placing a task and the executor confirming it are two steps; between
    /// them the task must not be offered again. Moving it out of `Ready` here is what
    /// prevents a second placement in the same or the next pass.
    pub fn mark_running(&mut self, task_id: &TaskId) {
        if let Some(task) = self.tasks.get_mut(task_id) {
            if task.status == TaskStatus::Ready {
                task.status = TaskStatus::Running;
            }
        }
    }

    /// Apply a terminal outcome to a task and propagate it to everything that depended on it.
    ///
    /// Returns which dependents became ready and which became impossible. The propagation is
    /// transitive: a cancellation cascades to the tasks that needed the cancelled one, and so
    /// on, so no subtree is left waiting on a result that will never arrive.
    pub fn on_completion(&mut self, task_id: TaskId, outcome: TaskStatus) -> CompletionEffects {
        let mut effects = CompletionEffects::default();
        if let Some(task) = self.tasks.get_mut(&task_id) {
            task.status = outcome;
        } else {
            return effects;
        }

        // Re-evaluate everything reachable from the completed task, breadth-first. A queue is
        // small — the tasks on one node — so a full pass per completion is cheap and avoids a
        // reverse-dependency index that could drift out of step with the graph.
        let mut to_revisit: VecDeque<TaskId> = VecDeque::new();
        to_revisit.push_back(task_id);

        while let Some(current) = to_revisit.pop_front() {
            let dependents: Vec<TaskId> = self
                .tasks
                .values()
                .filter(|t| t.depends_on.iter().any(|d| d.task_id == current))
                .map(|t| t.task_id)
                .collect();

            for dependent_id in dependents {
                // Only tasks still waiting can change; a running or terminal dependent is not
                // reconsidered because of an upstream completion.
                let current_status = match self.tasks.get(&dependent_id) {
                    Some(t) if is_waiting(t.status) => t.status,
                    _ => continue,
                };
                let _ = current_status;

                let next = self.evaluate(&self.tasks[&dependent_id]);
                if next == self.tasks[&dependent_id].status {
                    continue;
                }
                self.tasks.get_mut(&dependent_id).unwrap().status = next;
                match next {
                    TaskStatus::Ready => effects.newly_ready.push(dependent_id),
                    TaskStatus::Cancelled => {
                        effects.cancelled.push(dependent_id);
                        // A cancellation is itself a terminal event its own dependents must
                        // learn about.
                        to_revisit.push_back(dependent_id);
                    }
                    _ => {}
                }
            }
        }

        effects
    }

    /// Remove a terminal task from the queue.
    ///
    /// Kept separate from `on_completion` so a caller can propagate the outcome first and drop
    /// the record only once nothing else needs to read its final status.
    pub fn forget(&mut self, task_id: &TaskId) -> Option<Task> {
        self.tasks.remove(task_id)
    }

    /// Derive a waiting task's status from the current state of its dependencies.
    ///
    /// Ready when all are satisfied; Cancelled when any is permanently unsatisfiable; Blocked
    /// otherwise. A task with no dependencies is ready immediately.
    fn evaluate(&self, task: &Task) -> TaskStatus {
        let mut all_satisfied = true;
        for dependency in &task.depends_on {
            let dep_status = self
                .tasks
                .get(&dependency.task_id)
                .map(|t| t.status)
                // A dependency that has been forgotten counts as gone; treat its absence as a
                // terminal state so a satisfied-by-terminal dependency clears rather than
                // hangs. `submit` refuses unknown dependencies, so this only happens after a
                // dependency was legitimately completed and forgotten.
                .unwrap_or(TaskStatus::Succeeded);

            if dependency.permanently_unsatisfiable_by(dep_status) {
                return TaskStatus::Cancelled;
            }
            if !dependency.satisfied_by(dep_status) {
                all_satisfied = false;
            }
        }
        if all_satisfied {
            TaskStatus::Ready
        } else {
            TaskStatus::Blocked
        }
    }
}

/// Whether a task is still waiting on its dependencies and can change because of an upstream
/// completion. A running or terminal task is not reconsidered.
fn is_waiting(status: TaskStatus) -> bool {
    matches!(status, TaskStatus::Blocked | TaskStatus::Created)
}

#[cfg(test)]
mod tests {
    use super::*;
    use archie_types::{DependencyCondition, QueueClass, TaskDependency, TraceId};

    fn task() -> Task {
        Task::new("t", TraceId::new(), QueueClass::Active)
    }

    fn depending_on(dep: TaskId, condition: DependencyCondition) -> Task {
        let mut t = task();
        t.depends_on = vec![TaskDependency {
            task_id: dep,
            condition,
        }];
        t
    }

    #[test]
    fn a_task_with_no_dependencies_is_ready_at_once() {
        let mut queue = TaskQueue::new();
        let status = queue.submit(task()).unwrap();
        assert_eq!(status, TaskStatus::Ready);
        assert_eq!(queue.ready().len(), 1);
    }

    #[test]
    fn a_task_waits_for_its_dependency() {
        let mut queue = TaskQueue::new();
        let dep = task();
        let dep_id = dep.task_id;
        queue.submit(dep).unwrap();

        let status = queue
            .submit(depending_on(dep_id, DependencyCondition::Succeeded))
            .unwrap();
        assert_eq!(status, TaskStatus::Blocked);
        assert_eq!(
            queue.ready().len(),
            1,
            "only the dependency is ready; the dependent waits"
        );
    }

    #[test]
    fn completing_a_dependency_makes_the_dependent_ready() {
        let mut queue = TaskQueue::new();
        let dep = task();
        let dep_id = dep.task_id;
        queue.submit(dep).unwrap();
        let mut dependent = depending_on(dep_id, DependencyCondition::Succeeded);
        let dependent_id = dependent.task_id;
        dependent.status = TaskStatus::Blocked;
        queue.submit(dependent).unwrap();

        let effects = queue.on_completion(dep_id, TaskStatus::Succeeded);
        assert_eq!(effects.newly_ready, vec![dependent_id]);
        assert_eq!(queue.get(&dependent_id).unwrap().status, TaskStatus::Ready);
    }

    #[test]
    fn a_failed_dependency_cancels_a_task_that_needed_success() {
        // The difference between a queue and a leak: a task blocked on a dependency that can
        // never satisfy it is cancelled promptly, not left waiting forever.
        let mut queue = TaskQueue::new();
        let dep = task();
        let dep_id = dep.task_id;
        queue.submit(dep).unwrap();
        let dependent = depending_on(dep_id, DependencyCondition::Succeeded);
        let dependent_id = dependent.task_id;
        queue.submit(dependent).unwrap();

        let effects = queue.on_completion(dep_id, TaskStatus::Failed);
        assert_eq!(effects.cancelled, vec![dependent_id]);
        assert_eq!(
            queue.get(&dependent_id).unwrap().status,
            TaskStatus::Cancelled
        );
    }

    #[test]
    fn a_failed_dependency_satisfies_a_compensation_task() {
        // A task that runs *because* another failed — a recovery path — becomes ready when it
        // does.
        let mut queue = TaskQueue::new();
        let dep = task();
        let dep_id = dep.task_id;
        queue.submit(dep).unwrap();
        let recovery = depending_on(dep_id, DependencyCondition::Failed);
        let recovery_id = recovery.task_id;
        queue.submit(recovery).unwrap();

        let effects = queue.on_completion(dep_id, TaskStatus::Failed);
        assert_eq!(effects.newly_ready, vec![recovery_id]);
    }

    #[test]
    fn cancellation_cascades_through_the_subtree() {
        // a -> b -> c, all on success. a fails: b is impossible, and so is c, transitively.
        let mut queue = TaskQueue::new();
        let a = task();
        let a_id = a.task_id;
        queue.submit(a).unwrap();
        let b = depending_on(a_id, DependencyCondition::Succeeded);
        let b_id = b.task_id;
        queue.submit(b).unwrap();
        let c = depending_on(b_id, DependencyCondition::Succeeded);
        let c_id = c.task_id;
        queue.submit(c).unwrap();

        let effects = queue.on_completion(a_id, TaskStatus::Failed);
        assert!(effects.cancelled.contains(&b_id));
        assert!(
            effects.cancelled.contains(&c_id),
            "the cancellation of b must cascade to c, or c waits on a result that will \
             never come"
        );
    }

    #[test]
    fn a_running_task_is_not_offered_again() {
        let mut queue = TaskQueue::new();
        let t = task();
        let id = t.task_id;
        queue.submit(t).unwrap();
        assert_eq!(queue.ready().len(), 1);

        queue.mark_running(&id);
        assert!(
            queue.ready().is_empty(),
            "a task the scheduler placed must not be offered for a second placement"
        );
    }

    #[test]
    fn an_unknown_dependency_is_refused() {
        let mut queue = TaskQueue::new();
        let phantom = TaskId::new();
        let err = queue
            .submit(depending_on(phantom, DependencyCondition::Succeeded))
            .unwrap_err();
        assert!(matches!(err, SubmitError::UnknownDependency { .. }));
    }

    #[test]
    fn a_duplicate_submission_is_refused() {
        let mut queue = TaskQueue::new();
        let t = task();
        let id = t.task_id;
        let mut same = task();
        same.task_id = id;
        queue.submit(t).unwrap();
        assert_eq!(
            queue.submit(same).unwrap_err(),
            SubmitError::AlreadyQueued(id)
        );
    }

    #[test]
    fn a_self_dependency_is_refused() {
        let mut queue = TaskQueue::new();
        let mut t = task();
        let id = t.task_id;
        t.depends_on = vec![TaskDependency {
            task_id: id,
            condition: DependencyCondition::Succeeded,
        }];
        assert_eq!(
            queue.submit(t).unwrap_err(),
            SubmitError::SelfDependency(id)
        );
    }

    #[test]
    fn a_terminal_dependency_is_satisfied_by_any_outcome() {
        let mut queue = TaskQueue::new();
        let dep = task();
        let dep_id = dep.task_id;
        queue.submit(dep).unwrap();
        let dependent = depending_on(dep_id, DependencyCondition::Terminal);
        let dependent_id = dependent.task_id;
        queue.submit(dependent).unwrap();

        // Even a failure satisfies a Terminal dependency — it is "when this finishes, however".
        let effects = queue.on_completion(dep_id, TaskStatus::Failed);
        assert_eq!(effects.newly_ready, vec![dependent_id]);
        assert!(effects.cancelled.is_empty());
    }
}
