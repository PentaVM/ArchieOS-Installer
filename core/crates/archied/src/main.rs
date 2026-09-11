//! `archied` — the ArchieOS daemon.
//!
//! One daemon, modular crates behind it. The specification is explicit that v0.1 should
//! avoid unnecessary microservices; splitting into separate daemons is a later decision
//! that the crate boundaries already permit.
//!
//! # What it does
//!
//! 1. Executes the boot sequence (`archie_core::Runtime::boot`).
//! 2. Prints the boot report and reports ARCHIE READY.
//! 3. Serves the control protocol on `/run/archie/archied.sock`.
//! 4. Runs the fabric: announces periodically, processes inbound messages,
//!    ages out silent peers.
//! 5. On SIGTERM, says goodbye to peers and removes its socket.
//!
//! Under systemd it runs in the foreground as `Type=simple`; readiness is observable
//! through the control endpoint and `archie ping`. See the unit in `distro/archiso`.

use archie_core::boot::BootReport;
use archie_core::config::ArchieConfig;
use archie_core::control::serve_connection;
use archie_core::endpoint;
use archie_core::runtime::Runtime;
use archie_observability::{init_logging, LogFormat};
use archie_state::StatePaths;
use clap::Parser;
use notify::Notifier;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;
use tracing::{error, info, warn};

mod notify;

#[derive(Debug, Parser)]
#[command(
    name = "archied",
    version,
    about = "The ArchieOS daemon: node identity, fabric, topology, scheduling."
)]
struct Args {
    /// Configuration file. Defaults to <config-dir>/archie.toml.
    #[arg(long, value_name = "PATH")]
    config: Option<String>,

    /// Relocate the entire ArchieOS tree under one directory.
    ///
    /// This is what makes it possible to run several simulated nodes on one machine.
    #[arg(long, value_name = "DIR")]
    root: Option<String>,

    /// Human-readable logs instead of JSON. JSON is the default because journald
    /// consumes it structurally.
    #[arg(long)]
    text_logs: bool,

    /// Run the boot sequence, print the report, and exit without serving.
    #[arg(long)]
    check: bool,
}

