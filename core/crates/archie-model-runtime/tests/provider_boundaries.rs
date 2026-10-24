use archie_model_runtime::{InferenceClient, ProviderEndpoint};
use archie_types::Locality;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

#[test]
fn a_stream_without_a_terminal_event_is_not_completed() {
    let mut assembler = archie_model_runtime::SseAssembler::new();
    assembler
        .feed(b"data: {\"choices\":[{\"delta\":{\"content\":\"partial\"}}]}\n\n")
        .unwrap();
    assert!(assembler.close().is_err());
}

#[test]
fn a_complete_oversized_sse_line_is_refused_before_decoding() {
    let mut assembler = archie_model_runtime::SseAssembler::new();
    let input = format!(
        "data: {{}}{}\n",
        " ".repeat(archie_model_runtime::stream::MAX_EVENT_LINE)
    );
    assert!(matches!(
        assembler.feed(input.as_bytes()),
        Err(archie_model_runtime::SseError::LineTooLong)
    ));
}

#[test]
fn streamed_request_content_is_bounded_before_serializing_or_connecting() {
    let request = archie_model_runtime::ToolChatRequest {
        model: "model".into(),
        messages: vec![archie_model_runtime::ToolChatMessage::User(
            "x".repeat(2 * 1024 * 1024 + 1),
        )],
        tools: vec![],
        max_tokens: 128,
        temperature: 0.2,
    };
    assert!(request.validate().is_err());
}

#[test]
fn a_remote_https_host_cannot_claim_local_inference() {
    assert!(ProviderEndpoint::openai_compatible(
        "https://cloud.example/v1/",
        Locality::Local,
        None,
    )
    .is_err());
}

#[tokio::test]
async fn retry_after_consumes_the_same_total_deadline() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/v1/", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                break;
            };
            let mut request = [0; 4096];
            let _ = socket.read(&mut request).await;
            let _ = socket.write_all(b"HTTP/1.1 429 Too Many Requests\r\nRetry-After: 1\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
        }
    });
    let client = InferenceClient::new(
        ProviderEndpoint::openai_compatible(&endpoint, Locality::Local, None)
            .unwrap()
            .with_limits(Duration::from_millis(60), 4096)
            .unwrap(),
    )
    .unwrap();
    let started = Instant::now();
    assert!(client.models().await.is_err());
    server.abort();
    assert!(
        started.elapsed() < Duration::from_millis(350),
        "elapsed {:?}",
        started.elapsed()
    );
}

#[tokio::test]
async fn unknown_length_body_is_refused_incrementally() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/v1/", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = [0; 4096];
        let _ = socket.read(&mut request).await;
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n")
            .await
            .unwrap();
        socket.write_all(&[b'x'; 128]).await.unwrap();
        tokio::time::sleep(Duration::from_secs(2)).await;
    });
    let client = InferenceClient::new(
        ProviderEndpoint::openai_compatible(&endpoint, Locality::Local, None)
            .unwrap()
            .with_limits(Duration::from_secs(3), 64)
            .unwrap(),
    )
    .unwrap();
    let started = Instant::now();
    assert!(matches!(
        client.models().await,
        Err(archie_model_runtime::RuntimeError::ResponseTooLarge)
    ));
    assert!(started.elapsed() < Duration::from_secs(1));
    server.abort();
}

fn local_descriptor(endpoint: String, id: &str, quality: u8) -> archie_types::ModelDescriptor {
    let mut provider = archie_model_runtime::provider::builtin_provider("ollama").unwrap();
    provider.base_url = endpoint;
    provider.models = vec!["model".into()];
    let mut model = provider.descriptors(&[]).unwrap().remove(0);
    model.id = archie_types::ModelId::parse(id).unwrap();
    model.quality_tier = quality;
    model
}

#[tokio::test]
async fn stream_retries_share_the_total_deadline() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/v1/", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            let _ = socket.read(&mut request).await;
            let _ = socket.write_all(b"HTTP/1.1 429 Too Many Requests\r\nRetry-After: 1\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
        }
    });
    let model = local_descriptor(endpoint.clone(), "local.stream", 0);
    let client = InferenceClient::new(
        ProviderEndpoint::openai_compatible(&endpoint, Locality::Local, None)
            .unwrap()
            .with_limits(Duration::from_millis(60), 4096)
            .unwrap(),
    )
    .unwrap();
    let request = archie_model_runtime::ToolChatRequest {
        model: "model".into(),
        messages: vec![archie_model_runtime::ToolChatMessage::User("Hello".into())],
        tools: vec![],
        max_tokens: 128,
        temperature: 0.2,
    };
    let started = Instant::now();
    let mut events = Vec::new();
    assert!(client
        .chat_stream(
            &model,
            archie_types::PrivacyGrade::Restricted,
            &request,
            |event| events.push(event)
        )
        .await
        .is_err());
    assert!(events.is_empty());
    assert!(started.elapsed() < Duration::from_millis(350));
    server.abort();
}

#[tokio::test]
async fn fallback_models_share_one_question_deadline() {
    let first = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let second = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut registry = archie_registry::ModelRegistry::new();
    registry
        .register(local_descriptor(
            format!("http://{}/v1/", first.local_addr().unwrap()),
            "local.first",
            100,
        ))
        .unwrap();
    registry
        .register(local_descriptor(
            format!("http://{}/v1/", second.local_addr().unwrap()),
            "local.second",
            0,
        ))
        .unwrap();
    let first_server = tokio::spawn(async move {
        let (mut socket, _) = first.accept().await.unwrap();
        let mut request = [0; 4096];
        let _ = socket.read(&mut request).await;
        tokio::time::sleep(Duration::from_millis(25)).await;
        socket
            .write_all(
                b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            )
            .await
            .unwrap();
    });
    let (connected, observed) = tokio::sync::oneshot::channel();
    let second_server = tokio::spawn(async move {
        let (_socket, _) = second.accept().await.unwrap();
        let _ = connected.send(());
        tokio::time::sleep(Duration::from_secs(2)).await;
    });
    let routed = archie_model_runtime::routed::Routed::new(registry);
    let question = archie_model_runtime::routed::Question {
        role: archie_types::ModelRole::CognitiveResponse,
        privacy: archie_types::PrivacyGrade::Restricted,
        system: "Answer",
        user: "Hello",
        image: None,
        max_tokens: 128,
        timeout: Duration::from_millis(80),
    };
    let started = Instant::now();
    assert!(routed.ask(&question).await.is_err());
    assert!(started.elapsed() < Duration::from_millis(250));
    observed.await.unwrap();
    first_server.await.unwrap();
    second_server.abort();
}
