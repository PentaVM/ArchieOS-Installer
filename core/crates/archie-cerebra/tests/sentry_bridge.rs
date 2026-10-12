//! The "hecho cuando" of PENDIENTES §3: a purpose-made port-scan alert travels
//! alerts → pillar references → a Cerebra pass, with its trace — and the refusals of
//! §4 are typed outcomes, not failures.
//!
//! Every alert here is a fixture: synthetic hardware addresses, RFC 1918 addresses and
//! details written for this test. No capture, no device, no network took part.

use archie_cerebra::pillars::Pillar;
use archie_cerebra::sentry_bridge::{
    consider_alerts, record_alerts, AlertInput, AlertOrigin, AlertPassOutcome, AlertSeverity,
    BridgeError, PassRefusal, MAX_ALERTS_PER_BATCH,
};
use archie_cerebra::{Cerebra, CerebraConfig};
use archie_observability::{Stage, StageOutcome};
use archie_registry::ModelRegistry;
use archie_router::Router;
use archie_types::{
    Availability, Capability, DataRetention, Locality, ModelDescriptor, ModelId, PrivacyGrade,
    ProviderKind, TokenCost, TraceId,
};
use archie_vault::{ContextDomain, ContextGateway, EntryClass, Vault, VaultKey};
use std::collections::BTreeMap;

/// A fixture device. The address is the one `archie-lan`'s own tests use for a
/// synthetic television; nothing with this address was ever on a network of ours.
const FIXTURE_DEVICE: &str = "54:4e:f0:07:3d:e8";

/// The alert the audit raises when one device opens 100 ports of one host in a minute
/// — built by hand here, exactly as `archie-lan`'s port-scan fixture builds it.
fn fixture_port_scan(at_ms: u64) -> AlertInput {
    AlertInput::measured(
        at_ms,
        AlertSeverity::High,
        "port-scan",
        FIXTURE_DEVICE,
        "opened 100 ports of 192.168.1.1 within a minute",
    )
    .with_provenance(vec!["netwatch-fixture:told.json".to_string()])
}

fn fixture_batch() -> Vec<AlertInput> {
    vec![
        AlertInput::measured(
            10_000,
            AlertSeverity::Notice,
            "new-device",
            FIXTURE_DEVICE,
            "first seen, at 192.168.1.84",
        ),
        AlertInput::measured(
            12_000,
            AlertSeverity::Warning,
            "cleartext-login",
            FIXTURE_DEVICE,
            "opened Telnet to 192.168.1.1; its passwords are readable on the wire",
        ),
        fixture_port_scan(15_000),
    ]
}

fn domain() -> ContextDomain {
    ContextDomain::personal("owner").unwrap()
}

fn cerebra() -> Cerebra {
    Cerebra::new(Router::default(), ContextGateway::new(), CerebraConfig::default())
}

fn local_model() -> ModelDescriptor {
    ModelDescriptor {
        id: ModelId::parse("local.qwen").unwrap(),
        display_name: "local adviser".into(),
        provider: ProviderKind::LlamaCpp,
        locality: Locality::Local,
        node: None,
        context_window: 8192,
        max_output_tokens: 2048,
        capabilities: vec![Capability::known("compute.inference")],
        max_privacy_grade: PrivacyGrade::Restricted,
        retention: DataRetention::ZeroVerified,
        availability: Availability::Ready,
        cost: TokenCost::FREE,
        latency_ema_ms: Some(100),
        quality_tier: 60,
        required_accelerator_bytes: None,
        labels: BTreeMap::new(),
    }
}

fn open_vault() -> Vault {
    Vault::open(&VaultKey::from_bytes([7; 32])).unwrap()
}

