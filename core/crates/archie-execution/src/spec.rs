//! What may be executed, stated as a type rather than a string.
//!
//! RFC-007 §7. The execution layer's first job is to make the dangerous thing
//! unrepresentable.
//!
//! # There is no command line here
//!
//! A [`TaskExecutionSpec`] has no field that holds a shell fragment, and no constructor
//! that takes one. The executable is a [`ResolvedExecutable`], which can only be produced
//! by resolving a module from the trusted registry — so an executable path cannot be
//! chosen by a caller, an operator typing quickly, or a model writing plausible text.
//!
//! This matters more than it looks. `sh -c` with anything interpolated into it is the
//! single most reliable way to turn a scheduling system into a remote shell, and the
//! defence is not careful quoting. It is having nowhere to put the string.
//!
//! # The environment is an allowlist
//!
//! A worker inherits nothing by default. `PATH`, `HOME`, proxy variables and every
//! credential the daemon happens to hold stay in the daemon unless the module's manifest
//! names them. Inheriting the parent environment is how a sandboxed process ends up
//! holding an API token nobody meant to give it.
//!
//! # A requirement the node cannot meet is a placement failure, not a downgrade
//!
//! If a spec requires enforcement this node cannot provide — a Landlock ABI that is too
//! old, a cgroup subtree that was never delegated — the task does not run here in a
//! weaker form. It is refused, with the reason, so the scheduler can place it elsewhere
//! or fail loudly. Silently running untrusted work with less isolation than its manifest
//! demanded is the worst available outcome.

use archie_types::{Capability, ModuleId, QueueClass, TaskId, Timestamp, TraceId};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// An executable that came from the trusted module registry.
///
/// The only way to obtain one is [`ResolvedExecutable::from_registry`], which the
/// registry calls after matching a [`ModuleId`] to an installed manifest. The fields are
/// private so a value cannot be assembled from a string that merely looks like a path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedExecutable {
    program: PathBuf,
    args: Vec<String>,
    /// Which module's manifest this came from, for the audit record.
    module: ModuleId,
    module_version: String,
}

impl ResolvedExecutable {
    /// Build from a manifest entry. Called by the registry, which owns the trust
    /// decision; this constructor only checks that the result is well formed.
    ///
    /// Rejects a relative program path. Resolving one would depend on the executor's
    /// working directory and `PATH`, which makes what actually runs a function of the
    /// daemon's ambient state rather than of the manifest.
    pub fn from_registry(
        module: ModuleId,
        module_version: impl Into<String>,
        program: impl Into<PathBuf>,
        args: Vec<String>,
    ) -> Result<Self, SpecError> {
        let program = program.into();
        if !is_posix_absolute(&program) {
            return Err(SpecError::RelativeProgram(program.display().to_string()));
        }
        if program.components().any(|c| c.as_os_str() == "..") {
            return Err(SpecError::PathTraversal(program.display().to_string()));
        }
        if args.iter().any(|a| a.contains('\0')) {
            return Err(SpecError::NulInArgument);
        }
        Ok(Self {
            program,
            args,
            module,
            module_version: module_version.into(),
        })
    }

    pub fn program(&self) -> &PathBuf {
        &self.program
    }
    pub fn args(&self) -> &[String] {
        &self.args
    }
    pub fn module(&self) -> &ModuleId {
        &self.module
    }
    pub fn module_version(&self) -> &str {
        &self.module_version
    }
}

/// A filesystem grant, as the task runner will hand it to Landlock.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FilesystemGrant {
    pub path: PathBuf,
    pub read: bool,
    pub write: bool,
    /// Whether the worker may execute files beneath this path. Off by default: a data
    /// directory that is also executable is a sandbox with a door in it.
    #[serde(default)]
    pub execute: bool,
}

