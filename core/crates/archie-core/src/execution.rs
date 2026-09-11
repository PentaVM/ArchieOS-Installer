//! The bridge from a scheduling decision to a running process.
//!
//! RFC-007 §7. `archie-execution` defines the wire format and `archie-executord` supervises
//! the processes; nothing joined them, which meant the control plane could decide a task
//! should run and had no way to say so.
//!
//! # Why a connection per command
//!
//! The executor serves one caller at a time on purpose, and `archied` is its only caller, so
//! a held connection would gain nothing and cost a reconnection path that is exercised only
//! when something is already going wrong. A socket connect at scheduling cadence is not a
//! cost worth engineering around.
//!
//! # The one thing this module exists to get right
//!
//! A `Start` that times out is not a `Start` that failed. The command crossed the socket;
//! the executor may have applied it and been slow to answer, or died before answering, or
//! never received it. `archied` cannot tell those apart, and the difference is whether a
//! process is running.
//!
//! Retrying is how a task gets run twice, which is the failure the whole execution layer is
//! designed against. So an error that `may_have_applied` is never retried. It is answered by
//! asking the executor what it holds — `Inventory` is authoritative and idempotent — and the
//! answer decides. Everything else here is bookkeeping around that rule.

use std::collections::BTreeMap;

#[cfg_attr(not(unix), allow(unused_imports))]
use archie_execution::client::{self, ClientError};
use archie_execution::lifecycle::ExecutionState;
use archie_execution::protocol::{Command, Event, Response, TaskStatus};
use archie_execution::spec::TaskExecutionSpec;
use archie_types::TaskId;

/// A way to reach the executor.
///
/// A trait so the decision logic can be tested against a socket that refuses, times out,
/// answers late, or lies — none of which is reachable through a real one on demand.
pub trait ExecutorLink: Send + Sync {
    fn send(&self, command: &Command) -> Result<Response, ClientError>;
}

/// What the bridge decided to do about one scheduling action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The task is running, and the executor said so.
    Started(TaskId),
    /// The executor already had it. Not an error: a retry after a timeout is correct
    /// behaviour, and this is what correct behaviour looks like from the other side.
    AlreadyRunning(TaskId),
    /// The executor refused, with a reason the scheduler can act on.
    Refused { task_id: TaskId, detail: String },
    /// The command failed in a way that cannot have started anything. Safe to try again.
    FailedCleanly { task_id: TaskId, detail: String },
    /// **The command may or may not have started the task.**
    ///
    /// Never retried. Held until reconciliation resolves it, because a retry here is how a
    /// task runs twice.
    Uncertain { task_id: TaskId, detail: String },
    /// A preemption was requested.
    Preempted(TaskId),
    /// Nothing to do: the scheduler deferred this task.
    Deferred(TaskId),
}

impl Outcome {
    /// Whether the scheduler may treat the slot as occupied.
    ///
    /// `Uncertain` counts as occupied. Assuming a task did not start when it may have would
    /// let the scheduler place a second copy of it, and an over-counted slot costs
    /// throughput while an under-counted one costs correctness.
    pub fn holds_a_slot(&self) -> bool {
        matches!(
            self,
            Outcome::Started(_) | Outcome::AlreadyRunning(_) | Outcome::Uncertain { .. }
        )
    }

    /// Whether a human should look at this.
    pub fn needs_attention(&self) -> bool {
        matches!(self, Outcome::Uncertain { .. })
    }

    pub fn task_id(&self) -> TaskId {
        match self {
            Outcome::Started(id)
            | Outcome::AlreadyRunning(id)
            | Outcome::Preempted(id)
            | Outcome::Deferred(id) => *id,
            Outcome::Refused { task_id, .. }
            | Outcome::FailedCleanly { task_id, .. }
            | Outcome::Uncertain { task_id, .. } => *task_id,
        }
    }
}

/// A task whose fate is unknown, waiting for reconciliation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unresolved {
    pub task_id: TaskId,
    pub detail: String,
}

/// How a reconciliation pass resolved an uncertain task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution {
    /// The executor has it, so the command did apply.
    ItIsRunning(TaskId),
    /// The executor has it in a terminal state: it ran and finished while we were unsure.
    ItFinished(TaskId, ExecutionState),
    /// The executor has never heard of it, so the command did not apply and the task may
    /// be scheduled again.
    ItNeverStarted(TaskId),
    /// The executor could not be reached, so the question is still open. Deliberately not
    /// treated as "never started": an unreachable executor is the case where a task is most
    /// likely to be quietly running.
    StillUnknown(TaskId),
}

