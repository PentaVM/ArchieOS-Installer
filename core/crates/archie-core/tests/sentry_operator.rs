#![cfg(target_os = "linux")]
use archie_core::{
    config::ArchieConfig,
    control::{request_over, ControlRequest, ControlResponse},
    operator::serve_operator_connection,
    Runtime,
};
use archie_sentry::{contracts::Command, engine::ExecutionStatus};
use archie_state::{DeviceClass, StatePaths};
use archie_types::{Timestamp, TraceId};
use std::{
    os::unix::fs::PermissionsExt,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, UnixStream},
};

async fn fixture(
    allowed: bool,
    safety: bool,
) -> (Arc<Runtime>, Arc<AtomicUsize>, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let root = std::env::temp_dir().join(format!("sentry-operator-{}", TraceId::new()));
    std::fs::create_dir(&root).unwrap();
    let token = root.join("token");
    std::fs::write(&token, "fixture-token").unwrap();
    std::fs::set_permissions(&token, std::fs::Permissions::from_mode(0o600)).unwrap();
    let (first, _) = UnixStream::pair().unwrap();
    let uids = if allowed {
        vec![first.peer_cred().unwrap().uid()]
    } else {
        vec![]
    };
    let config = ArchieConfig::from_toml(&format!("[policy]\nsafety_boundary_present={safety}\n[sentry]\noperator_uids={uids:?}\n[sentry.home_assistant]\nendpoint='http://{}/'\ntoken_file='{}'\n[[sentry.home_assistant.resources]]\nresource_id='office-switch'\nentity='switch.office'\n", listener.local_addr().unwrap(), token.display())).unwrap();
    let core = Arc::new(
        Runtime::boot(StatePaths::rooted(&root), config)
            .await
            .unwrap(),
    );
    core.state
        .propose_device_pairing("office-switch", &"a".repeat(64), DeviceClass::HomeAssistant)
        .unwrap();
    core.state.confirm_device_pairing("office-switch").unwrap();
    let applies = Arc::new(AtomicUsize::new(0));
    let sends = applies.clone();
    let gate_root = root.clone();
    let server = tokio::spawn(async move {
        let mut on = false;
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut chunk = [0; 2048];
            loop {
                let count = stream.read(&mut chunk).await.unwrap();
                if count == 0 {
                    break;
                }
                request.extend_from_slice(&chunk[..count]);
                assert!(request.len() < 8192);
                if let Some(end) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                    let header = String::from_utf8_lossy(&request[..end]);
                    let length = header
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .and_then(|n| n.trim().parse::<usize>().ok())
                        })
                        .unwrap_or(0);
                    if request.len() >= end + 4 + length {
                        break;
                    }
                }
            }
            let header = String::from_utf8_lossy(&request);
            assert!(header
                .to_ascii_lowercase()
                .contains("authorization: bearer fixture-token"));
            let response = if header.starts_with("POST /api/services/switch/turn_on ") {
                on = true;
                sends.fetch_add(1, Ordering::SeqCst);
                while gate_root.join("hold-apply").exists() {
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
                b"[]".to_vec()
            } else if header.starts_with("GET /api/states/switch.office ") {
                serde_json::to_vec(&serde_json::json!({"entity_id":"switch.office","state":if on {"on"} else {"off"},"attributes":{},"last_reported":Timestamp::now().to_rfc3339()})).unwrap()
            } else {
                assert!(header.starts_with("POST /api/services/homeassistant/update_entity "));
                b"[]".to_vec()
            };
            stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", response.len()).as_bytes()).await.unwrap();
            stream.write_all(&response).await.unwrap();
        }
    });
    (core, applies, server)
}
async fn connect(core: Arc<Runtime>) -> (UnixStream, tokio::task::JoinHandle<()>) {
    let (server, client) = UnixStream::pair().unwrap();
    let task = tokio::spawn(async move {
        serve_operator_connection(server, core).await.unwrap();
    });
    (client, task)
}
async fn prepare(client: &mut UnixStream) -> (archie_sentry::contracts::Intent, String) {
    match request_over(
        client,
        &ControlRequest::SentryPrepare {
            resource_id: "office-switch".into(),
            command: Command::SwitchSet { on: true },
        },
    )
    .await
    .unwrap()
    {
        ControlResponse::SentryPrepared { intent, digest } => (*intent, digest),
        other => panic!("preparation failed: {other:?}"),
    }
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL and foreground terminal; run under script(1)"]
async fn operator_executes_exactly_one_reviewed_intent_and_records_authority() {
    let (core, sends, ha) = fixture(true, true).await;
    let (mut client, server) = connect(core.clone()).await;
    let (intent, digest) = prepare(&mut client).await;
    let approval = ControlRequest::SentryApprove {
        intent_id: intent.intent_id,
        digest,
    };
    match request_over(&mut client, &approval).await.unwrap() {
        ControlResponse::SentryResult(result) => {
            assert_eq!(result.status, ExecutionStatus::Verified)
        }
        other => panic!("execution failed: {other:?}"),
    }
    assert!(request_over(&mut client, &approval)
        .await
        .unwrap()
        .is_error());
    assert_eq!(sends.load(Ordering::SeqCst), 1);
    assert!(core
        .state
        .recent_audit(10)
        .unwrap()
        .iter()
        .any(|event| event.principal.starts_with("human:unix-uid-")
            && event.action == "capability.grant"));
    drop(client);
    server.await.unwrap();
    ha.abort();
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL and foreground terminal; run under script(1)"]
async fn changed_digest_consumes_approval_without_dispatch() {
    let (core, sends, ha) = fixture(true, true).await;
    let (mut client, server) = connect(core).await;
    let (intent, digest) = prepare(&mut client).await;
    assert!(request_over(
        &mut client,
        &ControlRequest::SentryApprove {
            intent_id: intent.intent_id.clone(),
            digest: "0".repeat(64)
        }
    )
    .await
    .unwrap()
    .is_error());
    assert!(request_over(
        &mut client,
        &ControlRequest::SentryApprove {
            intent_id: intent.intent_id,
            digest
        }
    )
    .await
    .unwrap()
    .is_error());
    assert_eq!(sends.load(Ordering::SeqCst), 0);
    drop(client);
    server.await.unwrap();
    ha.abort();
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL and foreground terminal; run under script(1)"]
async fn pairing_revocation_and_default_policy_still_deny_the_operator() {
    for safety in [true, false] {
        let (core, sends, ha) = fixture(true, safety).await;
        let (mut client, server) = connect(core.clone()).await;
        let (intent, digest) = prepare(&mut client).await;
        if safety {
            core.state.revoke_device("office-switch").unwrap();
        }
        assert!(request_over(
            &mut client,
            &ControlRequest::SentryApprove {
                intent_id: intent.intent_id,
                digest
            }
        )
        .await
        .unwrap()
        .is_error());
        assert_eq!(sends.load(Ordering::SeqCst), 0);
        drop(client);
        server.await.unwrap();
        ha.abort();
    }
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL fixture"]
async fn unlisted_uid_and_context_free_dispatch_cannot_prepare_an_operation() {
    let (core, sends, ha) = fixture(false, true).await;
    let request = ControlRequest::SentryPrepare {
        resource_id: "office-switch".into(),
        command: Command::SwitchSet { on: true },
    };
    assert!(request.is_mutating());
    assert!(core.handle_control(request.clone()).await.is_error());
    let (mut client, server) = connect(core.clone()).await;
    assert!(request_over(&mut client, &request)
        .await
        .unwrap()
        .is_error());
    assert!(core.state.recent_device_operations(10).unwrap().is_empty());
    assert_eq!(sends.load(Ordering::SeqCst), 0);
    drop(client);
    server.await.unwrap();
    ha.abort();
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL and foreground terminal; run under script(1)"]
async fn closing_a_prepared_session_cancels_it_and_another_session_cannot_approve() {
    let (core, sends, ha) = fixture(true, true).await;
    let (mut client, server) = connect(core.clone()).await;
    let (intent, digest) = prepare(&mut client).await;
    drop(client);
    server.await.unwrap();
    assert_eq!(
        core.state
            .get_device_operation(intent.operation_id)
            .unwrap()
            .unwrap()
            .state,
        archie_state::DeviceOperationState::Cancelled
    );
    let (mut client, server) = connect(core).await;
    assert!(request_over(
        &mut client,
        &ControlRequest::SentryApprove {
            intent_id: intent.intent_id,
            digest
        }
    )
    .await
    .unwrap()
    .is_error());
    assert_eq!(sends.load(Ordering::SeqCst), 0);
    drop(client);
    server.await.unwrap();
    ha.abort();
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL and foreground terminal; run under script(1)"]
async fn pipelining_an_approval_closes_the_connection_without_dispatch() {
    let (core, sends, ha) = fixture(true, true).await;
    let (mut client, server) = connect(core).await;
    let (intent, digest) = prepare(&mut client).await;
    let mut bytes = archie_core::control::encode(&ControlRequest::SentryApprove {
        intent_id: intent.intent_id,
        digest,
    })
    .unwrap();
    bytes.extend(archie_core::control::encode(&ControlRequest::Ping).unwrap());
    client.write_all(&bytes).await.unwrap();
    server.await.unwrap();
    assert_eq!(sends.load(Ordering::SeqCst), 0);
    ha.abort();
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL and foreground terminal; run under script(1)"]
async fn disconnect_after_dispatch_keeps_a_queryable_fence_across_core_restart() {
    let (core, sends, ha) = fixture(true, true).await;
    let root = std::path::Path::new(
        &core
            .config
            .sentry
            .home_assistant
            .as_ref()
            .unwrap()
            .token_file,
    )
    .parent()
    .unwrap()
    .to_path_buf();
    std::fs::write(root.join("hold-apply"), "fixture gate").unwrap();
    let config = core.config.clone();
    let (mut client, server) = connect(core.clone()).await;
    let (intent, digest) = prepare(&mut client).await;
    client
        .write_all(
            &archie_core::control::encode(&ControlRequest::SentryApprove {
                intent_id: intent.intent_id.clone(),
                digest,
            })
            .unwrap(),
        )
        .await
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        while sends.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    drop(client);
    tokio::time::timeout(std::time::Duration::from_secs(2), server)
        .await
        .unwrap()
        .unwrap();
    ha.abort();
    drop(core);
    let restarted = Arc::new(
        Runtime::boot(StatePaths::rooted(&root), config)
            .await
            .unwrap(),
    );
    let query = ControlRequest::SentryReceipt {
        intent_id: intent.intent_id.clone(),
    };
    assert!(!query.is_mutating());
    assert!(restarted.handle_control(query.clone()).await.is_error());
    let (mut client, server) = connect(restarted).await;
    match request_over(&mut client, &query).await.unwrap() {
        ControlResponse::SentryReceipt(record) => {
            assert_eq!(record.intent_id, intent.intent_id);
            assert_eq!(record.operation_id, intent.operation_id);
            assert!(
                record.result.is_none(),
                "unknown outcome must remain fenced, never successful"
            );
        }
        other => panic!("durable fence disappeared: {other:?}"),
    }
    assert_eq!(sends.load(Ordering::SeqCst), 1);
    drop(client);
    server.await.unwrap();
    std::fs::remove_file(root.join("hold-apply")).unwrap();
}

#[tokio::test]
#[ignore = "requires built CLI/daemon, isolated PostgreSQL and python3 PTY fixture"]
async fn native_cli_reviews_on_a_terminal_and_executes_through_the_daemon() {
    use archie_sentry::journal::Journal;
    struct Daemon(std::process::Child);
    impl Drop for Daemon {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let (core, sends, ha) = fixture(true, true).await;
    let root = std::path::Path::new(
        &core
            .config
            .sentry
            .home_assistant
            .as_ref()
            .unwrap()
            .token_file,
    )
    .parent()
    .unwrap()
    .to_path_buf();
    std::fs::write(
        root.join("etc/archie.toml"),
        toml::to_string(&core.config).unwrap(),
    )
    .unwrap();
    let binaries = std::path::Path::new(&std::env::var("CARGO_TARGET_DIR").unwrap()).join("debug");
    let log = std::fs::File::create(root.join("daemon.log")).unwrap();
    let mut daemon = Daemon(
        std::process::Command::new(binaries.join("archied"))
            .args(["--root", root.to_str().unwrap()])
            .stdout(log.try_clone().unwrap())
            .stderr(log)
            .spawn()
            .unwrap(),
    );
    for _ in 0..50 {
        assert!(
            daemon.0.try_wait().unwrap().is_none(),
            "fixture daemon exited"
        );
        if root.join("run/archied.sock").exists() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    let code = r#"
import os, pty, fcntl, termios, subprocess, sys, select, time, re, json
master, slave = pty.openpty()
def terminal():
    os.setsid()
    fcntl.ioctl(0, termios.TIOCSCTTY, 0)
child = subprocess.Popen([sys.argv[1], '--root', sys.argv[2], '--json', 'sentry-execute', 'office-switch', 'switch-on'],
    stdin=slave, stdout=slave, stderr=slave, preexec_fn=terminal)
os.close(slave)
output = b''
confirmed = False
deadline = time.monotonic() + 15
try:
    while time.monotonic() < deadline:
        if select.select([master], [], [], .1)[0]:
            try: block = os.read(master, 65536)
            except OSError: break
            if not block: break
            output += block
            candidates = re.findall(rb'[0-9a-f]{64}', output)
            if not confirmed and candidates and b'Type the complete digest to approve' in output:
                digest = candidates[-1]
                os.write(master, digest + b'\n')
                confirmed = True
        if child.poll() is not None:
            continue
    assert child.wait(timeout=2) == 0, output.decode(errors='replace')
    assert confirmed, 'interactive confirmation was not requested'
    text = output.decode(errors='replace')
    result = None
    for match in re.finditer(r'\{', text):
        try: value, _ = json.JSONDecoder().raw_decode(text[match.start():])
        except json.JSONDecodeError: continue
        if isinstance(value, dict) and value.get('response') == 'sentry_result': result = value
    assert result and result['status'] == 'verified', text
    print(json.dumps({'intent_id': result['intent_id']}))
finally:
    if child.poll() is None: child.kill(); child.wait()
    os.close(master)
"#;
    let cli = binaries.join("archie");
    let output = tokio::task::spawn_blocking(move || {
        std::process::Command::new("python3")
            .args(["-c", code, cli.to_str().unwrap(), root.to_str().unwrap()])
            .output()
            .unwrap()
    })
    .await
    .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let record = core
        .state
        .sentry_journal()
        .unwrap()
        .load(value["intent_id"].as_str().unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(record.result.unwrap().status, ExecutionStatus::Verified);
    assert_eq!(sends.load(Ordering::SeqCst), 1);
    drop(daemon);
    ha.abort();
}
