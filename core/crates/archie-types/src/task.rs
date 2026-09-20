//! The persistent task DAG.
//!
//! # Why this exists
//!
//! A research program runs for weeks. An LLM context does not. Long-horizon reasoning
//! therefore lives here — as durable rows with explicit states and dependencies — and
//! never in a conversation buffer. A worker may die, migrate, or be replaced by a
//! different model entirely, and the program survives because the graph does.
//!
//! Every field is what you would need to *recover* the task on a cold node.

use crate::error::{Result, TypeError};
use crate::ids::{AgentId, NodeId, TaskId, TraceId};
use crate::time::Timestamp;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;

/// Lifecycle state of one task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Created,
    /// Dependencies are not yet satisfied.
    Blocked,
    /// Dependencies satisfied; waiting for a scheduler slot.
    Ready,
    /// Claimed by a worker but not yet executing.
    Active,
    Running,
    /// Halted pending a human decision. Never auto-advances.
    WaitingApproval,
    WaitingRetry,
    Paused,
    Succeeded,
    Failed,
    Cancelled,
}

impl TaskStatus {
    /// Terminal states never transition again.
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Cancelled)
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Blocked => "blocked",
            Self::Ready => "ready",
            Self::Active => "active",
            Self::Running => "running",
            Self::WaitingApproval => "waiting_approval",
            Self::WaitingRetry => "waiting_retry",
            Self::Paused => "paused",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    /// The states reachable in one step from `self`.
    ///
    /// Encoded explicitly rather than left to callers: an unchecked status write is how
    /// a task ends up "running" on a node that crashed three days ago.
    pub fn allowed_next(&self) -> &'static [TaskStatus] {
        use TaskStatus::*;
        match self {
            Created => &[Blocked, Ready, Cancelled],
            Blocked => &[Ready, Cancelled, Paused, Failed],
            Ready => &[Active, Blocked, Paused, Cancelled],
            Active => &[Running, Ready, WaitingApproval, Paused, Failed, Cancelled],
            Running => &[
                Succeeded,
                Failed,
                WaitingApproval,
                WaitingRetry,
                Paused,
                Cancelled,
            ],
            WaitingApproval => &[Ready, Active, Running, Cancelled, Failed],
            WaitingRetry => &[Ready, Cancelled, Failed],
            Paused => &[Ready, Blocked, Cancelled],
            Succeeded | Failed | Cancelled => &[],
        }
    }

    pub fn can_transition_to(&self, next: TaskStatus) -> bool {
        self.allowed_next().contains(&next)
    }
}

impl fmt::Display for TaskStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Scheduling class. Determines preemption, not just ordering.
///
/// Ordered so that `Active < Background < DeepIdle`; lower sorts first and preempts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueueClass {
    /// Direct user tasks and active engineering programs. Preempts everything.
    Active,
    /// Literature monitoring, ingestion, citation crawling.
    Background,
    /// Verification, contradiction hunting, reproduction, hypothesis generation.
    /// Runs only on otherwise-wasted compute and must checkpoint before yielding.
    DeepIdle,
}

impl QueueClass {
    /// Whether a task of this class may evict a running task of `other`.
    pub fn preempts(&self, other: QueueClass) -> bool {
        *self < other
    }

    /// Whether tasks in this class must checkpoint before releasing resources.
    ///
    /// Idle work is interrupted routinely, so losing its progress would make idle
    /// compute worthless rather than merely slow.
    pub fn must_checkpoint_before_yield(&self) -> bool {
        matches!(self, Self::Background | Self::DeepIdle)
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Background => "background",
            Self::DeepIdle => "deep_idle",
        }
    }
}

impl fmt::Display for QueueClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DependencyCondition {
    /// Satisfied only when the dependency succeeded.
    Succeeded,
    /// Satisfied when the dependency reached any terminal state.
    Terminal,
    /// Satisfied only when the dependency failed (recovery/compensation paths).
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskDependency {
    pub task_id: TaskId,
    pub condition: DependencyCondition,
}

impl TaskDependency {
    pub fn on_success(task_id: TaskId) -> Self {
        Self {
            task_id,
            condition: DependencyCondition::Succeeded,
        }
    }

