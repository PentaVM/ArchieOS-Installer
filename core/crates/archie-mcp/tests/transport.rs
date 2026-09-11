//! The HTTP transport, against a real socket.
//!
//! Every test here runs a loopback TCP listener and speaks actual HTTP to it. Nothing
//! is stubbed: the assertions about what Archie puts on the wire are made by reading
//! the bytes the server received, because a transport test that inspects its own
//! request builder proves only that the builder was called.

use archie_mcp::{
    CallerAuthority, HttpTransport, McpGate, McpRegistry, McpServerManifest, McpTransport,
    ToolAllowlist, ToolProposal, TransportError, PROTOCOL_VERSION,
};
use archie_types::{CapabilitySet, PrivacyGrade};
use archie_vault::{EntryClass, Vault, VaultKey, VaultRef};
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const T0: i64 = 1_760_000_000_000;

/// A server that answers each request in turn, and reports what it was sent.
async fn serve(
    responses: Vec<(&'static str, &'static str)>,
) -> (String, tokio::sync::oneshot::Receiver<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (sender, receiver) = tokio::sync::oneshot::channel();

    tokio::spawn(async move {
        let mut seen = Vec::new();
        for (content_type, body) in responses {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buffer = vec![0_u8; 64 * 1024];
            let read = socket.read(&mut buffer).await.unwrap();
            buffer.truncate(read);
            seen.push(String::from_utf8_lossy(&buffer).to_string());

            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nMcp-Session-Id: session-42\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            socket.write_all(response.as_bytes()).await.unwrap();
            let _ = socket.flush().await;
        }
        let _ = sender.send(seen);
    });

    (format!("http://{address}/mcp"), receiver)
}

fn manifest(url: &str) -> McpServerManifest {
    McpServerManifest {
        id: "local-tools".into(),
        display_name: "Local tools".into(),
        transport: McpTransport::StreamableHttp { url: url.into() },
        secret_handle: None,
        capabilities: CapabilitySet::new(),
        tools: ToolAllowlist::named(["create_issue"]),
        max_privacy_grade: PrivacyGrade::Internal,
        timeout_ms: 5_000,
        max_response_bytes: 65_536,
        enabled: true,
    }
}

const INIT_OK: &str = r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-06-18","capabilities":{},"serverInfo":{"name":"test","version":"1"}}}"#;

fn authorized(manifest: &McpServerManifest, tool: &str) -> archie_mcp::AuthorizedCall {
    let mut registry = McpRegistry::new();
    registry.register(manifest.clone()).unwrap();
    McpGate::authorize(
        &registry,
        &CallerAuthority::new(CapabilitySet::new()),
        &ToolProposal::new(
            &manifest.id,
            tool,
            json!({"title": "a bug"}),
            PrivacyGrade::Internal,
            "task-1",
            "trace-1",
        ),
    )
    .allowed()
    .expect("gate should admit")
    .clone()
}

