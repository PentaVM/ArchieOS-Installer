//! `archie-task-runner` — the last program before the worker.
//!
//! RFC-007 §7 step 7. Runs once per task, lives for milliseconds, and then ceases to
//! exist by becoming the worker.
//!
//! ```text
//! archie-executord forks
//!   this program                    still holds the executor's privileges
//!     read the spec from stdin      bounded
//!     plan the sandbox              pure; no kernel state changed yet
//!     join the task's cgroup        budgets start counting
//!     apply Landlock                irreversible
//!     replace stdin with /dev/null   the handoff pipe does not outlive the handoff
//!     exec the worker               this program is gone
//! ```
//!
//! # Every failure exits without exec'ing
//!
//! There is no path here that runs the worker in a weaker state than its manifest asked
//! for. That is the entire reason this program exists rather than the executor doing the
//! work inline: a supervisor that has already started other tasks cannot apply an
//! irreversible restriction, so the choice would become "confine everything the same" or
//! "confine nothing", and both are wrong.

use std::io::IsTerminal;
use std::process::ExitCode;

use archie_task_runner::{abi, handoff, plan};

/// `EX_USAGE`: the executor and the runner disagree about what was sent.
const EX_USAGE: u8 = 64;
/// `EX_UNAVAILABLE`: the node cannot provide what the task requires.
const EX_UNAVAILABLE: u8 = 69;
/// `EX_OSERR`: the kernel refused something that should have worked. Only reachable
/// where there is a kernel to refuse.
#[cfg(target_os = "linux")]
const EX_OSERR: u8 = 71;

fn main() -> ExitCode {
    match run() {
        Ok(never) => match never {},
        Err(exit) => exit,
    }
}

/// Everything up to the exec.
///
/// Returns `Infallible` on success because success means this process was replaced: there
/// is no code after a successful `exec`, and saying so in the type keeps anyone from
/// adding some.
fn run() -> Result<std::convert::Infallible, ExitCode> {
    // stdin is the handoff channel. A terminal there means a person ran this by hand,
    // which is worth saying plainly rather than blocking forever on a read that will
    // never end.
    if std::io::stdin().is_terminal() {
        eprintln!(
            "archie-task-runner: this program is spawned by archie-executord and reads a \
             task specification from stdin; it is not run by hand"
        );
        return Err(ExitCode::from(EX_USAGE));
    }

    let handoff = handoff::read_from(std::io::stdin().lock()).map_err(|e| {
        eprintln!("archie-task-runner: {e}");
        ExitCode::from(EX_USAGE)
    })?;
    let spec = &handoff.spec;

    // Ask the kernel once, and decide everything before changing any kernel state. If
    // this fails there is nothing to unwind.
    let probe = abi::probe();
    let plan = plan::plan(spec, &probe).map_err(|e| {
        eprintln!("archie-task-runner: {e}");
        // A task refused for want of confinement is a placement problem the scheduler can
        // act on; a malformed spec is not. They exit differently so the executor can tell
        // them apart without parsing a message.
        match e {
            plan::PlanError::ConfinementUnavailable { .. } => ExitCode::from(EX_UNAVAILABLE),
            _ => ExitCode::from(EX_USAGE),
        }
    })?;

    eprintln!("archie-task-runner: {}", plan.explain());

    apply_and_exec(&handoff, &plan)
}

/// Put this process into the task's cgroup.
///
/// Before Landlock, and it has to be: joining afterwards would mean writing
/// `cgroup.procs`, which the ruleset has by then forbidden. Before `exec` too, so the
/// worker's first allocation already counts against its own budget rather than the
/// executor's.
///
/// Writing this process's pid moves the whole thread group, and `exec` keeps the cgroup —
/// so the worker inherits the membership without ever being able to change it.
#[cfg(target_os = "linux")]
fn join_cgroup(procs_path: &str) -> Result<(), String> {
    use std::io::Write;

    // Opened for append rather than truncate. `cgroup.procs` is a kernel interface where
    // a write moves one process; truncating is meaningless and, on some kernels,
    // an error that reads like a permission problem.
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(procs_path)
        .map_err(|e| format!("cannot open {procs_path}: {e}"))?;

    write!(file, "{}", std::process::id()).map_err(|e| format!("cannot join {procs_path}: {e}"))
}

#[cfg(target_os = "linux")]
fn apply_and_exec(
    handoff: &archie_task_runner::Handoff,
    plan: &plan::Plan,
) -> Result<std::convert::Infallible, ExitCode> {
    use archie_task_runner::ruleset;
    use std::os::unix::process::CommandExt;

    let spec = &handoff.spec;

    // The cgroup first. After Landlock this write is forbidden by the ruleset we are
    // about to install, and after exec it would be the worker's first allocations that
    // counted against the wrong budget.
    if let Some(procs) = &handoff.cgroup_procs {
        if let Err(e) = join_cgroup(procs) {
            eprintln!("archie-task-runner: {e}");
            // Not a downgrade: a task that was given a cgroup is one whose budget was
            // meant to be enforced, and running it outside that cgroup would put it
            // beyond every limit on the machine.
            return Err(ExitCode::from(EX_OSERR));
        }
    }

    // Landlock, irreversibly. After this, nothing this process or the worker it becomes
    // can widen the restrictions.
    if let Some(abi) = plan.abi {
        if let Err(e) = ruleset::apply(abi, &plan.rules) {
            eprintln!("archie-task-runner: {e}");
            return Err(ExitCode::from(EX_OSERR));
        }
    }

    let mut command = std::process::Command::new(spec.executable.program());
    command.args(spec.executable.args());
    command.current_dir(&spec.working_directory);

    // Built, never inherited. This process holds whatever the executor held, and none of
    // it belongs to the worker unless the manifest named it.
    command.env_clear();
    for (name, value) in spec.resolve_environment(|name| std::env::var(name).ok()) {
        command.env(name, value);
    }

    // The handoff pipe must not outlive the handoff. A worker holding the other end of a
    // pipe the executor still has is a channel nobody designed and nobody watches — and
    // an open descriptor is a hole through a filesystem sandbox that no ruleset can
    // close, because a ruleset governs opening, not what is already open.
    command.stdin(std::process::Stdio::null());

    // `exec` rather than spawn-and-wait: this program has nothing left to do, and staying
    // alive would put a process between the executor and the worker that the executor
    // does not know about — its pid would be the one supervised, and the worker's exit
    // would arrive translated rather than raw.
    let error = command.exec();

    // Only reachable if exec failed.
    eprintln!(
        "archie-task-runner: could not exec {}: {error}",
        spec.executable.program().display()
    );
    Err(ExitCode::from(EX_OSERR))
}

#[cfg(not(target_os = "linux"))]
fn apply_and_exec(
    _handoff: &archie_task_runner::Handoff,
    _plan: &plan::Plan,
) -> Result<std::convert::Infallible, ExitCode> {
    // Running a worker here would mean running it unconfined on a host with no Landlock
    // and no cgroups, which is precisely the outcome the plan exists to prevent.
    eprintln!(
        "archie-task-runner: this program applies Linux sandboxing and has nothing to \
         apply here; a worker started now would be unconfined"
    );
    Err(ExitCode::from(EX_UNAVAILABLE))
}