impl FilesystemGrant {
    pub fn read_only(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            read: true,
            write: false,
            execute: false,
        }
    }

    pub fn read_write(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            read: true,
            write: true,
            execute: false,
        }
    }

    /// A grant that permits nothing is a configuration mistake rather than a very strict
    /// grant: it can only produce a worker that fails on first access for a reason
    /// nobody can see from the manifest.
    pub fn grants_anything(&self) -> bool {
        self.read || self.write || self.execute
    }
}

/// What network access a task may have.
///
/// Deliberately coarse. A per-host allowlist belongs in the egress broker that already
/// exists for providers, not in every worker's sandbox, and pretending to enforce one
/// here would claim a guarantee the task runner cannot make.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkGrant {
    /// No sockets at all. The default, and correct for almost all compute work.
    #[default]
    None,
    /// Unix domain sockets only — enough to talk to the local control channel.
    LocalOnly,
    /// Outbound IP. Only for tasks whose manifest declares they need it.
    Outbound,
}

/// Enforcement a task's manifest insists on.
///
/// Each of these is a requirement, not a preference. A node that cannot provide one is
/// not a node that runs this task with less.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct RequiredEnforcement {
    /// The task must run with its filesystem access restricted by the kernel.
    pub filesystem_sandbox: bool,
    /// The task must run inside a cgroup whose limits the kernel enforces.
    pub cgroup_limits: bool,
    /// The task must be freezable, so a preemption deadline can be made to mean
    /// something.
    pub freezable: bool,
}

impl RequiredEnforcement {
    /// Nothing required. Correct for trusted first-party workers on a node that has not
    /// yet been configured for delegation.
    pub fn none() -> Self {
        Self::default()
    }

    /// What an untrusted workload should ask for.
    pub fn untrusted() -> Self {
        Self {
            filesystem_sandbox: true,
            cgroup_limits: true,
            freezable: true,
        }
    }

    pub fn requires_anything(&self) -> bool {
        self.filesystem_sandbox || self.cgroup_limits || self.freezable
    }
}

/// The resource budget the executor will write into the task's cgroup.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ResourceBudget {
    /// CPU cores. Fractional is meaningful: 0.5 is half a core.
    pub cpu_cores: f64,
    pub memory_high_bytes: u64,
    pub memory_max_bytes: u64,
    /// Relative IO weight, 1..=10000 as cgroup v2 defines it.
    pub io_weight: u16,
}

impl Default for ResourceBudget {
    fn default() -> Self {
        Self {
            cpu_cores: 1.0,
            memory_high_bytes: 512 * 1024 * 1024,
            memory_max_bytes: 1024 * 1024 * 1024,
            io_weight: 100,
        }
    }
}

impl ResourceBudget {
    /// Whether this budget could be written to a cgroup without producing a worker that
    /// cannot run.
    pub fn problems(&self) -> Vec<String> {
        let mut found = Vec::new();
        // Finiteness first, so a NaN budget reports once and as itself. Written this
        // way rather than as `!(cpu_cores > 0.0)`, which also catches NaN but reads like
        // a typo — and a reviewer who "simplifies" it to `<= 0.0` silently lets NaN
        // through, because every comparison against NaN is false.
        if !self.cpu_cores.is_finite() {
            found.push("cpu_cores must be a finite number".into());
        } else if self.cpu_cores <= 0.0 {
            found.push("cpu_cores must be greater than zero".into());
        }
        if self.memory_max_bytes == 0 {
            found.push("memory_max_bytes must be greater than zero".into());
        }
        // A high watermark above the hard limit never throttles: the kernel kills before
        // reclaim is ever attempted, which turns a soft budget into a hard one silently.
        if self.memory_high_bytes > self.memory_max_bytes {
            found.push(
                "memory_high_bytes must not exceed memory_max_bytes, or reclaim never \
                 runs before the kill"
                    .into(),
            );
        }
        if self.io_weight == 0 || self.io_weight > 10_000 {
            found.push("io_weight must be within 1..=10000".into());
        }
        found
    }
}

