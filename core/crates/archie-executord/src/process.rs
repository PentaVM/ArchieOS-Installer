//! The real launcher: a spec becomes an operating-system process.
//!
//! RFC-007 §7. Everything up to here has been logic behind a trait; this is the
//! implementation that actually calls `fork`/`exec`.
//!
//! # It spawns the runner, not the worker
//!
//! Landlock is irreversible for a process tree, so whatever applies it must be the
//! process about to *become* the worker — never this one, which has other tasks still to
//! start. So the child is always `archie-task-runner`, handed the task on its stdin; it
//! confines itself and then `exec`s the worker, keeping the same pid. What this launcher
//! supervises is therefore the worker, with the runner having vanished into it.
//!
//! A launcher configured without a runner path spawns the worker directly and reports
//! that it enforces nothing. That is the honest configuration for a development host, and
//! [`ProcessLauncher::strength`] is what `Supervisor` checks requirements against — so a
//! task demanding confinement is refused there rather than started here without it.
//!
//! # The environment is built, never inherited
//!
//! `Command` inherits the parent environment by default, which for a daemon holding
//! signing keys and provider credentials is the wrong default in the strongest sense.
//! `env_clear` comes first and the allowlist is applied on top, so a variable reaches a
//! worker only because a manifest named it.

use archie_execution::{SandboxStrength, TaskExecutionSpec};
use archie_task_runner::Handoff;
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::{Child as StdChild, Command};
use std::sync::Mutex;

use crate::launcher::{Child, Exit, LaunchError, Launcher};

/// Launches workers as child processes of the executor.
pub struct ProcessLauncher {
    /// Children we started, by pid.
    ///
    /// Held because `std::process::Child` is what can reap an exit status, and a pid
    /// alone cannot: waiting on a pid we did not spawn is not possible, and a child we
    /// never wait on becomes a zombie holding its pid until the executor exits.
    children: Mutex<HashMap<u32, StdChild>>,
    /// What this launcher can actually enforce. Not aspirational.
    strength: SandboxStrength,
    /// The per-task sandbox launcher every worker is started through.
    ///
    /// `None` means workers are exec'd directly and nothing is confined — correct only
    /// where `strength` also says nothing is enforced, which is what keeps the two from
    /// disagreeing.
    runner: Option<PathBuf>,
}

impl Default for ProcessLauncher {
    fn default() -> Self {
        Self::new()
    }
}

impl ProcessLauncher {
    /// A launcher that enforces nothing beyond process separation.
    ///
    /// The honest state until the task runner and cgroup delegation are wired. A task
    /// whose manifest requires confinement will be refused rather than started here.
    pub fn new() -> Self {
        Self {
            children: Mutex::new(HashMap::new()),
            strength: SandboxStrength::none(),
            runner: None,
        }
    }

    /// A launcher that reports what the node was measured to provide.
    ///
    /// Takes the strength rather than probing for it, so the measurement happens once at
    /// startup in one place and a launcher cannot quietly disagree with what the node
    /// advertised to the scheduler.
    pub fn with_strength(strength: SandboxStrength) -> Self {
        Self {
            children: Mutex::new(HashMap::new()),
            strength,
            runner: None,
        }
    }

    /// Start workers through the per-task sandbox launcher at this path.
    ///
    /// The path is configuration rather than a constant so a test can point at something
    /// harmless, and so an operator can see in one place which program is trusted to hold
    /// the executor's privileges for the moment before an exec.
    pub fn through_runner(mut self, runner: impl Into<PathBuf>) -> Self {
        self.runner = Some(runner.into());
        self
    }

    /// The sandbox launcher workers are started through, if any.
    pub fn runner(&self) -> Option<&PathBuf> {
        self.runner.as_ref()
    }

    pub fn strength(&self) -> &SandboxStrength {
        &self.strength
    }

    /// Build the command for a spec, without running it.
    ///
    /// Separated so the argument, environment and directory handling can be tested
    /// without spawning anything — and so a review can read exactly what will be exec'd.
    pub fn command_for(&self, spec: &TaskExecutionSpec) -> Command {
        // The runner receives the task on stdin and applies the environment itself, after
        // it has confined the process. Building the environment here as well would mean
        // the credentials passed through an extra process for no reason.
        let mut command = match &self.runner {
            Some(runner) => {
                let mut command = Command::new(runner);
                command.stdin(std::process::Stdio::piped());
                command
            }
            None => {
                let mut command = Command::new(spec.executable.program());
                command.args(spec.executable.args());
                // Clear first. A daemon's environment holds signing keys and provider
                // credentials, and inheriting it is how a worker ends up with a token
                // nobody meant to give it.
                command.env_clear();
                for (name, value) in spec.resolve_environment(|name| std::env::var(name).ok()) {
                    command.env(name, value);
                }
                command
            }
        };
        command.current_dir(&spec.working_directory);
        command
    }

