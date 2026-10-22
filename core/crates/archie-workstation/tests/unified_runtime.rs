//! Real PostgreSQL + filesystem Vault + captured HTTP inference. This proves
//! integration/restart/isolation, not accuracy or subscription entitlement.
use archie_memory::inference::InferenceStatus;
use archie_model_runtime::{
    provider::{save_provider, ProviderProtocol, ProviderSpec},
    routed::Routed,
};
use archie_policy::PolicyEngine;
use archie_state::StateStore;
use archie_types::{Locality, PrivacyGrade};
use archie_vault::ContextDomain;
use archie_workstation::{
    ai::{InferenceRequest, InferenceStorage, RunnerChoice, UnifiedRuntime},
    ai_policy::{RuntimeAuthority, RuntimeProfile},
};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

struct Database {
    connection: String,
    schema: String,
}
impl Database {
    fn new() -> Self {
        let connection =
            std::env::var("ARCHIE_TEST_DATABASE_URL").expect("explicit isolated fixture required");
        let schema = format!("archie_test_unified_{}", uuid::Uuid::new_v4().simple());
        Self { connection, schema }
    }
    fn open(&self) -> StateStore {
        StateStore::connect(&self.connection, &self.schema).unwrap()
    }
}
impl Drop for Database {
    fn drop(&mut self) {
        if let Ok(mut client) = postgres::Client::connect(&self.connection, postgres::NoTls) {
            let _ = client.batch_execute(&format!("DROP SCHEMA {} CASCADE", self.schema));
        }
    }
}
fn runtime(database: &Database, dir: &std::path::Path) -> UnifiedRuntime {
    UnifiedRuntime {
        storage: Arc::new(InferenceStorage {
            store: Arc::new(database.open()),
            authority: RuntimeAuthority::new(
                dir.join("policy.toml"),
                Arc::new(RwLock::new(PolicyEngine::default())),
            ),
            vault_dir: dir.join("vault"),
        }),
        catalog: Routed::from_paths(dir.join("models"), dir.join("providers.toml")).unwrap(),
        codex: None,
    }
}
fn request(domain: ContextDomain, prompt: &str) -> InferenceRequest {
    InferenceRequest {
        domain,
        focus: "restart-proof".into(),
        privacy: PrivacyGrade::Public,
        prompt: prompt.into(),
        evidence: vec![],
        runner: RunnerChoice::Api,
        max_output_tokens: 128,
        budget: Default::default(),
        recall_memory: true,
        reply_schema: None,
    }
}

#[test]
#[ignore = "requires isolated PG18 + Timescale2.29.2/vector0.8.6; captured HTTP runner"]
fn real_router_vault_state_reopen_and_scoped_memory() {
    let database = Database::new();
    let dir = tempfile::tempdir().unwrap();
    let domain = ContextDomain::personal("fixture").unwrap();
    RuntimeProfile::for_domain(domain.clone())
        .create(&dir.path().join("policy.toml"))
        .unwrap();
    std::fs::create_dir(dir.path().join("models")).unwrap();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let listener = rt
        .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
        .unwrap();
    let address = listener.local_addr().unwrap();
    save_provider(
        &dir.path().join("providers.toml"),
        ProviderSpec {
            id: "fixture".into(),
            protocol: ProviderProtocol::OpenAiCompatible,
            base_url: format!("http://{address}/v1/"),
            locality: Locality::Local,
            credential_env: None,
            models: vec!["fixture-model".into()],
        },
    )
    .unwrap();
    let (send, recv) = std::sync::mpsc::channel();
    rt.spawn(async move {
        for _ in 0..2 {
            let (mut stream,_)=listener.accept().await.unwrap();let mut buffer=Vec::new();
            loop {
                let mut chunk=[0;4096];let n=stream.read(&mut chunk).await.unwrap();assert!(n>0);buffer.extend_from_slice(&chunk[..n]);
                if let Some(header_end)=buffer.windows(4).position(|w|w==b"\r\n\r\n") {
                    let header=String::from_utf8_lossy(&buffer[..header_end]);
                    let length:usize=header.lines().find_map(|line|line.to_ascii_lowercase().strip_prefix("content-length:").map(|s|s.trim().parse().unwrap())).unwrap();
                    if buffer.len()>=header_end+4+length {send.send(String::from_utf8(buffer[header_end+4..].to_vec()).unwrap()).unwrap();break;}
                }
                assert!(buffer.len()<262144);
            }
            let body=r#"{"model":"fixture-model","choices":[{"message":{"role":"assistant","content":"Attributed proposal ALPHA-41"},"finish_reason":"stop"}],"usage":{"prompt_tokens":20,"completion_tokens":5,"total_tokens":25}}"#;
            stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\nContent-Type: application/json\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();
        }
    });
    let mut service = runtime(&database, dir.path());
    let first = rt
        .block_on(service.execute(&request(domain.clone(), "First public request"), None))
        .unwrap();
    assert_eq!(first.receipt.status, InferenceStatus::Completed);
    assert_eq!(first.text.as_deref(), Some("Attributed proposal ALPHA-41"));
    assert!(recv.recv().unwrap().contains("First public request"));
    let sealed = std::fs::read_to_string(dir.path().join("vault/entries.json")).unwrap();
    assert!(!sealed.contains("ALPHA-41"));
    assert!(!serde_json::to_string(&first.receipt)
        .unwrap()
        .contains("First public request"));
    drop(service);
    let mut reopened = runtime(&database, dir.path());
    let recalled = rt
        .block_on(reopened.recall(
            first.receipt.binding.clone(),
            first.receipt.receipt_id.clone(),
        ))
        .unwrap();
    assert_eq!(recalled.text, first.text);
    let second = rt
        .block_on(reopened.execute(&request(domain.clone(), "Second public request"), None))
        .unwrap();
    assert_ne!(
        second.receipt.binding.task_id,
        first.receipt.binding.task_id
    );
    let observed = recv.recv().unwrap();
    assert!(observed.contains("ALPHA-41"));
    assert!(observed.contains("unverified proposals"));
    assert!(second
        .receipt
        .admitted_evidence_refs
        .iter()
        .any(|r| Some(r) == first.receipt.result_ref.as_ref()));
    let mut forged = first.receipt.binding.clone();
    forged.domain = ContextDomain::work("other", "project").unwrap();
    assert!(rt
        .block_on(reopened.recall(forged, first.receipt.receipt_id.clone()))
        .is_err());
    let mut crossing = request(domain, "never sent");
    crossing.evidence.push(
        ContextDomain::work("other", "project")
            .unwrap()
            .reference("private/one")
            .unwrap(),
    );
    assert!(rt.block_on(reopened.execute(&crossing, None)).is_err());
    drop(reopened);
    drop(rt);
}

