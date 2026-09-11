//! `archie-executord` — the process that turns a scheduling decision into a process.
//!
//! RFC-007 §7. Deliberately small: it spawns, supervises and reports. It makes no model
//! calls, holds no policy, parses no provider data, and opens no network socket.
//!
//! Synchronous on purpose. An executor is a few hundred lines of "accept a command, do
//! the obvious thing, answer" plus a reaping loop, and an async runtime would add a
//! scheduler inside the thing whose job is to be the simplest trustworthy part of the
//! system. The one place it costs something — serving a command while a child exits — is
//! handled by reaping on a timer rather than by concurrency.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use archie_execution::SandboxStrength;
use archie_executord::dispatch::Dispatcher;
use archie_executord::process::ProcessLauncher;
use archie_executord::supervisor::Supervisor;
use archie_types::Timestamp;

/// How often to collect finished children.
///
/// Short enough that a completed task frees its slot promptly, long enough that an idle
/// node is not waking up constantly to ask a question whose answer is almost always no.
const REAP_INTERVAL: Duration = Duration::from_millis(250);

fn main() -> std::process::ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    // Measure what this node can enforce, once, here. Every later decision reads this
    // value rather than probing again, so the launcher, the scheduler's admission check
    // and the log line all agree about the same node.
    let strength = detect_strength();
    tracing::info!(
        landlock_abi = ?strength.landlock_abi,
        cgroup_enforced = strength.cgroup_enforced,
        freeze_available = strength.freeze_available,
        "measured what this node can enforce"
    );
    if !strength.cgroup_enforced {
        // Not a failure. A node without delegation runs trusted first-party work
        // perfectly well; it simply cannot admit anything that requires confinement, and
        // saying so at startup is better than every such task being refused later with
        // nobody having read why.
        tracing::warn!(
            "no delegated cgroup subtree: resource budgets will not be enforced, and \
             tasks requiring confinement will be refused rather than run weakened. \
             Configure Delegate= on archie-executord.service (RFC-007 §6.1)"
        );
    }

    // Every worker is started through the sandbox launcher, never exec'd directly.
    // Configured rather than hardcoded so an operator can see in one place which program
    // is trusted to hold this daemon's privileges for the moment before an exec.
    let runner = std::env::var("ARCHIE_TASK_RUNNER")
        .unwrap_or_else(|_| "/usr/bin/archie-task-runner".to_string());
    tracing::info!(runner = %runner, "workers will be started through the sandbox launcher");

    // Durable task records. Without them a restart cannot tell its own tasks from
    // someone else's, and every cgroup on the node reads as untracked.
    let journal = archie_executord::journal::Journal::new(
        std::env::var("STATE_DIRECTORY").unwrap_or_else(|_| "/var/lib/archie-executor".to_string())
            + "/tasks",
    );
    let mut dispatcher_inner = Dispatcher::new(Supervisor::new(
        ProcessLauncher::with_strength(strength).through_runner(runner),
        strength,
    ));
    match journal.prepare() {
        Ok(()) => {
            tracing::info!(path = %journal.directory().display(), "task records are durable");
            dispatcher_inner = dispatcher_inner.with_journal(journal);
        }
        // Not fatal. The executor works and only loses its memory across a restart, and
        // refusing to run at all would let a full disk take the node down entirely.
        Err(e) => tracing::warn!(error = %e, "running without durable task records"),
    }

    let dispatcher = Arc::new(Mutex::new(dispatcher_inner));

    // Look before doing anything. A cgroup in this executor's subtree at startup means
    // the previous instance died without cleaning up — and whatever is in it may still be
    // running, which is the one situation where starting a task "again" runs it twice.
    match dispatcher
        .lock()
        .expect("dispatcher mutex")
        .reconcile(Timestamp::now())
    {
        Ok(summary) if summary.needs_attention() => {
            // Deliberately a warning. This executor has no durable task records yet, so
            // anything it finds at startup is untracked by definition — and untracked
            // means something is running that nothing is supervising.
            tracing::warn!(summary = %summary.describe(), "startup reconciliation");
        }
        Ok(summary) => tracing::info!(summary = %summary.describe(), "startup reconciliation"),
        Err(e) => tracing::warn!(error = %e, "could not reconcile at startup"),
    }

    // Reaping runs on its own thread so a slow command cannot delay noticing that a
    // worker finished — and so a worker finishing cannot wait on a command being served.
    {
        let dispatcher = Arc::clone(&dispatcher);
        std::thread::Builder::new()
            .name("archie-reaper".into())
            .spawn(move || reap_loop(dispatcher))
            .expect("spawn reaper thread");
    }

    serve(dispatcher)
}

fn reap_loop(dispatcher: Arc<Mutex<Dispatcher<ProcessLauncher>>>) {
    loop {
        std::thread::sleep(REAP_INTERVAL);
        // The lock is held only for the reap itself. Holding it across the sleep would
        // block every command for a quarter second at a time.
        let (finished, queued, dropped) = {
            let mut guard = dispatcher.lock().expect("dispatcher mutex");
            let finished = guard.reap(Timestamp::now());
            let events = guard.events();
            (finished, events.len(), events.dropped())
        };

        if finished > 0 {
            tracing::info!(
                finished,
                queued,
                "tasks finished and were queued for archied"
            );
        }

        // A growing queue means archied is not draining. Worth saying, because the
        // consequence is silent: once it overflows, the control plane's picture of what
        // is running starts drifting from the truth.
        if dropped > 0 {
            tracing::warn!(
                dropped,
                queued,
                "events have been dropped; archied is not draining often enough, and its \
                 view of this node will be incomplete until it falls back to an inventory"
            );
        }
    }
}

