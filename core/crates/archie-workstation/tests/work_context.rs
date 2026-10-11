use archie_cerebra::pillars::{Pillar, PillarContext};
use archie_types::{
    task::{QueueClass, Task},
    PrivacyGrade, TraceId,
};
use archie_vault::{ContextDomain, Vault, VaultKey, VaultRef};
use archie_workstation::enterprise_bridge::EnterpriseAdapter;

fn adapter() -> EnterpriseAdapter {
    EnterpriseAdapter::new(
        "studio",
        VaultRef::parse("vault:credentials/board").unwrap(),
    )
}

#[test]
fn corporate_egress_rejects_personal_context_even_when_graded_public() {
    let mut task = Task::new("private task", TraceId::new(), QueueClass::Background);
    task.labels
        .insert("archie.privacy_grade".into(), "public".into());
    task.labels
        .insert("archie.context_domain".into(), "personal/owner".into());
    assert!(adapter().prepare_for_export(&task).is_err());
    task.labels
        .insert("archie.context_domain".into(), "invalid".into());
    assert!(adapter().prepare_for_export(&task).is_err());
}

#[test]
fn an_invalid_quarantine_marker_never_releases_work() {
    let mut task = Task::new("plans", TraceId::new(), QueueClass::Background);
    task.labels
        .insert("archie.context_domain".into(), "work/studio/house".into());
    task.labels
        .insert("archie.privacy_grade".into(), "public".into());
    for marker in ["TRUE", "maybe", ""] {
        task.labels
            .insert("archie.quarantine".into(), marker.into());
        assert!(
            adapter().prepare_for_export(&task).is_err(),
            "marker {marker:?} must not release quarantine"
        );
    }
}

#[test]
fn admitted_work_context_is_bound_to_task_and_quarantine_is_preserved() {
    let domain = ContextDomain::work("studio", "house").unwrap();
    let mut task = Task::new("house plans", TraceId::new(), QueueClass::Background);
    task.labels
        .insert("archie.context_domain".into(), domain.to_string());
    task.labels
        .insert("archie.context_kind".into(), "work".into());
    let mut context = PillarContext::new(domain, task.task_id.to_string()).unwrap();
    context
        .include(
            Pillar::Wanda,
            VaultRef::parse("vault:work/studio/house/manager").unwrap(),
        )
        .unwrap();
    task.labels
        .insert("archie.quarantine".into(), "true".into());
    assert!(adapter()
        .cognitive_observation(&task, &context, 101)
        .is_err());
    task.labels.remove("archie.quarantine");
    let observation = adapter()
        .cognitive_observation(&task, &context, 101)
        .unwrap();
    assert_eq!(observation.task_id, task.task_id.to_string());
    assert_eq!(observation.privacy, PrivacyGrade::Restricted);
    task.labels
        .insert("archie.context_domain".into(), "work/studio/other".into());
    assert!(adapter()
        .cognitive_observation(&task, &context, 101)
        .is_err());
}

#[tokio::test]
async fn a_missing_transport_cannot_report_delivery_or_open_credentials() {
    let mut vault = Vault::open(&VaultKey::from_bytes([9; 32])).unwrap();
    assert!(adapter().sync_to_remote(&[], &mut vault).await.is_err());
    assert!(vault.audit_log().is_empty());
}

#[test]
fn a_project_bound_adapter_rejects_another_company_or_project() {
    let adapter = adapter()
        .in_domain(ContextDomain::work("studio", "house").unwrap())
        .unwrap();
    let mut task = Task::new("plans", TraceId::new(), QueueClass::Background);
    task.labels
        .insert("archie.privacy_grade".into(), "public".into());
    for domain in ["work/other/house", "work/studio/other"] {
        task.labels
            .insert("archie.context_domain".into(), domain.into());
        assert!(adapter.prepare_for_export(&task).is_err());
    }
    task.labels
        .insert("archie.context_domain".into(), "work/studio/house".into());
    assert!(adapter.prepare_for_export(&task).is_ok());
    assert!(super_adapter_personal().is_err());
}

fn super_adapter_personal() -> archie_types::error::Result<EnterpriseAdapter> {
    adapter().in_domain(ContextDomain::personal("owner").unwrap())
}

#[tokio::test]
async fn the_authenticated_board_does_not_list_personal_tasks() {
    use archie_types::task::TaskGraph;
    use archie_workstation::board_api::{router, BoardState};
    use std::sync::Arc;
    use tokio::sync::{Mutex, RwLock};
    let mut graph = TaskGraph::new();
    let mut personal = Task::new("private diary", TraceId::new(), QueueClass::Background);
    personal
        .labels
        .insert("archie.context_domain".into(), "personal/owner".into());
    let personal_task_id = personal.task_id;
    graph.insert(personal).unwrap();
    let mut work = Task::new("house plans", TraceId::new(), QueueClass::Background);
    work.labels
        .insert("archie.context_domain".into(), "work/studio/house".into());
    graph.insert(work).unwrap();
    let state = BoardState {
        adapter: Arc::new(
            adapter()
                .in_domain(ContextDomain::work("studio", "house").unwrap())
                .unwrap(),
        ),
        task_graph: Arc::new(RwLock::new(graph)),
        vault: Arc::new(Mutex::new(
            Vault::open(&VaultKey::from_bytes([9; 32])).unwrap(),
        )),
        auth_token_hash: ring::digest::digest(&ring::digest::SHA256, b"test-token")
            .as_ref()
            .to_vec(),
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let tasks = state.task_graph.clone();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, router(state)).await.unwrap() });
    let response = reqwest::Client::new()
        .get(format!("http://{address}/tasks"))
        .bearer_auth("test-token")
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());
    let cards: Vec<serde_json::Value> = response.json().await.unwrap();
    assert_eq!(cards.len(), 1);
    assert_eq!(cards[0]["title"], "house plans");
    let moved = reqwest::Client::new()
        .put(format!("http://{address}/tasks/{personal_task_id}/move"))
        .bearer_auth("test-token")
        .json(&serde_json::json!({"new_status":"ready"}))
        .send()
        .await
        .unwrap();
    assert_eq!(moved.status(), reqwest::StatusCode::BAD_REQUEST);
    let created = reqwest::Client::new()
        .post(format!("http://{address}/tasks"))
        .bearer_auth("test-token")
        .json(&serde_json::json!({
            "title":"new plans", "requested_privacy":"public",
            "metadata":{"archie.context_domain":"personal/owner", "archie.quarantine":"false"}
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(created.status(), reqwest::StatusCode::CREATED);
    let graph = tasks.read().await;
    let task = graph.iter().find(|task| task.title == "new plans").unwrap();
    assert_eq!(task.labels["archie.context_domain"], "work/studio/house");
    assert_eq!(task.labels["archie.quarantine"], "true");
    assert_eq!(task.labels["archie.privacy_grade"], "restricted");
    server.abort();
}
