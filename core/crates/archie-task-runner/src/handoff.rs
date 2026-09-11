//! How the executor hands a task to the runner.
//!
//! RFC-007 §7 step 7.
//!
//! # Not argv, and not a file
//!
//! A `TaskExecutionSpec` on the command line would be visible in `/proc/*/cmdline` to
//! every process on the machine, including the workers this system is confining. It would
//! also mean the environment allowlist — the names of the variables a task is trusted
//! with — is world-readable.
//!
//! A file would need somewhere to live, permissions to get right, and cleanup that
//! survives a crash. It would also be readable for as long as it existed, by anything
//! that could reach the directory.
//!
//! So the spec arrives on **stdin**, which exists only between these two processes, dies
//! with them, and cannot be listed by a third. The runner reads it to end-of-file and
//! then replaces stdin with `/dev/null` before exec — a worker inheriting a pipe the
//! executor still holds is a channel nobody designed and nobody is watching.
//!
//! # A bounded read, before anything is parsed
//!
//! The runner has the executor's privileges until it drops them. An unbounded read here
//! would let a malfunctioning executor make this process allocate without limit, in the
//! window where it is most privileged.

use archie_execution::spec::TaskExecutionSpec;
use serde::{Deserialize, Serialize};

/// The largest spec the runner will read.
///
/// Generous for a spec with a long argument list and many grants; far below anything that
/// threatens a process. Deliberately the same order as the executor's own command limit,
/// because a spec that fits through that socket must fit through this pipe.
pub const MAX_SPEC_BYTES: usize = 256 * 1024;

/// Everything the executor tells the runner.
///
/// The spec is the *task's* contract — what it may do, and what it requires — and it is
/// the same on every node. The cgroup is *this node's* mechanism, chosen by this
/// executor, and it does not belong in a document the scheduler and the registry also
/// read. So they travel together and stay separate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Handoff {
    pub spec: TaskExecutionSpec,
    /// The absolute path of the cgroup this task must join, when the node has one.
    ///
    /// `None` on a node with no delegation. That is not a failure: the supervisor has
    /// already refused anything whose manifest required enforced limits, so a task
    /// arriving without a cgroup is one that never asked for one.
    #[serde(default)]
    pub cgroup_procs: Option<String>,
}

/// Why the handoff failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HandoffError {
    /// The spec exceeded [`MAX_SPEC_BYTES`].
    TooLarge { bytes: usize },
    /// Nothing arrived. The executor spawned this program and then said nothing, which
    /// means the two are out of step rather than that a task was malformed.
    Empty,
    /// The bytes were not a spec.
    Malformed(String),
}

impl std::fmt::Display for HandoffError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooLarge { bytes } => write!(
                f,
                "the spec is {bytes} bytes, over the {MAX_SPEC_BYTES}-byte limit; the \
                 runner holds the executor's privileges until it drops them, so this read \
                 is bounded"
            ),
            Self::Empty => f.write_str(
                "the executor spawned this runner and sent no spec; the two are out of \
                 step",
            ),
            Self::Malformed(detail) => write!(f, "the spec did not decode: {detail}"),
        }
    }
}

impl std::error::Error for HandoffError {}

/// Decode a spec from the bytes the executor sent.
///
/// Split from the reading so the size rule and the decode rule are both testable without
/// a pipe, and so the size check provably happens first.
pub fn decode(bytes: &[u8]) -> Result<Handoff, HandoffError> {
    if bytes.len() > MAX_SPEC_BYTES {
        return Err(HandoffError::TooLarge { bytes: bytes.len() });
    }
    if bytes.iter().all(|b| b.is_ascii_whitespace()) {
        return Err(HandoffError::Empty);
    }
    serde_json::from_slice(bytes).map_err(|e| HandoffError::Malformed(e.to_string()))
}

/// Read the spec the executor wrote to this process's stdin.
///
/// Bounded: reads at most one byte past the limit, so an unbounded sender is stopped
/// rather than followed. Returns [`HandoffError::TooLarge`] rather than a truncated spec,
/// because a spec missing its last grant would confine a worker to less than it was
/// meant to have and look entirely valid doing it.
pub fn read_from<R: std::io::Read>(mut reader: R) -> Result<Handoff, HandoffError> {
    use std::io::Read;

    let mut buffer = Vec::new();
    let read = (&mut reader)
        .take(MAX_SPEC_BYTES as u64 + 1)
        .read_to_end(&mut buffer)
        .map_err(|e| HandoffError::Malformed(e.to_string()))?;

    if read > MAX_SPEC_BYTES {
        return Err(HandoffError::TooLarge { bytes: read });
    }
    decode(&buffer)
}