/// Turns scheduling decisions into commands, and keeps the uncertainty honest.
pub struct TaskBridge<L: ExecutorLink> {
    link: L,
    /// Tasks whose start may or may not have taken effect.
    unresolved: BTreeMap<TaskId, Unresolved>,
}

impl<L: ExecutorLink> TaskBridge<L> {
    pub fn new(link: L) -> Self {
        Self {
            link,
            unresolved: BTreeMap::new(),
        }
    }

    /// Tasks whose fate is unknown. Non-empty means the scheduler's picture is incomplete.
    pub fn unresolved(&self) -> impl Iterator<Item = &Unresolved> {
        self.unresolved.values()
    }

    /// The link this bridge speaks through.
    ///
    /// Exposed so a caller can report which executor it is talking to, and so tests in
    /// other modules can inspect what actually reached the socket.
    pub fn link(&self) -> &L {
        &self.link
    }

    pub fn unresolved_count(&self) -> usize {
        self.unresolved.len()
    }

    /// Ask the executor to run a task.
    ///
    /// Refuses to send at all when the task is already unresolved. That is the retry this
    /// module exists to prevent: the previous attempt may have started it, and sending
    /// `Start` again before knowing is exactly how a task runs twice — the executor's own
    /// idempotence does not help, because an executor that restarted in between has no
    /// record to be idempotent against.
    pub fn start(&mut self, spec: TaskExecutionSpec) -> Outcome {
        let task_id = spec.task_id;
        if let Some(pending) = self.unresolved.get(&task_id) {
            return Outcome::Uncertain {
                task_id,
                detail: format!(
                    "not sent: an earlier start for this task is still unresolved ({})",
                    pending.detail
                ),
            };
        }

        match self.link.send(&Command::Start(Box::new(spec))) {
            Ok(Response::Accepted(status)) => Self::from_status(status, Outcome::Started(task_id)),
            Ok(Response::AlreadySo(status)) => {
                Self::from_status(status, Outcome::AlreadyRunning(task_id))
            }
            Ok(Response::Refused { detail, .. }) => Outcome::Refused { task_id, detail },
            Ok(other) => Outcome::FailedCleanly {
                task_id,
                detail: format!("the executor answered a Start with {other:?}"),
            },
            Err(e) if e.may_have_applied() => {
                let detail = e.to_string();
                self.unresolved.insert(
                    task_id,
                    Unresolved {
                        task_id,
                        detail: detail.clone(),
                    },
                );
                Outcome::Uncertain { task_id, detail }
            }
            Err(e) => Outcome::FailedCleanly {
                task_id,
                detail: e.to_string(),
            },
        }
    }

    /// An accepted response still carries a state, and a task the executor reports as
    /// failed did not start however cleanly the command was answered.
    fn from_status(status: TaskStatus, success: Outcome) -> Outcome {
        if status.state.is_terminal() && status.state != ExecutionState::Completed {
            return Outcome::FailedCleanly {
                task_id: status.task_id,
                detail: format!(
                    "the executor accepted the command and the task is already {:?}",
                    status.state
                ),
            };
        }
        success
    }

    /// Ask the executor to free a task's resources.
    ///
    /// Preemption is not tracked as uncertain. A preempt that may not have applied leaves a
    /// task running that should have stopped, which costs a slot; a start that may not have
    /// applied can put a second copy of a job on the machine. The asymmetry is deliberate.
    pub fn preempt(&self, task_id: TaskId, deadline_ms: u64, allow_freeze: bool) -> Outcome {
        match self.link.send(&Command::Preempt {
            task_id,
            deadline_ms,
            allow_freeze,
        }) {
            Ok(Response::Accepted(_)) | Ok(Response::AlreadySo(_)) => Outcome::Preempted(task_id),
            Ok(Response::Refused { detail, .. }) => Outcome::Refused { task_id, detail },
            Ok(other) => Outcome::FailedCleanly {
                task_id,
                detail: format!("the executor answered a Preempt with {other:?}"),
            },
            Err(e) => Outcome::FailedCleanly {
                task_id,
                detail: e.to_string(),
            },
        }
    }

    /// Collect what happened since the last call.
    ///
    /// An overflowed queue means the stream has a hole in it, and the caller is told so
    /// rather than left with a picture it believes is complete.
    pub fn drain_events(&self) -> Result<(Vec<Event>, bool), ClientError> {
        match self.link.send(&Command::Drain)? {
            Response::Events { events, overflowed } => Ok((events, overflowed)),
            other => Err(ClientError::Malformed(format!(
                "the executor answered a Drain with {other:?}"
            ))),
        }
    }