    /// What the executor writes to the runner's stdin.
    pub fn handoff_for(&self, spec: &TaskExecutionSpec, cgroup_procs: Option<String>) -> Handoff {
        Handoff {
            spec: spec.clone(),
            cgroup_procs,
        }
    }
}

impl Launcher for ProcessLauncher {
    fn launch(
        &self,
        spec: &TaskExecutionSpec,
        cgroup_procs: Option<&str>,
    ) -> Result<Child, LaunchError> {
        // Refuse before spawning rather than after. A worker that started and was then
        // killed for being inadmissible has already run its first instructions, and for
        // anything with side effects that is indistinguishable from having run.
        if let Some(shortfall) = self.strength.shortfall(&spec.required_enforcement) {
            return Err(LaunchError::SandboxUnavailable(shortfall.to_string()));
        }

        if !spec.working_directory.is_dir() {
            return Err(LaunchError::WorkingDirectoryMissing(
                spec.working_directory.display().to_string(),
            ));
        }

        match self.command_for(spec).spawn() {
            Ok(mut child) => {
                // Hand the task over, then close the pipe. The runner reads to
                // end-of-file, so leaving it open would leave it waiting for a spec that
                // has already arrived in full.
                if self.runner.is_some() {
                    let handoff = self.handoff_for(spec, cgroup_procs.map(str::to_string));
                    if let Err(e) = write_handoff(&mut child, &handoff) {
                        // The runner has not confined anything yet and has not exec'd, so
                        // killing it here leaves nothing behind. Letting it live would
                        // leave a process waiting forever on a spec that will never come.
                        let _ = child.kill();
                        let _ = child.wait();
                        return Err(e);
                    }
                }

                let pid = child.id();
                self.children
                    .lock()
                    .expect("launcher mutex")
                    .insert(pid, child);
                Ok(Child { pid })
            }
            Err(e) => Err(classify_spawn_error(
                &e,
                &spec.executable.program().display().to_string(),
            )),
        }
    }

    fn is_alive(&self, child: Child) -> bool {
        let mut children = self.children.lock().expect("launcher mutex");
        match children.get_mut(&child.pid) {
            // `try_wait` is the only honest answer available: it reaps if the child has
            // finished and reports nothing if it has not. Checking the pid against the
            // process table instead would be wrong after a pid is reused.
            Some(handle) => matches!(handle.try_wait(), Ok(None)),
            None => false,
        }
    }

    fn try_wait(&self, child: Child) -> Option<Exit> {
        let mut children = self.children.lock().expect("launcher mutex");
        let handle = children.get_mut(&child.pid)?;
        match handle.try_wait() {
            Ok(Some(status)) => Some(exit_of(status)),
            // Still running, or the status could not be read. Neither is a completion,
            // and reporting one would have the supervisor free a slot that is still in
            // use.
            Ok(None) | Err(_) => None,
        }
    }

    fn terminate(&self, child: Child) -> Result<(), LaunchError> {
        // `std::process` has no portable SIGTERM. On Linux the executor will send one
        // directly; here the distinction between a polite stop and a kill is preserved in
        // the record even when the mechanism is the same, because the *intent* is what an
        // audit needs and a future signal implementation slots in without changing it.
        self.kill(child)
    }

    fn kill(&self, child: Child) -> Result<(), LaunchError> {
        let mut children = self.children.lock().expect("launcher mutex");
        let Some(handle) = children.get_mut(&child.pid) else {
            // Not ours, or already reaped. Killing something we do not own would be far
            // worse than doing nothing.
            return Ok(());
        };
        match handle.kill() {
            Ok(()) => {
                // Reap immediately. An unwaited child is a zombie holding its pid, and a
                // held pid is one that cannot be reused — including by a later task.
                let _ = handle.wait();
                Ok(())
            }
            // Already gone. The goal was that it not be running, and it is not.
            Err(e) if e.kind() == std::io::ErrorKind::InvalidInput => Ok(()),
            Err(e) => Err(LaunchError::Os(e.to_string())),
        }
    }
}

