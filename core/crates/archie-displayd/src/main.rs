//! `archie-displayd` — the ArchieOS observatory bridge.
//!
//! Runs unprivileged, reads `archied` over its control socket, and serves the display
//! on loopback. It holds no authority: see `docs/RFC-006-DISPLAY-ISOLATION.md`.

use archie_displayd::{http, Observatory};
use archie_observability::{init_logging, LogFormat};
use archie_state::StatePaths;
use clap::Parser;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;
use tracing::{error, info};

#[derive(Debug, Parser)]
#[command(
    name = "archie-displayd",
    version,
    about = "Read-only bridge that feeds the ArchieOS observatory display."
)]
struct Args {
    /// Relocate the ArchieOS tree, matching `archied --root`.
    #[arg(long, value_name = "DIR")]
    root: Option<String>,

    /// Loopback address to serve on. Must be loopback; anything else is refused.
    #[arg(long, default_value = "127.0.0.1:7666")]
    bind: SocketAddr,

    /// Serve a recorded snapshot instead of a live daemon.
    ///
    /// The served data is always marked `fixture`, whatever the file claims, so a
    /// recording can never be mistaken for the current state of a real system.
    #[arg(long, value_name = "PATH")]
    fixture: Option<String>,

    /// How often to poll `archied`, in milliseconds.
    #[arg(long, default_value_t = 1000)]
    poll_ms: u64,

    /// Disable the loopback world-data adapter. The local Archie observatory remains
    /// fully available and `/api/world` reports an explicit disabled state.
    #[arg(long)]
    disable_world_feed: bool,

    /// How often to read the isolated local world adapter, in milliseconds.
    #[arg(long, default_value_t = 60_000)]
    world_poll_ms: u64,

    /// Human-readable logs instead of JSON.
    #[arg(long)]
    text_logs: bool,
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

    let observatory = match &args.fixture {
        Some(path) => match Observatory::from_fixture_file(path) {
            Ok(observatory) => {
                info!(fixture = %path, "serving a recorded snapshot");
                Arc::new(observatory)
            }
            Err(e) => {
                error!(error = %e, fixture = %path, "cannot load fixture");
                eprintln!("archie-displayd: cannot load fixture {path}: {e}");
                return std::process::ExitCode::from(78); // EX_CONFIG
            }
        },
        None => {
            let paths = match &args.root {
                Some(root) => StatePaths::rooted(root),
                None => StatePaths::from_env(),
            };
            Arc::new(if args.disable_world_feed {
                Observatory::live_without_world(paths)
            } else {
                Observatory::live(paths)
            })
        }
    };

    let listener = match http::bind(args.bind).await {
        Ok(listener) => listener,
        Err(e) => {
            error!(error = %e, address = %args.bind, "cannot bind display endpoint");
            eprintln!("archie-displayd: {e}");
            return std::process::ExitCode::FAILURE;
        }
    };
    let address = listener.local_addr().unwrap_or(args.bind);
    info!(address = %address, fixture = observatory.is_fixture(), "observatory listening");
    println!("archie-displayd: http://{address}");

    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    // Poll the daemon on its own cadence, independent of client requests, so a native UI
    // refresh never blocks on the control socket.
    if !observatory.is_fixture() {
        let daemon_observatory = observatory.clone();
        let mut shutdown = shutdown_rx.clone();
        let interval = Duration::from_millis(args.poll_ms.max(100));
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tokio::select! {
                    _ = shutdown.changed() => {
                        if *shutdown.borrow() { return; }
                    }
                    _ = ticker.tick() => daemon_observatory.refresh().await,
                }
            }
        });

        // The provider runs in a different sandbox. This process only polls its fixed
        // loopback endpoint, so Internet failure cannot delay control-plane snapshots.
        let world_observatory = observatory.clone();
        let mut shutdown = shutdown_rx.clone();
        let interval = Duration::from_millis(args.world_poll_ms.max(10_000));
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tokio::select! {
                    _ = shutdown.changed() => {
                        if *shutdown.borrow() { return; }
                    }
                    _ = ticker.tick() => world_observatory.refresh_world().await,
                }
            }
        });

        // Machine telemetry is read from /proc and /sys by this process, so it keeps
        // working when archied does not. Its own cadence, because PSI averages over ten
        // seconds at its shortest window: polling faster returns the same numbers and
        // costs a syscall for nothing, and polling much slower would let a machine finish
        // thrashing before the display admitted it started.
        let kernel_observatory = observatory.clone();
        let mut shutdown = shutdown_rx.clone();
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(Duration::from_secs(2));
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tokio::select! {
                    _ = shutdown.changed() => {
                        if *shutdown.borrow() { return; }
                    }
                    _ = ticker.tick() => kernel_observatory.refresh_kernel().await,
                }
            }
        });
    }

    let serving = {
        let observatory = observatory.clone();
        tokio::spawn(http::serve(
            listener,
            move |request| {
                let observatory = observatory.clone();
                Box::pin(async move { observatory.route(request).await })
            },
            shutdown_rx,
        ))
    };

    wait_for_shutdown().await;
    info!("shutting down");
    let _ = shutdown_tx.send(true);
    let _ = tokio::time::timeout(Duration::from_secs(3), serving).await;
    std::process::ExitCode::SUCCESS
}

#[cfg(unix)]
async fn wait_for_shutdown() {
    use tokio::signal::unix::{signal, SignalKind};
    let Ok(mut term) = signal(SignalKind::terminate()) else {
        let _ = tokio::signal::ctrl_c().await;
        return;
    };
    tokio::select! {
        _ = term.recv() => {}
        _ = tokio::signal::ctrl_c() => {}
    }
}

#[cfg(not(unix))]
async fn wait_for_shutdown() {
    let _ = tokio::signal::ctrl_c().await;
}