#[tokio::main]
async fn main() -> std::process::ExitCode {
    let args = Args::parse();

    init_logging(
        if args.text_logs {
            LogFormat::Text
        } else {
            LogFormat::Json
        },
        "info",
    );

    let paths = match &args.root {
        Some(root) => StatePaths::rooted(root),
        None => StatePaths::from_env(),
    };

    let config_path = args
        .config
        .clone()
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| paths.main_config());

    let config = match ArchieConfig::load(&config_path) {
        Ok(config) => config,
        Err(e) => {
            // Refusing to boot on bad configuration is deliberate. Booting with
            // silently-ignored settings could leave a safety flag at its default while
            // the operator believes they set it.
            error!(error = %e, path = %config_path.display(), "invalid configuration");
            eprintln!("archied: {e}");
            return std::process::ExitCode::from(78); // EX_CONFIG
        }
    };

    let runtime = match Runtime::boot(paths.clone(), config).await {
        Ok(runtime) => Arc::new(runtime),
        Err(e) => {
            error!(error = %e, "boot failed");
            eprintln!("archied: boot failed: {e}");
            return std::process::ExitCode::FAILURE;
        }
    };

    // The boot report goes to stdout as well as the log: under systemd it lands in the
    // journal, and on a console it is what the operator actually reads.
    print!("{}", runtime.boot_report().render());

    if args.check {
        return exit_code_for(runtime.boot_report());
    }
    if !runtime.is_ready() {
        error!("node did not reach ARCHIE READY");
        return std::process::ExitCode::FAILURE;
    }

    let listener = match endpoint::bind(&paths).await {
        Ok(listener) => listener,
        Err(e) => {
            error!(error = %e, "cannot bind control endpoint");
            eprintln!("archied: cannot bind control endpoint: {e}");
            return std::process::ExitCode::FAILURE;
        }
    };
    info!("control endpoint ready");

    // Tell systemd we are up, and not a moment earlier. Under Type=notify this is what
    // releases everything ordered After=archied.service — so it belongs here, after the
    // control endpoint is actually bound, rather than at exec time where Type=simple
    // put it and where archie-displayd raced an unbound socket.
    let notifier = Notifier::from_env();
    let watchdog_interval = notifier.watchdog_interval();
    if let Err(e) = notifier.ready(readiness_status(&runtime)) {
        // We believe we are supervised and could not say so. Not fatal — the daemon
        // works — but systemd will eventually time the unit out, so it must be loud.
        warn!(error = %e, "could not notify systemd of readiness");
    }
    if notifier.environment().is_supervised() {
        info!(
            watchdog = ?watchdog_interval,
            "systemd notified: ARCHIE READY"
        );
    }

    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    let control = tokio::spawn(serve_control(
        listener,
        runtime.clone(),
        shutdown_rx.clone(),
    ));
    let fabric = tokio::spawn(run_fabric(
        runtime.clone(),
        shutdown_rx.clone(),
        notifier.clone(),
        watchdog_interval,
    ));

    wait_for_shutdown().await;
    info!("shutting down");
    // Without STOPPING=1 a clean exit can be read as a failure and restarted.
    let _ = notifier.stopping("archied shutting down");

    // Tell peers before going away, so they mark this node gone immediately instead of
    // waiting out a timeout.
    if let Err(e) = runtime.fabric.goodbye("archied shutting down").await {
        warn!(error = %e, "could not announce departure");
    }

    let _ = shutdown_tx.send(true);
    let _ = tokio::time::timeout(Duration::from_secs(5), fabric).await;
    control.abort();
    // Wait until the listener has actually dropped. It owns the exclusive node lock,
    // so cleaning runtime files before this point could let a replacement race us.
    let _ = control.await;
    endpoint::cleanup(&paths);

    info!("stopped");
    std::process::ExitCode::SUCCESS
}

/// The one-line summary `systemctl status archied` shows.
///
/// Built from what the node actually found, so an operator reading it learns something
/// the boot report would otherwise have scrolled away.
fn readiness_status(runtime: &Runtime) -> String {
    let topology = runtime.topology();
    format!(
        "ARCHIE READY - node {} - {} of {} modules loaded",
        runtime.node_id().short(),
        topology.loaded_count(),
        topology.states.len()
    )
}

/// The status line while running, refreshed periodically.
async fn running_status(runtime: &Runtime) -> String {
    let peers = runtime.fabric.peer_count().await;
    let reachable = runtime.fabric.reachable_peers().await.len();
    format!(
        "READY - {} of {} modules - {peers} peers ({reachable} reachable)",
        runtime.topology().loaded_count(),
        runtime.topology().states.len()
    )
}

fn exit_code_for(report: &BootReport) -> std::process::ExitCode {
    if report.is_ready() {
        std::process::ExitCode::SUCCESS
    } else {
        std::process::ExitCode::FAILURE
    }
}

/// Accept control connections until shutdown.
async fn serve_control(
    listener: endpoint::ControlListener,
    runtime: Arc<Runtime>,
    mut shutdown: watch::Receiver<bool>,
) {
    loop {
        tokio::select! {
            _ = shutdown.changed() => {
                if *shutdown.borrow() {
                    return;
                }
            }
            accepted = endpoint::accept(&listener) => {
                match accepted {
                    Ok(stream) => {
                        // Each client gets its own task: one slow or wedged client must
                        // not block the operator's next `archie status`.
                        let runtime = runtime.clone();
                        tokio::spawn(async move {
                            let result = serve_connection(stream, |request| {
                                let runtime = runtime.clone();
                                async move { runtime.handle_control(request).await }
                            })
                            .await;
                            if let Err(e) = result {
                                warn!(error = %e, "control connection ended with an error");
                            }
                        });
                    }
                    Err(e) => {
                        warn!(error = %e, "control accept failed");
                        // Back off rather than spinning on a persistent accept error.
                        tokio::time::sleep(Duration::from_millis(200)).await;
                    }
                }
            }
        }
    }
}

