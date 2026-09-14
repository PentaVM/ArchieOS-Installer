//! Downloads against a loopback server that misbehaves in each of the ways a real one can.

use archie_policy::{Action, CapabilityRequest, Constraints, PolicyEngine, Principal, Scope};
use archie_toolpack::{
    download, tool_resource, Catalog, FetchError, InstallAdmission, InstallGate, InstallProposal,
};
use archie_types::{AgentId, Timestamp, TraceId};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const BODY: &[u8] = b"the artifact the catalog describes, byte for byte";

/// Answers each connection by request path, then closes it.
async fn server() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            tokio::spawn(async move {
                let mut request = Vec::new();
                let mut buffer = [0u8; 1024];
                while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                    match socket.read(&mut buffer).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => request.extend_from_slice(&buffer[..n]),
                    }
                }
                let line = String::from_utf8_lossy(&request);
                let path = line.split_whitespace().nth(1).unwrap_or("/").to_string();
                let response: Vec<u8> = match path.as_str() {
                    "/good" => [format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", BODY.len()).as_bytes(), BODY].concat(),
                    "/longer" => [b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n".as_slice(), BODY, b"and then some more bytes"].concat(),
                    "/announced" => format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", BODY.len() + 1).into_bytes(),
                    "/redirect" => b"HTTP/1.1 302 Found\r\nLocation: https://elsewhere.example.net/artifact\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec(),
                    _ => b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec(),
                };
                let _ = socket.write_all(&response).await;
                let _ = socket.shutdown().await;
            });
        }
    });
    port
}

fn authorized(
    url: &str,
    sha256: &str,
    size: usize,
    root: &Path,
) -> archie_toolpack::AuthorizedInstall {
    let catalog = Catalog::parse(&format!(
        r#"version = 1
name = "fetch tests"
redirect_hosts = ["objects.example.org"]

[[tool]]
id = "fetched"
title = "Fetched"
version = "1.0.0"
license = "MIT"
risk = "read_only"

[tool.artifact]
url = "{url}"
sha256 = "{sha256}"
size = {size}
format = "binary"
binaries = ["fetched"]
"#
    ))
    .unwrap();
    let principal = Principal::Agent {
        agent: AgentId::new(),
    };
    let lease = PolicyEngine::default()
        .decide(&CapabilityRequest {
            trace_id: TraceId::new(),
            task_id: None,
            principal: principal.clone(),
            resource: tool_resource(root, "fetched"),
            scope: Scope::OneOperation { operation: 5 },
            constraints: Constraints::with_actions([Action::Write]),
            justification: "fetch".into(),
        })
        .lease
        .unwrap();
    let proposal = InstallProposal {
        tool_id: "fetched".into(),
        operation_id: 5,
        task_id: None,
    };
    match InstallGate::authorize(
        &catalog,
        root,
        &lease,
        &principal,
        false,
        &proposal,
        Timestamp::now(),
    ) {
        InstallAdmission::Allowed(install) => install,
        other => panic!("{other:?}"),
    }
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "archie-toolpack-fetch-{name}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[tokio::test]
async fn a_download_is_believed_only_when_size_and_digest_match() {
    let port = server().await;
    let dir = scratch("verify");
    let sha = hex::encode(Sha256::digest(BODY));
    let timeout = Duration::from_secs(10);

    let good = authorized(
        &format!("http://127.0.0.1:{port}/good"),
        &sha,
        BODY.len(),
        &dir,
    );
    let dest = dir.join("good");
    download(&good, &dest, timeout).await.unwrap();
    assert_eq!(std::fs::read(&dest).unwrap(), BODY);

    let wrong = authorized(
        &format!("http://127.0.0.1:{port}/good"),
        &"0".repeat(64),
        BODY.len(),
        &dir,
    );
    let dest = dir.join("wrong");
    assert!(matches!(
        download(&wrong, &dest, timeout).await,
        Err(FetchError::Digest)
    ));
    assert!(!dest.exists(), "a file that failed its digest is removed");

    let longer = authorized(
        &format!("http://127.0.0.1:{port}/longer"),
        &sha,
        BODY.len(),
        &dir,
    );
    let dest = dir.join("longer");
    assert!(matches!(
        download(&longer, &dest, timeout).await,
        Err(FetchError::TooLarge(_))
    ));
    assert!(!dest.exists());

    let announced = authorized(
        &format!("http://127.0.0.1:{port}/announced"),
        &sha,
        BODY.len(),
        &dir,
    );
    assert!(matches!(
        download(&announced, &dir.join("announced"), timeout).await,
        Err(FetchError::AnnouncedSize { .. })
    ));

    let redirected = authorized(
        &format!("http://127.0.0.1:{port}/redirect"),
        &sha,
        BODY.len(),
        &dir,
    );
    let dest = dir.join("redirected");
    assert!(matches!(
        download(&redirected, &dest, timeout).await,
        Err(FetchError::RedirectRefused)
    ));
    assert!(!dest.exists());

    let missing = authorized(
        &format!("http://127.0.0.1:{port}/missing"),
        &sha,
        BODY.len(),
        &dir,
    );
    assert!(matches!(
        download(&missing, &dir.join("missing"), timeout).await,
        Err(FetchError::Status(404))
    ));

    // An existing file at the destination is never overwritten.
    let occupied = dir.join("occupied");
    std::fs::write(&occupied, b"keep me").unwrap();
    assert!(download(&good, &occupied, timeout).await.is_err());
    assert_eq!(
        std::fs::read(&occupied).unwrap(),
        b"keep me",
        "and a failure does not delete it either"
    );
}
