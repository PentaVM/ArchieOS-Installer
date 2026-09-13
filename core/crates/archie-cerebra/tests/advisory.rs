//! Synthetic structured evidence; no sensory recordings or transcripts.
use archie_cerebra::{
    advisory::{evaluate_output, AdvisoryInterpreter, AdvisoryResult, MAX_PROPOSAL_BYTES},
    Cerebra, CerebraConfig, Observation, Trigger,
};
use archie_memory::{
    assess_hypothesis, AnalysisPolicy, AnalyticalAssessment, AnalyticalSignal, EvidenceStance,
    ProposalDisposition,
};
use archie_model_runtime::ProviderEndpoint;
use archie_observability::Stage;
use archie_registry::ModelRegistry;
use archie_router::Router;
use archie_types::*;
use archie_vault::{ContextGateway, EntryClass, Vault, VaultKey, VaultRef};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

const NOW: i64 = 1_800_000_000_000;
fn assessment(disputed: bool) -> AnalyticalAssessment {
    let signals: Vec<_> = (0..2)
        .map(|i| AnalyticalSignal {
            signal_id: format!("signal:{i}"),
            hypothesis_id: "hypothesis:one".into(),
            evidence_ref: format!("vault:evidence/{i}"),
            evidence_sha256: format!("sha256:{}", "a".repeat(64)),
            source_root: format!("source:{i}"),
            semantic_key: "synthetic:one".into(),
            stance: if disputed && i == 1 {
                EvidenceStance::Contradicts
            } else {
                EvidenceStance::Supports
            },
            reliability_permille: 900,
            relevance_permille: 900,
            observed_at_ms: NOW,
            expires_at_ms: None,
            machine_analysis_allowed: true,
            consent_required: false,
            consent_granted: false,
        })
        .collect();
    assess_hypothesis("hypothesis:one", &signals, &AnalysisPolicy::default(), NOW).unwrap()
}
fn output() -> Value {
    json!({"proposal_id":"proposal:one","hypothesis_id":"hypothesis:one","model_id":"test.model",
        "model_sha256":format!("sha256:{}", "a".repeat(64)),"stance":"supports","score_permille":900,
        "evidence_refs":["vault:evidence/0","vault:evidence/1"],"produced_at_ms":NOW})
}
fn check(value: Value) -> AdvisoryResult {
    evaluate_output(&value.to_string(), "test.model", &assessment(false))
}

#[test]
fn without_citations() {
    let mut v = output();
    v["evidence_refs"] = json!([]);
    assert_eq!(check(v).code(), "insufficient_citations");
}
#[test]
fn unadmitted_citation() {
    let mut v = output();
    v["evidence_refs"] = json!(["vault:unknown/0"]);
    assert_eq!(check(v).code(), "unsupported_evidence_reference");
}
#[test]
fn contradictory_stance() {
    let mut v = output();
    v["stance"] = json!("contradicts");
    assert_eq!(check(v).code(), "contradicts_assessment");
}
#[test]
fn high_uncertainty() {
    let mut v = output();
    v["score_permille"] = json!(100);
    assert_eq!(check(v).code(), "model_uncertain");
}
#[test]
fn invalid_json() {
    assert_eq!(
        evaluate_output("{", "test.model", &assessment(true)).code(),
        "invalid_proposal_json"
    );
}
#[test]
fn oversized_proposal() {
    assert_eq!(
        evaluate_output(
            &" ".repeat(MAX_PROPOSAL_BYTES + 1),
            "test.model",
            &assessment(true)
        )
        .code(),
        "proposal_too_large"
    );
}
#[test]
fn strict_fields_and_bounds() {
    let mut v = output();
    v["extra"] = json!(true);
    assert_eq!(check(v).code(), "invalid_proposal_json");
    let mut v = output();
    v["score_permille"] = json!(1001);
    assert_eq!(check(v).code(), "invalid_proposal");
    let mut v = output();
    v["proposal_id"] = json!("invalid identifier");
    assert_eq!(check(v).code(), "invalid_proposal");
}
#[test]
fn certainty_never_crosses_the_band() {
    for score in [0, 600, 950, 1000] {
        let mut v = output();
        v["score_permille"] = json!(score);
        let a = assessment(true);
        let before = serde_json::to_string(&a).unwrap();
        assert!(
            matches!(evaluate_output(&v.to_string(), "test.model", &a), AdvisoryResult::Gated(d) if d.disposition == ProposalDisposition::Abstain)
        );
        assert_eq!(serde_json::to_string(&a).unwrap(), before);
    }
    assert_eq!(check(output()).code(), "evidence_aligned"); // Gate unit path, never HTTP on settled evidence.
}

