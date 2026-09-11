//! The display must survive `archied` restarting underneath it.
//!
//! This is an explicit Wave E requirement: the observatory runs for months on a wall,
//! and the daemon it watches will be restarted for upgrades. A display that needs its
//! own restart afterwards is a display someone has to babysit.
//!
//! The test stands up a real control endpoint, serves the real protocol over it, kills
//! it, and brings it back — exercising the same socket path and the same codec the
//! daemon uses.

use archie_core::boot::{BootReport, BootStage};
use archie_core::control::{
    serve_connection, ControlRequest, ControlResponse, StatusView, TotalsView,
};
use archie_core::endpoint;
use archie_displayd::{Observatory, SnapshotSource};
use archie_state::StatePaths;
use std::collections::BTreeMap;
use std::sync::Arc;
use tokio::sync::watch;

/// Answer the read-only requests the bridge issues, as `archied` would.
fn respond(request: ControlRequest) -> ControlResponse {
    match request {
        ControlRequest::Status => ControlResponse::Status(Box::new(StatusView {
            node_id: "a".repeat(64),
            hostname: "reconnect-test".into(),
            profile: "dev-vm".into(),
            archieos_version: "0.1.0".into(),
            os: "Arch Linux".into(),
            kernel: "6.10.0".into(),
            architecture: "x86_64".into(),
            uptime_ms: 1_000,
            cpu_model: "Test CPU".into(),
            logical_cores: 4,
            physical_cores: 4,
            memory_total_bytes: 8 << 30,
            memory_available_bytes: 4 << 30,
            accelerators: vec![],
            storage: vec![],
            networks: vec![],
            capabilities: vec!["compute.cpu".into()],
            peers: 0,
            reachable_peers: 0,
            modules_loaded: 1,
            modules_installed: 1,
            models_registered: 0,
            active_tasks: 0,
            health: "ok".into(),
            fabric_transport: "loopback".into(),
            capability_changes: vec![],
            // Nobody has said where this fixture node is, which is the ordinary case.
            location: None,
        })),
        ControlRequest::Boot => {
            let mut report = BootReport::new();
            report.push(BootStage::ready("Node identity", true));
            report.push(BootStage::ready("State store", true));
            ControlResponse::Boot(Box::new(report))
        }
        ControlRequest::Topology => ControlResponse::Topology {
            capabilities: BTreeMap::new(),
            nodes: vec![],
            totals: TotalsView::default(),
        },
        ControlRequest::Modules => ControlResponse::Modules {
            modules: vec![],
            load_order: vec!["core.archie".into()],
        },
        ControlRequest::Models => ControlResponse::Models { models: vec![] },
        ControlRequest::Metrics => ControlResponse::Metrics(Box::default()),
        other => ControlResponse::error(format!("unexpected {}", other.name())),
    }
}

/// Run a control endpoint until `stop` fires.
async fn run_daemon(paths: StatePaths, mut stop: watch::Receiver<bool>) {
    let Ok(listener) = endpoint::bind(&paths).await else {
        return;
    };
    loop {
        tokio::select! {
            _ = stop.changed() => {
                if *stop.borrow() {
                    // Drop the listener before returning so the socket is released.
                    drop(listener);
                    endpoint::cleanup(&paths);
                    return;
                }
            }
            accepted = endpoint::accept(&listener) => {
                let Ok(stream) = accepted else { continue };
                tokio::spawn(async move {
                    serve_connection(stream, |request| async move { respond(request) })
                        .await
                        .ok();
                });
            }
        }
    }
}

fn temp_paths(tag: &str) -> StatePaths {
    let root = std::env::temp_dir().join(format!("archie-reconnect-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let paths = StatePaths::rooted(&root);
    paths.ensure_directories().unwrap();
    paths
}

#[tokio::test]
async fn the_display_recovers_after_the_daemon_restarts() {
    let paths = temp_paths("cycle");
    let observatory = Arc::new(Observatory::live(paths.clone()));

    // ── 1. daemon up ───────────────────────────────────────────────────────
    let (stop_tx, stop_rx) = watch::channel(false);
    let daemon = tokio::spawn(run_daemon(paths.clone(), stop_rx));
    wait_until(&observatory, true).await;

    let live = observatory.current().await;
    assert!(live.daemon_reachable, "should have connected");
    assert_eq!(live.source, SnapshotSource::Live);
    assert_eq!(live.load_order, vec!["core.archie".to_string()]);
    assert_eq!(live.staleness_ms, 0);

    // ── 2. daemon goes away ────────────────────────────────────────────────
    let _ = stop_tx.send(true);
    let _ = tokio::time::timeout(std::time::Duration::from_secs(5), daemon).await;
    wait_until(&observatory, false).await;

    let stale = observatory.current().await;
    assert!(!stale.daemon_reachable, "must stop claiming reachability");
    assert!(stale.error.is_some(), "must say why");
    // The last known state is still on screen — an operator learns more from it than
    // from a blank panel — but it is no longer presented as current.
    assert_eq!(
        stale.load_order,
        vec!["core.archie".to_string()],
        "the last known state should remain visible"
    );

    // ── 3. daemon comes back ───────────────────────────────────────────────
    let (stop_tx2, stop_rx2) = watch::channel(false);
    let daemon2 = tokio::spawn(run_daemon(paths.clone(), stop_rx2));
    wait_until(&observatory, true).await;

    let recovered = observatory.current().await;
    assert!(
        recovered.daemon_reachable,
        "the display must reconnect on its own, with no restart of its own"
    );
    assert!(
        recovered.error.is_none(),
        "a recovered snapshot must not carry a stale error"
    );
    assert_eq!(recovered.staleness_ms, 0);

    let _ = stop_tx2.send(true);
    let _ = tokio::time::timeout(std::time::Duration::from_secs(5), daemon2).await;
    std::fs::remove_dir_all(&paths.state_dir.parent().unwrap()).ok();
}

#[tokio::test]
async fn a_daemon_that_never_appears_is_reported_not_faked() {
    let paths = temp_paths("absent");
    let observatory = Observatory::live(paths.clone());

    for _ in 0..3 {
        observatory.refresh().await;
    }
    let snapshot = observatory.current().await;

    assert!(!snapshot.daemon_reachable);
    assert!(snapshot.modules.is_empty(), "nothing may be invented");
    assert!(snapshot.status.is_none());
    assert_eq!(snapshot.headline(), "ARCHIED UNREACHABLE");
    std::fs::remove_dir_all(paths.state_dir.parent().unwrap()).ok();
}

/// Poll until the observatory's reachability matches `expected`, or give up.
///
/// Polling rather than sleeping a fixed interval keeps the test fast when the socket
/// settles immediately and reliable when CI is slow.
async fn wait_until(observatory: &Observatory, expected: bool) {
    for _ in 0..80 {
        observatory.refresh().await;
        if observatory.current().await.daemon_reachable == expected {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("observatory never reached daemon_reachable = {expected}");
}
