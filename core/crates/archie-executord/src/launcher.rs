//! Turning a validated spec into an operating-system process.
//!
//! The launcher is behind a trait for one reason that matters and one that is
//! convenient. The convenient one is testability. The one that matters is that the real
//! launcher will grow a Landlock step, a cgroup step and a `task-runner` exec, all of
//! them Linux-only and none of them checkable on a development host — and the supervisor
//! logic around them (idempotency, reconciliation, deadlines) is exactly the part that
//! must be right and must be testable everywhere.

use archie_execution::TaskExecutionSpec;
use std::fmt;

/// A running child, as the supervisor sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Child {
    pub pid: u32,
}

/// How a worker ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Exit {
    /// Exited on its own with this code.
    Code(i32),
    /// Killed by a signal. Distinct from an exit code because a worker that was OOM-killed
    /// and one that chose to return 137 are different events, and only one of them is the
    /// node's fault.
    Signal(i32),
}

impl Exit {
    /// Whether this counts as success. Only a zero exit code does: a signal is never a
    /// successful completion, however the worker felt about it.
    pub fn is_success(&self) -> bool {
        matches!(self, Self::Code(0))
    }
}

impl fmt::Display for Exit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Code(code) => write!(f, "exit {code}"),
            Self::Signal(sig) => write!(f, "killed by signal {sig}"),
        }
    }
}

/// Why a launch failed.
///
/// Each variant is a different thing for an operator to do, which is why this is not a
/// string. "Failed to start" sends someone to read logs; "the program is not installed"
/// sends them to install it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchError {
    /// The program named in the manifest does not exist on this node.
    ProgramNotFound(String),
    /// It exists but is not executable by the executor's user.
    ProgramNotExecutable(String),
    /// The working directory does not exist.
    WorkingDirectoryMissing(String),
    /// The task's cgroup could not be created or written.
    CgroupUnavailable(String),
    /// The sandbox the spec required could not be applied. Never downgraded — a task
    /// that asked for confinement does not run without it.
    SandboxUnavailable(String),
    /// Anything else the OS reported, kept verbatim.
    Os(String),
}

impl fmt::Display for LaunchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ProgramNotFound(p) => write!(
                f,
                "{p} is not installed on this node; the module manifest names a program \
                 that is not here"
            ),
            Self::ProgramNotExecutable(p) => {
                write!(f, "{p} exists but is not executable by the executor's user")
            }
            Self::WorkingDirectoryMissing(p) => {
                write!(f, "working directory {p} does not exist")
            }
            Self::CgroupUnavailable(detail) => write!(
                f,
                "could not create the task's cgroup ({detail}); without it the budget \
                 would be advisory"
            ),
            Self::SandboxUnavailable(detail) => write!(
                f,
                "the confinement this task requires could not be applied ({detail}); it \
                 will not be run without it"
            ),
            Self::Os(detail) => write!(f, "{detail}"),
        }
    }
}

impl std::error::Error for LaunchError {}

/// Starts and observes worker processes.
///
/// Implementations must be honest about failure: a launcher that cannot apply a required
/// sandbox returns [`LaunchError::SandboxUnavailable`] rather than starting the worker
/// unconfined, because a caller has no way to detect the difference afterwards.
pub trait Launcher: Send + Sync {
    /// Start a worker. The spec has already been validated and admitted.
    ///
    /// `cgroup_procs` is the task's `cgroup.procs`, when the node has delegation. It is
    /// passed rather than looked up because only the caller knows which cgroup it made
    /// for this task, and a launcher that guessed could put a worker in the wrong one.
    fn launch(
        &self,
        spec: &TaskExecutionSpec,
        cgroup_procs: Option<&str>,
    ) -> Result<Child, LaunchError>;

    /// Whether a child is still running. Used by reconciliation, so it must answer about
    /// the actual system rather than about the supervisor's own records.
    fn is_alive(&self, child: Child) -> bool;

    /// Collect a finished child, if it has finished. `None` means still running.
    fn try_wait(&self, child: Child) -> Option<Exit>;

    /// Ask a worker to stop.
    fn terminate(&self, child: Child) -> Result<(), LaunchError>;

    /// Stop it whether it likes it or not.
    fn kill(&self, child: Child) -> Result<(), LaunchError>;
}

#[cfg(test)]
pub(crate) mod testing {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    /// A launcher that records what it was asked to do and answers from a script.
    ///
    /// Deliberately not a mock that asserts on calls. The supervisor's behaviour is what
    /// matters, so this only supplies answers and keeps a log the tests can read.
    #[derive(Default)]
    pub struct FakeLauncher {
        next_pid: Mutex<u32>,
        pub launched: Mutex<Vec<String>>,
        /// The cgroup each launch was told about, so a test can assert the path actually
        /// reached the launcher rather than being dropped on the way.
        pub cgroups: Mutex<Vec<Option<String>>>,
        exits: Mutex<HashMap<u32, Exit>>,
        alive: Mutex<HashMap<u32, bool>>,
        fail_next: Mutex<Option<LaunchError>>,
        pub terminated: Mutex<Vec<u32>>,
        pub killed: Mutex<Vec<u32>>,
    }

    impl FakeLauncher {
        pub fn new() -> Self {
            Self {
                next_pid: Mutex::new(1000),
                ..Default::default()
            }
        }

        /// Make the next launch fail with this error.
        pub fn fail_next_launch(&self, error: LaunchError) {
            *self.fail_next.lock().unwrap() = Some(error);
        }

        /// Declare that this child has finished with this outcome.
        pub fn finish(&self, child: Child, exit: Exit) {
            self.exits.lock().unwrap().insert(child.pid, exit);
            self.alive.lock().unwrap().insert(child.pid, false);
        }

        /// Make a child vanish without an exit status — what a crash during an outage
        /// looks like to reconciliation.
        pub fn vanish(&self, child: Child) {
            self.alive.lock().unwrap().insert(child.pid, false);
        }
    }

    impl Launcher for FakeLauncher {
        fn launch(
            &self,
            spec: &TaskExecutionSpec,
            cgroup_procs: Option<&str>,
        ) -> Result<Child, LaunchError> {
            self.cgroups
                .lock()
                .unwrap()
                .push(cgroup_procs.map(str::to_string));
            if let Some(error) = self.fail_next.lock().unwrap().take() {
                return Err(error);
            }
            let mut pid = self.next_pid.lock().unwrap();
            *pid += 1;
            let child = Child { pid: *pid };
            self.launched
                .lock()
                .unwrap()
                .push(spec.executable.program().display().to_string());
            self.alive.lock().unwrap().insert(child.pid, true);
            Ok(child)
        }

        fn is_alive(&self, child: Child) -> bool {
            *self.alive.lock().unwrap().get(&child.pid).unwrap_or(&false)
        }

        fn try_wait(&self, child: Child) -> Option<Exit> {
            self.exits.lock().unwrap().get(&child.pid).cloned()
        }

        fn terminate(&self, child: Child) -> Result<(), LaunchError> {
            self.terminated.lock().unwrap().push(child.pid);
            Ok(())
        }

        fn kill(&self, child: Child) -> Result<(), LaunchError> {
            self.killed.lock().unwrap().push(child.pid);
            self.alive.lock().unwrap().insert(child.pid, false);
            Ok(())
        }
    }
}
