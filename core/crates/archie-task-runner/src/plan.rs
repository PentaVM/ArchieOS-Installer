//! What the runner does, decided before it does any of it.
//!
//! RFC-007 §7 step 7. The runner's whole job is a sequence with no branches once it has
//! started, and this module is that sequence computed up front — so the decisions can be
//! read, tested and audited without a kernel, and so the part that actually touches the
//! kernel is a loop over a list rather than a tree of conditionals.
//!
//! # Order is the security property
//!
//! ```text
//! 1. join the task's cgroup      before any allocation counts against the wrong budget
//! 2. apply Landlock              irreversibly, to this process and its children
//! 3. close inherited descriptors anything still open outlives the sandbox
//! 4. exec the worker             into restrictions it never chose
//! ```
//!
//! Every step is irreversible by design and each must come after the last. Joining the
//! cgroup after Landlock would need to write `cgroup.procs`, which Landlock has by then
//! forbidden. Applying Landlock after `exec` is impossible. Closing descriptors after
//! `exec` is far too late: an open file descriptor is a hole through a filesystem sandbox
//! that no ruleset can close, because the ruleset governs *opening*, not what is already
//! open.

use archie_execution::spec::TaskExecutionSpec;

use crate::abi::Probe;
use crate::ruleset::{self, Rule};

/// Why the runner refused to start the worker.
///
/// Every one of these ends with the worker not running. That is the point: a task whose
/// manifest asked for confinement either gets it or does not run here, and this program
/// is the last place that can still make that true.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanError {
    /// The spec is malformed. It came from the executor, so this means the two are out of
    /// step — a far more alarming thing than a bad manifest.
    InvalidSpec(String),
    /// The task requires filesystem confinement and this kernel cannot provide it.
    ConfinementUnavailable { reason: String },
    /// The spec grants nothing at all, so the worker could not open its own executable.
    NoUsableGrants,
}

impl std::fmt::Display for PlanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidSpec(detail) => write!(
                f,
                "the execution spec is malformed ({detail}); it came from the executor, \
                 so the two are out of step"
            ),
            Self::ConfinementUnavailable { reason } => write!(
                f,
                "this task requires filesystem confinement and {reason}; it will not be \
                 run here unconfined"
            ),
            Self::NoUsableGrants => f.write_str(
                "the spec grants no usable filesystem access, so the worker could not \
                 open its own executable",
            ),
        }
    }
}

impl std::error::Error for PlanError {}

/// Everything the runner will do, in order.
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    /// The ABI the ruleset will be built against. `None` when the task did not require
    /// confinement and this kernel has none — the worker runs unconfined, deliberately
    /// and only because nothing asked otherwise.
    pub abi: Option<u32>,
    pub rules: Vec<Rule>,
    /// Whether confinement was *required*, as opposed to merely available.
    pub required: bool,
}

impl Plan {
    /// Whether this plan actually restricts anything.
    pub fn confines(&self) -> bool {
        self.abi.is_some() && !self.rules.is_empty()
    }

    /// A line for the audit record, said plainly.
    ///
    /// An operator reading "worker started" wants to know which of the two very different
    /// things happened, and this is the only place that still knows.
    pub fn explain(&self) -> String {
        match (self.abi, self.rules.len()) {
            (Some(abi), n) if n > 0 => {
                format!("confined by Landlock ABI {abi} with {n} granted paths")
            }
            (Some(abi), _) => format!(
                "Landlock ABI {abi} is available but the task granted no paths, so \
                 nothing is confined"
            ),
            (None, _) => "running unconfined: this kernel has no Landlock and the task did not \
                 require any"
                .into(),
        }
    }
}

