//! iOS companion adapter tests: a mock companion app on loopback that
//! answers POST /v1/intent and GET /v1/state from a script, capturing every
//! raw request. Loopback http is the only credential-bearing plaintext the
//! endpoint rules permit; production is HTTPS with a pinned certificate.
use archie_sentry::{
    adapters::ios_companion::{IosAction, IosCompanion, IosDevice},
    contracts::*,
    engine::*,
    registry::ResourceDescriptor,
};
use std::collections::BTreeMap;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    task::JoinHandle,
};

// ---------- mock companion server ----------

struct Reply {
    status: &'static str,
    body: &'static str,
}

async fn read_request(stream: &mut TcpStream) -> Vec<u8> {
    let mut request = vec![];
    let mut chunk = [0; 4096];
    loop {
        let size = stream.read(&mut chunk).await.unwrap();
        if size == 0 {
            break;
        }
        request.extend_from_slice(&chunk[..size]);
        if let Some(end) = request.windows(4).position(|w| w == b"\r\n\r\n") {
            let text = String::from_utf8_lossy(&request[..end]);
            let length = text
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .and_then(|s| s.trim().parse::<usize>().ok())
                })
                .unwrap_or(0);
            if request.len() >= end + 4 + length {
                break;
            }
        }
    }
    request
}

/// Scripted companion: answers one Reply per request, returns the raw requests.
async fn companion_server(replies: Vec<Reply>) -> (String, JoinHandle<Vec<Vec<u8>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        let mut requests = vec![];
        for reply in replies {
            let (mut stream, _) = listener.accept().await.unwrap();
            requests.push(read_request(&mut stream).await);
            let headers = format!(
                "HTTP/1.1 {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                reply.status,
                reply.body.len()
            );
            stream.write_all(headers.as_bytes()).await.unwrap();
            stream.write_all(reply.body.as_bytes()).await.unwrap();
        }
        requests
    });
    (format!("http://{address}"), task)
}

// ---------- builders ----------

const TOKEN: &str = "companion-test-token";

fn device(volume_shortcut: Option<&str>) -> IosDevice {
    IosDevice {
        actions: BTreeMap::from([
            ("maps".into(), IosAction::OpenUrl("maps://".into())),
            (
                "lamp".into(),
                IosAction::Shortcut("Alternar lámpara".into()),
            ),
        ]),
        volume_shortcut: volume_shortcut.map(str::to_string),
    }
}
fn adapter(url: &str, volume_shortcut: Option<&str>) -> IosCompanion {
    IosCompanion::new(
        "edge",
        "ios",
        url,
        TOKEN,
        None,
        BTreeMap::from([("iphone".into(), device(volume_shortcut))]),
    )
    .unwrap()
}
fn resource(caps: &[CommandKind]) -> ResourceDescriptor {
    ResourceDescriptor {
        resource_id: "iphone".into(),
        edge_id: "edge".into(),
        adapter_id: "ios".into(),
        paired: true,
        capabilities: caps.to_vec(),
    }
}
fn request_text(request: &[u8]) -> String {
    String::from_utf8_lossy(request).into_owned()
}

// ---------- configuration ----------

#[test]
fn configuration_is_validated() {
    let devices = || BTreeMap::from([("iphone".into(), device(None))]);
    // HTTPS beyond loopback without a pinned certificate is refused.
    assert!(IosCompanion::new(
        "edge",
        "ios",
        "https://192.168.1.70:8443",
        TOKEN,
        None,
        devices()
    )
    .is_err());
    // A malformed pinned certificate is refused.
    assert!(IosCompanion::new(
        "edge",
        "ios",
        "https://192.168.1.70:8443",
        TOKEN,
        Some("not a pem"),
        devices()
    )
    .is_err());
    // http beyond loopback with credentials is refused by the endpoint rules.
    assert!(IosCompanion::new(
        "edge",
        "ios",
        "http://192.168.1.70:8080",
        TOKEN,
        None,
        devices()
    )
    .is_err());
    // Public hosts are refused.
    assert!(IosCompanion::new(
        "edge",
        "ios",
        "https://203.0.113.9:8443",
        TOKEN,
        Some("x"),
        devices()
    )
    .is_err());
    // Empty token, path-bearing base, empty device map.
    assert!(IosCompanion::new("edge", "ios", "http://127.0.0.1:9", "", None, devices()).is_err());
    assert!(IosCompanion::new(
        "edge",
        "ios",
        "http://127.0.0.1:9/api",
        TOKEN,
        None,
        devices()
    )
    .is_err());
    assert!(IosCompanion::new(
        "edge",
        "ios",
        "http://127.0.0.1:9",
        TOKEN,
        None,
        BTreeMap::new()
    )
    .is_err());
    // A URL without a scheme is not a valid action.
    let mut bad = device(None);
    bad.actions
        .insert("evil".into(), IosAction::OpenUrl("no-scheme-here".into()));
    assert!(IosCompanion::new(
        "edge",
        "ios",
        "http://127.0.0.1:9",
        TOKEN,
        None,
        BTreeMap::from([("iphone".into(), bad)]),
    )
    .is_err());
    // A Shortcut name with control bytes is not valid.
    let mut bad = device(None);
    bad.actions
        .insert("evil".into(), IosAction::Shortcut("line\nbreak".into()));
    assert!(IosCompanion::new(
        "edge",
        "ios",
        "http://127.0.0.1:9",
        TOKEN,
        None,
        BTreeMap::from([("iphone".into(), bad)]),
    )
    .is_err());
}

