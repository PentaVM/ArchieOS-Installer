use archie_core::{
    config::ArchieConfig,
    control::{request_over, serve_connection, ControlRequest, ControlResponse},
    endpoint, Runtime,
};
use archie_state::StatePaths;
use archie_types::TraceId;
use std::{sync::Arc, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

const UUID: &str = "urn:uuid:e3248000-80ce-11db-8000-202b20a4fe3b";

fn attributes(uuid: Option<&str>) -> Vec<u8> {
    fn attr(out: &mut Vec<u8>, tag: u8, name: &str, value: &[u8]) {
        out.push(tag);
        out.extend_from_slice(&(name.len() as u16).to_be_bytes());
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(&(value.len() as u16).to_be_bytes());
        out.extend_from_slice(value);
    }
    let mut bytes = vec![1, 1, 0, 0, 0, 0, 0, 1, 1];
    attr(&mut bytes, 0x47, "attributes-charset", b"utf-8");
    bytes.push(4);
    attr(&mut bytes, 0x23, "printer-state", &3i32.to_be_bytes());
    if let Some(uuid) = uuid {
        attr(&mut bytes, 0x45, "printer-uuid", uuid.as_bytes());
    }
    bytes.push(3);
    bytes
}

async fn printer(uuid: Option<&str>) -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("ipp://{}/ipp/print", listener.local_addr().unwrap());
    let body = attributes(uuid);
    let task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        let mut chunk = [0; 2048];
        loop {
            let count = stream.read(&mut chunk).await.unwrap();
            assert!(count > 0);
            request.extend_from_slice(&chunk[..count]);
            assert!(request.len() < 8192);
            if let Some(end) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                let text = String::from_utf8_lossy(&request[..end]);
                let length: usize = text
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .map(|n| n.trim().parse().unwrap())
                    })
                    .unwrap();
                if request.len() >= end + 4 + length {
                    assert!(text.starts_with("POST /ipp/print HTTP/1.1"));
                    assert_eq!(&request[end + 4..end + 8], &[1, 1, 0, 11]); // Get-Printer-Attributes only.
                    break;
                }
            }
        }
        stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/ipp\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).as_bytes()).await.unwrap();
        stream.write_all(&body).await.unwrap();
    });
    (endpoint, task)
}

async fn runtime(endpoint: &str) -> Arc<Runtime> {
    let config = ArchieConfig::from_toml(&format!("[[sentry.printers]]\nresource_id='office-printer'\nendpoint='{endpoint}'\nexpected_uuid='{UUID}'\n")).unwrap();
    let root = std::env::temp_dir().join(format!("archie-sentry-printer-{}", TraceId::new()));
    Arc::new(
        Runtime::boot(StatePaths::rooted(root), config)
            .await
            .unwrap(),
    )
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL 18 fixture"]
async fn ipp_diagnostic_roundtrips_through_the_real_core_socket() {
    let (uri, printer) = printer(Some(UUID)).await;
    let runtime = runtime(&uri).await;
    let listener = endpoint::bind(&runtime.paths).await.unwrap();
    let core = runtime.clone();
    let server = tokio::spawn(async move {
        let stream = endpoint::accept(&listener).await.unwrap();
        serve_connection(stream, |request| {
            let core = core.clone();
            async move { core.handle_control(request).await }
        })
        .await
        .unwrap();
    });
    let mut stream = endpoint::connect(&runtime.paths).await.unwrap();
    let request = ControlRequest::SentryPrinter {
        resource_id: "office-printer".into(),
    };
    let response = request_over(&mut stream, &request).await.unwrap();
    match &response {
        ControlResponse::SentryPrinter {
            resource_id,
            snapshot,
        } => {
            assert_eq!(resource_id, "office-printer");
            assert_eq!(snapshot.uuid.as_deref(), Some(UUID));
        }
        other => panic!("unexpected response: {other:?}"),
    }
    assert!(!serde_json::to_string(&response).unwrap().contains(&uri));
    assert!(runtime.state.list_devices().unwrap().is_empty());
    assert!(runtime
        .state
        .recent_device_operations(10)
        .unwrap()
        .is_empty());
    drop(stream);
    server.await.unwrap();
    printer.await.unwrap();
    endpoint::cleanup(&runtime.paths);
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL 18 fixture"]
async fn printer_diagnostic_refuses_missing_or_changed_device_identity() {
    for uuid in [None, Some("urn:uuid:e3248000-80ce-11db-8000-202b20a4fe3c")] {
        let (uri, printer) = printer(uuid).await;
        let core = runtime(&uri).await;
        let response = core
            .handle_control(ControlRequest::SentryPrinter {
                resource_id: "office-printer".into(),
            })
            .await;
        assert_eq!(
            response,
            ControlResponse::error("Sentry printer identity mismatch")
        );
        printer.await.unwrap();
    }
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL 18 fixture"]
async fn unknown_printer_resource_never_opens_a_network_connection() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let core = runtime(&format!(
        "ipp://{}/ipp/print",
        listener.local_addr().unwrap()
    ))
    .await;
    for resource_id in ["unknown", "bad/resource"] {
        assert!(core
            .handle_control(ControlRequest::SentryPrinter {
                resource_id: resource_id.into()
            })
            .await
            .is_error());
    }
    assert!(
        tokio::time::timeout(Duration::from_millis(50), listener.accept())
            .await
            .is_err()
    );
}
