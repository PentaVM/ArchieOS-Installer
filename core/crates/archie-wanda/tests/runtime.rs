use archie_types::{
    Availability, Capability, DataRetention, Locality, ModelDescriptor, ModelId, PrivacyGrade,
    ProviderKind, TokenCost,
};
use archie_vault::ContextDomain;
use archie_wanda::{
    config::{CoreBinding, DomainBinding, SourceBinding, WandaConfig},
    runtime::query,
    store::WandaStore,
    types::*,
};
use axum::{extract::State, routing::post, Json, Router};
use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};
fn fixture(path: &std::path::Path, url: &str, locality: Locality) -> WandaStore {
    fixture_with(path, url, locality, None)
}
/// With a Core root, the manifest goes where Core reads its catalog, so both see one model.
fn fixture_with(
    path: &std::path::Path,
    url: &str,
    locality: Locality,
    core: Option<CoreBinding>,
) -> WandaStore {
    let domain = ContextDomain::personal("owner").unwrap();
    std::fs::write(path.join("source.token"), "source-fixture-secret").unwrap();
    let models = match core.as_ref().and_then(|core| core.root.as_ref()) {
        Some(root) => archie_state::StatePaths::rooted(root).models(),
        None => path.join("models"),
    };
    std::fs::create_dir_all(&models).unwrap();
    let descriptor = ModelDescriptor {
        id: ModelId::parse("fixture.reasoner").unwrap(),
        display_name: "Fixture".into(),
        provider: ProviderKind::OpenAiCompatible,
        locality,
        node: None,
        context_window: 64000,
        max_output_tokens: 4096,
        capabilities: vec![Capability::known("cognition.reasoning")],
        max_privacy_grade: PrivacyGrade::Restricted,
        retention: DataRetention::ZeroVerified,
        availability: Availability::Ready,
        cost: TokenCost::FREE,
        latency_ema_ms: Some(100),
        quality_tier: 90,
        required_accelerator_bytes: None,
        labels: BTreeMap::from([("base-url".into(), url.into())]),
    };
    std::fs::write(
        models.join("model.toml"),
        toml::to_string(&descriptor).unwrap(),
    )
    .unwrap();
    let store = WandaStore::new(WandaConfig {
        state_dir: path.join("state"),
        graph_binary: path.join("absent-cli"),
        models_dir: models,
        operator_token_file: path.join("owner.token"),
        domains: vec![DomainBinding {
            domain: domain.clone(),
            graph_path: path.join("graph.tsv"),
        }],
        sources: vec![SourceBinding {
            device_id: "phone".into(),
            domain: domain.clone(),
            source: EventSource::AndroidNotification,
            token_file: Some(path.join("source.token")),
            token_env: None,
            allow_presumed_owner: false,
            enabled: true,
            chat_domains: BTreeMap::new(),
        }],
        quality_tier: 50,
        deep_quality_tier: 80,
        retention_hours: 168,
        parallel_queries: 1,
        core,
        lncm: None,
    })
    .unwrap();
    store
        .ingest(
            "source-fixture-secret",
            &[WandaEvent {
                schema_version: 1,
                event_id: "one".into(),
                device_id: "phone".into(),
                actor: Actor::ConfirmedOwner,
                author: Author::Counterparty,
                source: EventSource::AndroidNotification,
                chat_id: "one".into(),
                sender: "Ana".into(),
                source_message_id: None,
                sent_at_ms: None,
                observed_at_ms: 1000,
                text: "Me encantan los tulipanes. PRIVATE-CONTEXT".into(),
                media_omitted: false,
                coverage: Coverage::NotificationPreview,
                monitor_id: None,
                window_id: None,
            }],
            1000,
        )
        .unwrap();
    store
}
async fn answer(
    State(calls): State<Arc<AtomicUsize>>,
    Json(request): Json<serde_json::Value>,
) -> Json<serde_json::Value> {
    calls.fetch_add(1, Ordering::SeqCst);
    let context = request["messages"][1]["content"].as_str().unwrap();
    assert!(context.contains("PRIVATE-CONTEXT"));
    let start = context.find("vault:personal/owner/wanda/events/").unwrap();
    let reference = context[start..]
        .split(|c: char| c.is_whitespace() || c == '"' || c == ']')
        .next()
        .unwrap();
    let marker = context.find("\"now_ms\":").unwrap() + 9;
    let now: i64 = context[marker..]
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect::<String>()
        .parse()
        .unwrap();
    let content=serde_json::json!({"recommendations":[{"id":"gift","kind":"gift","title":"Considerar flores","explanation":"El mensaje expresa gusto por tulipanes.","suggestion":"Podrías preguntarle si le gustaría recibirlos.","subject_ids":[],"evidence_refs":[reference],"standing":"inferred","alternatives":[],"expires_at_ms":now+60_000,"review_condition":"Verifica la preferencia antes de comprar."}]}).to_string();
    Json(
        serde_json::json!({"model":"fixture.reasoner","choices":[{"index":0,"message":{"role":"assistant","content":content},"finish_reason":"stop"}]}),
    )
}
#[tokio::test]
async fn local_inference_uses_existing_gateway_and_routed_runtime() {
    let calls = Arc::new(AtomicUsize::new(0));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(
        axum::serve(
            listener,
            Router::new()
                .route("/v1/chat/completions", post(answer))
                .with_state(calls.clone()),
        )
        .into_future(),
    );
    let dir = tempfile::tempdir().unwrap();
    let store = fixture(
        dir.path(),
        &format!("http://{address}/v1/"),
        Locality::Local,
    );
    let result = query(
        &store,
        &ContextDomain::personal("owner").unwrap(),
        "¿Qué detalle podría tener?",
        true,
        1000,
    )
    .await
    .unwrap();
    assert_eq!(
        result.recommendations.len(),
        1,
        "state={}, reason={:?}, calls={}",
        result.state,
        result.reason,
        calls.load(Ordering::SeqCst)
    );
    assert_eq!(result.model.as_deref(), Some("fixture.reasoner"));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    task.abort();
}
#[tokio::test]
async fn a_cloud_manifest_cannot_receive_private_relationship_context() {
    let calls = Arc::new(AtomicUsize::new(0));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(
        axum::serve(
            listener,
            Router::new()
                .route("/v1/chat/completions", post(answer))
                .with_state(calls.clone()),
        )
        .into_future(),
    );
    let dir = tempfile::tempdir().unwrap();
    let store = fixture(
        dir.path(),
        &format!("http://{address}/v1/"),
        Locality::Cloud,
    );
    let result = query(
        &store,
        &ContextDomain::personal("owner").unwrap(),
        "Consulta privada",
        false,
        1000,
    )
    .await
    .unwrap();
    assert!(result.recommendations.is_empty());
    assert_eq!(result.state, "no_model");
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    task.abort();
}
use std::future::IntoFuture;