#[cfg(unix)]
fn managed_fixture(database: &Database, dir: &std::path::Path) -> UnifiedRuntime {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir_all(dir.join("models")).unwrap();
    let domain = ContextDomain::personal("fixture").unwrap();
    let mut profile = RuntimeProfile::for_domain(domain);
    profile.max_duration_ms = 2000;
    profile.create(&dir.join("policy.toml")).unwrap();
    let script = dir.join("managed.py");
    let source=r#"#!/usr/bin/env python3
import json,sys,time,pathlib
ROOT=pathlib.Path(@@ROOT@@)
def send(v): print(json.dumps(v),flush=True)
def read(path,default):
    try: return json.loads(path.read_text())
    except FileNotFoundError: return default
for line in sys.stdin:
    r=json.loads(line);m=r.get('method');i=r.get('id');p=r.get('params',{})
    with (ROOT/'rpc.jsonl').open('a') as log: log.write(json.dumps(r)+'\n')
    if m=='initialize': send({'id':i,'result':{}})
    elif m=='account/read': send({'id':i,'result':{'account':{'type':'chatgpt','planType':'fixture'},'workspaceRouting':{'chatgptAccountId':'fixture-managed-account'}}})
    elif m in ('thread/start','thread/resume'):
        if m=='thread/start':
            n=read(ROOT/'counter.json',0)+1;(ROOT/'counter.json').write_text(json.dumps(n));thread='thread-'+str(n)
            (ROOT/(thread+'.json')).write_text('[]')
        else: thread=p['threadId']
        send({'id':i,'result':{'thread':{'id':thread}}})
    elif m=='turn/start':
        prompt=''.join(x['text'] for x in p['input']);path=ROOT/(thread+'.json');history=read(path,[])
        contaminated=any('UNCOMMITTED_MARKER' in old for old in history)
        history.append(prompt);path.write_text(json.dumps(history))
        send({'id':i,'result':{'turn':{'id':'turn-one','status':'inProgress'}}})
        mode=(ROOT/'mode').read_text() if (ROOT/'mode').exists() else 'normal'
        if mode in ('cancel','cas'):
            (ROOT/'attempt-started').write_text('started')
            while mode=='cancel' or not (ROOT/'release-turn').exists(): time.sleep(.01)
        text='UNCOMMITTED_LEAK' if contaminated else ('COMMITTED_ALPHA' if 'seed committed' in prompt else 'clean proposal')
        if mode=='failed': text='UNCOMMITTED_MARKER'
        send({'method':'item/completed','params':{'threadId':thread,'turnId':'turn-one','item':{'type':'agentMessage','id':'message','phase':'final_answer','text':text}}})
        send({'method':'turn/completed','params':{'threadId':thread,'turn':{'id':'turn-one','status':'failed' if mode=='failed' else 'completed'}}})
"#.replace("@@ROOT@@",&serde_json::to_string(&dir.to_string_lossy()).unwrap());
    std::fs::write(&script, source).unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut service = runtime(database, dir);
    service.codex = Some(
        archie_workstation::codex::CodexClient::new(
            script.as_os_str(),
            dir,
            Duration::from_secs(2),
        )
        .unwrap(),
    );
    service
}
fn managed_request(prompt: &str) -> InferenceRequest {
    let mut input = request(ContextDomain::personal("fixture").unwrap(), prompt);
    input.runner = RunnerChoice::Codex {
        model: "fixture-model".into(),
    };
    input.budget = archie_workstation::ai::InferenceBudget::ManagedByteTime {
        max_output_bytes: 65536,
    };
    input
}
async fn wait_file(path: std::path::PathBuf) {
    let start = Instant::now();
    while !path.exists() {
        assert!(start.elapsed() < Duration::from_secs(2));
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

#[test]
#[cfg(unix)]
#[ignore = "requires isolated PG18 extensions and Python managed-protocol fixture"]
fn managed_service_never_resumes_failed_cancelled_or_cas_rejected_history() {
    for mode in ["failed", "cancel", "cas"] {
        let database = Database::new();
        let dir = tempfile::tempdir().unwrap();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let mut service = managed_fixture(&database, dir.path());
        let first = rt
            .block_on(service.execute(&managed_request("seed committed"), None))
            .unwrap();
        assert_eq!(first.text.as_deref(), Some("COMMITTED_ALPHA"));
        assert_eq!(first.budget.as_ref().unwrap().generation_token_limit, None);
        assert_eq!(first.budget.as_ref().unwrap().max_output_bytes, 65536);
        std::fs::write(dir.path().join("mode"), mode).unwrap();
        let second = rt.block_on(async {
            let (send, recv) = tokio::sync::watch::channel(false);
            let path = dir.path().to_owned();
            let store = service.storage.store.clone();
            let focus = first.receipt.binding.focus_id.clone();
            let interference = async move {
                if mode != "failed" {
                    wait_file(path.join("attempt-started")).await;
                    if mode == "cancel" {
                        send.send(true).unwrap();
                    } else {
                        tokio::task::spawn_blocking(move || {
                            use archie_memory::{
                                execution::{ExecutionKind, RetainedTaskState},
                                storage::CheckpointStore,
                            };
                            let mut head =
                                store.execution_head_at(&focus, i64::MAX).unwrap().unwrap();
                            let previous = head.checkpoint_id.clone();
                            head.checkpoint_id = format!("competing:{}", uuid::Uuid::new_v4());
                            head.parent_checkpoint_id = Some(previous.clone());
                            head.recorded_at_ms = archie_types::Timestamp::now()
                                .as_millis()
                                .max(head.recorded_at_ms + 1);
                            head.kind = ExecutionKind::Decision;
                            head.evidence_ref = "fixture:competing-writer".into();
                            head.state = RetainedTaskState::default();
                            store.append_checkpoint(&head, Some(&previous)).unwrap();
                        })
                        .await
                        .unwrap();
                        tokio::time::sleep(Duration::from_millis(5)).await;
                        std::fs::write(path.join("release-turn"), "release").unwrap();
                    }
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
                drop(send);
            };
            let input = managed_request("UNCOMMITTED_MARKER must not enter later context");
            let (result, ()) = tokio::join!(service.execute(&input, Some(recv)), interference);
            result
        });
        if mode == "cas" {
            assert!(second.is_err());
        } else {
            let outcome = second.unwrap();
            assert!(outcome.text.is_none());
            assert!(outcome.receipt.result_ref.is_none());
            assert_eq!(
                outcome.receipt.status,
                if mode == "cancel" {
                    InferenceStatus::Cancelled
                } else {
                    InferenceStatus::Failed
                }
            );
        }
        drop(service);
        std::fs::write(dir.path().join("mode"), "normal").unwrap();
        let mut reopened = runtime(&database, dir.path());
        reopened.codex = Some(
            archie_workstation::codex::CodexClient::new(
                dir.path().join("managed.py").as_os_str(),
                dir.path(),
                Duration::from_secs(2),
            )
            .unwrap(),
        );
        let third = rt
            .block_on(reopened.execute(&managed_request("new admitted turn"), None))
            .unwrap();
        assert_eq!(third.text.as_deref(), Some("clean proposal"), "mode {mode}");
        let log = std::fs::read_to_string(dir.path().join("rpc.jsonl")).unwrap();
        assert!(
            !log.contains("thread/resume"),
            "mutable external history resumed after {mode}"
        );
        let recalled = rt
            .block_on(reopened.recall(
                first.receipt.binding.clone(),
                first.receipt.receipt_id.clone(),
            ))
            .unwrap();
        assert_eq!(recalled.text, first.text);
        assert!(recalled.budget.is_none());
        drop(reopened);
        drop(rt);
    }
}

#[test]
#[cfg(unix)]
#[ignore = "requires isolated PG18 extensions and Python managed-protocol fixture"]
fn managed_hard_token_budget_is_rejected_before_any_managed_rpc() {
    let database = Database::new();
    let dir = tempfile::tempdir().unwrap();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let mut service = managed_fixture(&database, dir.path());
    let mut input = managed_request("one-token hard bound");
    input.max_output_tokens = 1;
    input.budget = Default::default();
    let result = rt.block_on(service.execute(&input, None));
    assert!(
        result.is_err(),
        "unsupported hard generation-token budget was silently ignored"
    );
    assert!(result.unwrap_err().contains("unsupported"));
    assert!(!dir.path().join("rpc.jsonl").exists());
    drop(service);
    drop(rt);
}

#[test]
#[cfg(unix)]
#[ignore = "requires isolated PG18 extensions and Python managed-protocol fixture"]
fn frozen_account_is_checked_before_admitted_context_or_thread_creation() {
    let database = Database::new();
    let dir = tempfile::tempdir().unwrap();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let mut service = managed_fixture(&database, dir.path());
    let error = rt
        .block_on(service.execute_for_account(
            &managed_request("never disclose this capsule"),
            None,
            "different-frozen-account",
        ))
        .unwrap_err();
    assert!(error.contains("frozen"));
    let log = std::fs::read_to_string(dir.path().join("rpc.jsonl")).unwrap();
    assert!(!log.contains("thread/start"));
    assert!(!log.contains("turn/start"));
    assert!(!dir.path().join("vault/vault.key").exists());
    drop(service);
    drop(rt);
}

async fn read_http(stream: &mut tokio::net::TcpStream) -> String {
    let mut buffer = Vec::new();
    loop {
        let mut chunk = [0; 4096];
        let n = stream.read(&mut chunk).await.unwrap();
        assert!(n > 0);
        buffer.extend_from_slice(&chunk[..n]);
        if let Some(end) = buffer.windows(4).position(|w| w == b"\r\n\r\n") {
            let header = String::from_utf8_lossy(&buffer[..end]);
            let length: usize = header
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .map(|s| s.trim().parse().unwrap())
                })
                .unwrap();
            if buffer.len() >= end + 4 + length {
                return String::from_utf8(buffer[end + 4..].to_vec()).unwrap();
            }
        }
        assert!(buffer.len() < 262144);
    }
}
async fn reply_http(stream: &mut tokio::net::TcpStream, text: &str) {
    let body=serde_json::json!({"model":"fixture-model","choices":[{"message":{"role":"assistant","content":text},"finish_reason":"stop"}]}).to_string();
    stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\nContent-Type: application/json\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();
}
fn configure_api(dir: &std::path::Path, address: std::net::SocketAddr) {
    std::fs::create_dir_all(dir.join("models")).unwrap();
    save_provider(
        &dir.join("providers.toml"),
        ProviderSpec {
            id: "fixture".into(),
            protocol: ProviderProtocol::OpenAiCompatible,
            base_url: format!("http://{address}/v1/"),
            locality: Locality::Local,
            credential_env: None,
            models: vec!["fixture-model".into()],
        },
    )
    .unwrap();
}

#[test]
#[ignore = "requires isolated PG18 extensions and captured HTTP runner"]
fn original_policy_lease_clamps_transport_and_expired_result_stays_failed() {
    let database = Database::new();
    let dir = tempfile::tempdir().unwrap();
    let domain = ContextDomain::personal("fixture").unwrap();
    RuntimeProfile::for_domain(domain.clone())
        .create(&dir.path().join("policy.toml"))
        .unwrap();
    // Return a valid answer after the original grant expires. A fresh finish
    // policy check must not silently renew that original operation.
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let listener = rt
        .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
        .unwrap();
    configure_api(dir.path(), listener.local_addr().unwrap());
    rt.spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        read_http(&mut stream).await;
        tokio::time::sleep(Duration::from_millis(150)).await;
        reply_http(&mut stream, "expired model output").await;
    });
    let mut service = runtime(&database, dir.path());
    service.storage = Arc::new(InferenceStorage {
        store: Arc::new(database.open()),
        authority: RuntimeAuthority::new(
            dir.path().join("policy.toml"),
            Arc::new(RwLock::new(PolicyEngine::new(
                archie_policy::PolicyConfig {
                    max_lease_lifetime_ms: 40,
                    ..Default::default()
                },
            ))),
        ),
        vault_dir: dir.path().join("vault"),
    });
    let started = Instant::now();
    let result = rt
        .block_on(service.execute(&request(domain, "short original lease"), None))
        .unwrap();
    assert_eq!(result.receipt.status, InferenceStatus::Failed);
    assert!(result.receipt.result_ref.is_none());
    assert!(result.text.is_none());
    assert!(started.elapsed() < Duration::from_millis(140));
    drop(service);
    drop(rt);
    let reopened = database.open();
    let receipt = reopened
        .inference_receipt(&result.receipt.binding, &result.receipt.receipt_id)
        .unwrap()
        .unwrap();
    assert_eq!(receipt.status, InferenceStatus::Failed);
    assert!(receipt.result_ref.is_none());
}