/// Write the task to the runner's stdin and close it.
///
/// Closing matters: the runner reads to end-of-file, so a pipe left open is a runner that
/// waits forever for a spec it already has in full.
fn write_handoff(child: &mut StdChild, handoff: &Handoff) -> Result<(), LaunchError> {
    use std::io::Write;

    let bytes = serde_json::to_vec(handoff)
        .map_err(|e| LaunchError::Os(format!("cannot encode the task handoff: {e}")))?;

    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| LaunchError::Os("the runner was spawned without a stdin pipe".into()))?;

    stdin
        .write_all(&bytes)
        .map_err(|e| LaunchError::Os(format!("cannot send the task to the runner: {e}")))?;
    // Dropping closes it, which is the end-of-file the runner is waiting for.
    drop(stdin);
    Ok(())
}

/// Turn a spawn failure into something an operator can act on.
///
/// The distinction earns its keep: "not installed" sends someone to install a package,
/// "not executable" sends them to check a mode bit, and a generic failure sends them to
/// read logs that will not say either.
fn classify_spawn_error(error: &std::io::Error, program: &str) -> LaunchError {
    match error.kind() {
        std::io::ErrorKind::NotFound => LaunchError::ProgramNotFound(program.to_string()),
        std::io::ErrorKind::PermissionDenied => {
            LaunchError::ProgramNotExecutable(program.to_string())
        }
        _ => LaunchError::Os(format!("spawning {program}: {error}")),
    }
}

