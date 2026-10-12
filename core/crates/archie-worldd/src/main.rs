//! `archie-worldd` — the only ArchieOS display component allowed Internet egress.

use archie_observability::{init_logging, LogFormat};
use archie_worldd::{http, WorldService};
use clap::Parser;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;
use tracing::{error, info, warn};

#[derive(Debug, Parser)]
#[command(
    name = "archie-worldd",
    version,
    about = "Fetch and normalize authoritative world data."
)]
struct Args {
    /// Loopback-only API consumed by archie-displayd.
    #[arg(long, default_value = "127.0.0.1:7667")]
    bind: SocketAddr,

    /// Provider refresh cadence. The provider service independently enforces backoff.
    #[arg(long, default_value_t = 60_000)]
    poll_ms: u64,

    #[arg(long)]
    text_logs: bool,

    /// Unified LAN device view (`archie-sentry-devices-v1`), as written by
    /// `archie sentry-devices --json`. Read at startup and on every poll tick;
    /// a rejected or unreadable file keeps the last known device graph.
    #[arg(long)]
    lan_view: Option<std::path::PathBuf>,
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

    let listener = match http::bind(args.bind).await {
        Ok(listener) => listener,
        Err(error) => {
            error!(%error, address = %args.bind, "cannot bind world endpoint");
            return std::process::ExitCode::FAILURE;
        }
    };
    let service = Arc::new(WorldService::live_usgs());
    if let Some(path) = &args.lan_view {
        refresh_lan_view(&service, path).await;
    }
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    let refresher = {
        let service = service.clone();
        let lan_view = args.lan_view.clone();
        let mut shutdown = shutdown_rx.clone();
        let interval = Duration::from_millis(args.poll_ms.max(10_000));
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tokio::select! {
                    _ = shutdown.changed() => if *shutdown.borrow() { return; },
                    _ = ticker.tick() => {
                        service.refresh().await;
                        if let Some(path) = &lan_view {
                            refresh_lan_view(&service, path).await;
                        }
                    },
                }
            }
        })
    };

    let serving = {
        let service = service.clone();
        tokio::spawn(http::serve(
            listener,
            move |request| {
                let service = service.clone();
                Box::pin(async move {
                    match request.path.as_str() {
                        "/v1/world" => match serde_json::to_vec(&service.current().await) {
                            Ok(body) => http::Response::json(body),
                            Err(_) => http::Response::error(500, b"cannot encode world snapshot\n"),
                        },
                        "/healthz" => http::Response::json(b"{\"ok\":true}"),
                        _ => http::Response::error(404, b"not found\n"),
                    }
                })
            },
            shutdown_rx,
        ))
    };

    info!(address = %args.bind, "world adapter listening");
    wait_for_shutdown().await;
    let _ = shutdown_tx.send(true);
    let _ = tokio::time::timeout(Duration::from_secs(3), serving).await;
    let _ = tokio::time::timeout(Duration::from_secs(3), refresher).await;
    std::process::ExitCode::SUCCESS
}

/// Read the unified device view and fold it into the world graph. A missing or
/// rejected file keeps the last known graph; the log says which happened.
async fn refresh_lan_view(service: &WorldService, path: &std::path::Path) {
    let size = match tokio::fs::metadata(path).await {
        Ok(metadata) => metadata.len(),
        Err(error) => {
            warn!(%error, "lan view unreadable; keeping the last known device graph");
            return;
        }
    };
    if size > archie_worldd::lan::MAX_VIEW_BYTES as u64 {
        warn!("lan view exceeds the size limit; keeping the last known device graph");
        return;
    }
    match tokio::fs::read(path).await {
        Ok(bytes) => match service.ingest_lan_view(&bytes).await {
            Ok(report) => info!(devices = report.devices, "lan view ingested into the world graph"),
            Err(error) => warn!(%error, "lan view rejected; keeping the last known device graph"),
        },
        Err(error) => warn!(%error, "lan view unreadable; keeping the last known device graph"),
    }
}

#[cfg(unix)]
async fn wait_for_shutdown() {
    use tokio::signal::unix::{signal, SignalKind};
    let Ok(mut term) = signal(SignalKind::terminate()) else {
        let _ = tokio::signal::ctrl_c().await;
        return;
    };
    tokio::select! { _ = term.recv() => {}, _ = tokio::signal::ctrl_c() => {} }
}

#[cfg(not(unix))]
async fn wait_for_shutdown() {
    let _ = tokio::signal::ctrl_c().await;
}