/// Harness² pins Bonsai with its reasoning on, a manifest that offers reasoning and not
/// direct answers. The pinned model answers with the switch its manifest sets; a request
/// that pins nothing does not drift onto it.
#[test]
#[ignore = "requires isolated PG18 extensions and captured HTTP runner"]
fn a_pinned_reasoning_model_answers_with_its_manifest_switch() {
    let database = Database::new();
    let dir = tempfile::tempdir().unwrap();
    let domain = ContextDomain::personal("fixture").unwrap();
    RuntimeProfile::for_domain(domain.clone())
        .create(&dir.path().join("policy.toml"))
        .unwrap();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let listener = rt
        .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
        .unwrap();
    let address = listener.local_addr().unwrap();
    std::fs::create_dir_all(dir.path().join("models/local")).unwrap();
    std::fs::write(
        dir.path().join("models/local/reasoner.toml"),
        format!(
            r#"id = "local.fixture-reasoner"
display_name = "Fixture reasoner"
provider = "llama-cpp"
locality = "local"
context_window = 16384
max_output_tokens = 4096
capabilities = ["cognition.reasoning"]
max_privacy_grade = "restricted"
availability = "ready"
quality_tier = 55

[cost]
input_micros_per_mtok = 0
output_micros_per_mtok = 0

[labels]
base-url = "http://{address}/v1/"
provider-model = "fixture-model"
thinking = "on"
"#
        ),
    )
    .unwrap();
    let (send, recv) = std::sync::mpsc::channel();
    rt.spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        send.send(read_http(&mut stream).await).unwrap();
        reply_http(&mut stream, "reasoned proposal SIGMA-3").await;
    });
    let mut service = runtime(&database, dir.path());
    assert!(rt
        .block_on(service.execute(&request(domain.clone(), "unpinned request"), None))
        .is_err());
    let mut pinned = request(domain, "pinned reasoning request");
    pinned.runner = RunnerChoice::ApiPinned {
        model: "local.fixture-reasoner".into(),
    };
    let result = rt.block_on(service.execute(&pinned, None)).unwrap();
    assert_eq!(result.receipt.status, InferenceStatus::Completed);
    assert_eq!(result.receipt.model_id, "local.fixture-reasoner");
    assert_eq!(result.text.as_deref(), Some("reasoned proposal SIGMA-3"));
    let wire = recv.recv().unwrap();
    assert!(wire.contains("pinned reasoning request"));
    assert!(wire.contains("\"chat_template_kwargs\":{\"enable_thinking\":true}"));
    drop(service);
    drop(rt);
}