/// Announce periodically, process inbound fabric traffic, age out silent peers.
async fn run_fabric(
    runtime: Arc<Runtime>,
    mut shutdown: watch::Receiver<bool>,
    notifier: Notifier,
    watchdog_interval: Option<Duration>,
) {
    let config = runtime.fabric.config();
    let mut announce = tokio::time::interval(Duration::from_millis(
        config.announce_interval_ms.max(1) as u64,
    ));
    // If the daemon is descheduled, fire once on catch-up rather than bursting every
    // missed tick at a network that is probably already unhappy.
    announce.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    // Refresh the hardware view far less often than we announce; probing is not free.
    let mut refresh = tokio::time::interval(Duration::from_secs(60));
    refresh.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    // The watchdog is petted from THIS loop, deliberately. A dedicated timer task would
    // keep petting while the fabric and control plane were wedged, proving only that the
    // timer lives — which is exactly the failure a watchdog exists to catch. Arming it
    // at a very long interval when systemd set none keeps the select! arm uniform
    // without generating traffic.
    let mut watchdog =
        tokio::time::interval(watchdog_interval.unwrap_or_else(|| Duration::from_secs(86_400)));
    watchdog.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    // Ask the fabric to introduce itself immediately rather than waiting a full
    // interval; a node joining should become visible in seconds.
    if let Err(e) = runtime.fabric.probe().await {
        warn!(error = %e, "initial probe failed");
    }

    loop {
        tokio::select! {
            _ = shutdown.changed() => {
                if *shutdown.borrow() {
                    return;
                }
            }
            _ = watchdog.tick(), if watchdog_interval.is_some() => {
                if let Err(e) = notifier.watchdog() {
                    warn!(error = %e, "watchdog ping failed");
                }
            }
            _ = announce.tick() => {
                if let Err(e) = runtime.fabric.announce().await {
                    warn!(error = %e, "announce failed");
                }
                let forgotten = runtime.fabric.forget_stale_peers().await;
                for node_id in forgotten {
                    info!(peer = %node_id, "forgot peer after prolonged silence");
                }
            }
            _ = refresh.tick() => {
                runtime.refresh_local_descriptor().await;
                // Keep `systemctl status archied` current. The peer count is the field
                // most likely to have changed since boot, and it is the one an operator
                // checking on a fabric actually wants.
                if let Err(e) = notifier.status(running_status(&runtime).await) {
                    warn!(error = %e, "could not update systemd status");
                }
            }
            event = runtime.fabric.receive_once() => {
                match event {
                    Ok(event) => runtime.apply_fabric_event(&event).await,
                    Err(e) => {
                        warn!(error = %e, "fabric receive failed");
                        tokio::time::sleep(Duration::from_millis(200)).await;
                    }
                }
            }
        }
    }
}

/// Wait for SIGTERM (systemd stop) or SIGINT (Ctrl-C).
#[cfg(unix)]
async fn wait_for_shutdown() {
    use tokio::signal::unix::{signal, SignalKind};
    let mut term = match signal(SignalKind::terminate()) {
        Ok(signal) => signal,
        Err(e) => {
            error!(error = %e, "cannot install SIGTERM handler");
            return;
        }
    };
    tokio::select! {
        _ = term.recv() => info!("received SIGTERM"),
        _ = tokio::signal::ctrl_c() => info!("received SIGINT"),
    }
}

#[cfg(not(unix))]
async fn wait_for_shutdown() {
    let _ = tokio::signal::ctrl_c().await;
    info!("received Ctrl-C");
}