fn exit_of(status: std::process::ExitStatus) -> Exit {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(signal) = status.signal() {
            // A worker killed by a signal did not choose its fate, and the distinction
            // decides whose problem it is: SIGKILL from the OOM killer is the node's,
            // exit(1) is the worker's.
            return Exit::Signal(signal);
        }
    }
    Exit::Code(status.code().unwrap_or(-1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use archie_execution::{
        FilesystemGrant, RequiredEnforcement, ResolvedExecutable, ResourceBudget, RestartPolicy,
    };
    use archie_types::{ModuleId, QueueClass, TaskId, Timestamp, TraceId};
    use std::path::PathBuf;

    /// A program that reliably exists and exits zero.
    ///
    /// Unix only. There is no Windows equivalent worth using here: `cmd.exe /c exit 0`
    /// goes through cmd's own command-line parsing, which mangles the arguments Rust
    /// quotes for it, and chasing that would be testing cmd rather than the launcher.
    /// The target platform is Linux, and Linux CI runs this for real.
    #[cfg(unix)]
    fn true_program() -> PathBuf {
        for candidate in ["/bin/true", "/usr/bin/true"] {
            let path = PathBuf::from(candidate);
            if path.is_file() {
                return path;
            }
        }
        panic!("no /bin/true on this host");
    }

    fn spec_for(program: PathBuf, args: Vec<String>) -> TaskExecutionSpec {
        // `from_registry` requires a POSIX-absolute path; on Windows the test program is
        // not one, so the check is bypassed here deliberately and only here. The rule it
        // enforces is about manifests, and this is not a manifest.
        let executable = ResolvedExecutable::from_registry(
            ModuleId::parse("test.worker").unwrap(),
            "1.0.0",
            if program.to_str().unwrap().starts_with('/') {
                program.clone()
            } else {
                PathBuf::from("/placeholder")
            },
            args,
        )
        .unwrap();

        TaskExecutionSpec {
            task_id: TaskId::new(),
            trace_id: TraceId::new(),
            executable,
            working_directory: std::env::temp_dir(),
            environment_allowlist: vec![],
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
            requested_by: "test".into(),
            requested_at: Timestamp::now(),
        }
    }

    #[test]
    fn the_command_is_built_from_the_spec_and_nothing_else() {
        // Readable without spawning, which is also how a reviewer checks what will run.
        let launcher = ProcessLauncher::new();
        let mut spec = spec_for(PathBuf::from("/bin/true"), vec!["--flag".into()]);
        spec.environment_allowlist = vec!["ARCHIE_TEST_ALLOWED".into()];

        let command = launcher.command_for(&spec);
        assert!(command.get_program().to_str().unwrap().contains("true"));
        let args: Vec<_> = command.get_args().collect();
        assert_eq!(args, vec!["--flag"]);
        assert_eq!(
            command.get_current_dir(),
            Some(spec.working_directory.as_path())
        );
    }

    #[test]
    fn a_worker_inherits_nothing_the_manifest_did_not_name() {
        // A daemon's environment holds signing keys and provider credentials. This is
        // the check that they stay there.
        std::env::set_var("ARCHIE_TEST_SECRET", "do-not-leak");
        std::env::set_var("ARCHIE_TEST_ALLOWED", "fine");

        let launcher = ProcessLauncher::new();
        let mut spec = spec_for(PathBuf::from("/bin/true"), vec![]);
        spec.environment_allowlist = vec!["ARCHIE_TEST_ALLOWED".into()];

        let command = launcher.command_for(&spec);
        let envs: Vec<_> = command
            .get_envs()
            .map(|(k, v)| {
                (
                    k.to_str().unwrap().to_string(),
                    v.map(|v| v.to_str().unwrap().to_string()),
                )
            })
            .collect();

        assert!(
            envs.iter()
                .any(|(k, v)| k == "ARCHIE_TEST_ALLOWED" && v.as_deref() == Some("fine")),
            "an allowlisted variable must reach the worker"
        );
        assert!(
            !envs.iter().any(|(k, _)| k == "ARCHIE_TEST_SECRET"),
            "inheriting the parent environment is how a sandboxed worker ends up holding \
             a token nobody meant to give it"
        );

        std::env::remove_var("ARCHIE_TEST_SECRET");
        std::env::remove_var("ARCHIE_TEST_ALLOWED");
    }

    #[test]
    #[cfg(unix)]
    fn a_task_requiring_confinement_is_refused_before_anything_is_spawned() {
        // A worker started and then killed for being inadmissible has already run its
        // first instructions, and for anything with side effects that is
        // indistinguishable from having run.
        let launcher = ProcessLauncher::new();
        let mut spec = spec_for(true_program(), vec![]);
        spec.required_enforcement = RequiredEnforcement::untrusted();

        match launcher.launch(&spec, None) {
            Err(LaunchError::SandboxUnavailable(detail)) => {
                assert!(detail.contains("weaker form"))
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn a_configured_launcher_spawns_the_runner_and_not_the_worker() {
        // Landlock is irreversible for a process tree, so the confining process must be
        // the one about to become the worker. The executor never execs a worker directly
        // once a runner is configured.
        let launcher = ProcessLauncher::new().through_runner("/usr/bin/archie-task-runner");
        let spec = spec_for(PathBuf::from("/usr/lib/archie/workers/crawler"), vec![]);

        let command = launcher.command_for(&spec);
        assert_eq!(
            command.get_program(),
            std::ffi::OsStr::new("/usr/bin/archie-task-runner"),
            "the child is the runner; the worker is what the runner becomes"
        );
        assert_eq!(
            command.get_args().count(),
            0,
            "the task travels on stdin, not argv, where every process could read it"
        );
    }

    #[test]
    fn the_runner_gets_no_environment_because_it_applies_one_itself() {
        // Building the environment here as well would pass the credentials through an
        // extra process for no reason. The runner has the spec and sets it after
        // confining.
        std::env::set_var("ARCHIE_TEST_RUNNER_SECRET", "do-not-leak");
        let launcher = ProcessLauncher::new().through_runner("/usr/bin/archie-task-runner");
        let mut spec = spec_for(PathBuf::from("/usr/lib/archie/workers/crawler"), vec![]);
        spec.environment_allowlist = vec!["ARCHIE_TEST_RUNNER_SECRET".into()];

        let command = launcher.command_for(&spec);
        assert!(
            command.get_envs().count() == 0,
            "the runner receives the spec and applies the environment itself"
        );
        std::env::remove_var("ARCHIE_TEST_RUNNER_SECRET");
    }

    #[test]
    fn the_handoff_carries_the_cgroup_the_caller_chose() {
        // Passed rather than looked up: only the caller knows which cgroup it made for
        // this task, and a launcher that guessed could put a worker in the wrong one.
        let launcher = ProcessLauncher::new().through_runner("/usr/bin/archie-task-runner");
        let spec = spec_for(PathBuf::from("/usr/lib/archie/workers/crawler"), vec![]);

        let procs = "/sys/fs/cgroup/system.slice/archie-executord.service/task-1/cgroup.procs";
        let handoff = launcher.handoff_for(&spec, Some(procs.into()));
        assert_eq!(handoff.cgroup_procs.as_deref(), Some(procs));
        assert_eq!(handoff.spec.task_id, spec.task_id);

        // And a node without delegation sends none, which the runner reads as "this task
        // never asked for enforced limits".
        assert!(launcher.handoff_for(&spec, None).cgroup_procs.is_none());
    }

    #[test]
    fn a_launcher_without_a_runner_execs_the_worker_directly() {
        // The honest development configuration, and the one where `strength` must also
        // say nothing is enforced — that is what keeps the two from disagreeing.
        let launcher = ProcessLauncher::new();
        assert!(launcher.runner().is_none());

        let spec = spec_for(PathBuf::from("/usr/lib/archie/workers/crawler"), vec![]);
        let command = launcher.command_for(&spec);
        assert!(command.get_program().to_str().unwrap().ends_with("crawler"));
        assert!(!launcher.strength().filesystem_enforced);
    }

    #[test]
    fn a_bare_launcher_admits_it_enforces_nothing() {
        // The honest state until the task runner and cgroup delegation are wired.
        let launcher = ProcessLauncher::new();
        assert_eq!(*launcher.strength(), SandboxStrength::none());
        assert!(!launcher.strength().filesystem_enforced);
        assert!(!launcher.strength().cgroup_enforced);
    }

    #[test]
    #[cfg(unix)]
    fn a_missing_working_directory_is_reported_as_such() {
        let launcher = ProcessLauncher::new();
        let mut spec = spec_for(true_program(), vec![]);
        spec.working_directory = std::env::temp_dir().join("archie-definitely-not-here");

        match launcher.launch(&spec, None) {
            Err(LaunchError::WorkingDirectoryMissing(path)) => {
                assert!(path.contains("definitely-not-here"))
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    #[cfg(unix)]
    fn a_missing_program_says_it_is_not_installed() {
        // "Not installed" sends someone to install a package; "failed to start" sends
        // them to read logs that will not say so.
        let launcher = ProcessLauncher::new();
        let spec = spec_for(std::env::temp_dir().join("archie-no-such-worker"), vec![]);

        match launcher.launch(&spec, None) {
            Err(LaunchError::ProgramNotFound(_)) | Err(LaunchError::Os(_)) => {}
            other => panic!("expected a not-found style failure, got {other:?}"),
        }
    }

    #[test]
    #[cfg(unix)]
    fn a_real_child_is_started_and_reaped() {
        // The vertical slice, for real: a spec becomes a process, the process ends, and
        // the launcher reports how. Unix only — see `true_program`.
        let launcher = ProcessLauncher::new();
        let spec = spec_for(true_program(), vec![]);

        let child = launcher.launch(&spec, None).expect("spawn /bin/true");

        // Wait without asserting on timing: a test that races the scheduler is a test
        // that fails on a loaded machine.
        let mut exit = None;
        for _ in 0..200 {
            if let Some(e) = launcher.try_wait(child) {
                exit = Some(e);
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        let exit = exit.expect("the child should have finished within two seconds");
        assert!(exit.is_success(), "got {exit}");
        assert!(!launcher.is_alive(child));
    }

    #[test]
    #[cfg(unix)]
    fn a_failing_child_reports_its_status_rather_than_a_signal() {
        // A worker that chose to exit non-zero and one that was killed are different
        // failures, and only the second is the node's problem.
        let launcher = ProcessLauncher::new();
        let spec = spec_for(PathBuf::from("/bin/false"), vec![]);
        if !std::path::Path::new("/bin/false").is_file() {
            return;
        }

        let child = launcher.launch(&spec, None).expect("spawn /bin/false");
        let mut exit = None;
        for _ in 0..200 {
            if let Some(e) = launcher.try_wait(child) {
                exit = Some(e);
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        match exit.expect("the child should have finished") {
            Exit::Code(code) => assert_ne!(code, 0),
            Exit::Signal(sig) => panic!("it chose to exit, not to be killed by {sig}"),
        }
    }

    #[test]
    fn killing_a_child_we_do_not_own_does_nothing() {
        // Far worse than doing nothing would be killing someone else's process because a
        // pid was reused.
        let launcher = ProcessLauncher::new();
        assert!(launcher.kill(Child { pid: 999_999 }).is_ok());
        assert!(launcher.terminate(Child { pid: 999_999 }).is_ok());
        assert!(!launcher.is_alive(Child { pid: 999_999 }));
    }

    #[test]
    fn a_launcher_reports_the_strength_it_was_given() {
        // Taken rather than probed, so the measurement happens once at startup and a
        // launcher cannot quietly disagree with what the node advertised.
        let measured = SandboxStrength {
            landlock_abi: Some(4),
            filesystem_enforced: true,
            syscall_policy: false,
            network_policy: true,
            cgroup_enforced: true,
            freeze_available: true,
        };
        let launcher = ProcessLauncher::with_strength(measured);
        assert_eq!(*launcher.strength(), measured);

        let mut spec = spec_for(PathBuf::from("/bin/true"), vec![]);
        spec.required_enforcement = RequiredEnforcement::untrusted();
        assert!(
            launcher
                .strength()
                .shortfall(&spec.required_enforcement)
                .is_none(),
            "a node that can enforce it should admit the task"
        );
    }
}