#[tokio::test]
async fn proactive_refresh_coalesces_unchanged_context_but_processes_new_messages() {
    let calls = Arc::new(AtomicUsize::new(0));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let model_task = tokio::spawn(
        axum::serve(
            listener,
            Router::new()
                .route("/v1/chat/completions", post(answer))
                .with_state(calls.clone()),
        )
        .into_future(),
    );
    let dir = tempfile::tempdir().unwrap();
    let store = fixture(
        dir.path(),
        &format!("http://{address}/v1/"),
        Locality::Local,
    );
    let domain = ContextDomain::personal("owner").unwrap();
    let mut event = store.events(&domain, 1, 1000).unwrap().remove(0).event;
    event.event_id = "live-1".into();
    event.observed_at_ms = archie_wanda::server::now_ms();
    store
        .ingest(
            "source-fixture-secret",
            &[event.clone()],
            event.observed_at_ms,
        )
        .unwrap();
    let service = archie_wanda::server::Service::new(store);
    service.refresh().await;
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    service.refresh().await;
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let api_task = tokio::spawn(
        axum::serve(
            listener,
            archie_wanda::server::router_with_service(service.clone()),
        )
        .into_future(),
    );
    event.event_id = "live-2".into();
    event.source_message_id = Some("live-2".into());
    event.text = "PRIVATE-CONTEXT tenemos que entregar el plano el viernes".into();
    event.observed_at_ms = archie_wanda::server::now_ms();
    let response = reqwest::Client::new()
        .post(format!("http://{address}/v1/events"))
        .bearer_auth("source-fixture-secret")
        .json(&vec![event])
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());
    service.refresh().await;
    assert_eq!(
        calls.load(Ordering::SeqCst),
        2,
        "new context cannot be held back by an unexpired recommendation"
    );
    api_task.abort();
    model_task.abort();
}

async fn refuse(State(calls): State<Arc<AtomicUsize>>) -> axum::http::StatusCode {
    calls.fetch_add(1, Ordering::SeqCst);
    axum::http::StatusCode::INTERNAL_SERVER_ERROR
}

#[tokio::test]
async fn a_failing_model_is_retried_with_backoff_not_every_tick() {
    let calls = Arc::new(AtomicUsize::new(0));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let model_task = tokio::spawn(
        axum::serve(
            listener,
            Router::new()
                .route("/v1/chat/completions", post(refuse))
                .with_state(calls.clone()),
        )
        .into_future(),
    );
    let dir = tempfile::tempdir().unwrap();
    let store = fixture(
        dir.path(),
        &format!("http://{address}/v1/"),
        Locality::Local,
    );
    let domain = ContextDomain::personal("owner").unwrap();
    let mut event = store.events(&domain, 1, 1000).unwrap().remove(0).event;
    event.event_id = "live-1".into();
    event.observed_at_ms = archie_wanda::server::now_ms();
    store
        .ingest(
            "source-fixture-secret",
            &[event.clone()],
            event.observed_at_ms,
        )
        .unwrap();
    let service = archie_wanda::server::Service::new(store);
    service.refresh().await;
    let first = calls.load(Ordering::SeqCst);
    assert!(first >= 1, "the model was never asked");
    service.refresh().await;
    service.refresh().await;
    assert_eq!(
        calls.load(Ordering::SeqCst),
        first,
        "the same evidence waits out its backoff after a failure"
    );
    model_task.abort();
}