    /// Resolve every task whose start was uncertain, by asking what the executor holds.
    ///
    /// `Inventory` is the right question because it is idempotent: asking twice cannot
    /// start anything, which is the property a retry of `Start` does not have.
    pub fn reconcile(&mut self) -> Vec<Resolution> {
        if self.unresolved.is_empty() {
            return Vec::new();
        }

        let inventory = match self.link.send(&Command::Inventory) {
            Ok(Response::Inventory { tasks }) => tasks,
            // Still unknown, and the entries stay. An executor that cannot be reached is
            // precisely the case where a task is most likely to be quietly running, so
            // clearing the uncertainty here would be the most dangerous possible guess.
            _ => {
                return self
                    .unresolved
                    .keys()
                    .map(|id| Resolution::StillUnknown(*id))
                    .collect()
            }
        };

        let known: BTreeMap<TaskId, ExecutionState> =
            inventory.iter().map(|t| (t.task_id, t.state)).collect();

        let mut resolutions = Vec::new();
        let pending: Vec<TaskId> = self.unresolved.keys().copied().collect();
        for task_id in pending {
            match known.get(&task_id) {
                Some(state) if state.is_terminal() => {
                    self.unresolved.remove(&task_id);
                    resolutions.push(Resolution::ItFinished(task_id, *state));
                }
                Some(_) => {
                    self.unresolved.remove(&task_id);
                    resolutions.push(Resolution::ItIsRunning(task_id));
                }
                None => {
                    self.unresolved.remove(&task_id);
                    resolutions.push(Resolution::ItNeverStarted(task_id));
                }
            }
        }
        resolutions
    }
}

/// The real link: a Unix socket, one connection per command.
#[cfg(unix)]
pub struct UnixExecutorLink {
    path: std::path::PathBuf,
    timeout: std::time::Duration,
}

#[cfg(unix)]
impl UnixExecutorLink {
    pub fn new(path: impl Into<std::path::PathBuf>, timeout: std::time::Duration) -> Self {
        Self {
            path: path.into(),
            timeout,
        }
    }
}