/// How a task should be restarted, if at all.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RestartPolicy {
    /// Never. Correct for anything with side effects.
    #[default]
    Never,
    /// Only when it exited non-zero.
    OnFailure { max_attempts: u32 },
}

/// Everything the executor needs to materialize one task as a process.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskExecutionSpec {
    pub task_id: TaskId,
    pub trace_id: TraceId,
    /// The executable, resolved from the trusted registry. There is no other way in.
    pub executable: ResolvedExecutable,
    pub working_directory: PathBuf,

    /// Environment variables to pass, by name. The executor reads each from its own
    /// environment; a name that is not set is simply absent rather than empty, since an
    /// empty `HOME` and an unset `HOME` mean different things to most programs.
    #[serde(default)]
    pub environment_allowlist: Vec<String>,
    /// Variables set explicitly for this task. Kept separate from the allowlist so an
    /// audit record can distinguish what was inherited from what was decided.
    #[serde(default)]
    pub environment_overrides: BTreeMap<String, String>,

    #[serde(default)]
    pub required_capabilities: Vec<Capability>,
    #[serde(default)]
    pub filesystem_grants: Vec<FilesystemGrant>,
    #[serde(default)]
    pub network: NetworkGrant,
    #[serde(default)]
    pub required_enforcement: RequiredEnforcement,

    pub budget: ResourceBudget,
    pub queue_class: QueueClass,

    /// Whether the worker implements the checkpoint protocol. A worker that does not is
    /// never sent a checkpoint request — asking and waiting out the deadline every time
    /// would add the deadline to every preemption for nothing.
    #[serde(default)]
    pub supports_checkpoint: bool,
    /// How long to wait for a checkpoint before falling back to the kernel.
    #[serde(default = "default_checkpoint_deadline_ms")]
    pub checkpoint_deadline_ms: u64,

    /// The policy lease authorising this task, if it needed one.
    #[serde(default)]
    pub lease_reference: Option<String>,

    /// Where the task is expected to write artifacts. Also granted read-write.
    #[serde(default)]
    pub artifact_directory: Option<PathBuf>,

    #[serde(default)]
    pub restart: RestartPolicy,
    /// Wall-clock deadline. Zero means none.
    #[serde(default)]
    pub timeout_ms: u64,

    /// Who asked for this, for the audit trail.
    pub requested_by: String,
    pub requested_at: Timestamp,
}

fn default_checkpoint_deadline_ms() -> u64 {
    5_000
}

/// Why a spec was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpecError {
    RelativeProgram(String),
    PathTraversal(String),
    NulInArgument,
    RelativeWorkingDirectory(String),
    EmptyGrant(String),
    InvalidBudget(String),
    /// The environment allowlist named something that is not a legal variable name.
    InvalidEnvironmentName(String),
    /// The spec requires enforcement this node cannot provide.
    EnforcementUnavailable(String),
    /// A checkpoint deadline on a worker that cannot checkpoint.
    CheckpointDeadlineWithoutSupport,
}

impl std::fmt::Display for SpecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RelativeProgram(p) => write!(
                f,
                "program {p:?} is relative; what runs would depend on the executor's \
                 working directory and PATH rather than on the manifest"
            ),
            Self::PathTraversal(p) => write!(f, "path {p:?} contains a traversal"),
            Self::NulInArgument => f.write_str("an argument contains a NUL byte"),
            Self::RelativeWorkingDirectory(p) => {
                write!(f, "working directory {p:?} must be absolute")
            }
            Self::EmptyGrant(p) => write!(
                f,
                "the grant for {p:?} permits nothing; a worker that fails on first access \
                 for a reason invisible in the manifest is worse than no grant"
            ),
            Self::InvalidBudget(detail) => write!(f, "resource budget: {detail}"),
            Self::InvalidEnvironmentName(name) => {
                write!(f, "{name:?} is not a legal environment variable name")
            }
            Self::EnforcementUnavailable(detail) => write!(
                f,
                "this node cannot provide enforcement this task requires ({detail}); it \
                 will not be run here in a weaker form"
            ),
            Self::CheckpointDeadlineWithoutSupport => f.write_str(
                "a checkpoint deadline was set on a worker that does not implement the \
                 checkpoint protocol",
            ),
        }
    }
}

