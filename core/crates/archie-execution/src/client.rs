//! The control plane's side of the executor socket.
//!
//! RFC-007 §7. `archied` speaks; `archie-executord` answers.
//!
//! # An absent executor is not a failed command
//!
//! The two daemons restart independently. `archied` will, routinely, try to talk to an
//! executor that is starting, restarting, or not installed — and each of those is a
//! different situation:
//!
//! ```text
//! socket missing       the executor is not running (or not installed)
//! connection refused   the socket is stale; the executor died without cleaning up
//! timeout              it is alive and not answering
//! refusal              it answered, and said no
//! ```
//!
//! Collapsing them into "the command failed" would have the scheduler treat a node whose
//! executor is restarting exactly like a node that refused the work — retrying a task
//! forever against a socket that does not exist, or worse, placing it elsewhere and then
//! finding it running here after all.
//!
//! # Every call has a deadline
//!
//! There is no untimed read. An executor that accepts a connection and then stops
//! answering must not be able to hold the control plane's thread indefinitely; the
//! scheduler has other nodes to place work on, and a hung call is the one failure that
//! spreads.

use std::time::Duration;

use crate::protocol::{Command, Response};

/// How long to wait for a reply before giving up on it.
///
/// Generous for a command the executor answers from memory, short enough that a hung
/// executor is noticed within one scheduling pass rather than several.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);

/// The largest response the client will read.
///
/// An `Inventory` on a busy node is the big one, and it is bounded by the number of tasks
/// rather than by anything a worker controls. The limit exists so a malfunctioning
/// executor cannot make the *control plane* allocate without bound — the direction of
/// trust here runs both ways.
pub const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;

/// Why a command did not produce an answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientError {
    /// The socket does not exist. The executor is not running, or not installed.
    ///
    /// Deliberately distinct from every other variant: a node in this state has not
    /// refused anything, and treating it as a refusal would strand work that simply needs
    /// to wait.
    ExecutorAbsent { path: String },
    /// The socket exists but nothing is listening — the executor died without removing
    /// it. Almost always transient, and it is worth saying which kind of "not there" this
    /// is, because a stale socket means something crashed.
    SocketStale { path: String },
    /// It accepted the connection and did not answer in time.
    Timeout { after: Duration },
    /// The command could not be turned into bytes, so nothing left this process.
    ///
    /// Kept apart from `Malformed` because the two have opposite safety properties and
    /// were previously the same variant: a command that could not be encoded provably did
    /// not run, while a reply that could not be decoded means the executor answered, which
    /// means it processed the command.
    Unsendable(String),
    /// The reply was not a response, or was too large.
    ///
    /// The executor answered. Whatever it said, it had the command.
    Malformed(String),
    /// The connection broke mid-exchange. The command may well have run: the executor
    /// applies before it answers, so this is explicitly *not* a statement that nothing
    /// happened.
    Interrupted(String),
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ExecutorAbsent { path } => write!(
                f,
                "no executor socket at {path}; this node is not running an executor, so \
                 it has not refused anything"
            ),
            Self::SocketStale { path } => write!(
                f,
                "the socket at {path} exists but nothing is listening; the executor died \
                 without cleaning up"
            ),
            Self::Timeout { after } => write!(
                f,
                "the executor accepted the connection and did not answer within {after:?};                  the command was already written, so it may have run"
            ),
            Self::Unsendable(detail) => write!(
                f,
                "the command could not be encoded ({detail}); nothing left this process"
            ),
            Self::Malformed(detail) => write!(f, "the executor's reply did not decode: {detail}"),
            Self::Interrupted(detail) => write!(
                f,
                "the connection broke mid-exchange ({detail}); the command may have run, \
                 because the executor applies before it answers"
            ),
        }
    }
}

impl std::error::Error for ClientError {}

impl ClientError {
    /// Whether this node should simply be waited for rather than worked around.
    ///
    /// An absent or stale socket means the executor is between lives. The scheduler
    /// should hold the task, not place it elsewhere — placing it elsewhere and then
    /// finding the original executor come back with the task already running is the
    /// duplicate-execution bug arriving by a different road.
    pub fn is_transient(&self) -> bool {
        matches!(
            self,
            Self::ExecutorAbsent { .. } | Self::SocketStale { .. } | Self::Timeout { .. }
        )
    }

