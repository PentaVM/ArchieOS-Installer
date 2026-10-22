use archie_core::control::{decode, encode, ControlRequest};
use archie_memory::inference::ContextDomain;

#[test]
fn ai_inference_is_typed_and_requires_operator_dispatch() {
    let request = decode::<ControlRequest>(
        r#"{"request":"ai_infer","request_body":{"domain":"personal/operator","focus":"conversation","privacy":"public","prompt":"hello","evidence":[],"runner":{"runner":"api"},"max_output_tokens":32}}"#,
    );
    // Construct through the public contract as well: wire JSON never names an
    // executable, client directory, policy file, or a credential location.
    let request = request.unwrap();
    assert!(request.is_mutating());
    assert!(request.is_ai());
    assert_eq!(request.name(), "ai-infer");
    let encoded = encode(&request).unwrap();
    assert_eq!(
        decode::<ControlRequest>(std::str::from_utf8(&encoded).unwrap()).unwrap(),
        request
    );
}

#[test]
fn ai_recall_requires_an_exact_receipt_binding_and_operator_dispatch() {
    let request = ControlRequest::AiRecall {
        binding: archie_memory::inference::InferenceBinding {
            schema_version: 1,
            domain: ContextDomain::personal("operator").unwrap(),
            focus_id: "focus:one".into(),
            task_id: "task:one".into(),
            objective_ref: "objective:one".into(),
            runner_id: "api".into(),
            provider_id: "local".into(),
            account_id: "local".into(),
        },
        receipt_id: "receipt:one".into(),
    };
    assert!(
        request.is_mutating(),
        "recall discloses admitted Vault content"
    );
    assert!(request.is_ai());
    let encoded = encode(&request).unwrap();
    assert_eq!(
        decode::<ControlRequest>(std::str::from_utf8(&encoded).unwrap()).unwrap(),
        request
    );
}

#[test]
fn ai_wire_does_not_accept_process_configuration() {
    assert!(decode::<ControlRequest>(
        r#"{"request":"ai_infer","request_body":{"domain":"personal/operator","focus":"conversation","privacy":"public","prompt":"hello","evidence":[],"runner":{"runner":"api"},"max_output_tokens":32},"executable":"/tmp/evil"}"#,
    ).is_err());
}