impl std::error::Error for SpecError {}

/// What enforcement a node can actually apply, discovered at runtime.
///
/// Reported honestly and never optimistically: claiming a stronger sandbox than the
/// kernel provides is worse than claiming none, because the claim is what a manifest's
/// requirement is checked against.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SandboxStrength {
    /// The Landlock ABI this kernel supports. `None` means Landlock is unavailable —
    /// not built in, or disabled at boot.
    pub landlock_abi: Option<u32>,
    pub filesystem_enforced: bool,
    pub syscall_policy: bool,
    pub network_policy: bool,
    /// Whether the executor has a delegated cgroup subtree it can actually write.
    pub cgroup_enforced: bool,
    /// Whether the freezer is usable on this node's task cgroups.
    pub freeze_available: bool,
}

impl SandboxStrength {
    /// Nothing enforced. What a development host reports, and what a misconfigured node
    /// reports — the two are indistinguishable on purpose, so neither is mistaken for
    /// a working sandbox.
    pub fn none() -> Self {
        Self::default()
    }

    /// Whether this node can satisfy a task's stated requirements.
    ///
    /// Returns the specific shortfall rather than a bool, so a scheduler placing the
    /// task elsewhere can say why this node was rejected.
    pub fn shortfall(&self, required: &RequiredEnforcement) -> Option<SpecError> {
        if required.filesystem_sandbox && !self.filesystem_enforced {
            return Some(SpecError::EnforcementUnavailable(format!(
                "filesystem sandboxing required; landlock_abi={:?}",
                self.landlock_abi
            )));
        }
        if required.cgroup_limits && !self.cgroup_enforced {
            return Some(SpecError::EnforcementUnavailable(
                "kernel-enforced resource limits required, but no delegated cgroup \
                 subtree is available"
                    .into(),
            ));
        }
        if required.freezable && !self.freeze_available {
            return Some(SpecError::EnforcementUnavailable(
                "the task must be freezable, but cgroup.freeze is not usable here".into(),
            ));
        }
        None
    }
}

impl TaskExecutionSpec {
    /// Whether this spec is well formed, independent of any node.
    ///
    /// Returns every fault rather than the first: a spec with three problems should
    /// report three, not send an operator round the loop three times.
    pub fn problems(&self) -> Vec<SpecError> {
        let mut found = Vec::new();

        if !is_posix_absolute(&self.working_directory) {
            found.push(SpecError::RelativeWorkingDirectory(
                self.working_directory.display().to_string(),
            ));
        }

        for grant in &self.filesystem_grants {
            if !grant.grants_anything() {
                found.push(SpecError::EmptyGrant(grant.path.display().to_string()));
            }
            if grant.path.components().any(|c| c.as_os_str() == "..") {
                found.push(SpecError::PathTraversal(grant.path.display().to_string()));
            }
        }

        for name in &self.environment_allowlist {
            if !is_legal_env_name(name) {
                found.push(SpecError::InvalidEnvironmentName(name.clone()));
            }
        }
        for name in self.environment_overrides.keys() {
            if !is_legal_env_name(name) {
                found.push(SpecError::InvalidEnvironmentName(name.clone()));
            }
        }

        for problem in self.budget.problems() {
            found.push(SpecError::InvalidBudget(problem));
        }

        if !self.supports_checkpoint && self.checkpoint_deadline_ms > 0 {
            // Only a fault when a deadline was set deliberately; the default exists so
            // callers need not think about it, and a default on a non-checkpointing
            // worker is harmless because it is never used.
            if self.checkpoint_deadline_ms != default_checkpoint_deadline_ms() {
                found.push(SpecError::CheckpointDeadlineWithoutSupport);
            }
        }

        found
    }