// ---------- the honest command surface ----------

#[tokio::test]
async fn app_launch_posts_the_configured_url_and_verifies_via_state_report() {
    let (url, task) = companion_server(vec![
        Reply { status: "200 OK", body: "{\"id\":\"maps\",\"status\":\"completed\"}" },
        Reply {
            status: "200 OK",
            body: "{\"battery_percent\":82,\"volume_percent\":40,\"focus_active\":false,\"last_action\":\"maps\"}",
        },
    ])
    .await;
    let mut adapter = adapter(&url, None);
    let resource = resource(&[CommandKind::AppLaunch]);
    let command = Command::AppLaunch {
        app_id: "maps".into(),
    };
    assert!(adapter.apply(&resource, &command).await.unwrap().accepted);
    let observation = adapter.observe(&resource).await.unwrap();
    assert_eq!(
        observation.state,
        ResourceState::Host {
            reachable: true,
            awake: Some(true),
            session_locked: None, // iOS never reports the lock; never guessed
            volume_percent: Some(40),
            foreground_app: Some("maps".into()), // the app's own last-action report
        }
    );
    assert!(observation.evidence_id.starts_with("ios-"));
    assert!(command.matches(&observation.state));
    let requests = task.await.unwrap();
    assert_eq!(requests.len(), 2);
    let post = request_text(&requests[0]);
    assert!(post.starts_with("POST /v1/intent HTTP/1.1\r\n"), "{post}");
    assert!(post
        .to_lowercase()
        .contains(&format!("authorization: bearer {TOKEN}").to_lowercase()));
    assert!(post.contains("\"kind\":\"open_url\""));
    assert!(post.contains("\"target\":\"maps://\""));
    let get = request_text(&requests[1]);
    assert!(get.starts_with("GET /v1/state HTTP/1.1\r\n"), "{get}");
}

#[tokio::test]
async fn shortcut_actions_are_first_class() {
    let (url, task) = companion_server(vec![Reply {
        status: "200 OK",
        body: "{\"id\":\"lamp\",\"status\":\"completed\"}",
    }])
    .await;
    let mut adapter = adapter(&url, None);
    let resource = resource(&[CommandKind::AppLaunch]);
    let command = Command::AppLaunch {
        app_id: "lamp".into(),
    };
    assert!(adapter.apply(&resource, &command).await.unwrap().accepted);
    let requests = task.await.unwrap();
    let post = request_text(&requests[0]);
    assert!(post.contains("\"kind\":\"shortcut\""));
    // The Shortcut name travels as JSON data, never as URL text.
    assert!(post.contains("\"target\":\"Alternar lámpara\""));
}

#[tokio::test]
async fn volume_goes_through_the_configured_shortcut_and_reads_back() {
    let (url, task) = companion_server(vec![
        Reply { status: "200 OK", body: "{\"id\":\"volume\",\"status\":\"completed\"}" },
        Reply {
            status: "200 OK",
            body: "{\"battery_percent\":82,\"volume_percent\":55,\"focus_active\":null,\"last_action\":\"volume\"}",
        },
    ])
    .await;
    let mut adapter = adapter(&url, Some("Ajustar volumen Archie"));
    let resource = resource(&[CommandKind::VolumeSet]);
    let command = Command::VolumeSet { percent: 55 };
    assert!(adapter.apply(&resource, &command).await.unwrap().accepted);
    let observation = adapter.observe(&resource).await.unwrap();
    assert!(command.matches(&observation.state));
    let requests = task.await.unwrap();
    let post = request_text(&requests[0]);
    assert!(post.contains("\"kind\":\"shortcut\""));
    assert!(post.contains("\"target\":\"Ajustar volumen Archie\""));
    assert!(post.contains("\"input\":55"));
}