#[test]
fn a_fixture_port_scan_travels_alerts_to_pillar_references_to_a_traced_pass() {
    let domain = domain();
    let mut vault = open_vault();
    let mut models = ModelRegistry::new();
    models.register(local_model()).unwrap();
    let trace_id = TraceId::new();

    let outcome = consider_alerts(
        &cerebra(),
        &mut vault,
        &models,
        &domain,
        "sentry-review",
        &fixture_batch(),
        trace_id,
        20_000,
    )
    .unwrap();

    let (pass, records) = match outcome {
        AlertPassOutcome::Reasoning { pass, records } => (pass, records),
        other => panic!("a node with a local model reasons over the alerts: {other:?}"),
    };
    // The pass record carries the trace id it was given, and every reference it saw is
    // one of the alert records, tagged to the Sentry pillar.
    assert_eq!(pass.trace.trace_id, trace_id);
    assert_eq!(records.len(), 3);
    for record in &records {
        assert!(domain.contains(record.reference()), "{}", record.reference());
        assert_eq!(record.origin(), AlertOrigin::Measurement);
    }
    let capsule = pass.intent.capsule().expect("reasoning holds a capsule");
    assert_eq!(capsule.destination(), Locality::Local);
    assert_eq!(capsule.entries().len(), 3);
    // §4: the pass is graded restricted, and the trace says so wherever it handled it.
    assert_eq!(pass.trace.peak_privacy(), Some(PrivacyGrade::Restricted));
    assert!(pass.trace.last(Stage::Contextualize).is_some_and(|r| r.outcome.proceeded()));
    assert!(pass.trace.last(Stage::Reason).is_some_and(|r| r.outcome.proceeded()));
    // The observation was stamped with the newest alert's own time, not the caller's.
    assert_eq!(pass.trace.started_at_ms, 15_000);
}

#[test]
fn without_a_local_model_the_restricted_pass_is_refused_and_the_refusal_is_recorded() {
    let mut vault = open_vault();
    let trace_id = TraceId::new();
    let outcome = consider_alerts(
        &cerebra(),
        &mut vault,
        &ModelRegistry::new(),
        &domain(),
        "sentry-review",
        &[fixture_port_scan(15_000)],
        trace_id,
        20_000,
    )
    .unwrap();

    let (pass, refusal, records) = match outcome {
        AlertPassOutcome::Refused { pass, refusal, records } => (pass, refusal, records),
        other => {
            panic!("restricted context with no local model is a refusal, not silence: {other:?}")
        }
    };
    assert_eq!(refusal, PassRefusal::NoLocalModel);
    assert_eq!(pass.trace.trace_id, trace_id);
    assert!(pass.intent.is_silent());
    let reason = pass.trace.last(Stage::Reason).expect("the refusal is in the trace");
    assert_eq!(
        reason.outcome,
        StageOutcome::Refused { reason: "no_model_permitted".into() }
    );
    assert_eq!(reason.privacy, Some(PrivacyGrade::Restricted));
    // The evidence was still recorded: the refusal does not lose the alert.
    assert_eq!(records.len(), 1);
    assert!(vault.contains(records[0].reference()));
}

#[test]
fn a_malformed_or_oversized_batch_is_rejected_without_touching_the_vault() {
    let domain = domain();

    let oversized: Vec<AlertInput> = (0..=MAX_ALERTS_PER_BATCH as u64)
        .map(|i| fixture_port_scan(15_000 + i))
        .collect();
    let mut vault = open_vault();
    assert!(matches!(
        consider_alerts(&cerebra(), &mut vault, &ModelRegistry::new(), &domain, "t", &oversized, TraceId::new(), 20_000),
        Err(BridgeError::BatchTooLarge { found, cap }) if found == MAX_ALERTS_PER_BATCH + 1 && cap == MAX_ALERTS_PER_BATCH
    ));
    assert!(vault.audit_log().is_empty(), "nothing was stored");

    for (alert, field) in [
        (fixture_port_scan(0), "at_ms"),
        (
            AlertInput::measured(1, AlertSeverity::High, "port scan", FIXTURE_DEVICE, "d"),
            "rule",
        ),
        (
            AlertInput::measured(1, AlertSeverity::High, "port-scan", "", "d"),
            "device",
        ),
        (
            AlertInput::measured(1, AlertSeverity::High, "port-scan", FIXTURE_DEVICE, "two\nlines"),
            "detail",
        ),
        (fixture_port_scan(1).with_occurrences(0), "occurrences"),
        (
            fixture_port_scan(1).with_provenance(vec!["e".to_string(); 17]),
            "provenance",
        ),
    ] {
        let mut vault = open_vault();
        match record_alerts(&mut vault, &domain, "t", &[alert]) {
            Err(BridgeError::InvalidAlert { field: found, .. }) => assert_eq!(found, field),
            other => panic!("expected a typed invalid-alert error for {field}: {other:?}"),
        }
        assert!(vault.audit_log().is_empty(), "{field}: nothing was stored");
    }
}