/// A restricted answer in a focus is never recalled into a lower-grade request there, and
/// it does not block that request either (review of 2026-10-22: it used to abort it).
#[test]
#[ignore = "requires isolated PG18 extensions and captured HTTP runner"]
fn a_restricted_memory_is_left_out_of_a_public_request_not_fatal_to_it() {
    let database = Database::new();
    let dir = tempfile::tempdir().unwrap();
    let domain = ContextDomain::personal("fixture").unwrap();
    RuntimeProfile::for_domain(domain.clone())
        .create(&dir.path().join("policy.toml"))
        .unwrap();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let listener = rt
        .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
        .unwrap();
    configure_api(dir.path(), listener.local_addr().unwrap());
    let (send, recv) = std::sync::mpsc::channel();
    rt.spawn(async move {
        for answer in ["restricted proposal RHO-9", "public proposal PI-2"] {
            let (mut stream, _) = listener.accept().await.unwrap();
            send.send(read_http(&mut stream).await).unwrap();
            reply_http(&mut stream, answer).await;
        }
    });
    let mut service = runtime(&database, dir.path());
    let mut restricted = request(domain.clone(), "restricted local request");
    restricted.privacy = PrivacyGrade::Restricted;
    let sealed = rt.block_on(service.execute(&restricted, None)).unwrap();
    assert_eq!(sealed.receipt.status, InferenceStatus::Completed);
    recv.recv().unwrap();
    let public = rt
        .block_on(service.execute(&request(domain, "public follow-up"), None))
        .unwrap();
    assert_eq!(public.receipt.status, InferenceStatus::Completed);
    assert!(!recv.recv().unwrap().contains("RHO-9"));
    assert!(!public
        .receipt
        .admitted_evidence_refs
        .iter()
        .any(|r| Some(r) == sealed.receipt.result_ref.as_ref()));
    drop(service);
    drop(rt);
}