    /// Whether `status` satisfies this dependency.
    pub fn satisfied_by(&self, status: TaskStatus) -> bool {
        match self.condition {
            DependencyCondition::Succeeded => status == TaskStatus::Succeeded,
            DependencyCondition::Terminal => status.is_terminal(),
            DependencyCondition::Failed => {
                matches!(status, TaskStatus::Failed | TaskStatus::Cancelled)
            }
        }
    }

    /// Whether this dependency can never be satisfied given `status`.
    ///
    /// Distinguishing "not yet" from "never" is what lets the scheduler fail a subtree
    /// promptly instead of leaving it Blocked forever.
    pub fn permanently_unsatisfiable_by(&self, status: TaskStatus) -> bool {
        if !status.is_terminal() {
            return false;
        }
        !self.satisfied_by(status)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskError {
    pub code: String,
    pub message: String,
    pub retryable: bool,
    pub occurred_at: Timestamp,
}

/// A durable task. This is the complete recovery record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Task {
    pub task_id: TaskId,
    pub trace_id: TraceId,
    pub title: String,
    pub description: String,

    pub status: TaskStatus,
    pub queue_class: QueueClass,
    pub depends_on: Vec<TaskDependency>,

    /// Which agent owns the reasoning. Not which model runs it.
    pub agent: Option<AgentId>,
    /// Which node currently holds the claim, if any.
    pub assigned_node: Option<NodeId>,

    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    pub started_at: Option<Timestamp>,
    pub completed_at: Option<Timestamp>,

    pub attempt: u32,
    pub max_attempts: u32,

    /// Immutable task input.
    pub input: BTreeMap<String, JsonValue>,
    /// Worker-owned resumable progress. This is what makes preemption cheap.
    pub checkpoint: BTreeMap<String, JsonValue>,
    pub output: Option<BTreeMap<String, JsonValue>>,
    pub error: Option<TaskError>,

    /// Capabilities this task needs from whichever node runs it.
    pub required_capabilities: Vec<crate::capability::Capability>,
    pub labels: BTreeMap<String, String>,
}

impl Task {
    pub fn new(title: impl Into<String>, trace_id: TraceId, queue_class: QueueClass) -> Self {
        let now = Timestamp::now();
        Self {
            task_id: TaskId::new(),
            trace_id,
            title: title.into(),
            description: String::new(),
            status: TaskStatus::Created,
            queue_class,
            depends_on: Vec::new(),
            agent: None,
            assigned_node: None,
            created_at: now,
            updated_at: now,
            started_at: None,
            completed_at: None,
            attempt: 0,
            max_attempts: 1,
            input: BTreeMap::new(),
            checkpoint: BTreeMap::new(),
            output: None,
            error: None,
            required_capabilities: Vec::new(),
            labels: BTreeMap::new(),
        }
    }

    /// Move to `next`, rejecting illegal transitions.
    ///
    /// Also maintains the timestamps that make a task auditable, so callers cannot
    /// forget to set `completed_at` on a terminal state.
    pub fn transition_to(&mut self, next: TaskStatus) -> Result<()> {
        if !self.status.can_transition_to(next) {
            return Err(TypeError::IllegalTaskTransition {
                from: self.status.as_str(),
                to: next.as_str(),
            });
        }
        let now = Timestamp::now();
        if next == TaskStatus::Running && self.started_at.is_none() {
            self.started_at = Some(now);
        }
        if next.is_terminal() {
            self.completed_at = Some(now);
        }
        if next == TaskStatus::Succeeded {
            // A success that still carries an error is a contradiction that would
            // poison downstream reporting.
            self.error = None;
        }
        self.status = next;
        self.updated_at = now;
        Ok(())
    }