#[cfg(test)]
mod tests {
    use super::*;
    use archie_execution::spec::{
        FilesystemGrant, RequiredEnforcement, ResolvedExecutable, ResourceBudget, RestartPolicy,
    };
    use archie_types::{ModuleId, QueueClass, TaskId, Timestamp, TraceId};
    use std::path::PathBuf;

    fn spec() -> TaskExecutionSpec {
        TaskExecutionSpec {
            task_id: TaskId::new(),
            trace_id: TraceId::new(),
            executable: ResolvedExecutable::from_registry(
                ModuleId::parse("research.crawler").unwrap(),
                "1.0.0",
                "/usr/lib/archie/workers/crawler",
                vec!["--depth".into(), "3".into()],
            )
            .unwrap(),
            working_directory: PathBuf::from("/var/lib/archie/work"),
            environment_allowlist: vec!["PATH".into()],
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
            requested_by: "executor".into(),
            requested_at: Timestamp::now(),
        }
    }

    fn handoff() -> Handoff {
        Handoff {
            spec: spec(),
            cgroup_procs: Some(
                "/sys/fs/cgroup/system.slice/archie-executord.service/task-1/cgroup.procs".into(),
            ),
        }
    }

    #[test]
    fn a_handoff_survives_the_pipe() {
        let original = handoff();
        let bytes = serde_json::to_vec(&original).unwrap();
        let decoded = read_from(bytes.as_slice()).unwrap();
        assert_eq!(decoded, original);
        assert_eq!(decoded.spec.executable.args(), &["--depth", "3"]);
        assert!(decoded.cgroup_procs.unwrap().ends_with("cgroup.procs"));
    }

    #[test]
    fn a_node_without_delegation_sends_no_cgroup_and_that_is_not_a_failure() {
        // The supervisor has already refused anything whose manifest required enforced
        // limits, so a task arriving without a cgroup is one that never asked for one.
        let without = Handoff {
            spec: spec(),
            cgroup_procs: None,
        };
        let bytes = serde_json::to_vec(&without).unwrap();
        assert_eq!(read_from(bytes.as_slice()).unwrap(), without);
    }

    #[test]
    fn an_oversized_spec_is_refused_rather_than_truncated() {
        // A truncated spec missing its last grant would confine a worker to less than it
        // was meant to have, and look entirely valid doing it.
        let huge = vec![b'x'; MAX_SPEC_BYTES + 1];
        let err = read_from(huge.as_slice()).unwrap_err();
        assert_eq!(
            err,
            HandoffError::TooLarge {
                bytes: MAX_SPEC_BYTES + 1
            }
        );
        assert!(err.to_string().contains("privileges until it drops them"));
    }

    #[test]
    fn the_reader_stops_rather_than_following_an_unbounded_sender() {
        // Reads at most one byte past the limit. A sender that never stops must not be
        // able to make this process allocate in the window where it is most privileged.
        struct Endless;
        impl std::io::Read for Endless {
            fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
                buf.fill(b'x');
                Ok(buf.len())
            }
        }
        let err = read_from(Endless).unwrap_err();
        assert!(matches!(err, HandoffError::TooLarge { .. }));
    }

    #[test]
    fn silence_says_the_two_processes_are_out_of_step() {
        // The executor spawned this program and then said nothing. That is not a
        // malformed task; it is the two halves disagreeing about what just happened.
        for empty in [&b""[..], b"   ", b"\n\n", b"\t \r\n"] {
            let err = read_from(empty).unwrap_err();
            assert_eq!(err, HandoffError::Empty, "{empty:?}");
            assert!(err.to_string().contains("out of step"));
        }
    }

    #[test]
    fn garbage_is_a_refusal_not_a_panic() {
        for bad in [&b"{"[..], b"null", b"[]", b"{\"task_id\":\"nope\"}"] {
            let err = read_from(bad).unwrap_err();
            assert!(matches!(err, HandoffError::Malformed(_)), "{bad:?}");
        }
    }

    #[test]
    fn the_size_check_happens_before_the_parse() {
        // Parsing 200 MB and then deciding it was too long is not a limit.
        let huge_but_valid_prefix = {
            let mut bytes = serde_json::to_vec(&handoff()).unwrap();
            bytes.resize(MAX_SPEC_BYTES + 1, b' ');
            bytes
        };
        assert!(matches!(
            decode(&huge_but_valid_prefix),
            Err(HandoffError::TooLarge { .. })
        ));
    }

    #[test]
    fn a_spec_at_exactly_the_limit_is_still_read() {
        // The boundary belongs to the sender: they were told the limit and honoured it.
        let at_limit = vec![b' '; MAX_SPEC_BYTES];
        assert_eq!(decode(&at_limit), Err(HandoffError::Empty));
    }
}
