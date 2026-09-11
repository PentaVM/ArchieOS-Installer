//! The control socket: how `archied` reaches the executor.
//!
//! RFC-007 §7. Local only, and deliberately unable to be anything else.
//!
//! # Authentication is the filesystem
//!
//! The socket lives in the executor's runtime directory with mode `0660` and group
//! ownership shared with `archied`. Anyone who can open it is already someone who can
//! read that directory, and on a node where that is true the game is over for reasons
//! this socket cannot help with.
//!
//! There is no token, no key, and no TCP listener. A token would have to be stored
//! somewhere both processes can read — which is the same filesystem check, plus a secret
//! to leak. A TCP listener would turn a local privilege boundary into a network one, and
//! this process can create cgroups and spawn processes.
//!
//! # One command per line, and a bounded one
//!
//! The framing is newline-delimited JSON with a hard cap on line length. A control plane
//! that has gone wrong and is sending an unbounded line must not be able to make the
//! executor allocate until the OOM killer picks between them — and if it does pick, it
//! will pick the executor, because the executor is the one that grew.

use archie_execution::protocol::{Command, RefusalReason, Response};

/// The largest command the executor will read.
///
/// Generous for a `TaskExecutionSpec` with a long argument list and an environment
/// allowlist, and far below anything that threatens the process. A control plane sending
/// more than this is malfunctioning, and the honest response is to say so rather than to
/// keep reading.
pub const MAX_COMMAND_BYTES: usize = 256 * 1024;

/// Why a line could not be turned into a command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameError {
    /// The line exceeded [`MAX_COMMAND_BYTES`].
    TooLarge { bytes: usize },
    /// The line was not valid JSON, or not a command.
    Malformed(String),
}

impl std::fmt::Display for FrameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooLarge { bytes } => write!(
                f,
                "command of {bytes} bytes exceeds the {MAX_COMMAND_BYTES}-byte limit; a \
                 control plane sending more than this is malfunctioning"
            ),
            Self::Malformed(detail) => write!(f, "could not decode command: {detail}"),
        }
    }
}

impl std::error::Error for FrameError {}

/// Decode one line into a command.
///
/// Separated from the socket so the framing rules are testable without one, and so the
/// size check provably happens before the parse — reading a 200 MB line and *then*
/// deciding it was too long is not a limit.
pub fn decode(line: &[u8]) -> Result<Command, FrameError> {
    if line.len() > MAX_COMMAND_BYTES {
        return Err(FrameError::TooLarge { bytes: line.len() });
    }
    serde_json::from_slice(line).map_err(|e| FrameError::Malformed(e.to_string()))
}

/// Encode a response as one line.
///
/// Falls back to a hand-built refusal if serialization fails. A response that cannot be
/// encoded would otherwise leave the control plane waiting forever on a command it
/// believes is still running, which is worse than any message.
pub fn encode(response: &Response) -> Vec<u8> {
    let mut line = serde_json::to_vec(response).unwrap_or_else(|e| {
        serde_json::to_vec(&Response::Refused {
            task_id: None,
            reason: RefusalReason::NotApplicable,
            detail: format!("the executor could not encode its own response: {e}"),
        })
        .unwrap_or_else(|_| br#"{"response":"refused"}"#.to_vec())
    });
    line.push(b'\n');
    line
}

/// The refusal sent when a line could not be decoded.
pub fn refusal_for(error: &FrameError) -> Response {
    Response::Refused {
        task_id: None,
        reason: RefusalReason::InvalidSpec,
        detail: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use archie_execution::lifecycle::ExecutionState;
    use archie_execution::protocol::TaskStatus;
    use archie_types::TaskId;

    #[test]
    fn a_command_round_trips_through_a_line() {
        let command = Command::Stop {
            task_id: TaskId::new(),
            grace_ms: 10_000,
        };
        let line = serde_json::to_vec(&command).unwrap();
        assert_eq!(decode(&line).unwrap(), command);
    }

    #[test]
    fn an_oversized_line_is_refused_before_it_is_parsed() {
        // Reading a 200 MB line and then deciding it was too long is not a limit. The
        // check is first so a malfunctioning control plane cannot make the executor
        // allocate until the OOM killer has to choose between them — and it would choose
        // the executor, because the executor is the one that grew.
        let huge = vec![b'x'; MAX_COMMAND_BYTES + 1];
        let err = decode(&huge).unwrap_err();
        assert_eq!(
            err,
            FrameError::TooLarge {
                bytes: MAX_COMMAND_BYTES + 1
            }
        );
        assert!(err.to_string().contains("malfunctioning"));
    }

    #[test]
    fn a_line_at_exactly_the_limit_is_still_read() {
        // The boundary belongs to the caller: they were told the limit, and a command of
        // exactly that size honoured it.
        let at_limit = vec![b'x'; MAX_COMMAND_BYTES];
        assert!(matches!(decode(&at_limit), Err(FrameError::Malformed(_)),));
    }

    #[test]
    fn garbage_is_a_refusal_not_a_panic() {
        for bad in [&b""[..], b"{", b"null", b"[]", b"{\"command\":\"fly\"}"] {
            let err = decode(bad).unwrap_err();
            assert!(matches!(err, FrameError::Malformed(_)), "{bad:?}");
            // And it becomes a response the control plane can act on.
            let response = refusal_for(&err);
            match response {
                Response::Refused { reason, .. } => {
                    assert_eq!(reason, RefusalReason::InvalidSpec)
                }
                other => panic!("unexpected {other:?}"),
            }
        }
    }

    #[test]
    fn a_response_is_one_line_and_ends_in_a_newline() {
        // The framing is the contract; a response without its terminator would hang the
        // reader on the other side.
        let response = Response::Accepted(TaskStatus {
            task_id: TaskId::new(),
            state: ExecutionState::Running,
            pid: Some(42),
            exit_code: None,
            exit_signal: None,
            failure_reason: None,
        });
        let line = encode(&response);

        assert_eq!(*line.last().unwrap(), b'\n');
        assert_eq!(
            line.iter().filter(|b| **b == b'\n').count(),
            1,
            "a response containing a newline would be read as two"
        );
        assert_eq!(
            serde_json::from_slice::<Response>(&line[..line.len() - 1]).unwrap(),
            response
        );
    }

    #[test]
    fn an_inventory_of_many_tasks_still_encodes_as_one_line() {
        let tasks: Vec<TaskStatus> = (0..200)
            .map(|i| TaskStatus {
                task_id: TaskId::new(),
                state: ExecutionState::Running,
                pid: Some(i),
                exit_code: None,
                exit_signal: None,
                failure_reason: Some("a reason with \"quotes\" and \n a newline".into()),
            })
            .collect();

        let line = encode(&Response::Inventory { tasks });
        assert_eq!(
            line.iter().filter(|b| **b == b'\n').count(),
            1,
            "a failure reason containing a newline must be escaped, not framed"
        );
    }
}