#[test]
#[ignore = "requires isolated PG18 extensions and captured HTTP runner"]
fn lowering_the_privacy_ceiling_hides_previously_sealed_restricted_results() {
    let database = Database::new();
    let dir = tempfile::tempdir().unwrap();
    let domain = ContextDomain::personal("fixture").unwrap();
    let profile = RuntimeProfile::for_domain(domain.clone());
    profile.create(&dir.path().join("policy.toml")).unwrap();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let listener = rt
        .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
        .unwrap();
    configure_api(dir.path(), listener.local_addr().unwrap());
    rt.spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        read_http(&mut stream).await;
        reply_http(&mut stream, "restricted proposal RHO-7").await;
    });
    let mut service = runtime(&database, dir.path());
    let mut restricted = request(domain, "restricted local request");
    restricted.privacy = PrivacyGrade::Restricted;
    let sealed = rt.block_on(service.execute(&restricted, None)).unwrap();
    assert_eq!(sealed.receipt.status, InferenceStatus::Completed);
    let binding = sealed.receipt.binding.clone();
    let receipt = sealed.receipt.receipt_id.clone();
    assert_eq!(
        rt.block_on(service.recall(binding.clone(), receipt.clone()))
            .unwrap()
            .text
            .as_deref(),
        Some("restricted proposal RHO-7")
    );
    let lowered = RuntimeProfile {
        max_privacy: PrivacyGrade::Public,
        ..profile
    };
    std::fs::write(
        dir.path().join("policy.toml"),
        toml::to_string_pretty(&lowered).unwrap(),
    )
    .unwrap();
    assert!(rt.block_on(service.recall(binding, receipt)).is_err());
    drop(service);
    drop(rt);
}