    /// Validate invariants that must hold for any stored task.
    pub fn validate(&self) -> Result<()> {
        if self.title.trim().is_empty() {
            return Err(TypeError::InvalidTask("title must not be empty".into()));
        }
        if self.max_attempts == 0 {
            return Err(TypeError::InvalidTask("max_attempts must be >= 1".into()));
        }
        if self.attempt > self.max_attempts {
            return Err(TypeError::InvalidTask(
                "attempt cannot exceed max_attempts".into(),
            ));
        }
        if self.depends_on.iter().any(|d| d.task_id == self.task_id) {
            return Err(TypeError::InvalidTask(
                "a task cannot depend on itself".into(),
            ));
        }
        let mut seen = HashSet::new();
        for dependency in &self.depends_on {
            if !seen.insert(dependency.task_id) {
                return Err(TypeError::InvalidTask(
                    "duplicate dependency on the same task".into(),
                ));
            }
        }
        if self.updated_at < self.created_at {
            return Err(TypeError::InvalidTask(
                "updated_at cannot precede created_at".into(),
            ));
        }
        if self.status.is_terminal() && self.completed_at.is_none() {
            return Err(TypeError::InvalidTask(
                "terminal tasks require completed_at".into(),
            ));
        }
        if !self.status.is_terminal() && self.completed_at.is_some() {
            return Err(TypeError::InvalidTask(
                "only terminal tasks may carry completed_at".into(),
            ));
        }
        if self.status == TaskStatus::Succeeded && self.error.is_some() {
            return Err(TypeError::InvalidTask(
                "a succeeded task cannot carry an error".into(),
            ));
        }
        Ok(())
    }

    pub fn may_retry(&self) -> bool {
        self.attempt < self.max_attempts
            && self.error.as_ref().map(|e| e.retryable).unwrap_or(false)
    }
}

/// Result of evaluating one task's dependencies against the graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DependencyVerdict {
    /// All dependencies satisfied; the task may become Ready.
    Satisfied,
    /// Some dependency has not finished yet.
    Waiting,
    /// Some dependency reached a terminal state that can never satisfy the condition.
    Unsatisfiable,
}

/// A collection of tasks that can be reasoned about as a graph.
///
/// Kept as a plain in-memory view; durability is `archie-state`'s job.
#[derive(Debug, Clone, Default)]
pub struct TaskGraph {
    tasks: HashMap<TaskId, Task>,
}

impl TaskGraph {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, task: Task) -> Result<()> {
        task.validate()?;
        self.tasks.insert(task.task_id, task);
        Ok(())
    }

    pub fn get(&self, id: &TaskId) -> Option<&Task> {
        self.tasks.get(id)
    }

    pub fn get_mut(&mut self, id: &TaskId) -> Option<&mut Task> {
        self.tasks.get_mut(id)
    }