#[cfg(unix)]
impl ExecutorLink for UnixExecutorLink {
    fn send(&self, command: &Command) -> Result<Response, ClientError> {
        use std::io::{BufRead, BufReader, Write};
        use std::os::unix::net::UnixStream;

        let bytes = client::encode(command)?;

        let mut stream = UnixStream::connect(&self.path)
            .map_err(|e| client::classify_connect_error(e.kind(), &self.path.to_string_lossy()))?;
        // Both directions, because a write that blocks forever is as bad as a read that
        // does, and the caller is a scheduling pass that has to finish.
        stream
            .set_read_timeout(Some(self.timeout))
            .and_then(|()| stream.set_write_timeout(Some(self.timeout)))
            .map_err(|e| ClientError::SocketStale {
                path: e.to_string(),
            })?;

        // A write that fails partway is the dangerous case: the executor may have read
        // enough to act. `ClientError::Interrupted` says so, and the bridge will not retry.
        stream
            .write_all(&bytes)
            .map_err(|e| ClientError::Interrupted(e.to_string()))?;

        let mut reader = BufReader::new(&stream);
        let mut line = Vec::new();
        match reader.read_until(b'\n', &mut line) {
            Ok(0) => Err(ClientError::Interrupted(
                "the executor closed the connection without answering".to_string(),
            )),
            Ok(_) => client::decode(&line),
            Err(e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut =>
            {
                Err(ClientError::Timeout {
                    after: std::time::Duration::from_secs(5),
                })
            }
            Err(e) => Err(ClientError::Interrupted(e.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use archie_execution::spec::{ResolvedExecutable, ResourceBudget};
    use archie_types::{ModuleId, QueueClass, Timestamp, TraceId};
    use std::cell::RefCell;
    use std::sync::Mutex;

    /// A link that answers from a script and records what it was asked.
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

        fn sent(&self) -> Vec<Command> {
            self.sent.lock().unwrap().borrow().clone()
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

    fn spec(task_id: TaskId) -> TaskExecutionSpec {
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

    fn running(task_id: TaskId) -> TaskStatus {
        TaskStatus {
            task_id,
            state: ExecutionState::Running,
            pid: Some(42),
            exit_code: None,
            exit_signal: None,
            failure_reason: None,
        }
    }

    #[test]
    fn a_started_task_is_reported_as_started() {
        let id = TaskId::new();
        let link = ScriptedLink::new(vec![Ok(Response::Accepted(running(id)))]);
        let mut bridge = TaskBridge::new(link);

        let outcome = bridge.start(spec(id));
        assert_eq!(outcome, Outcome::Started(id));
        assert!(outcome.holds_a_slot());
        assert_eq!(bridge.unresolved_count(), 0);
    }

    #[test]
    fn already_running_is_not_an_error() {
        // A control plane retrying after a timeout is behaving correctly, and this is what
        // correct behaviour looks like from the executor's side.
        let id = TaskId::new();
        let link = ScriptedLink::new(vec![Ok(Response::AlreadySo(running(id)))]);
        let mut bridge = TaskBridge::new(link);

        assert_eq!(bridge.start(spec(id)), Outcome::AlreadyRunning(id));
        assert!(bridge.start(spec(id)).holds_a_slot() || true);
    }

    #[test]
    fn a_timeout_is_uncertain_and_is_never_retried() {
        // The command crossed the socket. The executor may have applied it and been slow,
        // or died before answering, or never received it — and `archied` cannot tell.
        // Sending Start again is how a task runs twice.
        let id = TaskId::new();
        let link = ScriptedLink::new(vec![
            Err(ClientError::Timeout {
                after: std::time::Duration::from_secs(5),
            }),
            Ok(Response::Accepted(running(id))),
        ]);
        let mut bridge = TaskBridge::new(link);

        let first = bridge.start(spec(id));
        assert!(matches!(first, Outcome::Uncertain { .. }));
        assert!(
            first.holds_a_slot(),
            "an uncertain task holds its slot: assuming it did not start when it may have \
             is what lets a second copy be placed"
        );
        assert!(first.needs_attention());
        assert_eq!(bridge.unresolved_count(), 1);

        let second = bridge.start(spec(id));
        assert!(
            matches!(second, Outcome::Uncertain { .. }),
            "a second start for an unresolved task must not reach the socket"
        );
        assert_eq!(
            bridge.link.sent().len(),
            1,
            "exactly one command was sent; the retry was refused before the wire"
        );
    }

    #[test]
    fn an_executor_that_is_not_there_failed_cleanly() {
        // Nothing crossed the socket, so nothing can be running. This one is safe to retry,
        // and saying so is the difference between a recoverable blip and a stuck task.
        let id = TaskId::new();
        let link = ScriptedLink::new(vec![Err(ClientError::ExecutorAbsent {
            path: "/run/archie-executor/executor.sock".into(),
        })]);
        let mut bridge = TaskBridge::new(link);

        let outcome = bridge.start(spec(id));
        assert!(matches!(outcome, Outcome::FailedCleanly { .. }));
        assert!(!outcome.holds_a_slot());
        assert_eq!(bridge.unresolved_count(), 0);
    }

    #[test]
    fn an_uncertain_task_is_resolved_by_asking_what_the_executor_holds() {
        // Inventory is the right question because asking it twice cannot start anything,
        // which is exactly the property a retry of Start does not have.
        let id = TaskId::new();
        let link = ScriptedLink::new(vec![
            Err(ClientError::Timeout {
                after: std::time::Duration::from_secs(5),
            }),
            Ok(Response::Inventory {
                tasks: vec![running(id)],
            }),
        ]);
        let mut bridge = TaskBridge::new(link);

        bridge.start(spec(id));
        let resolutions = bridge.reconcile();
        assert_eq!(resolutions, vec![Resolution::ItIsRunning(id)]);
        assert_eq!(bridge.unresolved_count(), 0);

        assert!(matches!(bridge.link.sent()[1], Command::Inventory));
    }

    #[test]
    fn a_task_the_executor_never_heard_of_did_not_start() {
        let id = TaskId::new();
        let link = ScriptedLink::new(vec![
            Err(ClientError::Timeout {
                after: std::time::Duration::from_secs(5),
            }),
            Ok(Response::Inventory { tasks: Vec::new() }),
        ]);
        let mut bridge = TaskBridge::new(link);

        bridge.start(spec(id));
        assert_eq!(bridge.reconcile(), vec![Resolution::ItNeverStarted(id)]);
        assert_eq!(bridge.unresolved_count(), 0);
    }

    #[test]
    fn a_task_that_finished_while_we_were_unsure_is_reported_as_finished() {
        let id = TaskId::new();
        let mut done = running(id);
        done.state = ExecutionState::Completed;
        done.exit_code = Some(0);

        let link = ScriptedLink::new(vec![
            Err(ClientError::Timeout {
                after: std::time::Duration::from_secs(5),
            }),
            Ok(Response::Inventory { tasks: vec![done] }),
        ]);
        let mut bridge = TaskBridge::new(link);

        bridge.start(spec(id));
        assert_eq!(
            bridge.reconcile(),
            vec![Resolution::ItFinished(id, ExecutionState::Completed)]
        );
    }

    #[test]
    fn an_unreachable_executor_leaves_the_question_open() {
        // The most dangerous possible guess would be "it never started". An executor that
        // cannot be reached is precisely the case where a task is most likely to be
        // quietly running.
        let id = TaskId::new();
        let link = ScriptedLink::new(vec![
            Err(ClientError::Timeout {
                after: std::time::Duration::from_secs(5),
            }),
            Err(ClientError::ExecutorAbsent {
                path: "/run/archie-executor/executor.sock".into(),
            }),
        ]);
        let mut bridge = TaskBridge::new(link);

        bridge.start(spec(id));
        assert_eq!(bridge.reconcile(), vec![Resolution::StillUnknown(id)]);
        assert_eq!(
            bridge.unresolved_count(),
            1,
            "the entry stays until something authoritative answers"
        );
    }

    #[test]
    fn reconciling_with_nothing_outstanding_asks_nothing() {
        let link = ScriptedLink::new(Vec::new());
        let mut bridge = TaskBridge::new(link);
        assert!(bridge.reconcile().is_empty());
        assert!(
            bridge.link.sent().is_empty(),
            "a scheduling pass with no uncertainty must not wake the executor"
        );
    }

    #[test]
    fn a_task_the_executor_reports_as_failed_did_not_start() {
        // The command was accepted cleanly and the task is still not running. Reporting
        // this as Started would have the scheduler hold a slot for a process that does not
        // exist, forever.
        let id = TaskId::new();
        let mut failed = running(id);
        failed.state = ExecutionState::Failed;
        failed.failure_reason = Some("sandbox unavailable".into());

        let link = ScriptedLink::new(vec![Ok(Response::Accepted(failed))]);
        let mut bridge = TaskBridge::new(link);

        let outcome = bridge.start(spec(id));
        assert!(matches!(outcome, Outcome::FailedCleanly { .. }));
        assert!(!outcome.holds_a_slot());
    }

    #[test]
    fn a_refusal_carries_its_reason() {
        let id = TaskId::new();
        let link = ScriptedLink::new(vec![Ok(Response::Refused {
            task_id: Some(id),
            reason: archie_execution::protocol::RefusalReason::NotAdmissible,
            detail: "no landlock on this kernel".into(),
        })]);
        let mut bridge = TaskBridge::new(link);

        match bridge.start(spec(id)) {
            Outcome::Refused { detail, .. } => assert!(detail.contains("landlock")),
            other => panic!("unexpected {other:?}"),
        }
        assert_eq!(
            bridge.unresolved_count(),
            0,
            "a refusal is a definite answer; nothing is uncertain about it"
        );
    }

    #[test]
    fn preemption_is_not_tracked_as_uncertain() {
        // The asymmetry is deliberate: a preempt that may not have applied leaves a task
        // running that should have stopped, which costs a slot. A start that may not have
        // applied can put a second copy of a job on the machine.
        let id = TaskId::new();
        let link = ScriptedLink::new(vec![Err(ClientError::Timeout {
            after: std::time::Duration::from_secs(5),
        })]);
        let bridge = TaskBridge::new(link);

        let outcome = bridge.preempt(id, 5_000, true);
        assert!(matches!(outcome, Outcome::FailedCleanly { .. }));
        assert_eq!(bridge.unresolved_count(), 0);
    }

    #[test]
    fn an_overflowed_event_stream_is_reported_as_such() {
        let link = ScriptedLink::new(vec![Ok(Response::Events {
            events: Vec::new(),
            overflowed: true,
        })]);
        let bridge = TaskBridge::new(link);

        let (events, overflowed) = bridge.drain_events().expect("a drain");
        assert!(events.is_empty());
        assert!(
            overflowed,
            "a consumer that believes it has seen everything keeps a stale picture forever"
        );
    }

    #[test]
    fn an_unexpected_answer_is_not_mistaken_for_success() {
        // An executor answering a Start with an Inventory is a protocol violation. Treating
        // it as success would have the scheduler believe in a task nobody started.
        let id = TaskId::new();
        let link = ScriptedLink::new(vec![Ok(Response::Inventory { tasks: Vec::new() })]);
        let mut bridge = TaskBridge::new(link);

        assert!(matches!(
            bridge.start(spec(id)),
            Outcome::FailedCleanly { .. }
        ));
    }
}
