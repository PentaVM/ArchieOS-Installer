//! Actual Core/PG/Vault/Unix/pinned-worker integration with deterministic model
//! replies. This tests orchestration and authority, not model quality.
#![cfg(target_os = "linux")]

use archie_core::{
    control::{request_over, ControlRequest, ControlResponse},
    operator::serve_operator_connection,
    ArchieConfig, Runtime,
};
use archie_memory::inference::{ContextDomain, InferenceStatus};
use archie_model_runtime::provider::{save_provider, ProviderProtocol, ProviderSpec};
use archie_state::StatePaths;
use archie_types::{Locality, TraceId};
use archie_workstation::{ai_policy::RuntimeProfile, harness2::UPSTREAM_COMMIT};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn stage_worker(source: &Path, target: &Path) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir_all(target).unwrap();
    std::fs::set_permissions(target, std::fs::Permissions::from_mode(0o700)).unwrap();
    for entry in std::fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        if entry.file_name() == "__pycache__" {
            continue;
        }
        let destination = target.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            stage_worker(&entry.path(), &destination);
        } else {
            assert!(entry.file_type().unwrap().is_file());
            std::fs::copy(entry.path(), &destination).unwrap();
            std::fs::set_permissions(destination, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
    }
}

fn configure_worker(paths: &StatePaths) {
    let python = PathBuf::from(
        std::env::var_os("ARCHIE_HARNESS2_PYTHON")
            .expect("explicit supported interpreter required"),
    );
    let worker_source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .unwrap()
        .join("tools/harness2");
    // DrvFS marks the checkout writable by other users. Copy the pinned source
    // into a private disposable Linux directory rather than weaken launch checks.
    let worker_dir = paths.state_dir.join("private-worker");
    std::fs::create_dir_all(&worker_dir).unwrap();
    for name in ["worker.py", "upstream-manifest.json"] {
        std::fs::copy(worker_source.join(name), worker_dir.join(name)).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            worker_dir.join(name),
            std::fs::Permissions::from_mode(0o600),
        )
        .unwrap();
    }
    stage_worker(&worker_source.join("vendor"), &worker_dir.join("vendor"));
    let worker = worker_dir.join("worker.py");
    archie_workstation::harness_runtime::create_program_config(
        &paths.state_dir.join("inference/harness2.toml"),
        &python,
        &worker,
    )
    .unwrap();
}

#[tokio::test]
#[ignore = "requires isolated PG18 and ARCHIE_HARNESS2_PYTHON naming POSIX Python3.12/3.13"]
async fn kernel_owner_runs_pinned_harness_through_core_with_attributed_stage_receipts() {
    let root = std::env::temp_dir().join(format!("archie-core-harness-{}", TraceId::new()));
    let paths = StatePaths::rooted(&root);
    paths.ensure_directories().unwrap();
    configure_worker(&paths);
    let domain = ContextDomain::personal("harness-fixture").unwrap();
    RuntimeProfile::for_domain(domain.clone())
        .create(&paths.config_dir.join("inference.toml"))
        .unwrap();
    let runtime = Arc::new(
        Runtime::boot(paths.clone(), ArchieConfig::default())
            .await
            .unwrap(),
    );
    let http = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let provider = ProviderSpec {
        id: "harness-fixture".into(),
        protocol: ProviderProtocol::OpenAiCompatible,
        base_url: format!("http://{}/v1/", http.local_addr().unwrap()),
        locality: Locality::Local,
        credential_env: None,
        models: vec!["harness-fixture-model".into()],
    };
    let mut descriptor = provider.descriptors(&[]).unwrap().remove(0);
    // This deterministic fixture accepts the complete upstream proposer prompt.
    // Generic discovered catalogs intentionally retain a small unknown budget.
    descriptor.context_window = 32768;
    let model = descriptor.id.to_string();
    std::fs::create_dir_all(paths.models()).unwrap();
    std::fs::write(
        paths.models().join("fixture.toml"),
        toml::to_string(&descriptor).unwrap(),
    )
    .unwrap();
    let responses = tokio::spawn(async move {
        let mut base = BTreeMap::<String, String>::new();
        for _ in 0..8 {
            let (mut stream, _) = http.accept().await.unwrap();
            let mut bytes = Vec::new();
            let body = loop {
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
                                .map(|n| n.trim().parse().unwrap())
                        })
                        .unwrap();
                    if bytes.len() >= end + 4 + length {
                        break serde_json::from_slice::<Value>(&bytes[end + 4..end + 4 + length])
                            .unwrap();
                    }
                }
            };
            let message = body["messages"][1]["content"].as_str().unwrap();
            let stage: Value = serde_json::from_str(message)
                .or_else(|_| {
                    serde_json::from_str(message.rsplit("Current request:\n").next().unwrap())
                })
                .unwrap();
            let instruction = stage["instruction"].as_str().unwrap();
            let text = if instruction.contains("exactly artifact") {
                if base.is_empty() {
                    base = serde_json::from_value(stage["harness"].clone()).unwrap();
                }
                json!({"artifact":"Attributed fixture artifact", "trajectory":"[ACTION] Read admitted task\n[OBSERVATION] Fixture evidence only"}).to_string()
            } else if instruction.contains("exactly harness") {
                let mut candidate = base.clone();
                candidate.insert(
                    "LongTermMEMORY.md".into(),
                    "Preserve fixture attribution.".into(),
                );
                json!({"harness":candidate,"notes":"Deterministic integration fixture."})
                    .to_string()
            } else {
                "<experience>\ncue: fixture\nlesson: preserve attribution\nevidence: observed fixture artifact\nconfidence: limited\n</experience>".into()
            };
            let body = json!({"model":"harness-fixture-model","choices":[{"message":{"role":"assistant","content":text},"finish_reason":"stop"}],"usage":{"prompt_tokens":20,"completion_tokens":20,"total_tokens":40}}).to_string();
            stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\nContent-Type: application/json\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
        }
    });
    let (server, mut client) = tokio::net::UnixStream::pair().unwrap();
    let host = runtime.clone();
    let connection = tokio::spawn(async move {
        serve_operator_connection(server, host).await.unwrap();
    });
    let request = archie_core::control::decode::<ControlRequest>(&json!({
        "request":"ai_harness", "request_body":{
            "domain":domain.path(), "focus":"harness-fixture", "task_family":"summaries", "run_id":"core-harness-fixture",
            "prompt":"Summarize only this public deterministic fixture.", "runner":{"runner":"api_pinned","model":model}, "duration_ms":60000,
        }
    }).to_string()).unwrap();
    assert!(runtime.handle_control(request.clone()).await.is_error());
    for duration in [0_u64, 300001] {
        let mut invalid = serde_json::to_value(&request).unwrap();
        invalid["request_body"]["duration_ms"] = json!(duration);
        let invalid: ControlRequest = serde_json::from_value(invalid).unwrap();
        assert!(request_over(&mut client, &invalid)
            .await
            .unwrap()
            .is_error());
    }
    let result = tokio::time::timeout(Duration::from_secs(65), request_over(&mut client, &request))
        .await
        .unwrap()
        .unwrap();
    let outcome = match result {
        ControlResponse::AiHarnessResult { outcome } => outcome,
        other => panic!("Core Harness² refused: {other:?}"),
    };
    assert_eq!(outcome.result.upstream_commit, UPSTREAM_COMMIT);
    assert_eq!(outcome.result.ledger["R"], 4);
    assert_eq!(outcome.result.ledger["P"], 3);
    assert_eq!(outcome.result.promotion.status, "baseline_retained");
    assert!(!outcome.result.promotion.improvement_verified);
    assert_eq!(outcome.receipts.len(), 8);
    for receipt in &outcome.receipts {
        assert_eq!(receipt.status, InferenceStatus::Completed);
        assert_eq!(receipt.binding.domain, domain);
        assert_eq!(
            runtime
                .state
                .inference_receipt(&receipt.binding, &receipt.receipt_id)
                .unwrap()
                .as_ref(),
            Some(receipt)
        );
    }
    drop(client);
    connection.await.unwrap();
    responses.await.unwrap();
}