    pub fn len(&self) -> usize {
        self.tasks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tasks.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &Task> {
        self.tasks.values()
    }

    /// Evaluate whether `id`'s dependencies are met.
    ///
    /// A dependency on a task that is not in the graph is treated as `Waiting`, not
    /// `Satisfied` — refusing to run is the safe reading of missing information.
    pub fn dependency_verdict(&self, id: &TaskId) -> DependencyVerdict {
        let Some(task) = self.tasks.get(id) else {
            return DependencyVerdict::Waiting;
        };
        let mut all_satisfied = true;
        for dependency in &task.depends_on {
            match self.tasks.get(&dependency.task_id) {
                None => all_satisfied = false,
                Some(dep) => {
                    if dependency.permanently_unsatisfiable_by(dep.status) {
                        return DependencyVerdict::Unsatisfiable;
                    }
                    if !dependency.satisfied_by(dep.status) {
                        all_satisfied = false;
                    }
                }
            }
        }
        if all_satisfied {
            DependencyVerdict::Satisfied
        } else {
            DependencyVerdict::Waiting
        }
    }

    /// Tasks whose dependencies are met and which are waiting to run.
    ///
    /// Sorted by queue class then creation time so that scheduling is deterministic:
    /// the same graph must always yield the same order, or debugging becomes guesswork.
    pub fn runnable(&self) -> Vec<&Task> {
        let mut ready: Vec<&Task> = self
            .tasks
            .values()
            .filter(|t| matches!(t.status, TaskStatus::Ready | TaskStatus::Blocked))
            .filter(|t| self.dependency_verdict(&t.task_id) == DependencyVerdict::Satisfied)
            .collect();
        ready.sort_by(|a, b| {
            a.queue_class
                .cmp(&b.queue_class)
                .then(a.created_at.cmp(&b.created_at))
                .then(a.task_id.0.cmp(&b.task_id.0))
        });
        ready
    }

    /// Detect a dependency cycle, returning the tasks involved.
    ///
    /// A cycle is a construction bug, but one that would otherwise present as a set of
    /// tasks silently stuck in `Blocked` forever.
    pub fn find_cycle(&self) -> Option<Vec<TaskId>> {
        #[derive(Clone, Copy, PartialEq)]
        enum Mark {
            Visiting,
            Done,
        }
        let mut marks: HashMap<TaskId, Mark> = HashMap::new();
        let mut stack: Vec<TaskId> = Vec::new();

        // Iterative DFS; recursion would risk a stack overflow on a deep research graph.
        let mut roots: Vec<TaskId> = self.tasks.keys().copied().collect();
        roots.sort_by_key(|id| id.0);

        for root in roots {
            if marks.get(&root) == Some(&Mark::Done) {
                continue;
            }
            let mut work: Vec<(TaskId, bool)> = vec![(root, false)];
            while let Some((current, expanded)) = work.pop() {
                if expanded {
                    marks.insert(current, Mark::Done);
                    stack.pop();
                    continue;
                }
                match marks.get(&current) {
                    Some(Mark::Done) => continue,
                    Some(Mark::Visiting) => {
                        let start = stack.iter().position(|t| *t == current).unwrap_or(0);
                        let mut cycle = stack[start..].to_vec();
                        cycle.push(current);
                        return Some(cycle);
                    }
                    None => {}
                }
                marks.insert(current, Mark::Visiting);
                stack.push(current);
                work.push((current, true));
                if let Some(task) = self.tasks.get(&current) {
                    let mut deps: Vec<TaskId> = task.depends_on.iter().map(|d| d.task_id).collect();
                    deps.sort_by_key(|id| id.0);
                    for dep in deps {
                        if self.tasks.contains_key(&dep) {
                            work.push((dep, false));
                        }
                    }
                }
            }
        }
        None
}

/// Simplified column views for Kanban boards.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KanbanColumn {
    Backlog,
    InProgress,
    Review,
    Done,
    Archived,
}

impl From<TaskStatus> for KanbanColumn {
    fn from(status: TaskStatus) -> Self {
        match status {
            TaskStatus::Created
            | TaskStatus::Blocked
            | TaskStatus::Ready
            | TaskStatus::WaitingRetry => KanbanColumn::Backlog,
            TaskStatus::Active | TaskStatus::Running => KanbanColumn::InProgress,
            TaskStatus::WaitingApproval => KanbanColumn::Review,
            TaskStatus::Succeeded => KanbanColumn::Done,
            TaskStatus::Paused | TaskStatus::Failed | TaskStatus::Cancelled => KanbanColumn::Archived,
        }
    }
}

/// A safe, sanitized view of a Task for external APIs and UIs.
///
/// Strips inputs, checkpoints, outputs, and errors that could contain
/// sensitive information inappropriate for an external Kanban dashboard.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KanbanCard {
    pub task_id: TaskId,
    pub title: String,
    pub status: TaskStatus,
    pub column: KanbanColumn,
    pub queue_class: QueueClass,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    pub labels: BTreeMap<String, String>,
}