    pub fn is_valid(&self) -> bool {
        self.problems().is_empty()
    }

    /// Whether this node may run this task, given what it can actually enforce.
    pub fn admissible_on(&self, strength: &SandboxStrength) -> Result<(), SpecError> {
        if let Some(problem) = self.problems().into_iter().next() {
            return Err(problem);
        }
        match strength.shortfall(&self.required_enforcement) {
            Some(shortfall) => Err(shortfall),
            None => Ok(()),
        }
    }

    /// The environment the worker will actually see.
    ///
    /// Built from the allowlist and the overrides, never from the parent's environment
    /// wholesale. `lookup` is injected so this is testable without touching the process
    /// environment, and so the executor can decide what "the environment" means.
    pub fn resolve_environment<F>(&self, mut lookup: F) -> BTreeMap<String, String>
    where
        F: FnMut(&str) -> Option<String>,
    {
        let mut env = BTreeMap::new();
        for name in &self.environment_allowlist {
            if let Some(value) = lookup(name) {
                env.insert(name.clone(), value);
            }
        }
        // Overrides win: they were decided for this task, and the allowlist is only a
        // statement about what may be inherited.
        for (name, value) in &self.environment_overrides {
            env.insert(name.clone(), value.clone());
        }
        env
    }
}

/// Whether a path is absolute *on the target*, which is always Linux.
///
/// Not `Path::is_absolute`, which answers for the host doing the compiling: on Windows
/// it calls `/usr/lib/archie/workers/crawler` relative, because Windows wants a drive
/// letter. These paths describe where a worker lives on an ArchieOS node, and that answer
/// must not change depending on where the code was built or tested.
fn is_posix_absolute(path: &std::path::Path) -> bool {
    path.to_str().is_some_and(|p| p.starts_with('/'))
}