#[test]
fn a_hypothesis_is_recorded_as_a_hypothesis_never_a_measurement() {
    let mut vault = open_vault();
    let hypothesis = AlertInput {
        origin: AlertOrigin::Hypothesis,
        ..fixture_port_scan(15_000)
    };
    let batch = record_alerts(&mut vault, &domain(), "t", &[hypothesis]).unwrap();
    let record = &batch.records()[0];
    assert_eq!(record.origin(), AlertOrigin::Hypothesis);
    assert!(record.content().contains("\"origin\":\"hypothesis\""), "{}", record.content());
    assert!(!record.content().contains("measurement"));
    // And what the vault holds says the same, verbatim.
    let disclosure = vault
        .disclose("test", record.reference(), PrivacyGrade::Restricted, 20_000)
        .unwrap();
    assert!(disclosure.text().unwrap().contains("\"origin\":\"hypothesis\""));
}

#[test]
fn an_empty_batch_runs_no_pass_and_stores_nothing() {
    let mut vault = open_vault();
    let outcome = consider_alerts(
        &cerebra(),
        &mut vault,
        &ModelRegistry::new(),
        &domain(),
        "sentry-review",
        &[],
        TraceId::new(),
        20_000,
    )
    .unwrap();
    assert!(matches!(outcome, AlertPassOutcome::NoAlerts));
    assert!(outcome.pass().is_none(), "no pass record exists");
    assert!(vault.audit_log().is_empty());
}

#[test]
fn replays_dedupe_and_a_foreign_record_at_the_reference_is_a_collision() {
    let domain = domain();
    let mut vault = open_vault();

    // The same alert twice in one batch is one reference, stored once.
    let batch = record_alerts(
        &mut vault,
        &domain,
        "t",
        &[fixture_port_scan(15_000), fixture_port_scan(15_000)],
    )
    .unwrap();
    assert_eq!(batch.records().len(), 1);
    assert_eq!(batch.context().references().len(), 1);
    assert!(batch.context().references()[batch.records()[0].reference()].contains(&Pillar::Sentry));

    // Re-reading the same alerts.jsonl after a restart is idempotent.
    let again = record_alerts(&mut vault, &domain, "t", &[fixture_port_scan(15_000)]).unwrap();
    assert_eq!(again.records().len(), 1);

    // Something else sitting at the alert's reference is named, never overwritten.
    let reference = batch.records()[0].reference().clone();
    let mut foreign = open_vault();
    foreign
        .store(
            "test",
            reference.clone(),
            EntryClass::PrivateFact,
            PrivacyGrade::Restricted,
            "something else entirely",
            1,
            None,
        )
        .unwrap();
    assert!(matches!(
        record_alerts(&mut foreign, &domain, "t", &[fixture_port_scan(15_000)]),
        Err(BridgeError::ReferenceCollision { reference: found }) if found == reference
    ));
}

#[test]
fn severity_comes_from_the_rule_and_is_never_lowered() {
    let domain = domain();
    // The alert said notice; the rule table says a port-scan is high.
    let low = AlertInput::measured(1, AlertSeverity::Notice, "port-scan", FIXTURE_DEVICE, "d");
    assert_eq!(recorded_severity(&domain, &low), AlertSeverity::High);
    // The alert said high — the gateway case of address-moved — and the table's
    // notice does not lower it.
    let gateway = AlertInput::measured(1, AlertSeverity::High, "address-moved", FIXTURE_DEVICE, "d");
    assert_eq!(recorded_severity(&domain, &gateway), AlertSeverity::High);
    // A rule the table does not know keeps the alert's own severity.
    let unknown = AlertInput::measured(1, AlertSeverity::Warning, "future-rule", FIXTURE_DEVICE, "d");
    assert_eq!(recorded_severity(&domain, &unknown), AlertSeverity::Warning);
}

/// The severity that landed in the stored record for one alert.
fn recorded_severity(domain: &ContextDomain, alert: &AlertInput) -> AlertSeverity {
    let mut vault = open_vault();
    let batch = record_alerts(&mut vault, domain, "t", std::slice::from_ref(alert)).unwrap();
    assert_eq!(batch.records().len(), 1);
    batch.records()[0].severity()
}