#[cfg(unix)]
struct VaultLockHolder {
    child: std::process::Child,
    release: std::path::PathBuf,
}
#[cfg(unix)]
impl VaultLockHolder {
    fn new(dir: &std::path::Path) -> Self {
        std::fs::create_dir_all(dir.join("vault")).unwrap();
        let release = dir.join("release-vault-lock");
        let ready = dir.join("vault-lock-ready");
        let child=std::process::Command::new("python3").arg("-c").arg("import fcntl,pathlib,sys,time\nf=open(sys.argv[1],'a');fcntl.flock(f,fcntl.LOCK_EX);pathlib.Path(sys.argv[2]).write_text('locked')\nwhile not pathlib.Path(sys.argv[3]).exists(): time.sleep(.005)")
            .arg(dir.join("vault/.inference.lock")).arg(&ready).arg(&release)
            .stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn().unwrap();
        let start = Instant::now();
        while !ready.exists() {
            assert!(start.elapsed() < Duration::from_secs(2));
            std::thread::sleep(Duration::from_millis(5));
        }
        Self { child, release }
    }
    fn release(&mut self) {
        std::fs::write(&self.release, "release").unwrap();
        self.child.wait().unwrap();
    }
}
#[cfg(unix)]
impl Drop for VaultLockHolder {
    fn drop(&mut self) {
        let _ = std::fs::write(&self.release, "release");
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
#[cfg(unix)]
#[ignore = "requires isolated PG18 extensions, real cross-process file lock and captured HTTP runner"]
fn shared_vault_lock_covers_key_creation_and_two_distinct_focus_commits() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let database = Database::new();
    let dir = tempfile::tempdir().unwrap();
    let domain = ContextDomain::personal("fixture").unwrap();
    RuntimeProfile::for_domain(domain.clone())
        .create(&dir.path().join("policy.toml"))
        .unwrap();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let listener = rt
        .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
        .unwrap();
    configure_api(dir.path(), listener.local_addr().unwrap());
    let count = Arc::new(AtomicUsize::new(0));
    let observed = count.clone();
    rt.spawn(async move {
        let (mut one, _) = listener.accept().await.unwrap();
        read_http(&mut one).await;
        observed.fetch_add(1, Ordering::SeqCst);
        let (mut two, _) = listener.accept().await.unwrap();
        read_http(&mut two).await;
        observed.fetch_add(1, Ordering::SeqCst);
        let ((), ()) = tokio::join!(
            reply_http(&mut one, "result one"),
            reply_http(&mut two, "result two")
        );
    });
    let mut holder = VaultLockHolder::new(dir.path());
    let mut one = runtime(&database, dir.path());
    let mut two = runtime(&database, dir.path());
    let mut first = request(domain.clone(), "concurrent one");
    first.focus = "writer-one".into();
    let mut second = request(domain, "concurrent two");
    second.focus = "writer-two".into();
    let (blocked, results) = rt.block_on(async {
        let both = async { tokio::join!(one.execute(&first, None), two.execute(&second, None)) };
        let release = async {
            tokio::time::sleep(Duration::from_millis(100)).await;
            let blocked =
                count.load(Ordering::SeqCst) == 0 && !dir.path().join("vault/vault.key").exists();
            holder.release();
            blocked
        };
        let (results, blocked) = tokio::join!(both, release);
        (blocked, results)
    });
    assert!(
        blocked,
        "shared cross-process lock did not cover Vault/key initialization"
    );
    let (first, second) = (results.0.unwrap(), results.1.unwrap());
    drop(one);
    drop(two);
    let reopened = runtime(&database, dir.path());
    for result in [first, second] {
        let recalled = rt
            .block_on(reopened.recall(
                result.receipt.binding.clone(),
                result.receipt.receipt_id.clone(),
            ))
            .unwrap();
        assert_eq!(recalled.text, result.text);
    }
    drop(reopened);
    drop(rt);
}

#[test]
#[cfg(unix)]
#[ignore = "requires isolated PG18 extensions, cross-process Vault lock and captured HTTP runner"]
fn an_expired_prepared_grant_never_dispatches_after_waiting_for_vault() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let database = Database::new();
    let dir = tempfile::tempdir().unwrap();
    let domain = ContextDomain::personal("fixture").unwrap();
    RuntimeProfile::for_domain(domain.clone())
        .create(&dir.path().join("policy.toml"))
        .unwrap();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let listener = rt
        .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
        .unwrap();
    configure_api(dir.path(), listener.local_addr().unwrap());
    let count = Arc::new(AtomicUsize::new(0));
    let observed = count.clone();
    rt.spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        read_http(&mut stream).await;
        observed.fetch_add(1, Ordering::SeqCst);
        reply_http(&mut stream, "must not dispatch").await;
    });
    let mut service = runtime(&database, dir.path());
    service.storage = Arc::new(InferenceStorage {
        store: Arc::new(database.open()),
        authority: RuntimeAuthority::new(
            dir.path().join("policy.toml"),
            Arc::new(RwLock::new(PolicyEngine::new(
                archie_policy::PolicyConfig {
                    max_lease_lifetime_ms: 40,
                    ..Default::default()
                },
            ))),
        ),
        vault_dir: dir.path().join("vault"),
    });
    let mut holder = VaultLockHolder::new(dir.path());
    let result = rt.block_on(async {
        let release = async {
            tokio::time::sleep(Duration::from_millis(150)).await;
            holder.release();
        };
        let input = request(domain, "grant expires while context is blocked");
        let (result, ()) = tokio::join!(service.execute(&input, None), release);
        result.unwrap()
    });
    assert_eq!(
        count.load(Ordering::SeqCst),
        0,
        "request dispatched after original grant expired"
    );
    assert_eq!(result.receipt.status, InferenceStatus::Failed);
    assert!(result.text.is_none());
    assert!(result.receipt.result_ref.is_none());
    drop(service);
    drop(rt);
    let reopened = database.open();
    assert_eq!(
        reopened
            .inference_receipt(&result.receipt.binding, &result.receipt.receipt_id)
            .unwrap()
            .unwrap()
            .status,
        InferenceStatus::Failed
    );
}