/// Whether a name may be used as an environment variable.
///
/// POSIX allows rather more than this; the restriction to alphanumerics and underscore,
/// not starting with a digit, is deliberate. Anything outside it in a name is far more
/// likely to be an injection attempt or a parsing accident than a real variable.
fn is_legal_env_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with(|c: char| c.is_ascii_digit())
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn module() -> ModuleId {
        ModuleId::parse("research.crawler").unwrap()
    }

    fn executable() -> ResolvedExecutable {
        ResolvedExecutable::from_registry(
            module(),
            "1.2.0",
            "/usr/lib/archie/workers/crawler",
            vec!["--depth".into(), "3".into()],
        )
        .unwrap()
    }

    fn spec() -> TaskExecutionSpec {
        TaskExecutionSpec {
            task_id: TaskId::new(),
            trace_id: TraceId::new(),
            executable: executable(),
            working_directory: PathBuf::from("/var/lib/archie/work"),
            environment_allowlist: vec!["PATH".into(), "LANG".into()],
            environment_overrides: BTreeMap::new(),
            required_capabilities: vec![],
            filesystem_grants: vec![FilesystemGrant::read_only("/usr/lib/archie")],
            network: NetworkGrant::None,
            required_enforcement: RequiredEnforcement::none(),
            budget: ResourceBudget::default(),
            queue_class: QueueClass::Background,
            supports_checkpoint: false,
            checkpoint_deadline_ms: default_checkpoint_deadline_ms(),
            lease_reference: None,
            artifact_directory: None,
            restart: RestartPolicy::Never,
            timeout_ms: 0,
            requested_by: "scheduler".into(),
            requested_at: Timestamp::now(),
        }
    }

    #[test]
    fn there_is_no_way_to_express_a_shell_command() {
        // The type has no field holding a command line, and the only constructor for an
        // executable takes a program and structured arguments. This test exists to fail
        // loudly if someone adds one.
        let exe = executable();
        assert_eq!(exe.args(), &["--depth", "3"]);
        // Asserted POSIX-absolute rather than `is_absolute()`, which answers for the
        // host: on Windows this path is "relative", and the target is always Linux.
        assert!(exe.program().to_str().unwrap().starts_with('/'));
        assert_eq!(exe.module_version(), "1.2.0");
    }

    #[test]
    fn a_relative_program_is_refused() {
        // Resolving it would depend on the executor's working directory and PATH, so
        // what actually ran would be a function of the daemon's ambient state.
        let err =
            ResolvedExecutable::from_registry(module(), "1.0.0", "crawler", vec![]).unwrap_err();
        assert!(matches!(err, SpecError::RelativeProgram(_)));
        assert!(err.to_string().contains("PATH"));
    }

    #[test]
    fn a_traversal_in_the_program_path_is_refused() {
        let err = ResolvedExecutable::from_registry(
            module(),
            "1.0.0",
            "/usr/lib/archie/../../bin/sh",
            vec![],
        )
        .unwrap_err();
        assert!(matches!(err, SpecError::PathTraversal(_)));
    }

    #[test]
    fn a_nul_byte_in_an_argument_is_refused() {
        let err =
            ResolvedExecutable::from_registry(module(), "1.0.0", "/bin/true", vec!["a\0b".into()])
                .unwrap_err();
        assert_eq!(err, SpecError::NulInArgument);
    }

    #[test]
    fn a_worker_inherits_nothing_it_was_not_given() {
        // The daemon's environment holds credentials. A worker sees exactly the names
        // its manifest asked for and nothing else.
        let spec = spec();
        let env = spec.resolve_environment(|name| match name {
            "PATH" => Some("/usr/bin".into()),
            "LANG" => Some("C.UTF-8".into()),
            "ARCHIE_SIGNING_KEY" => Some("super-secret".into()),
            "AWS_SECRET_ACCESS_KEY" => Some("also-secret".into()),
            _ => None,
        });

        assert_eq!(env.len(), 2);
        assert_eq!(env.get("PATH").unwrap(), "/usr/bin");
        assert!(
            !env.contains_key("ARCHIE_SIGNING_KEY"),
            "inheriting the parent environment is how a sandboxed process ends up \
             holding a token nobody meant to give it"
        );
        assert!(!env.contains_key("AWS_SECRET_ACCESS_KEY"));
    }

    #[test]
    fn an_allowlisted_name_that_is_unset_is_absent_not_empty() {
        // An empty HOME and an unset HOME mean different things to most programs.
        let mut spec = spec();
        spec.environment_allowlist = vec!["HOME".into()];
        let env = spec.resolve_environment(|_| None);
        assert!(env.is_empty());
    }

    #[test]
    fn an_override_beats_the_allowlist() {
        let mut spec = spec();
        spec.environment_overrides
            .insert("PATH".into(), "/opt/archie/bin".into());
        let env = spec.resolve_environment(|_| Some("/usr/bin".into()));
        assert_eq!(env.get("PATH").unwrap(), "/opt/archie/bin");
    }

    #[test]
    fn an_illegal_environment_name_is_refused() {
        let mut spec = spec();
        spec.environment_allowlist = vec!["PATH; rm -rf /".into(), "2FOO".into(), "".into()];
        let problems = spec.problems();
        assert_eq!(problems.len(), 3);
        assert!(problems
            .iter()
            .all(|p| matches!(p, SpecError::InvalidEnvironmentName(_))));
    }

    #[test]
    fn a_grant_that_permits_nothing_is_a_mistake_not_a_strict_grant() {
        let mut spec = spec();
        spec.filesystem_grants = vec![FilesystemGrant {
            path: PathBuf::from("/data"),
            read: false,
            write: false,
            execute: false,
        }];
        let problems = spec.problems();
        assert!(matches!(problems.first(), Some(SpecError::EmptyGrant(_))));
    }

    #[test]
    fn a_memory_high_above_max_never_throttles() {
        // The kernel would kill before reclaim was ever attempted, turning a soft budget
        // into a hard one without saying so.
        let mut spec = spec();
        spec.budget.memory_high_bytes = 2 * 1024 * 1024 * 1024;
        spec.budget.memory_max_bytes = 1024 * 1024 * 1024;
        let problems = spec.problems();
        assert!(problems
            .iter()
            .any(|p| matches!(p, SpecError::InvalidBudget(_))));
    }

    #[test]
    fn every_fault_is_reported_not_the_first() {
        let mut spec = spec();
        spec.working_directory = PathBuf::from("relative/path");
        spec.budget.cpu_cores = 0.0;
        spec.budget.io_weight = 0;
        spec.environment_allowlist = vec!["bad name".into()];
        assert!(
            spec.problems().len() >= 4,
            "an operator should not go round the loop once per fault"
        );
    }

    #[test]
    fn a_node_that_cannot_enforce_refuses_rather_than_downgrading() {
        // The whole point. An untrusted task that demands a filesystem sandbox does not
        // run here without one — it is refused so the scheduler can place it elsewhere.
        let mut spec = spec();
        spec.required_enforcement = RequiredEnforcement::untrusted();

        let bare = SandboxStrength::none();
        let err = spec.admissible_on(&bare).unwrap_err();
        assert!(matches!(err, SpecError::EnforcementUnavailable(_)));
        assert!(err.to_string().contains("weaker form"));

        let full = SandboxStrength {
            landlock_abi: Some(4),
            filesystem_enforced: true,
            syscall_policy: true,
            network_policy: true,
            cgroup_enforced: true,
            freeze_available: true,
        };
        assert!(spec.admissible_on(&full).is_ok());
    }

    #[test]
    fn the_shortfall_names_what_is_missing() {
        let mut spec = spec();
        spec.required_enforcement = RequiredEnforcement {
            filesystem_sandbox: false,
            cgroup_limits: true,
            freezable: false,
        };
        let partial = SandboxStrength {
            landlock_abi: Some(4),
            filesystem_enforced: true,
            syscall_policy: false,
            network_policy: false,
            cgroup_enforced: false,
            freeze_available: false,
        };
        let err = spec.admissible_on(&partial).unwrap_err();
        assert!(
            err.to_string().contains("delegated cgroup"),
            "a scheduler rejecting this node should be able to say why"
        );
    }

    #[test]
    fn a_task_requiring_nothing_runs_on_a_bare_node() {
        // A trusted first-party worker on a node without delegation configured is the
        // ordinary case during bring-up, and must not be blocked by the mechanism that
        // protects untrusted work.
        assert!(spec().admissible_on(&SandboxStrength::none()).is_ok());
        assert!(!RequiredEnforcement::none().requires_anything());
        assert!(RequiredEnforcement::untrusted().requires_anything());
    }

    #[test]
    fn no_network_is_the_default() {
        assert_eq!(NetworkGrant::default(), NetworkGrant::None);
        assert_eq!(spec().network, NetworkGrant::None);
    }

    #[test]
    fn a_deliberate_checkpoint_deadline_on_a_worker_that_cannot_checkpoint_is_refused() {
        let mut spec = spec();
        spec.supports_checkpoint = false;
        spec.checkpoint_deadline_ms = 30_000;
        assert!(spec
            .problems()
            .contains(&SpecError::CheckpointDeadlineWithoutSupport));

        // The default is not a deliberate choice and is harmless, since it is never used.
        spec.checkpoint_deadline_ms = default_checkpoint_deadline_ms();
        assert!(spec.is_valid());
    }

    #[test]
    fn a_well_formed_spec_survives_serialization() {
        let spec = spec();
        let json = serde_json::to_string(&spec).unwrap();
        let decoded: TaskExecutionSpec = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, spec);
        assert!(decoded.is_valid());
    }
}