impl From<&Task> for KanbanCard {
    fn from(task: &Task) -> Self {
        Self {
            task_id: task.task_id,
            title: task.title.clone(),
            status: task.status,
            column: KanbanColumn::from(task.status),
            queue_class: task.queue_class,
            created_at: task.created_at,
            updated_at: task.updated_at,
            labels: task.labels.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(title: &str, class: QueueClass) -> Task {
        Task::new(title, TraceId::new(), class)
    }

    #[test]
    fn legal_transition_is_accepted() {
        let mut t = task("t", QueueClass::Active);
        assert!(t.transition_to(TaskStatus::Ready).is_ok());
        assert!(t.transition_to(TaskStatus::Active).is_ok());
        assert!(t.transition_to(TaskStatus::Running).is_ok());
        assert!(t.started_at.is_some());
    }

    #[test]
    fn illegal_transition_is_rejected() {
        let mut t = task("t", QueueClass::Active);
        // created -> running skips the queue and would hide a scheduling bug.
        let err = t.transition_to(TaskStatus::Running).unwrap_err();
        assert!(matches!(err, TypeError::IllegalTaskTransition { .. }));
        assert_eq!(t.status, TaskStatus::Created);
    }

    #[test]
    fn terminal_states_are_final() {
        let mut t = task("t", QueueClass::Active);
        t.transition_to(TaskStatus::Ready).unwrap();
        t.transition_to(TaskStatus::Cancelled).unwrap();
        assert!(t.completed_at.is_some());
        assert!(t.transition_to(TaskStatus::Ready).is_err());
    }

    #[test]
    fn success_clears_a_stale_error() {
        let mut t = task("t", QueueClass::Active);
        t.transition_to(TaskStatus::Ready).unwrap();
        t.transition_to(TaskStatus::Active).unwrap();
        t.transition_to(TaskStatus::Running).unwrap();
        t.error = Some(TaskError {
            code: "transient".into(),
            message: "a retryable blip".into(),
            retryable: true,
            occurred_at: Timestamp::now(),
        });
        t.transition_to(TaskStatus::Succeeded).unwrap();
        assert!(t.error.is_none());
        assert!(t.validate().is_ok());
    }

    #[test]
    fn validate_rejects_self_dependency() {
        let mut t = task("t", QueueClass::Active);
        t.depends_on.push(TaskDependency::on_success(t.task_id));
        assert!(t.validate().is_err());
    }

    #[test]
    fn validate_rejects_duplicate_dependencies() {
        let mut t = task("t", QueueClass::Active);
        let other = TaskId::new();
        t.depends_on.push(TaskDependency::on_success(other));
        t.depends_on.push(TaskDependency::on_success(other));
        assert!(t.validate().is_err());
    }

    #[test]
    fn validate_rejects_terminal_without_completion_time() {
        let mut t = task("t", QueueClass::Active);
        t.status = TaskStatus::Succeeded;
        assert!(t.validate().is_err());
    }

    #[test]
    fn active_preempts_idle_work() {
        assert!(QueueClass::Active.preempts(QueueClass::DeepIdle));
        assert!(QueueClass::Active.preempts(QueueClass::Background));
        assert!(QueueClass::Background.preempts(QueueClass::DeepIdle));
        assert!(!QueueClass::DeepIdle.preempts(QueueClass::Active));
        assert!(!QueueClass::Active.preempts(QueueClass::Active));
    }

    #[test]
    fn idle_classes_must_checkpoint() {
        assert!(!QueueClass::Active.must_checkpoint_before_yield());
        assert!(QueueClass::Background.must_checkpoint_before_yield());
        assert!(QueueClass::DeepIdle.must_checkpoint_before_yield());
    }

    #[test]
    fn dependency_waits_then_satisfies() {
        let mut graph = TaskGraph::new();
        let mut a = task("a", QueueClass::Active);
        let mut b = task("b", QueueClass::Active);
        b.depends_on.push(TaskDependency::on_success(a.task_id));
        b.transition_to(TaskStatus::Blocked).unwrap();
        let a_id = a.task_id;
        let b_id = b.task_id;
        graph.insert(a.clone()).unwrap();
        graph.insert(b).unwrap();

        assert_eq!(graph.dependency_verdict(&b_id), DependencyVerdict::Waiting);

        a.transition_to(TaskStatus::Ready).unwrap();
        a.transition_to(TaskStatus::Active).unwrap();
        a.transition_to(TaskStatus::Running).unwrap();
        a.transition_to(TaskStatus::Succeeded).unwrap();
        graph.insert(a).unwrap();
        assert_eq!(
            graph.dependency_verdict(&b_id),
            DependencyVerdict::Satisfied
        );
        assert_eq!(
            graph.dependency_verdict(&a_id),
            DependencyVerdict::Satisfied
        );
    }

    #[test]
    fn failed_dependency_is_unsatisfiable_not_merely_waiting() {
        let mut graph = TaskGraph::new();
        let mut a = task("a", QueueClass::Active);
        let mut b = task("b", QueueClass::Active);
        b.depends_on.push(TaskDependency::on_success(a.task_id));
        b.transition_to(TaskStatus::Blocked).unwrap();
        let b_id = b.task_id;
        a.transition_to(TaskStatus::Ready).unwrap();
        a.transition_to(TaskStatus::Cancelled).unwrap();
        graph.insert(a).unwrap();
        graph.insert(b).unwrap();
        assert_eq!(
            graph.dependency_verdict(&b_id),
            DependencyVerdict::Unsatisfiable
        );
    }

    #[test]
    fn missing_dependency_waits_rather_than_running() {
        let mut graph = TaskGraph::new();
        let mut b = task("b", QueueClass::Active);
        b.depends_on.push(TaskDependency::on_success(TaskId::new()));
        b.transition_to(TaskStatus::Blocked).unwrap();
        let b_id = b.task_id;
        graph.insert(b).unwrap();
        assert_eq!(graph.dependency_verdict(&b_id), DependencyVerdict::Waiting);
    }

    #[test]
    fn runnable_orders_active_before_idle_and_is_deterministic() {
        let mut graph = TaskGraph::new();
        for class in [
            QueueClass::DeepIdle,
            QueueClass::Active,
            QueueClass::Background,
        ] {
            let mut t = task("t", class);
            t.transition_to(TaskStatus::Ready).unwrap();
            graph.insert(t).unwrap();
        }
        let order: Vec<QueueClass> = graph.runnable().iter().map(|t| t.queue_class).collect();
        assert_eq!(
            order,
            vec![
                QueueClass::Active,
                QueueClass::Background,
                QueueClass::DeepIdle
            ]
        );
        // Determinism: repeated calls agree.
        let again: Vec<QueueClass> = graph.runnable().iter().map(|t| t.queue_class).collect();
        assert_eq!(order, again);
    }

    #[test]
    fn acyclic_graph_reports_no_cycle() {
        let mut graph = TaskGraph::new();
        let a = task("a", QueueClass::Active);
        let mut b = task("b", QueueClass::Active);
        b.depends_on.push(TaskDependency::on_success(a.task_id));
        graph.insert(a).unwrap();
        graph.insert(b).unwrap();
        assert!(graph.find_cycle().is_none());
    }

    #[test]
    fn cycle_is_detected() {
        let mut graph = TaskGraph::new();
        let mut a = task("a", QueueClass::Active);
        let mut b = task("b", QueueClass::Active);
        a.depends_on.push(TaskDependency::on_success(b.task_id));
        b.depends_on.push(TaskDependency::on_success(a.task_id));
        graph.insert(a).unwrap();
        graph.insert(b).unwrap();
        let cycle = graph.find_cycle().expect("cycle must be detected");
        assert!(cycle.len() >= 2);
    }

    #[test]
    fn deep_chain_does_not_overflow_the_stack() {
        // Iterative DFS is the point of this test: a long research chain is normal.
        let mut graph = TaskGraph::new();
        let mut previous: Option<TaskId> = None;
        for i in 0..5_000 {
            let mut t = task(&format!("t{i}"), QueueClass::DeepIdle);
            if let Some(prev) = previous {
                t.depends_on.push(TaskDependency::on_success(prev));
            }
            previous = Some(t.task_id);
            graph.insert(t).unwrap();
        }
        assert!(graph.find_cycle().is_none());
    }

    #[test]
    fn retry_requires_both_budget_and_a_retryable_error() {
        let mut t = task("t", QueueClass::Active);
        t.max_attempts = 3;
        t.attempt = 1;
        assert!(!t.may_retry(), "no error recorded yet");
        t.error = Some(TaskError {
            code: "x".into(),
            message: "permanent".into(),
            retryable: false,
            occurred_at: Timestamp::now(),
        });
        assert!(!t.may_retry());
        t.error = Some(TaskError {
            code: "x".into(),
            message: "transient".into(),
            retryable: true,
            occurred_at: Timestamp::now(),
        });
        assert!(t.may_retry());
        t.attempt = 3;
        assert!(!t.may_retry(), "attempt budget exhausted");
    }

    #[test]
    fn task_round_trips_through_json() {
        let mut t = task("round trip", QueueClass::Background);
        t.description = "desc".into();
        t.input.insert("k".into(), JsonValue::from(1));
        let json = serde_json::to_string(&t).unwrap();
        assert_eq!(serde_json::from_str::<Task>(&json).unwrap(), t);
    }
}