    /// Whether the command might have taken effect despite the error.
    ///
    /// True whenever the bytes reached the executor, because it applies a command and then
    /// answers: anything that goes wrong after the write leaves the caller unable to tell
    /// whether the apply happened. The correct recovery is to ask — `Inventory` exists for
    /// this — rather than to resend and risk doing it twice.
    ///
    /// `Timeout` is in this set, and an earlier version of this function had it outside.
    /// That was wrong in the most expensive direction. A timeout happens *after* the
    /// command has been written to the socket: the executor may have read it, applied it,
    /// and been slow to answer, or be blocked behind a reaping pass with the command
    /// already in its buffer. Treating that as "nothing happened" is what makes a
    /// scheduler resend a `Start`, and resending a `Start` is how a task runs twice.
    ///
    /// `Malformed` is in it too, for the same reason: a reply that could not be parsed is
    /// still a reply, and the executor does not answer commands it has not processed.
    pub fn may_have_applied(&self) -> bool {
        matches!(
            self,
            Self::Interrupted(_) | Self::Timeout { .. } | Self::Malformed(_)
        )
    }
}

/// Encode a command as one line, ready for the socket.
pub fn encode(command: &Command) -> Result<Vec<u8>, ClientError> {
    let mut line = serde_json::to_vec(command)
        .map_err(|e| ClientError::Unsendable(format!("cannot encode command: {e}")))?;
    line.push(b'\n');
    Ok(line)
}

/// Decode one line of reply.
///
/// The size check precedes the parse for the same reason it does on the executor's side:
/// deciding a reply was too large after parsing it is not a limit.
pub fn decode(line: &[u8]) -> Result<Response, ClientError> {
    if line.len() > MAX_RESPONSE_BYTES {
        return Err(ClientError::Malformed(format!(
            "reply of {} bytes exceeds the {MAX_RESPONSE_BYTES}-byte limit",
            line.len()
        )));
    }
    if line.iter().all(|b| b.is_ascii_whitespace()) {
        // A closed connection reads as end-of-input, which is not an empty response — it
        // is the executor having gone away mid-exchange, and the command may have run.
        return Err(ClientError::Interrupted(
            "the executor closed the connection without answering".into(),
        ));
    }
    serde_json::from_slice(line).map_err(|e| ClientError::Malformed(e.to_string()))
}