#[tokio::test]
async fn initialize_negotiates_and_captures_the_session() {
    let (url, seen) = serve(vec![("application/json", INIT_OK)]).await;
    let transport = HttpTransport::connect(&manifest(&url)).unwrap();

    let result = transport.initialize(None).await.unwrap();
    assert_eq!(result.protocol_version, PROTOCOL_VERSION);
    assert_eq!(transport.session_id().as_deref(), Some("session-42"));

    let request = &seen.await.unwrap()[0];
    assert!(request.starts_with("POST /mcp "));
    assert!(request.contains(r#""method":"initialize""#));
    assert!(request.contains(PROTOCOL_VERSION));
    // Streamable HTTP: the client must accept either shape.
    assert!(request.contains("accept: application/json, text/event-stream"));
    // No sampling grant is offered, which is a property of the payload, not of the doc.
    assert!(!request.contains("sampling"));
}

#[tokio::test]
async fn a_tool_call_carries_the_session_and_the_negotiated_version() {
    let call_ok = r#"{"jsonrpc":"2.0","id":2,"result":{"content":[{"type":"text","text":"issue #7 created"}],"isError":false}}"#;
    let (url, seen) = serve(vec![
        ("application/json", INIT_OK),
        ("application/json", call_ok),
    ])
    .await;
    let m = manifest(&url);
    let transport = HttpTransport::connect(&m).unwrap();
    transport.initialize(None).await.unwrap();

    let result = transport
        .call_tool(&authorized(&m, "create_issue"), None)
        .await
        .unwrap();
    assert!(!result.is_error);
    assert_eq!(result.text(), "issue #7 created");

    let requests = seen.await.unwrap();
    let call = &requests[1];
    assert!(call.contains(r#""method":"tools/call""#));
    assert!(call.contains(r#""name":"create_issue""#));
    // The session the server assigned during initialize comes back on every request.
    assert!(call.contains("mcp-session-id: session-42"));
    assert!(call.contains(&format!("mcp-protocol-version: {PROTOCOL_VERSION}")));
}

#[tokio::test]
async fn a_server_answering_as_an_event_stream_is_understood() {
    // The half of streamable HTTP that a JSON-only client silently fails against.
    let sse = "event: message\ndata: {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"protocolVersion\":\"2025-06-18\",\"capabilities\":{}}}\n\n";
    let (url, _seen) = serve(vec![("text/event-stream", sse)]).await;
    let transport = HttpTransport::connect(&manifest(&url)).unwrap();

    let result = transport.initialize(None).await.unwrap();
    assert_eq!(result.protocol_version, PROTOCOL_VERSION);
}

#[tokio::test]
async fn a_notification_in_the_stream_is_skipped_rather_than_read_as_the_answer() {
    let sse = concat!(
        "data: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/progress\",\"params\":{}}\n\n",
        "data: {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"protocolVersion\":\"2025-06-18\",\"capabilities\":{}}}\n\n"
    );
    let (url, _seen) = serve(vec![("text/event-stream", sse)]).await;
    let transport = HttpTransport::connect(&manifest(&url)).unwrap();

    assert_eq!(
        transport.initialize(None).await.unwrap().protocol_version,
        PROTOCOL_VERSION
    );
}

#[tokio::test]
async fn a_response_answering_another_request_is_refused() {
    // Correlating the wrong response to a request would hand one task's tool output to
    // another. On a session-multiplexed transport that is a live possibility.
    let wrong_id =
        r#"{"jsonrpc":"2.0","id":99,"result":{"protocolVersion":"2025-06-18","capabilities":{}}}"#;
    let (url, _seen) = serve(vec![("application/json", wrong_id)]).await;
    let transport = HttpTransport::connect(&manifest(&url)).unwrap();

    assert!(matches!(
        transport.initialize(None).await,
        Err(TransportError::Protocol(
            archie_mcp::ProtocolError::MismatchedId { .. }
        ))
    ));
}

#[tokio::test]
async fn a_server_on_another_protocol_revision_is_refused() {
    let old =
        r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2024-01-01","capabilities":{}}}"#;
    let (url, _seen) = serve(vec![("application/json", old)]).await;
    let transport = HttpTransport::connect(&manifest(&url)).unwrap();

    assert!(matches!(
        transport.initialize(None).await,
        Err(TransportError::Protocol(
            archie_mcp::ProtocolError::UnsupportedVersion(_)
        ))
    ));
}

#[tokio::test]
async fn nothing_may_be_called_before_the_revision_is_agreed() {
    let (url, _seen) = serve(vec![]).await;
    let m = manifest(&url);
    let transport = HttpTransport::connect(&m).unwrap();

    assert!(matches!(
        transport
            .call_tool(&authorized(&m, "create_issue"), None)
            .await,
        Err(TransportError::NotInitialised)
    ));
    assert!(matches!(
        transport.list_tools(None).await,
        Err(TransportError::NotInitialised)
    ));
}

#[tokio::test]
async fn an_oversized_response_is_refused_before_it_is_collected() {
    let (url, _seen) = serve(vec![("application/json", INIT_OK)]).await;
    let mut m = manifest(&url);
    m.max_response_bytes = 32; // smaller than the response the server will send
    let transport = HttpTransport::connect(&m).unwrap();

    assert!(matches!(
        transport.initialize(None).await,
        Err(TransportError::ResponseTooLarge { .. })
    ));
}

#[tokio::test]
async fn a_brokered_credential_is_sent_as_a_bearer_token_and_appears_nowhere_else() {
    let call_ok = r#"{"jsonrpc":"2.0","id":2,"result":{"content":[{"type":"text","text":"ok"}],"isError":false}}"#;
    let (url, seen) = serve(vec![
        ("application/json", INIT_OK),
        ("application/json", call_ok),
    ])
    .await;

    let mut m = manifest(&url);
    let reference = VaultRef::parse("vault:tools/local").unwrap();
    m.secret_handle = Some(reference.clone());
    let transport = HttpTransport::connect(&m).unwrap();

    let mut vault = Vault::open(&VaultKey::from_bytes([3u8; 32])).unwrap();
    vault
        .store(
            "test",
            reference.clone(),
            EntryClass::Credential,
            PrivacyGrade::Secret,
            "tok_realsecretvalue",
            T0,
            None,
        )
        .unwrap();

    // Brokered immediately before each send, and consumed by it — a SealedCredential
    // cannot be cloned or held, so there is no way to obtain one earlier.
    transport
        .initialize(Some(
            vault
                .broker_credential("transport", &reference, T0)
                .unwrap(),
        ))
        .await
        .unwrap();

    let call = authorized(&m, "create_issue");
    assert_eq!(call.credential_ref(), Some(&reference));
    let result = transport
        .call_tool(
            &call,
            Some(
                vault
                    .broker_credential("transport", &reference, T0)
                    .unwrap(),
            ),
        )
        .await
        .unwrap();
    assert!(!result.is_error);

    let requests = seen.await.unwrap();
    assert!(requests[1].contains("authorization: Bearer tok_realsecretvalue"));
    // And the receipt that will be written down carries the name, not the secret.
    let receipt = serde_json::to_string(&call.receipt()).unwrap();
    assert!(!receipt.contains("tok_realsecretvalue"));
    assert!(receipt.contains("vault:tools/local"));

    // The vault recorded both brokerings, and recorded neither secret.
    let log = serde_json::to_string(vault.audit_log()).unwrap();
    assert!(!log.contains("tok_realsecretvalue"));
}

#[tokio::test]
async fn a_server_error_status_does_not_become_a_result() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut buffer = vec![0_u8; 8 * 1024];
        let _ = socket.read(&mut buffer).await.unwrap();
        let body = r#"{"error":"unauthorised"}"#;
        let response = format!(
            "HTTP/1.1 401 Unauthorized\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        socket.write_all(response.as_bytes()).await.unwrap();
    });

    let transport = HttpTransport::connect(&manifest(&format!("http://{address}/mcp"))).unwrap();
    match transport.initialize(None).await {
        Err(TransportError::Status { status, .. }) => assert_eq!(status, 401),
        other => panic!("expected a status error, got {other:?}"),
    }
}

#[tokio::test]
async fn the_transport_refuses_a_remote_plaintext_server() {
    let mut m = manifest("http://mcp.example.com/mcp");
    m.max_privacy_grade = PrivacyGrade::Public;
    // The manifest itself is invalid, and the transport refuses it rather than
    // assuming someone validated it on the way in.
    assert!(matches!(
        HttpTransport::connect(&m),
        Err(TransportError::Manifest(_))
    ));
}

#[tokio::test]
async fn a_stdio_manifest_is_not_carried_by_this_transport() {
    let mut m = manifest("http://127.0.0.1:1/mcp");
    m.transport = McpTransport::Stdio {
        command: "some-server".into(),
        args: vec![],
    };
    assert!(matches!(
        HttpTransport::connect(&m),
        Err(TransportError::NotHttp)
    ));
}

#[tokio::test]
async fn an_advertised_tool_list_is_bounded_and_still_not_an_authority() {
    let listing = r#"{"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"create_issue"},{"name":"delete_repository"}]}}"#;
    let (url, _seen) = serve(vec![
        ("application/json", INIT_OK),
        ("application/json", listing),
    ])
    .await;
    let m = manifest(&url);
    let transport = HttpTransport::connect(&m).unwrap();
    transport.initialize(None).await.unwrap();

    let advertised = transport.list_tools(None).await.unwrap();
    assert_eq!(advertised.tools.len(), 2);

    // The server offers two; the operator granted one.
    let names: Vec<&str> = advertised.tools.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(
        McpGate::permitted_tools(&m, names),
        vec!["create_issue".to_string()]
    );
}