/// A private copy of the pinned worker: launch refuses files others may write.
fn stage_private_worker(source: &std::path::Path, target: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir_all(target).unwrap();
    std::fs::set_permissions(target, std::fs::Permissions::from_mode(0o700)).unwrap();
    for entry in std::fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name();
        if name == "__pycache__" || name == "tests" {
            continue;
        }
        let destination = target.join(&name);
        if entry.file_type().unwrap().is_dir() {
            stage_private_worker(&entry.path(), &destination);
        } else {
            std::fs::copy(entry.path(), &destination).unwrap();
            std::fs::set_permissions(destination, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
    }
}

/// The reply a schema-constrained server can give: built only from the schema it was
/// sent, so a stage that arrives without one gets the reflection's tagged text.
fn reply_by_schema(
    schema: Option<&serde_json::Value>,
    base: &serde_json::Map<String, serde_json::Value>,
) -> String {
    let Some(schema) = schema else {
        return "<experience>\ncue: fixture\nlesson: keep the reply shape\nevidence: observed fixture artifact\nconfidence: limited\n</experience>".into();
    };
    let keys: Vec<&str> = schema["required"]
        .as_array()
        .unwrap()
        .iter()
        .map(|key| key.as_str().unwrap())
        .collect();
    match keys.as_slice() {
        ["artifact", "trajectory"] => serde_json::json!({
            "artifact": "Schema-shaped fixture artifact",
            "trajectory": "[ACTION] Read the admitted task\n[OBSERVATION] Fixture evidence only",
        })
        .to_string(),
        ["harness", "notes"] => {
            let mut harness = serde_json::Map::new();
            for file in schema["properties"]["harness"]["required"]
                .as_array()
                .unwrap()
            {
                let file = file.as_str().unwrap();
                let text = if file == "LongTermMEMORY.md" {
                    serde_json::json!("Keep every reply in the shape the stage names.")
                } else {
                    base[file].clone()
                };
                harness.insert(file.to_owned(), text);
            }
            serde_json::json!({ "harness": harness, "notes": "Deterministic fixture." }).to_string()
        }
        other => panic!("unexpected reply schema {other:?}"),
    }
}

/// Harness² on the unified service against a pinned llama.cpp manifest with reasoning on.
/// Every JSON stage reaches the server with the worker's own reply schema as
/// `response_format`; the reflection, which is tagged text, arrives with none. The server
/// answers only from the schema it sees, and the actual pinned upstream run completes.
#[test]
#[ignore = "requires isolated PG18 extensions, ARCHIE_HARNESS2_PYTHON (POSIX Python 3.12/3.13) and captured HTTP runner"]
fn harness2_json_stages_reach_llama_cpp_with_their_reply_schema() {
    use archie_workstation::{
        harness2::reply_schema,
        harness_runtime::{create_program_config, load_program, run_harness, HarnessRunRequest},
    };
    let python = std::path::PathBuf::from(
        std::env::var_os("ARCHIE_HARNESS2_PYTHON").expect("explicit supported interpreter"),
    );
    let database = Database::new();
    let dir = tempfile::tempdir().unwrap();
    let domain = ContextDomain::personal("fixture").unwrap();
    RuntimeProfile::for_domain(domain.clone())
        .create(&dir.path().join("policy.toml"))
        .unwrap();
    let source = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .unwrap()
        .join("tools/harness2");
    let worker = dir.path().join("worker");
    stage_private_worker(&source, &worker);
    let config = dir.path().join("inference/harness2.toml");
    create_program_config(&config, &python, &worker.join("worker.py")).unwrap();
    let program = load_program(&config, dir.path().join("harness-state"), None).unwrap();

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let listener = rt
        .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
        .unwrap();
    let address = listener.local_addr().unwrap();
    std::fs::create_dir_all(dir.path().join("models/local")).unwrap();
    // The fixture accepts the complete upstream proposer prompt.
    std::fs::write(
        dir.path().join("models/local/reasoner.toml"),
        format!(
            r#"id = "local.fixture-reasoner"
display_name = "Fixture reasoner"
provider = "llama-cpp"
locality = "local"
context_window = 32768
max_output_tokens = 4096
capabilities = ["cognition.reasoning"]
max_privacy_grade = "restricted"
availability = "ready"
quality_tier = 55

[cost]
input_micros_per_mtok = 0
output_micros_per_mtok = 0

[labels]
base-url = "http://{address}/v1/"
provider-model = "fixture-model"
thinking = "on"
"#
        ),
    )
    .unwrap();
    let server = rt.spawn(async move {
        let mut base = serde_json::Map::new();
        let mut seen = Vec::new();
        for _ in 0..8 {
            let (mut stream, _) = listener.accept().await.unwrap();
            let body: serde_json::Value =
                serde_json::from_str(&read_http(&mut stream).await).unwrap();
            assert_eq!(body["chat_template_kwargs"]["enable_thinking"], true);
            let message = body["messages"][1]["content"].as_str().unwrap();
            let stage: serde_json::Value =
                serde_json::from_str(message.rsplit("Current request:\n").next().unwrap()).unwrap();
            if base.is_empty() {
                if let Some(harness) = stage["harness"].as_object() {
                    base = harness.clone();
                }
            }
            let format = &body["response_format"];
            let schema = (!format.is_null()).then(|| {
                assert_eq!(format["type"], "json_schema");
                &format["json_schema"]["schema"]
            });
            seen.push(schema.cloned());
            reply_http(&mut stream, &reply_by_schema(schema, &base)).await;
        }
        let base: std::collections::BTreeMap<String, String> = base
            .into_iter()
            .map(|(file, text)| (file, text.as_str().unwrap().to_owned()))
            .collect();
        (seen, base)
    });
    let mut service = runtime(&database, dir.path());
    let request = HarnessRunRequest {
        domain: domain.clone(),
        focus: "schema-fixture".into(),
        task_family: "summaries".into(),
        run_id: "schema-constrained-fixture".into(),
        prompt: "Summarize only this public deterministic fixture.".into(),
        runner: RunnerChoice::ApiPinned {
            model: "local.fixture-reasoner".into(),
        },
        duration_ms: 120_000,
    };
    let outcome = rt
        .block_on(run_harness(&mut service, &program, &request, None))
        .unwrap();
    let (seen, seen_base) = rt.block_on(server).unwrap();
    assert_eq!(outcome.result.ledger["R"], 4);
    assert_eq!(outcome.result.ledger["P"], 3);
    assert_eq!(outcome.receipts.len(), 8);
    assert!(outcome
        .receipts
        .iter()
        .all(|receipt| receipt.status == InferenceStatus::Completed
            && receipt.model_id == "local.fixture-reasoner"));
    let base: std::collections::BTreeMap<String, String> = seen_base;
    let artifact = reply_schema("artifact", &base).unwrap();
    let harness = reply_schema("harness", &base).unwrap();
    assert_eq!(
        harness["properties"]["harness"]["properties"]["guardrails.md"]["enum"][0],
        base["guardrails.md"].as_str(),
        "the proposal schema pins the worker's own base guardrails"
    );
    let count = |expected: Option<&serde_json::Value>| {
        seen.iter()
            .filter(|schema| schema.as_ref() == expected)
            .count()
    };
    assert_eq!(count(Some(&artifact)), 4, "four solver rollouts");
    assert_eq!(count(Some(&harness)), 3, "three proposals");
    assert_eq!(count(None), 1, "one tagged reflection");
    // The run is the last stage: the reflection, without a schema.
    assert_eq!(seen.last().unwrap(), &None);
    drop(service);
    drop(rt);
}

/// The same run against a real llama.cpp server, so an operator can try a local model
/// such as Bonsai: `ARCHIE_TEST_LLAMA_URL` (its OpenAI base URL, loopback) and
/// `ARCHIE_TEST_LLAMA_MODEL` (the alias it serves) are required;
/// `ARCHIE_TEST_LLAMA_THINKING` (`on`/`off`, default `on`), `ARCHIE_TEST_LLAMA_MAX_OUTPUT`
/// (default 4096) and `ARCHIE_TEST_LLAMA_STAGE_MS` (each stage's deadline, default the
/// profile's minute, at most its 600000) are optional. It proves that every
/// stage's reply had a shape the worker accepts, not that the model answered well.
#[test]
#[ignore = "requires isolated PG18 extensions, ARCHIE_HARNESS2_PYTHON and a real llama.cpp server"]
fn harness2_completes_on_a_real_llama_cpp_server_under_reply_schemas() {
    use archie_workstation::harness_runtime::{
        create_program_config, load_program, run_harness, HarnessRunRequest,
    };
    let python = std::path::PathBuf::from(
        std::env::var_os("ARCHIE_HARNESS2_PYTHON").expect("explicit supported interpreter"),
    );
    let base_url = std::env::var("ARCHIE_TEST_LLAMA_URL").expect("a llama.cpp base URL");
    let alias = std::env::var("ARCHIE_TEST_LLAMA_MODEL").expect("the alias it serves");
    let thinking = std::env::var("ARCHIE_TEST_LLAMA_THINKING").unwrap_or_else(|_| "on".into());
    let max_output: u32 = std::env::var("ARCHIE_TEST_LLAMA_MAX_OUTPUT")
        .map(|value| value.parse().unwrap())
        .unwrap_or(4096);
    let database = Database::new();
    let dir = tempfile::tempdir().unwrap();
    let domain = ContextDomain::personal("fixture").unwrap();
    // A model on a CPU may need more than the default minute per stage.
    let mut profile = RuntimeProfile::for_domain(domain.clone());
    if let Ok(stage_ms) = std::env::var("ARCHIE_TEST_LLAMA_STAGE_MS") {
        profile.max_duration_ms = stage_ms.parse().unwrap();
    }
    profile.create(&dir.path().join("policy.toml")).unwrap();
    let source = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .unwrap()
        .join("tools/harness2");
    let worker = dir.path().join("worker");
    stage_private_worker(&source, &worker);
    let config = dir.path().join("inference/harness2.toml");
    create_program_config(&config, &python, &worker.join("worker.py")).unwrap();
    let program = load_program(&config, dir.path().join("harness-state"), None).unwrap();
    std::fs::create_dir_all(dir.path().join("models/local")).unwrap();
    std::fs::write(
        dir.path().join("models/local/real.toml"),
        format!(
            r#"id = "local.real-llama"
display_name = "Real llama.cpp model"
provider = "llama-cpp"
locality = "local"
context_window = 16384
max_output_tokens = {max_output}
capabilities = ["cognition.reasoning"]
max_privacy_grade = "restricted"
availability = "ready"
quality_tier = 55

[cost]
input_micros_per_mtok = 0
output_micros_per_mtok = 0

[labels]
base-url = "{base_url}"
provider-model = "{alias}"
thinking = "{thinking}"
"#
        ),
    )
    .unwrap();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let mut service = runtime(&database, dir.path());
    let request = HarnessRunRequest {
        domain,
        focus: "real-llama".into(),
        task_family: "summaries".into(),
        run_id: "real-llama-schema".into(),
        prompt: "Summarize in one sentence: ArchieOS keeps private context on the machine.".into(),
        runner: RunnerChoice::ApiPinned {
            model: "local.real-llama".into(),
        },
        duration_ms: 300_000,
    };
    let started = Instant::now();
    let outcome = rt
        .block_on(run_harness(&mut service, &program, &request, None))
        .unwrap();
    eprintln!(
        "real llama.cpp Harness² run: {:?}, status {}, ledger {}, promotion {:?}",
        started.elapsed(),
        outcome.result.status,
        outcome.result.ledger,
        outcome.result.promotion
    );
    assert_eq!(outcome.result.ledger["R"], 4);
    assert_eq!(outcome.result.ledger["P"], 3);
    assert_eq!(outcome.receipts.len(), 8);
    assert!(outcome
        .receipts
        .iter()
        .all(|receipt| receipt.status == InferenceStatus::Completed));
    drop(service);
    drop(rt);
}