fn pass() -> (archie_cerebra::Pass, ModelRegistry) {
    let mut registry = ModelRegistry::new();
    registry
        .register(ModelDescriptor {
            id: ModelId::parse("test.model").unwrap(),
            display_name: "synthetic".into(),
            provider: ProviderKind::NvidiaNim,
            locality: Locality::Local,
            node: None,
            context_window: 8192,
            max_output_tokens: 2048,
            capabilities: vec![Capability::known("compute.inference")],
            max_privacy_grade: PrivacyGrade::Restricted,
            retention: DataRetention::ZeroVerified,
            availability: Availability::Ready,
            cost: TokenCost::FREE,
            latency_ema_ms: None,
            quality_tier: 50,
            required_accelerator_bytes: None,
            labels: BTreeMap::new(),
        })
        .unwrap();
    let mut vault = Vault::open(&VaultKey::from_bytes([5; 32])).unwrap();
    let mut observation = Observation::new(
        Trigger::SystemEvent {
            source: "synthetic".into(),
        },
        "task:one",
        "analytical-test",
        NOW,
    )
    .graded(PrivacyGrade::Restricted);
    for i in 0..2 {
        let reference = VaultRef::parse(&format!("vault:evidence/{i}")).unwrap();
        vault
            .store(
                "test",
                reference.clone(),
                EntryClass::PrivateFact,
                PrivacyGrade::Restricted,
                &json!({"synthetic":true,"stance":if i==0 {"supports"} else {"contradicts"}})
                    .to_string(),
                NOW,
                None,
            )
            .unwrap();
        observation.context_refs.push(reference);
    }
    (
        Cerebra::new(
            Router::default(),
            ContextGateway::new(),
            CerebraConfig::default(),
        )
        .consider(&mut vault, &registry, &observation, TraceId::new(), NOW)
        .unwrap(),
        registry,
    )
}

async fn serve(body: String) -> (AdvisoryInterpreter, tokio::task::JoinHandle<Value>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = ProviderEndpoint::nvidia_nim(
        &format!("http://{}/v1", listener.local_addr().unwrap()),
        Locality::Local,
    )
    .unwrap();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        let mut buf = [0; 4096];
        let (head_end, length) = loop {
            let n = socket.read(&mut buf).await.unwrap();
            assert!(n > 0);
            request.extend_from_slice(&buf[..n]);
            assert!(request.len() <= 32768);
            if let Some(end) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                let header = String::from_utf8_lossy(&request[..end]);
                let length: usize = header
                    .lines()
                    .find_map(|l| {
                        l.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .map(|s| s.trim().parse().unwrap())
                    })
                    .unwrap();
                break (end + 4, length);
            }
        };
        while request.len() < head_end + length {
            let n = socket.read(&mut buf).await.unwrap();
            assert!(n > 0);
            request.extend_from_slice(&buf[..n]);
            assert!(request.len() <= 32768);
        }
        let response = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body);
        let _ = socket.write_all(response.as_bytes()).await;
        serde_json::from_slice(&request[head_end..head_end + length]).unwrap()
    });
    (AdvisoryInterpreter::new(endpoint).unwrap(), server)
}
#[tokio::test]
async fn nim_loopback_capsule_to_gate_to_trace() {
    let body = json!({"model":"test.model","choices":[{"message":{"role":"assistant","content":output().to_string()},"finish_reason":"stop"}]}).to_string();
    let (interpreter, server) = serve(body).await;
    let (mut pass, models) = pass();
    assert_eq!(
        interpreter
            .interpret(&mut pass, &models, &assessment(true), NOW)
            .await
            .unwrap()
            .code(),
        "evidence_disputed"
    );
    let request = server.await.unwrap();
    assert_eq!(request["model"], "test.model");
    assert!(request["messages"][2]["content"]
        .as_str()
        .unwrap()
        .contains("vault:evidence/0"));
    assert_eq!(
        pass.trace
            .last(Stage::Interpret)
            .unwrap()
            .decision_code
            .as_deref(),
        Some("evidence_disputed")
    );
    let trace = serde_json::to_string(&pass.trace).unwrap();
    assert!(!trace.contains("stance"));
    assert!(!trace.contains("model_sha256"));
    pass.trace.validate().unwrap();
}
#[tokio::test]
async fn settled_assessment_makes_zero_connections() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let interpreter = AdvisoryInterpreter::new(
        ProviderEndpoint::nvidia_nim(
            &format!("http://{}/v1", listener.local_addr().unwrap()),
            Locality::Local,
        )
        .unwrap(),
    )
    .unwrap();
    let (mut pass, models) = pass();
    assert_eq!(
        interpreter
            .interpret(&mut pass, &models, &assessment(false), NOW)
            .await
            .unwrap()
            .code(),
        "assessment_outside_ambiguous_band"
    );
    let mut connections = 0;
    if tokio::time::timeout(std::time::Duration::from_millis(100), listener.accept())
        .await
        .is_ok()
    {
        connections += 1;
    }
    assert_eq!(connections, 0);
}
#[tokio::test]
async fn oversized_http_response_is_traced() {
    let (interpreter, server) = serve("x".repeat(32769)).await;
    let (mut pass, models) = pass();
    assert_eq!(
        interpreter
            .interpret(&mut pass, &models, &assessment(true), NOW)
            .await
            .unwrap()
            .code(),
        "response_too_large"
    );
    server.await.unwrap();
    pass.trace.validate().unwrap();
}
#[tokio::test]
async fn capsule_expiry_fails_before_io() {
    let interpreter = AdvisoryInterpreter::new(
        ProviderEndpoint::nvidia_nim("http://127.0.0.1:1/v1", Locality::Local).unwrap(),
    )
    .unwrap();
    let (mut pass, models) = pass();
    assert_eq!(
        interpreter
            .interpret(&mut pass, &models, &assessment(true), NOW + 60000)
            .await
            .unwrap()
            .code(),
        "capsule_unavailable"
    );
}

