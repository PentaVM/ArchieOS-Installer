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