#[tokio::test]
#[ignore = "requires isolated PG18 and ARCHIE_HARNESS2_PYTHON naming POSIX Python3.12/3.13"]
async fn harness_total_deadline_cancels_the_active_model_and_records_no_success() {
    let root =
        std::env::temp_dir().join(format!("archie-core-harness-deadline-{}", TraceId::new()));
    let paths = StatePaths::rooted(&root);
    paths.ensure_directories().unwrap();
    configure_worker(&paths);
    let domain = ContextDomain::personal("harness-deadline").unwrap();
    let profile = RuntimeProfile::for_domain(domain.clone());
    assert_eq!(profile.max_duration_ms, 60000);
    profile
        .create(&paths.config_dir.join("inference.toml"))
        .unwrap();
    let runtime = Arc::new(
        Runtime::boot(paths.clone(), ArchieConfig::default())
            .await
            .unwrap(),
    );
    let http = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let provider = ProviderSpec {
        id: "harness-deadline".into(),
        protocol: ProviderProtocol::OpenAiCompatible,
        base_url: format!("http://{}/v1/", http.local_addr().unwrap()),
        locality: Locality::Local,
        credential_env: None,
        models: vec!["harness-deadline-model".into()],
    };
    let model = provider.descriptors(&[]).unwrap()[0].id.to_string();
    save_provider(&paths.inference_providers(), provider).unwrap();
    let (sent, started) = tokio::sync::oneshot::channel();
    let slow_model = tokio::spawn(async move {
        let (mut stream, _) = http.accept().await.unwrap();
        assert!(stream.read(&mut [0; 4096]).await.unwrap() > 0);
        sent.send(()).unwrap();
        // Deliberately no response: the 250ms whole-run deadline must stop a
        // model whose independent per-stage profile otherwise permits 60s.
        std::future::pending::<()>().await;
        drop(stream);
    });
    let (server, mut client) = tokio::net::UnixStream::pair().unwrap();
    let host = runtime.clone();
    let connection = tokio::spawn(async move {
        serve_operator_connection(server, host).await.unwrap();
    });
    let request = archie_core::control::decode::<ControlRequest>(&json!({
        "request":"ai_harness", "request_body":{
            "domain":domain.path(), "focus":"deadline", "task_family":"summaries", "run_id":"core-harness-deadline",
            "prompt":"A public fixture whose provider deliberately stalls.", "runner":{"runner":"api_pinned","model":model}, "duration_ms":250,
        }
    }).to_string()).unwrap();
    let result = tokio::time::timeout(Duration::from_secs(5), request_over(&mut client, &request))
        .await
        .unwrap()
        .unwrap();
    assert!(result.is_error());
    tokio::time::timeout(Duration::from_secs(1), started)
        .await
        .expect("fixture must reach actual HTTP inference before the total deadline")
        .unwrap();
    runtime.state.test_sql("DO $$ BEGIN IF EXISTS (SELECT 1 FROM inference_receipts WHERE payload_json::jsonb->>'status'='completed') THEN RAISE EXCEPTION 'deadline produced successful receipt'; END IF; IF NOT EXISTS (SELECT 1 FROM inference_receipts WHERE payload_json::jsonb->>'status'='cancelled') THEN RAISE EXCEPTION 'cancelled attempt was not durable'; END IF; END $$;").unwrap();
    drop(client);
    connection.await.unwrap();
    slow_model.abort();
}