/// Decide what the runner will do.
///
/// Pure. The kernel is asked exactly one question — the ABI — and it is asked by the
/// caller and handed in, so this whole decision can be exercised for any kernel from a
/// test on any host.
pub fn plan(spec: &TaskExecutionSpec, probe: &Probe) -> Result<Plan, PlanError> {
    if let Some(problem) = spec.problems().into_iter().next() {
        return Err(PlanError::InvalidSpec(problem.to_string()));
    }

    let required = spec.required_enforcement.filesystem_sandbox;

    let Some(abi) = probe.abi() else {
        if required {
            return Err(PlanError::ConfinementUnavailable {
                reason: probe.explain(),
            });
        }
        // Not required, not available. The worker runs unconfined, which is a decision
        // recorded in the plan rather than a silence.
        return Ok(Plan {
            abi: None,
            rules: Vec::new(),
            required,
        });
    };

    let rules = ruleset::rules_for(&spec.filesystem_grants, abi);

    // A confined worker with no readable path cannot open its own executable, so it would
    // die on exec with a message pointing at the binary rather than at the manifest that
    // forgot to grant it. Caught here, where the manifest is still in view.
    if required && rules.is_empty() {
        return Err(PlanError::NoUsableGrants);
    }

    Ok(Plan {
        abi: Some(abi),
        rules,
        required,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::abi::Unavailable;
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
                vec![],
            )
            .unwrap(),
            working_directory: PathBuf::from("/var/lib/archie/work"),
            environment_allowlist: vec![],
            environment_overrides: Default::default(),
            required_capabilities: vec![],
            filesystem_grants: vec![
                FilesystemGrant::read_only("/usr/lib/archie"),
                FilesystemGrant::read_write("/var/lib/archie/work"),
            ],
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

    #[test]
    fn a_task_requiring_confinement_will_not_run_without_it() {
        // This program is the last place that can still make "confined or not at all"
        // true, so it is the last place that may decide otherwise.
        let mut spec = spec();
        spec.required_enforcement = RequiredEnforcement::untrusted();

        let err = plan(&spec, &Probe::Unavailable(Unavailable::NoSuchSyscall)).unwrap_err();

        match &err {
            PlanError::ConfinementUnavailable { reason } => assert!(reason.contains("5.13")),
            other => panic!("unexpected {other:?}"),
        }
        assert!(err.to_string().contains("will not be run here unconfined"));
    }

    #[test]
    fn a_task_that_did_not_ask_runs_unconfined_and_the_plan_says_so() {
        // Not a silence: an operator reading "worker started" needs to know which of the
        // two very different things happened.
        let plan = plan(&spec(), &Probe::Unavailable(Unavailable::NotLinux)).unwrap();

        assert!(!plan.confines());
        assert!(plan.abi.is_none());
        assert!(!plan.required);
        assert!(plan.explain().contains("running unconfined"));
    }

    #[test]
    fn confinement_is_applied_when_available_even_if_not_demanded() {
        // Requirements govern refusal, not application. A trusted worker still gets
        // whatever the kernel can give.
        let plan = plan(&spec(), &Probe::Available(3)).unwrap();

        assert!(plan.confines());
        assert_eq!(plan.abi, Some(3));
        assert_eq!(plan.rules.len(), 2);
        assert!(!plan.required);
        assert!(plan.explain().contains("ABI 3 with 2 granted paths"));
    }

    #[test]
    fn a_confined_worker_with_no_grants_is_caught_here_not_at_exec() {
        // It could not open its own executable, and would die with a message pointing at
        // the binary rather than at the manifest that forgot to grant it.
        let mut spec = spec();
        spec.required_enforcement = RequiredEnforcement::untrusted();
        spec.filesystem_grants = Vec::new();

        assert_eq!(
            plan(&spec, &Probe::Available(4)).unwrap_err(),
            PlanError::NoUsableGrants
        );
    }

    #[test]
    fn an_unconfined_worker_with_no_grants_is_fine() {
        // Nothing is being restricted, so there is nothing it needs permission for.
        let mut spec = spec();
        spec.filesystem_grants = Vec::new();

        let plan = plan(&spec, &Probe::Available(4)).unwrap();
        assert!(!plan.confines());
        assert!(plan.explain().contains("granted no paths"));
    }

    #[test]
    fn a_malformed_spec_says_the_two_processes_are_out_of_step() {
        // It came from the executor, not from a manifest a human typed. That is a far
        // more alarming thing and should not read like a configuration error.
        let mut spec = spec();
        spec.working_directory = PathBuf::from("relative/path");

        let err = plan(&spec, &Probe::Available(4)).unwrap_err();
        assert!(matches!(err, PlanError::InvalidSpec(_)));
        assert!(err.to_string().contains("out of step"));
    }

    #[test]
    fn the_rules_never_exceed_what_the_abi_handles() {
        // A rule allowing an access the ruleset does not handle is rejected outright, and
        // one rejected rule means that path is not granted at all.
        for abi in 1..=5 {
            let plan = plan(&spec(), &Probe::Available(abi)).unwrap();
            for rule in &plan.rules {
                assert_eq!(
                    rule.allowed_access & !crate::rights::handled_access(abi),
                    0,
                    "abi {abi}"
                );
            }
        }
    }
}