#[test]
fn harness_wire_is_operator_bound_and_cannot_configure_worker_paths() {
    let json = r#"{"request":"ai_harness","request_body":{"domain":"personal/operator","focus":"conversation","task_family":"fixture","run_id":"core-fixture-run","prompt":"hello","runner":{"runner":"api_pinned","model":"fixture/core"},"duration_ms":1000}}"#;
    let request = decode::<ControlRequest>(json).unwrap();
    assert!(request.is_ai());
    assert!(request.is_mutating());
    assert_eq!(request.name(), "ai-harness");
    let encoded = encode(&request).unwrap();
    assert_eq!(
        decode::<ControlRequest>(std::str::from_utf8(&encoded).unwrap()).unwrap(),
        request
    );
    let malicious = json.replace(
        "\"duration_ms\":1000",
        "\"duration_ms\":1000,\"python\":\"/tmp/evil\",\"worker\":\"/tmp/evil.py\"",
    );
    assert!(decode::<ControlRequest>(&malicious).is_err());
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL fixture"]
async fn context_free_dispatch_refuses_ai_before_opening_inference_configuration() {
    let root = std::env::temp_dir().join(format!(
        "archie-ai-control-{}",
        archie_types::TraceId::new()
    ));
    let runtime = archie_core::Runtime::boot(
        archie_state::StatePaths::rooted(&root),
        archie_core::ArchieConfig::default(),
    )
    .await
    .unwrap();
    let request = decode::<ControlRequest>(
        r#"{"request":"ai_infer","request_body":{"domain":"personal/operator","focus":"conversation","privacy":"public","prompt":"hello","evidence":[],"runner":{"runner":"api"},"max_output_tokens":32}}"#,
    ).unwrap();
    let response = runtime.handle_control(request).await;
    assert!(
        matches!(response, archie_core::ControlResponse::Error { message } if message == "authenticated local operator session required")
    );
    let harness = decode::<ControlRequest>(
        r#"{"request":"ai_harness","request_body":{"domain":"personal/operator","focus":"conversation","task_family":"fixture","run_id":"core-fixture-run","prompt":"hello","runner":{"runner":"api_pinned","model":"fixture/core"},"duration_ms":1000}}"#,
    ).unwrap();
    assert!(
        matches!(runtime.handle_control(harness).await, archie_core::ControlResponse::Error { message } if message == "authenticated local operator session required")
    );
    assert!(!runtime.paths.state_dir.join("inference").exists());
}

#[cfg(target_os = "linux")]
#[tokio::test]
#[ignore = "requires isolated PostgreSQL fixture; captured HTTP is not model-quality proof"]
async fn kernel_owner_dispatch_uses_runtime_state_and_rechecks_scope_for_recall() {
    use archie_core::{
        control::request_over, operator::serve_operator_connection, ControlResponse,
    };
    use archie_model_runtime::provider::{save_provider, ProviderProtocol, ProviderSpec};
    use archie_types::Locality;
    use archie_workstation::ai_policy::RuntimeProfile;
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let root =
        std::env::temp_dir().join(format!("archie-ai-owner-{}", archie_types::TraceId::new()));
    let paths = archie_state::StatePaths::rooted(&root);
    paths.ensure_directories().unwrap();
    let domain = ContextDomain::personal("operator").unwrap();
    RuntimeProfile::for_domain(domain.clone())
        .create(&paths.config_dir.join("inference.toml"))
        .unwrap();
    let runtime = Arc::new(
        archie_core::Runtime::boot(paths.clone(), archie_core::ArchieConfig::default())
            .await
            .unwrap(),
    );
    // Install the provider after boot to prove dispatch reloads the shared catalog.
    let http = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    save_provider(
        &paths.inference_providers(),
        ProviderSpec {
            id: "core-fixture".into(),
            protocol: ProviderProtocol::OpenAiCompatible,
            base_url: format!("http://{}/v1/", http.local_addr().unwrap()),
            locality: Locality::Local,
            credential_env: None,
            models: vec!["core-fixture-model".into()],
        },
    )
    .unwrap();
    let (inflight, started) = tokio::sync::oneshot::channel();
    let response_server = tokio::spawn(async move {
        let (mut stream, _) = http.accept().await.unwrap();
        let mut bytes = Vec::new();
        loop {
            let mut chunk = [0; 4096];
            let count = stream.read(&mut chunk).await.unwrap();
            assert!(count > 0);
            bytes.extend_from_slice(&chunk[..count]);
            assert!(bytes.len() < 262144);
            if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                let header = String::from_utf8_lossy(&bytes[..end]);
                let length: usize = header
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .map(|length| length.trim().parse().unwrap())
                    })
                    .unwrap();
                if bytes.len() >= end + 4 + length {
                    break;
                }
            }
        }
        assert!(String::from_utf8_lossy(&bytes).contains("hello"));
        let body = r#"{"model":"core-fixture-model","choices":[{"message":{"role":"assistant","content":"Daemon scoped proposal"},"finish_reason":"stop"}],"usage":{"prompt_tokens":10,"completion_tokens":4,"total_tokens":14}}"#;
        stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\nContent-Type: application/json\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
        drop(stream);
        // A second model request deliberately withholds its response so the
        // operator disconnect is observed during actual HTTP inference.
        let (mut held, _) = http.accept().await.unwrap();
        assert!(held.read(&mut [0; 4096]).await.unwrap() > 0);
        inflight.send(()).unwrap();
        let _ = held.read(&mut [0; 1]).await;
    });
    let (server, mut client) = tokio::net::UnixStream::pair().unwrap();
    let host = runtime.clone();
    let connection = tokio::spawn(async move {
        serve_operator_connection(server, host).await.unwrap();
    });
    let request = decode::<ControlRequest>(
        r#"{"request":"ai_infer","request_body":{"domain":"personal/operator","focus":"conversation","privacy":"public","prompt":"hello","evidence":[],"runner":{"runner":"api"},"max_output_tokens":32}}"#,
    ).unwrap();
    let outcome = match request_over(&mut client, &request).await.unwrap() {
        ControlResponse::AiResult { outcome } => *outcome,
        other => panic!("owner inference failed: {other:?}"),
    };
    assert_eq!(outcome.text.as_deref(), Some("Daemon scoped proposal"));
    assert_eq!(
        outcome.receipt.status,
        archie_memory::inference::InferenceStatus::Completed
    );
    assert_eq!(
        runtime
            .state
            .inference_receipt(&outcome.receipt.binding, &outcome.receipt.receipt_id)
            .unwrap()
            .unwrap(),
        outcome.receipt
    );
    let recall = ControlRequest::AiRecall {
        binding: outcome.receipt.binding.clone(),
        receipt_id: outcome.receipt.receipt_id.clone(),
    };
    assert!(runtime.handle_control(recall.clone()).await.is_error());
    match request_over(&mut client, &recall).await.unwrap() {
        ControlResponse::AiResult { outcome: recalled } => {
            assert_eq!(recalled.receipt, outcome.receipt);
            assert_eq!(recalled.text, outcome.text);
            assert!(recalled.budget.is_none());
        }
        other => panic!("owner recall failed: {other:?}"),
    }
    // Current profile controls the same already-authenticated connection.
    let changed = RuntimeProfile::for_domain(ContextDomain::work("other", "project").unwrap());
    std::fs::write(
        paths.config_dir.join("inference.toml"),
        toml::to_string(&changed).unwrap(),
    )
    .unwrap();
    assert!(request_over(&mut client, &recall).await.unwrap().is_error());
    std::fs::write(
        paths.config_dir.join("inference.toml"),
        toml::to_string(&RuntimeProfile::for_domain(domain)).unwrap(),
    )
    .unwrap();
    client.write_all(&encode(&request).unwrap()).await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), started)
        .await
        .unwrap()
        .unwrap();
    drop(client);
    tokio::time::timeout(std::time::Duration::from_secs(5), connection)
        .await
        .unwrap()
        .unwrap();
    let cancelled = runtime
        .state
        .recent_session_inferences(&outcome.receipt.binding, 10)
        .unwrap();
    assert!(cancelled
        .iter()
        .any(|receipt| receipt.status == archie_memory::inference::InferenceStatus::Cancelled));
    assert_eq!(cancelled.iter().filter(|receipt| receipt.status == archie_memory::inference::InferenceStatus::Completed).count(), 1);
    response_server.abort();
    drop(runtime);
}
