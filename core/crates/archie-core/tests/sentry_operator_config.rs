#![cfg(target_os = "linux")]
use archie_core::{config::ArchieConfig, operator::KernelOperator};

fn ha_config() -> String {
    "[sentry]\noperator_uids=[1000]\n[sentry.home_assistant]\nendpoint='https://192.168.1.5:8123/'\ntoken_file='/etc/archie/ha.token'\n[[sentry.home_assistant.resources]]\nresource_id='office-switch'\nentity='switch.office'\n".into()
}

#[test]
fn operator_configuration_never_defaults_to_authority() {
    let defaults = ArchieConfig::default();
    assert!(defaults.sentry.operator_uids.is_empty());
    assert!(defaults.sentry.home_assistant.is_none());
    assert!(!defaults.policy.safety_boundary_present);
    assert!(ArchieConfig::from_toml(&ha_config()).is_ok());
    for bad in [
        ha_config().replace("[1000]", "[1000,1000]"),
        ha_config().replace("https://192.168.1.5:8123/", "http://192.168.1.5:8123/"),
        ha_config().replace("/etc/archie/ha.token", "relative.token"),
        ha_config().replace("switch.office", "lock.front_door"),
        ha_config().replace("office-switch", "bad/resource"),
    ] {
        assert!(ArchieConfig::from_toml(&bad).is_err());
    }
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn only_kernel_credentials_can_create_an_operator_context() {
    let (server, client) = tokio::net::UnixStream::pair().unwrap();
    let uid = client.peer_cred().unwrap().uid();
    assert!(KernelOperator::from_stream(&server, &[]).is_err());
    assert!(KernelOperator::from_stream(&server, &[uid.wrapping_add(1)]).is_err());
    let operator = KernelOperator::from_stream(&server, &[uid]).unwrap();
    assert_eq!(operator.uid(), uid);
    assert_eq!(operator.pid(), std::process::id());
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn replacing_the_controlling_terminal_invalidates_the_operator() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let path = std::env::temp_dir().join(format!("sentry-tty-{}", archie_types::TraceId::new()));
    let listener = tokio::net::UnixListener::bind(&path).unwrap();
    // A real session leader changes PTYs without changing its kernel PID/start time.
    let code = r#"
import os, pty, fcntl, termios, signal, socket, sys
os.setsid()
signal.signal(signal.SIGHUP, signal.SIG_IGN)
m1, s1 = pty.openpty()
fcntl.ioctl(s1, termios.TIOCSCTTY, 0)
os.tcsetpgrp(s1, os.getpgrp())
os.dup2(s1, 0)
sock = socket.socket(socket.AF_UNIX)
sock.connect(sys.argv[1])
sock.sendall(b'A')
assert sock.recv(1) == b'S'
fcntl.ioctl(s1, termios.TIOCNOTTY, 0)
m2, s2 = pty.openpty()
fcntl.ioctl(s2, termios.TIOCSCTTY, 0)
os.tcsetpgrp(s2, os.getpgrp())
os.dup2(s2, 0)
sock.sendall(b'B')
sock.recv(1)
"#;
    let mut child = std::process::Command::new("python3")
        .args(["-c", code, path.to_str().unwrap()])
        .spawn()
        .unwrap();
    let (mut stream, _) =
        tokio::time::timeout(std::time::Duration::from_secs(5), listener.accept())
            .await
            .unwrap()
            .unwrap();
    let uid = stream.peer_cred().unwrap().uid();
    let mut signal = [0];
    stream.read_exact(&mut signal).await.unwrap();
    assert_eq!(signal, [b'A']);
    let operator = KernelOperator::from_stream(&stream, &[uid]).unwrap();
    assert!(operator.check_present().is_ok());
    stream.write_all(b"S").await.unwrap();
    stream.read_exact(&mut signal).await.unwrap();
    assert_eq!(signal, [b'B']);
    let refused = operator.check_present().is_err();
    stream.write_all(b"X").await.unwrap();
    assert!(child.wait().unwrap().success());
    std::fs::remove_file(path).unwrap();
    assert!(
        refused,
        "a different terminal must invalidate the captured session"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn home_assistant_tokens_are_private_bounded_files_and_rotate_per_operation() {
    use std::os::unix::fs::{symlink, PermissionsExt};
    let root =
        std::env::temp_dir().join(format!("sentry-secrets-{}", archie_types::TraceId::new()));
    std::fs::create_dir(&root).unwrap();
    let token = root.join("ha.token");
    std::fs::write(&token, "fixture-token-one\n").unwrap();
    std::fs::set_permissions(&token, std::fs::Permissions::from_mode(0o600)).unwrap();
    let config = ArchieConfig::from_toml(
        &ha_config().replace("/etc/archie/ha.token", token.to_str().unwrap()),
    )
    .unwrap();
    let binding = config.sentry.home_assistant.as_ref().unwrap();
    assert!(binding.adapter().is_ok());
    std::fs::write(&token, "fixture-token-two").unwrap();
    assert!(binding.adapter().is_ok());
    std::fs::set_permissions(&token, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(binding.adapter().is_err());
    std::fs::set_permissions(&token, std::fs::Permissions::from_mode(0o600)).unwrap();
    std::fs::write(&token, "x".repeat(4097)).unwrap();
    assert!(binding.adapter().is_err());
    std::fs::remove_file(&token).unwrap();
    let actual = root.join("actual.token");
    std::fs::write(&actual, "fixture-token").unwrap();
    symlink(&actual, &token).unwrap();
    assert!(binding.adapter().is_err());
    std::fs::remove_file(token).unwrap();
    std::fs::remove_file(actual).unwrap();
    std::fs::remove_dir(root).unwrap();
}

#[tokio::test]
async fn the_next_adapter_sends_the_rotated_token_to_home_assistant() {
    use archie_sentry::{
        contracts::CommandKind, engine::EdgeAdapter, registry::ResourceDescriptor,
    };
    use std::os::unix::fs::PermissionsExt;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let root =
        std::env::temp_dir().join(format!("sentry-rotation-{}", archie_types::TraceId::new()));
    std::fs::create_dir(&root).unwrap();
    let token = root.join("ha.token");
    std::fs::write(&token, "fixture-token-one").unwrap();
    std::fs::set_permissions(&token, std::fs::Permissions::from_mode(0o600)).unwrap();
    let config = ArchieConfig::from_toml(
        &ha_config()
            .replace(
                "https://192.168.1.5:8123/",
                &format!("http://{}/", listener.local_addr().unwrap()),
            )
            .replace("/etc/archie/ha.token", token.to_str().unwrap()),
    )
    .unwrap();
    let server = tokio::spawn(async move {
        for index in 0..4 {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            let mut chunk = [0; 1024];
            loop {
                let n = stream.read(&mut chunk).await.unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&chunk[..n]);
                if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&bytes[..end]);
                    let length = headers
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .and_then(|n| n.trim().parse::<usize>().ok())
                        })
                        .unwrap_or(0);
                    if bytes.len() >= end + 4 + length {
                        break;
                    }
                }
                assert!(bytes.len() < 8192);
            }
            let headers = String::from_utf8_lossy(&bytes).to_ascii_lowercase();
            let expected = if index < 2 {
                "fixture-token-one"
            } else {
                "fixture-token-two"
            };
            assert!(headers.contains(&format!("authorization: bearer {expected}\r\n")));
            let body = if index % 2 == 0 {
                serde_json::json!([])
            } else {
                serde_json::json!({"entity_id":"switch.office","state":"off","attributes":{},
                    "last_reported":archie_types::Timestamp::now().to_rfc3339()})
            };
            let response = serde_json::to_vec(&body).unwrap();
            stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", response.len()).as_bytes()).await.unwrap();
            stream.write_all(&response).await.unwrap();
        }
    });
    let descriptor = ResourceDescriptor {
        resource_id: "office-switch".into(),
        edge_id: "local-core".into(),
        adapter_id: "home-assistant".into(),
        paired: true,
        capabilities: vec![CommandKind::SwitchSet],
    };
    let binding = config.sentry.home_assistant.unwrap();
    binding
        .adapter()
        .unwrap()
        .observe(&descriptor)
        .await
        .unwrap();
    std::fs::write(&token, "fixture-token-two").unwrap();
    binding
        .adapter()
        .unwrap()
        .observe(&descriptor)
        .await
        .unwrap();
    server.await.unwrap();
    std::fs::remove_file(token).unwrap();
    std::fs::remove_dir(root).unwrap();
}
