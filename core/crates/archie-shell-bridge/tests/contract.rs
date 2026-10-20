//! Contract tests: the bridge against the REAL `archie-displayd` request router, served
//! over a real loopback socket. Nothing here is mocked except where a test says so, and
//! those tests are about malformed or hostile peers, which a real displayd never is.

use archie_displayd::http;
use archie_displayd::{DisplaySnapshot, Observatory, SnapshotSource};
use archie_shell_bridge::{poll, Client, ClientError, Link, SCHEMA};
use archie_state::StatePaths;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener as StdListener};
use std::sync::Arc;

/// Serve `observatory` the way `archie-displayd`'s `main` does, on an ephemeral port.
async fn serve(observatory: Observatory) -> (SocketAddr, tokio::sync::watch::Sender<bool>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (stop, shutdown) = tokio::sync::watch::channel(false);
    let observatory = Arc::new(observatory);
    tokio::spawn(http::serve(
        listener,
        move |request| {
            let observatory = observatory.clone();
            Box::pin(async move { observatory.route(request).await })
        },
        shutdown,
    ));
    (addr, stop)
}

fn scratch_root(tag: &str) -> std::path::PathBuf {
    let dir =
        std::env::temp_dir().join(format!("archie-shell-bridge-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

async fn poll_blocking(addr: SocketAddr) -> archie_shell_bridge::Frame {
    tokio::task::spawn_blocking(move || poll(&Client::new(addr).unwrap()))
        .await
        .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_daemon_that_is_down_is_reported_as_unreachable_never_as_live() {
    // A real Observatory pointed at an empty root: there is no archied to answer.
    let observatory = Observatory::live_without_world(StatePaths::rooted(scratch_root("down")));
    observatory.refresh().await;
    observatory.refresh_kernel().await;
    let (addr, stop) = serve(observatory).await;

    let frame = poll_blocking(addr).await;
    let _ = stop.send(true);

    assert_eq!(frame.schema, SCHEMA);
    assert!(
        matches!(frame.link, Link::DaemonUnreachable { .. }),
        "got {:?}",
        frame.link
    );
    assert!(
        frame.system.is_none(),
        "no status was ever read, so none may be shown"
    );
    // Telemetry is independent of archied: still present, and honest about availability.
    let kernel = frame
        .kernel
        .expect("kernel telemetry is served without archied");
    assert_eq!(
        kernel.pressure.is_some(),
        kernel.pressure_availability.is_available(),
        "a pressure reading exists exactly when it is available"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_recording_is_never_presented_as_live_even_if_it_says_reachable() {
    let mut recording = DisplaySnapshot::unreachable("replay");
    recording.source = SnapshotSource::Live; // a captured-live file replayed
    recording.daemon_reachable = true;
    recording.error = None;
    let (addr, stop) = serve(Observatory::from_fixture(recording)).await;

    let frame = poll_blocking(addr).await;
    let _ = stop.send(true);

    assert_eq!(frame.link, Link::Fixture);
    let wire = serde_json::to_value(&frame).unwrap();
    assert_eq!(wire["catalog"]["source"], "fixture");
    assert_eq!(wire["inspector"]["source"], "fixture");
    assert_eq!(wire["workflows"]["source"], "fixture");
    assert_eq!(wire["workflows"]["workflows"][0]["observed"], false);
}

#[test]
fn nothing_listening_is_displayd_offline() {
    // Bind then drop: the port is free and refuses connections.
    let addr = StdListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap();
    let frame = poll(&Client::new(addr).unwrap());
    assert!(
        matches!(frame.link, Link::DisplaydOffline { .. }),
        "got {:?}",
        frame.link
    );
    assert!(frame.system.is_none() && frame.kernel.is_none());
    let wire = serde_json::to_value(&frame).unwrap();
    assert_eq!(wire["catalog"], serde_json::Value::Null);
    assert_eq!(wire["inspector"]["source"], "unavailable");
    assert_eq!(wire["workflows"]["state"], "unavailable");
}

/// A one-shot peer that answers whatever it is asked with `reply`.
fn hostile_peer(reply: Vec<u8>) -> SocketAddr {
    let listener = StdListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf);
            let _ = stream.write_all(&reply);
        }
    });
    addr
}

#[test]
fn an_answer_that_is_not_the_schema_is_invalid_not_a_crash() {
    let body = br#"{"unexpected":true}"#;
    let reply = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", body.len());
    let mut bytes = reply.into_bytes();
    bytes.extend_from_slice(body);
    let frame = poll(&Client::new(hostile_peer(bytes)).unwrap());
    assert!(
        matches!(frame.link, Link::Invalid { .. }),
        "got {:?}",
        frame.link
    );
}

#[test]
fn an_http_error_status_is_invalid_not_offline() {
    let reply = b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\n\r\n".to_vec();
    let frame = poll(&Client::new(hostile_peer(reply)).unwrap());
    match frame.link {
        Link::Invalid { reason } => assert!(reason.contains("500"), "{reason}"),
        other => panic!("got {other:?}"),
    }
}

#[test]
fn an_oversized_answer_is_refused() {
    let mut bytes = b"HTTP/1.1 200 OK\r\n\r\n".to_vec();
    bytes.resize(bytes.len() + 5 * 1024 * 1024, b'x');
    let addr = hostile_peer(bytes);
    let err = Client::new(addr).unwrap().get("/api/snapshot").unwrap_err();
    assert!(matches!(err, ClientError::TooLarge), "got {err:?}");
}

#[test]
fn a_non_loopback_address_is_refused_before_any_connection() {
    let err = Client::new("10.0.0.5:7666".parse().unwrap()).unwrap_err();
    assert!(matches!(err, ClientError::NotLoopback(_)));
}

#[test]
fn a_frame_serialises_with_an_explicit_link_state() {
    let addr = StdListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap();
    let frame = poll(&Client::new(addr).unwrap());
    let line = serde_json::to_string(&frame).unwrap();
    let value: serde_json::Value = serde_json::from_str(&line).unwrap();
    assert_eq!(value["schema"], SCHEMA);
    assert_eq!(value["link"]["state"], "displayd_offline");
    assert!(!line.contains('\n'), "one frame is exactly one line");
}

/// A peer that serves fixed JSON bodies by path, for as many requests as there are routes.
/// The bodies are built from the REAL `DisplaySnapshot`/`StatusView` types, but the peer is
/// not `archie-displayd`: a live `archied` cannot run on every machine these tests do.
fn routed_peer(routes: Vec<(&'static str, Vec<u8>)>) -> SocketAddr {
    let listener = StdListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for _ in 0..routes.len() + 1 {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let mut buf = [0u8; 2048];
            let n = stream.read(&mut buf).unwrap_or(0);
            let head = String::from_utf8_lossy(&buf[..n]).to_string();
            let path = head.split_whitespace().nth(1).unwrap_or("").to_string();
            let reply = match routes.iter().find(|(p, _)| *p == path) {
                Some((_, body)) => {
                    let mut bytes =
                        format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", body.len())
                            .into_bytes();
                    bytes.extend_from_slice(body);
                    bytes
                }
                None => b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n".to_vec(),
            };
            let _ = stream.write_all(&reply);
        }
    });
    addr
}

#[test]
fn a_reachable_live_snapshot_maps_status_fields_and_is_live() {
    use archie_core::control::StatusView;
    let status = StatusView {
        node_id: "n".into(),
        hostname: "archie-node".into(),
        profile: "workstation".into(),
        archieos_version: "0.1.0".into(),
        os: "linux".into(),
        kernel: "6.x".into(),
        architecture: "x86_64".into(),
        uptime_ms: 1,
        cpu_model: "cpu".into(),
        logical_cores: 8,
        physical_cores: 4,
        memory_total_bytes: 1,
        memory_available_bytes: 1,
        accelerators: vec![],
        storage: vec![],
        networks: vec![],
        capabilities: vec![],
        peers: 2,
        reachable_peers: 1,
        modules_loaded: 3,
        modules_installed: 35,
        models_registered: 5,
        active_tasks: 7,
        health: "healthy".into(),
        fabric_transport: "none".into(),
        capability_changes: vec![],
        location: None,
    };
    let mut snapshot = DisplaySnapshot::unreachable("x");
    snapshot.daemon_reachable = true;
    snapshot.error = None;
    snapshot.staleness_ms = 0;
    snapshot.status = Some(status);
    let mut old_server = serde_json::to_value(&snapshot).unwrap();
    old_server.as_object_mut().unwrap().remove("inspector");
    let addr = routed_peer(vec![(
        "/api/snapshot",
        serde_json::to_vec(&old_server).unwrap(),
    )]);

    let frame = poll(&Client::new(addr).unwrap());

    assert_eq!(frame.link, Link::Live);
    let wire = serde_json::to_value(&frame).unwrap();
    assert_eq!(wire["catalog"]["daemon_reachable"], true);
    assert_eq!(wire["catalog"]["source"], "live");
    assert_eq!(wire["inspector"]["state"], "unavailable");
    assert_eq!(wire["workflows"]["state"], "unavailable");
    let system = frame.system.expect("status was served");
    assert_eq!(system.hostname, "archie-node");
    assert_eq!(system.modules_loaded, 3);
    assert_eq!(system.modules_installed, 35);
    assert_eq!(system.active_tasks, 7);
    assert_eq!(system.source, "live");
    // The kernel route 404'd: it is absent, not invented.
    assert!(frame.kernel.is_none());
}