fn core_at(root: &std::path::Path) -> CoreBinding {
    CoreBinding {
        root: Some(root.to_path_buf()),
        focus: "wanda".into(),
        timeout_seconds: 30,
    }
}

async fn model_server(calls: Arc<AtomicUsize>) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/v1/chat/completions", post(answer))
                .with_state(calls),
        )
        .await
        .unwrap();
    });
    (format!("http://{address}/v1/"), task)
}

#[tokio::test]
async fn with_core_configured_and_no_core_running_nothing_reaches_the_model() {
    let calls = Arc::new(AtomicUsize::new(0));
    let (url, task) = model_server(calls.clone()).await;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("archie");
    let store = fixture_with(dir.path(), &url, Locality::Local, Some(core_at(&root)));
    let result = query(
        &store,
        &ContextDomain::personal("owner").unwrap(),
        "¿Qué detalle podría tener?",
        true,
        1000,
    )
    .await
    .unwrap();
    // Wanda does not go around Core when Core is configured and absent.
    assert_eq!(result.state, "core_unavailable", "{:?}", result.reason);
    assert!(result.recommendations.is_empty());
    assert!(result.receipt.is_none());
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    task.abort();
}

#[cfg(target_os = "linux")]
#[tokio::test]
#[ignore = "requires isolated PostgreSQL fixture; captured HTTP is not model-quality proof"]
async fn core_answers_wanda_with_a_receipt_and_its_pause_stops_her() {
    use archie_core::{
        control::{request_over, ControlRequest},
        operator::serve_operator_connection,
        ControlResponse,
    };
    use archie_workstation::ai_policy::RuntimeProfile;

    let calls = Arc::new(AtomicUsize::new(0));
    let (url, model) = model_server(calls.clone()).await;
    // Core's state must live under the temporary directory for the PostgreSQL fixture.
    let root = std::env::temp_dir().join(format!(
        "archie-wanda-core-{}",
        archie_types::TraceId::new()
    ));
    let paths = archie_state::StatePaths::rooted(&root);
    paths.ensure_directories().unwrap();
    let domain = ContextDomain::personal("owner").unwrap();
    let mut profile = RuntimeProfile::for_domain(domain.clone());
    profile.max_prompt_bytes = 65536;
    profile
        .create(&paths.config_dir.join("inference.toml"))
        .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let store = fixture_with(dir.path(), &url, Locality::Local, Some(core_at(&root)));
    let runtime = Arc::new(
        archie_core::Runtime::boot(paths.clone(), archie_core::ArchieConfig::default())
            .await
            .unwrap(),
    );
    // The operator socket archied serves, here for this runtime alone.
    let listener = tokio::net::UnixListener::bind(paths.control_socket()).unwrap();
    let host = runtime.clone();
    let socket = tokio::spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.unwrap();
            let host = host.clone();
            tokio::spawn(async move {
                let _ = serve_operator_connection(stream, host).await;
            });
        }
    });

    let result = query(&store, &domain, "¿Qué detalle podría tener?", true, 1000)
        .await
        .unwrap();
    assert_eq!(
        result.recommendations.len(),
        1,
        "state={}, reason={:?}",
        result.state,
        result.reason
    );
    assert_eq!(result.model.as_deref(), Some("fixture.reasoner"));
    let receipt = result.receipt.clone().expect("Core's receipt");
    assert!(uuid_like(&receipt), "{receipt}");
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    // Core's emergency pause stops Wanda too: nothing reaches the model while it holds.
    let mut operator = tokio::net::UnixStream::connect(paths.control_socket())
        .await
        .unwrap();
    assert!(matches!(
        request_over(&mut operator, &ControlRequest::AiPause)
            .await
            .unwrap(),
        ControlResponse::AiPolicy { paused: true, .. }
    ));
    let paused = query(&store, &domain, "¿Y otro detalle?", true, 1000)
        .await
        .unwrap();
    assert_eq!(paused.state, "core_refused", "{:?}", paused.reason);
    assert!(paused.recommendations.is_empty());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(matches!(
        request_over(&mut operator, &ControlRequest::AiResume)
            .await
            .unwrap(),
        ControlResponse::AiPolicy { paused: false, .. }
    ));
    socket.abort();
    model.abort();
    drop(runtime);
    let _ = std::fs::remove_dir_all(&root);
}

#[cfg(target_os = "linux")]
fn uuid_like(value: &str) -> bool {
    value.len() == 36 && value.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
}