#[tokio::test]
async fn unsupported_and_unconfigured_commands_never_send_requests() {
    let (url, task) = companion_server(vec![]).await;
    let mut adapter = adapter(&url, None); // no volume shortcut configured
    let resource = resource(&[
        CommandKind::VolumeSet,
        CommandKind::SessionLock,
        CommandKind::AppLaunch,
    ]);
    let volume_only = ResourceDescriptor {
        capabilities: vec![CommandKind::VolumeSet],
        ..resource.clone()
    };
    // VolumeSet without a configured volume Shortcut.
    assert_eq!(
        adapter
            .apply(&resource, &Command::VolumeSet { percent: 20 })
            .await
            .unwrap_err(),
        AdapterError::Unsupported
    );
    // SessionLock: iOS cannot be locked remotely. Said plainly, never tried.
    assert_eq!(
        adapter
            .apply(&resource, &Command::SessionLock { locked: true })
            .await
            .unwrap_err(),
        AdapterError::Unsupported
    );
    // An app_id outside the configured action table.
    assert_eq!(
        adapter
            .apply(
                &resource,
                &Command::AppLaunch {
                    app_id: "safari".into()
                }
            )
            .await
            .unwrap_err(),
        AdapterError::Unsupported
    );
    // Undeclared capability.
    assert_eq!(
        adapter
            .apply(
                &volume_only,
                &Command::AppLaunch {
                    app_id: "maps".into()
                }
            )
            .await
            .unwrap_err(),
        AdapterError::Unsupported
    );
    drop(task); // no request was ever made; the server task awaits none
}

// ---------- honest failures ----------

#[tokio::test]
async fn rejected_ack_and_id_mismatch_are_not_success() {
    let (url, task) = companion_server(vec![
        Reply {
            status: "200 OK",
            body: "{\"id\":\"maps\",\"status\":\"rejected\"}",
        },
        Reply {
            status: "200 OK",
            body: "{\"id\":\"other\",\"status\":\"completed\"}",
        },
        Reply {
            status: "200 OK",
            body: "not json",
        },
    ])
    .await;
    let mut adapter = adapter(&url, None);
    let resource = resource(&[CommandKind::AppLaunch]);
    let command = Command::AppLaunch {
        app_id: "maps".into(),
    };
    // The app refused (e.g. unknown Shortcut): Unavailable, not accepted.
    assert_eq!(
        adapter.apply(&resource, &command).await.unwrap_err(),
        AdapterError::Unavailable
    );
    // An ack echoing a different id is a protocol violation.
    assert_eq!(
        adapter.apply(&resource, &command).await.unwrap_err(),
        AdapterError::Protocol
    );
    // A malformed ack is a protocol violation.
    assert_eq!(
        adapter.apply(&resource, &command).await.unwrap_err(),
        AdapterError::Protocol
    );
    let _ = task.await;
}

#[tokio::test]
async fn authentication_and_unreachable_app_are_plain_errors() {
    let (url, task) = companion_server(vec![Reply {
        status: "401 Unauthorized",
        body: "{}",
    }])
    .await;
    let mut companion = adapter(&url, None);
    let resource = resource(&[CommandKind::AppLaunch]);
    assert_eq!(
        companion
            .apply(
                &resource,
                &Command::AppLaunch {
                    app_id: "maps".into()
                }
            )
            .await
            .unwrap_err(),
        AdapterError::Authentication
    );
    let _ = task.await;

    // Nothing listening: a plain transport error, no invented state.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let url_dead = format!("http://{address}");
    let mut dead = adapter(&url_dead, None);
    assert!(matches!(
        dead.observe(&resource).await.unwrap_err(),
        AdapterError::Transport | AdapterError::Unavailable
    ));
}

#[tokio::test]
async fn state_report_with_invalid_last_action_is_a_protocol_error() {
    let (url, task) = companion_server(vec![Reply {
        status: "200 OK",
        body: "{\"battery_percent\":82,\"volume_percent\":40,\"focus_active\":false,\"last_action\":\"not a valid id\"}",
    }])
    .await;
    let mut adapter = adapter(&url, None);
    assert_eq!(
        adapter
            .observe(&resource(&[CommandKind::AppLaunch]))
            .await
            .unwrap_err(),
        AdapterError::Protocol
    );
    let _ = task.await;
}