/// What this node can actually enforce.
///
/// Reports honestly and never optimistically. On a host that is not Linux, or one where
/// nothing has been configured, every field is false — and a development host and a
/// misconfigured node are indistinguishable on purpose, so neither is mistaken for one
/// where confinement works.
fn detect_strength() -> SandboxStrength {
    #[cfg(target_os = "linux")]
    {
        use archie_observability::kernel::cgroup::CgroupController;

        let controller = CgroupController::default();
        // Delegation is not probed by writing something: a probe that succeeded would
        // have created a cgroup nobody asked for, and one that failed would be
        // indistinguishable from a transient error. The unit either delegated a subtree
        // or it did not, and the daemon's own cgroup is where that shows.
        let cgroup_enforced = controller.is_available()
            && archie_observability::kernel::cgroup::read_self_cgroup().is_ok();

        // Ask the kernel rather than infer from a version string. Landlock can be
        // compiled out and can be disabled at boot with `lsm=`, so a 6.8 kernel without
        // it looks exactly like a 5.10 one — and the distribution's version would
        // confidently say otherwise.
        let landlock = archie_task_runner::probe();
        if !landlock.is_available() {
            tracing::warn!(reason = %landlock.explain(), "no filesystem confinement available");
        }

        SandboxStrength {
            landlock_abi: landlock.abi(),
            // The ABI says what *could* be enforced. Nothing is enforced until the task
            // runner applies a ruleset, so this stays false until that program exists —
            // claiming otherwise would admit tasks whose manifests require confinement
            // and then run them without it, which is the one outcome worse than
            // refusing them.
            filesystem_enforced: false,
            syscall_policy: false,
            network_policy: false,
            cgroup_enforced,
            freeze_available: cgroup_enforced,
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        SandboxStrength::none()
    }
}

#[cfg(unix)]
fn serve(dispatcher: Arc<Mutex<Dispatcher<ProcessLauncher>>>) -> std::process::ExitCode {
    use archie_executord::server::{decode, encode, refusal_for};
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::UnixListener;

    let path = socket_path();
    // A stale socket from a previous run would make bind fail with EADDRINUSE, which
    // after an unclean shutdown is the normal case rather than the exceptional one.
    let _ = std::fs::remove_file(&path);

    let listener = match UnixListener::bind(&path) {
        Ok(listener) => listener,
        Err(e) => {
            tracing::error!(path = %path.display(), error = %e, "cannot bind control socket");
            return std::process::ExitCode::from(69); // EX_UNAVAILABLE
        }
    };

    // 0660: the archie group is the authentication. Set after bind, because the mode at
    // creation is subject to the umask and a socket that was briefly world-writable is
    // still a socket that was world-writable.
    if let Err(e) = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o660)) {
        tracing::error!(error = %e, "cannot restrict the control socket; refusing to serve");
        let _ = std::fs::remove_file(&path);
        return std::process::ExitCode::from(69);
    }

    tracing::info!(path = %path.display(), "executor ready");

    for stream in listener.incoming() {
        let mut stream = match stream {
            Ok(stream) => stream,
            Err(e) => {
                tracing::warn!(error = %e, "accept failed");
                continue;
            }
        };

        let reader = BufReader::new(match stream.try_clone() {
            Ok(clone) => clone,
            Err(e) => {
                tracing::warn!(error = %e, "cannot read from connection");
                continue;
            }
        });

        for line in reader.split(b'\n') {
            let Ok(line) = line else { break };
            let response = match decode(&line) {
                Ok(command) => {
                    let mut guard = dispatcher.lock().expect("dispatcher mutex");
                    guard.apply(command, Timestamp::now())
                }
                Err(e) => {
                    tracing::warn!(error = %e, "rejected a malformed command");
                    refusal_for(&e)
                }
            };
            if stream.write_all(&encode(&response)).is_err() {
                // The caller hung up mid-exchange. The command already ran; dropping the
                // connection does not undo it, and the record is the durable answer.
                break;
            }
        }
    }

    std::process::ExitCode::SUCCESS
}

#[cfg(not(unix))]
fn serve(_dispatcher: Arc<Mutex<Dispatcher<ProcessLauncher>>>) -> std::process::ExitCode {
    // The executor's whole purpose is Unix process and cgroup supervision. Building on
    // another host is useful for checking the logic; running there is not a thing to
    // pretend to support.
    tracing::error!("archie-executord runs on Linux; there is nothing to supervise here");
    std::process::ExitCode::from(69)
}

#[cfg(unix)]
fn socket_path() -> std::path::PathBuf {
    std::env::var("RUNTIME_DIRECTORY")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from("/run/archie-executor"))
        .join("executor.sock")
}