/// Classify a connection failure.
///
/// Split out so the mapping is testable without a socket: the part that can be got wrong
/// is which OS error means "not running" versus "died badly", and those lead the
/// scheduler to different places.
pub fn classify_connect_error(kind: std::io::ErrorKind, path: &str) -> ClientError {
    match kind {
        std::io::ErrorKind::NotFound => ClientError::ExecutorAbsent {
            path: path.to_string(),
        },
        std::io::ErrorKind::ConnectionRefused => ClientError::SocketStale {
            path: path.to_string(),
        },
        // A permission failure is not transient and is not the executor's fault: it means
        // archied is not in the group the socket was created for, which no amount of
        // waiting fixes.
        std::io::ErrorKind::PermissionDenied => ClientError::Malformed(format!(
            "cannot open {path}: permission denied. archied must share the executor's \
             group for the socket's mode to admit it"
        )),
        other => ClientError::Interrupted(format!("connecting to {path}: {other:?}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lifecycle::ExecutionState;
    use crate::protocol::{RefusalReason, TaskStatus};
    use archie_types::TaskId;

    #[test]
    fn an_absent_executor_is_not_a_refusal() {
        // A node whose executor is not running has not refused anything. Treating it as a
        // refusal would strand work that only needs to wait.
        let absent = classify_connect_error(std::io::ErrorKind::NotFound, "/run/x/e.sock");
        assert_eq!(
            absent,
            ClientError::ExecutorAbsent {
                path: "/run/x/e.sock".into()
            }
        );
        assert!(absent.is_transient());
        assert!(!absent.may_have_applied());
        assert!(absent.to_string().contains("has not refused anything"));
    }

    #[test]
    fn a_stale_socket_says_something_crashed() {
        // Both are "not there", and only one of them means an executor died badly.
        let stale = classify_connect_error(std::io::ErrorKind::ConnectionRefused, "/run/x/e.sock");
        assert_eq!(
            stale,
            ClientError::SocketStale {
                path: "/run/x/e.sock".into()
            }
        );
        assert!(stale.is_transient());
        assert!(stale.to_string().contains("died without cleaning up"));
        assert_ne!(
            stale,
            classify_connect_error(std::io::ErrorKind::NotFound, "/run/x/e.sock")
        );
    }

    #[test]
    fn a_permission_failure_is_not_something_to_wait_out() {
        // archied is not in the executor's group. No amount of retrying fixes that, and
        // the message says what would.
        let denied = classify_connect_error(std::io::ErrorKind::PermissionDenied, "/run/x/e.sock");
        assert!(!denied.is_transient());
        assert!(denied.to_string().contains("share the executor's group"));
    }

    #[test]
    fn a_broken_connection_does_not_claim_nothing_happened() {
        // The executor applies and then answers, so a connection lost between the two
        // leaves the caller unable to tell. Resending would risk running it twice; the
        // correct recovery is to ask.
        let interrupted = ClientError::Interrupted("reset by peer".into());
        assert!(interrupted.may_have_applied());
        assert!(!interrupted.is_transient());
        assert!(interrupted
            .to_string()
            .contains("applies before it answers"));
    }

    #[test]
    fn a_closed_connection_reads_as_interrupted_not_as_an_empty_reply() {
        for empty in [&b""[..], b"  ", b"\n"] {
            let err = decode(empty).unwrap_err();
            assert!(
                err.may_have_applied(),
                "{empty:?} must not be mistaken for a reply meaning nothing happened"
            );
        }
    }

    #[test]
    fn a_timeout_is_worth_waiting_out_but_says_the_executor_was_alive() {
        let timeout = ClientError::Timeout {
            after: DEFAULT_TIMEOUT,
        };
        assert!(timeout.is_transient());
        assert!(
            timeout.may_have_applied(),
            "a timeout happens after the command was written: the executor may have read              it, applied it, and been slow to answer"
        );
        assert!(timeout.to_string().contains("accepted the connection"));
    }

    #[test]
    fn a_command_encodes_to_exactly_one_line() {
        let command = Command::Stop {
            task_id: TaskId::new(),
            grace_ms: 5_000,
        };
        let line = encode(&command).unwrap();

        assert_eq!(*line.last().unwrap(), b'\n');
        assert_eq!(
            line.iter().filter(|b| **b == b'\n').count(),
            1,
            "a command containing a newline would be read as two"
        );
        assert_eq!(
            serde_json::from_slice::<Command>(&line[..line.len() - 1]).unwrap(),
            command
        );
    }

    #[test]
    fn a_response_round_trips() {
        let response = Response::Refused {
            task_id: Some(TaskId::new()),
            reason: RefusalReason::NotAdmissible,
            detail: "no delegated cgroup subtree".into(),
        };
        let line = serde_json::to_vec(&response).unwrap();
        assert_eq!(decode(&line).unwrap(), response);
    }

    #[test]
    fn an_oversized_reply_is_refused_before_it_is_parsed() {
        // The direction of trust runs both ways: a malfunctioning executor must not be
        // able to make the control plane allocate without bound either.
        let huge = vec![b'x'; MAX_RESPONSE_BYTES + 1];
        let err = decode(&huge).unwrap_err();
        assert!(matches!(err, ClientError::Malformed(_)));
        assert!(err.to_string().contains("exceeds"));
    }

    #[test]
    fn a_large_but_legitimate_inventory_still_decodes() {
        // The bound has to clear the real worst case, or it becomes an outage on a busy
        // node rather than a safeguard.
        let tasks: Vec<TaskStatus> = (0..2_000)
            .map(|i| TaskStatus {
                task_id: TaskId::new(),
                state: ExecutionState::Running,
                pid: Some(i),
                exit_code: None,
                exit_signal: None,
                failure_reason: None,
            })
            .collect();
        let line = serde_json::to_vec(&Response::Inventory { tasks }).unwrap();

        assert!(line.len() < MAX_RESPONSE_BYTES);
        match decode(&line).unwrap() {
            Response::Inventory { tasks } => assert_eq!(tasks.len(), 2_000),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn only_an_interrupted_exchange_leaves_the_outcome_unknown() {
        // The predicate a caller uses to decide between asking and resending.
        let unknown = ClientError::Interrupted("x".into());
        // The line is whether the bytes reached the executor, not whether the exchange
        // looked tidy. Everything below failed before the command was written.
        let known_not_to_have_run = [
            ClientError::ExecutorAbsent { path: "p".into() },
            ClientError::SocketStale { path: "p".into() },
            ClientError::Unsendable("cannot encode".into()),
        ];
        let unknown_too = [
            ClientError::Timeout {
                after: DEFAULT_TIMEOUT,
            },
            ClientError::Malformed("x".into()),
        ];
        assert!(unknown.may_have_applied());
        for error in known_not_to_have_run {
            assert!(!error.may_have_applied(), "{error:?}");
        }
        for error in unknown_too {
            assert!(
                error.may_have_applied(),
                "{error:?} happens after the write; the executor may already have acted"
            );
        }
    }
}
