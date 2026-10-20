use archie_vault::ContextDomain;
use archie_wanda::{
    client::WandaClient,
    config::{DomainBinding, WandaConfig},
    server::router,
    store::WandaStore,
};
use std::future::IntoFuture;
#[tokio::test]
async fn authenticated_owner_view_and_voice_share_the_same_loopback_service() {
    let dir = tempfile::tempdir().unwrap();
    let owner = dir.path().join("owner.token");
    let store = WandaStore::new(WandaConfig {
        state_dir: dir.path().join("state"),
        graph_binary: dir.path().join("missing"),
        models_dir: dir.path().join("models"),
        operator_token_file: owner.clone(),
        domains: vec![DomainBinding {
            domain: ContextDomain::personal("owner").unwrap(),
            graph_path: dir.path().join("graph.tsv"),
        }],
        sources: vec![],
        quality_tier: 50,
        deep_quality_tier: 80,
        retention_hours: 168,
        lncm: None,
    })
    .unwrap();
    let token = std::fs::read_to_string(owner).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(axum::serve(listener, router(store)).into_future());
    let base = format!("http://{address}");
    let http = reqwest::Client::new();
    assert_eq!(
        http.get(format!("{base}/v1/domains"))
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    let domains: serde_json::Value = http
        .get(format!("{base}/v1/domains"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(domains["domains"][0], "personal/owner");
    let view: serde_json::Value = http
        .get(format!("{base}/v1/view?domain=personal/owner"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(view["graph"].is_null());
    assert_eq!(view["availability"], "graph_unavailable");
    let response = WandaClient::new(&base, &token)
        .unwrap()
        .query("personal/owner", "Consulta de prueba", false)
        .await
        .unwrap();
    assert!(response.recommendations.is_empty());
    assert_eq!(response.state, "no_model");
    assert_eq!(
        http.get(format!("{base}/v1/domains"))
            .bearer_auth(&token)
            .header("Host", "evil.example")
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    assert_eq!(
        http.get(format!("{base}/v1/domains"))
            .bearer_auth(&token)
            .header("Origin", "https://evil.example")
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    assert_eq!(
        http.post(format!("{base}/v1/query"))
            .bearer_auth(&token)
            .json(&serde_json::json!({"domain":"personal/owner","question":"","deep":false}))
            .send()
            .await
            .unwrap()
            .status(),
        400
    );
    assert_eq!(
        http.post(format!("{base}/v1/events"))
            .bearer_auth(&token)
            .json(&serde_json::json!([]))
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    let asset = http.get(format!("{base}/model.mjs")).send().await.unwrap();
    assert!(asset.headers()["content-type"]
        .to_str()
        .unwrap()
        .contains("javascript"));
    task.abort();
}