async fn refused_wire(content: String, code: &str) {
    let body = json!({"model":"test.model","choices":[{"message":{"role":"assistant","content":content},"finish_reason":"stop"}]}).to_string();
    let (interpreter, server) = serve(body).await;
    let (mut pass, models) = pass();
    assert_eq!(
        interpreter
            .interpret(&mut pass, &models, &assessment(true), NOW)
            .await
            .unwrap()
            .code(),
        code
    );
    server.await.unwrap();
    assert_eq!(
        pass.trace
            .last(Stage::Interpret)
            .unwrap()
            .decision_code
            .as_deref(),
        Some(code)
    );
    pass.trace.validate().unwrap();
}
#[tokio::test]
async fn uncited_http_proposal_is_explicitly_traced() {
    let mut v = output();
    v["evidence_refs"] = json!([]);
    refused_wire(v.to_string(), "insufficient_citations").await;
}
#[tokio::test]
async fn fabricated_http_citation_is_explicitly_traced() {
    let mut v = output();
    v["evidence_refs"] = json!(["vault:unknown/one"]);
    refused_wire(v.to_string(), "unsupported_evidence_reference").await;
}
#[tokio::test]
async fn malformed_http_proposal_is_explicitly_traced() {
    refused_wire("invalid-json".into(), "invalid_proposal_json").await;
}
#[tokio::test]
async fn partial_or_unadmitted_capsule_never_reaches_http() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let interpreter = AdvisoryInterpreter::new(
        ProviderEndpoint::nvidia_nim(
            &format!("http://{}/v1", listener.local_addr().unwrap()),
            Locality::Local,
        )
        .unwrap(),
    )
    .unwrap();
    for r in ["vault:evidence/0", "vault:unknown/one"] {
        let (mut pass, models) = pass();
        let mut vault = Vault::open(&VaultKey::from_bytes([6; 32])).unwrap();
        let reference = VaultRef::parse(r).unwrap();
        vault
            .store(
                "test",
                reference.clone(),
                EntryClass::PrivateFact,
                PrivacyGrade::Restricted,
                "{\"synthetic\":true}",
                NOW,
                None,
            )
            .unwrap();
        let request = archie_vault::ContextRequest::new(
            "task:one",
            "trace:one",
            "test",
            Locality::Local,
            60000,
        )
        .including(reference);
        let capsule = ContextGateway::new()
            .assemble(&mut vault, &request, NOW)
            .unwrap();
        if let archie_cerebra::Intent::Reason {
            capsule: existing, ..
        } = &mut pass.intent
        {
            **existing = capsule;
        }
        assert_eq!(
            interpreter
                .interpret(&mut pass, &models, &assessment(true), NOW)
                .await
                .unwrap()
                .code(),
            "capsule_evidence_not_admitted"
        );
    }
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), listener.accept())
            .await
            .is_err()
    );
}
