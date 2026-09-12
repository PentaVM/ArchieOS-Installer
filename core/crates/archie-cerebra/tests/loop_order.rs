//! The sequencer, exercised end to end without a network.
//!
//! The claim under test is the one `docs/CEREBRA.md` §2.1 makes: the order of the stages
//! is load-bearing, not incidental. Most of these tests are about what does *not*
//! happen.

use archie_cerebra::{Cerebra, CerebraConfig, Intent, Observation, Trigger};
use archie_memory::floor::ConversationalFloorState;
use archie_observability::{Stage, StageOutcome};
use archie_registry::ModelRegistry;
use archie_router::Router;
use archie_types::{
    Availability, Capability, DataRetention, Locality, ModelDescriptor, ModelId, PrivacyGrade,
    ProviderKind, TokenCost, TraceId,
};
use archie_vault::{ContextGateway, Entity, EntityKind, EntryClass, Vault, VaultKey, VaultRef};
use std::collections::BTreeMap;

const T0: i64 = 1_760_000_000_000;

fn model(id: &str, locality: Locality, quality: u8) -> ModelDescriptor {
    ModelDescriptor {
        id: ModelId::parse(id).unwrap(),
        display_name: id.into(),
        provider: match locality {
            Locality::Local => ProviderKind::LlamaCpp,
            _ => ProviderKind::NebiusTokenFactory,
        },
        locality,
        node: None,
        context_window: 131_072,
        max_output_tokens: 8192,
        capabilities: vec![Capability::known("compute.inference")],
        max_privacy_grade: match locality {
            Locality::Local => PrivacyGrade::Restricted,
            _ => PrivacyGrade::Internal,
        },
        retention: DataRetention::ZeroVerified,
        availability: Availability::Ready,
        cost: TokenCost::FREE,
        latency_ema_ms: Some(200),
        quality_tier: quality,
        required_accelerator_bytes: None,
        labels: BTreeMap::new(),
    }
}

fn registry(models: Vec<ModelDescriptor>) -> ModelRegistry {
    let mut registry = ModelRegistry::new();
    for m in models {
        registry.register(m).unwrap();
    }
    registry
}

fn vault_with(entries: &[(&str, PrivacyGrade, &str)]) -> (Vault, Vec<VaultRef>) {
    let mut vault = Vault::open(&VaultKey::from_bytes([5u8; 32])).unwrap();
    let mut refs = Vec::new();
    for (path, grade, text) in entries {
        let reference = VaultRef::parse(path).unwrap();
        vault
            .store(
                "test",
                reference.clone(),
                EntryClass::PrivateFact,
                *grade,
                text,
                T0,
                None,
            )
            .unwrap();
        refs.push(reference);
    }
    (vault, refs)
}

fn cerebra() -> Cerebra {
    Cerebra::new(
        Router::default(),
        ContextGateway::new(),
        CerebraConfig::default(),
    )
}

/// A floor state describing somebody who is still mid-thought.
fn still_speaking() -> ConversationalFloorState {
    ConversationalFloorState {
        observed_at_ms: T0,
        current_speaker_ref: None,
        speaker_hold_probability: 0.9,
        thought_completion_probability: 0.2,
        expected_silence_horizon_ms: 500,
        silence_scope_ref: None,
        interruption_utility: 0.1,
        response_utility: 0.5,
        urgency: 0.1,
        context_sufficiency: 0.9,
        predicted_intent_ref: None,
        intent_confidence: 0.5,
        resume_probability: 0.8,
        overlap: false,
        archie_speaking: false,
        explicit_question: false,
        requested_floor: false,
        acknowledgement_only: false,
        listen_preference: false,
    }
}

/// Somebody who finished a question and is waiting.
fn finished_asking() -> ConversationalFloorState {
    ConversationalFloorState {
        speaker_hold_probability: 0.1,
        thought_completion_probability: 0.95,
        resume_probability: 0.1,
        explicit_question: true,
        ..still_speaking()
    }
}

#[test]
fn a_pass_that_stays_silent_never_opens_the_vault() {
    // The property the whole ordering exists for. If contextualisation ran first, a user
    // pausing mid-sentence would cause their private notes to be decrypted in order to
    // conclude that Archie should say nothing.
    let (mut vault, refs) = vault_with(&[(
        "vault:notes/private",
        PrivacyGrade::Restricted,
        "the operator's private note",
    )]);
    let models = registry(vec![model("local.qwen3", Locality::Local, 60)]);

    let observation = Observation::new(Trigger::Utterance, "task-1", "answer-question", T0)
        .needing(refs[0].clone())
        .graded(PrivacyGrade::Restricted)
        .with_floor(still_speaking());

    let pass = cerebra()
        .consider(&mut vault, &models, &observation, TraceId::new(), T0)
        .unwrap();

    assert!(pass.intent.is_silent());
    assert!(pass.intent.capsule().is_none(), "no capsule should exist");

    // The vault's own audit log is the witness: one store from the fixture, and nothing
    // else. No disclosure was attempted, so none was refused either.
    let accesses = vault.audit_log();
    assert_eq!(
        accesses.len(),
        1,
        "the vault was touched during a silent pass: {:?}",
        accesses.iter().map(|a| a.action).collect::<Vec<_>>()
    );
}

#[test]
fn the_trace_says_why_it_stayed_silent() {
    let (mut vault, _) = vault_with(&[]);
    let models = registry(vec![model("local.qwen3", Locality::Local, 60)]);
    let observation = Observation::new(Trigger::Utterance, "task-1", "answer-question", T0)
        .with_floor(still_speaking());

    let pass = cerebra()
        .consider(&mut vault, &models, &observation, TraceId::new(), T0)
        .unwrap();

    let record = pass.trace.last(Stage::Interpret).unwrap();
    match &record.outcome {
        StageOutcome::Silent { reason } => {
            assert_eq!(reason, "speaker_likely_continuing");
        }
        other => panic!("expected a recorded silence, got {other:?}"),
    }
    // And it never reached the stages that follow.
    assert!(pass.trace.last(Stage::Contextualize).is_none());
    assert!(pass.trace.last(Stage::Reason).is_none());
    assert!(pass.trace.hud().silent);
}

#[test]
fn a_completed_question_reaches_a_model_with_its_context() {
    let (mut vault, refs) = vault_with(&[(
        "vault:notes/schedule",
        PrivacyGrade::Internal,
        "dentist on Thursday",
    )]);
    let models = registry(vec![model("local.qwen3", Locality::Local, 60)]);

    let observation = Observation::new(Trigger::Utterance, "task-1", "answer-question", T0)
        .needing(refs[0].clone())
        .graded(PrivacyGrade::Internal)
        .with_floor(finished_asking());

    let pass = cerebra()
        .consider(&mut vault, &models, &observation, TraceId::new(), T0)
        .unwrap();

    match &pass.intent {
        Intent::Reason {
            capsule,
            model,
            floor,
        } => {
            assert_eq!(model.as_str(), "local.qwen3");
            assert!(floor.is_some(), "a person is waiting on a turn");
            let rendered = capsule.render("task-1", T0).unwrap();
            assert!(rendered.contains("dentist on Thursday"));
        }
        Intent::StaySilent { reason } => panic!("should have proceeded, stayed silent: {reason}"),
    }

    let hud = pass.trace.hud();
    assert!(!hud.silent);
    assert_eq!(hud.privacy, Some(PrivacyGrade::Internal));
    assert_eq!(hud.context_source_count, 1);
}

#[test]
fn a_timer_needs_no_floor_and_proceeds_without_one() {
    // A loop that only works when a person speaks is a chat application. A timer has no
    // floor to take, so it must not be blocked waiting for one.
    let (mut vault, _) = vault_with(&[]);
    let models = registry(vec![model("local.qwen3", Locality::Local, 60)]);

    let observation = Observation::new(
        Trigger::Timer {
            schedule: "hourly".into(),
        },
        "task-1",
        "check-calendar",
        T0,
    );

    let pass = cerebra()
        .consider(&mut vault, &models, &observation, TraceId::new(), T0)
        .unwrap();

    match &pass.intent {
        Intent::Reason { floor, .. } => {
            assert!(floor.is_none(), "there was no turn to take");
        }
        Intent::StaySilent { reason } => panic!("a timer should proceed, got {reason}"),
    }
}

#[test]
fn a_critical_alert_overrides_turn_taking() {
    let (mut vault, _) = vault_with(&[]);
    let models = registry(vec![model("local.qwen3", Locality::Local, 60)]);

    let quiet = Observation::new(
        Trigger::Sensor {
            device: "smoke-01".into(),
        },
        "task-1",
        "alarm",
        T0,
    )
    .with_floor(still_speaking());
    let urgent = Observation::new(
        Trigger::Sensor {
            device: "smoke-01".into(),
        },
        "task-1",
        "alarm",
        T0,
    )
    .with_floor(still_speaking())
    .urgent();

    let c = cerebra();
    assert!(c
        .consider(&mut vault, &models, &quiet, TraceId::new(), T0)
        .unwrap()
        .intent
        .is_silent());
    assert!(!c
        .consider(&mut vault, &models, &urgent, TraceId::new(), T0)
        .unwrap()
        .intent
        .is_silent());
}

#[test]
fn a_muted_node_stays_silent_however_complete_the_thought() {
    let (mut vault, _) = vault_with(&[]);
    let models = registry(vec![model("local.qwen3", Locality::Local, 60)]);
    let muted = Cerebra::new(
        Router::default(),
        ContextGateway::new(),
        CerebraConfig {
            speech_available: false,
            ..Default::default()
        },
    );

    let observation = Observation::new(Trigger::Utterance, "task-1", "answer-question", T0)
        .with_floor(finished_asking());

    assert!(muted
        .consider(&mut vault, &models, &observation, TraceId::new(), T0)
        .unwrap()
        .intent
        .is_silent());
}

#[test]
fn context_that_cannot_travel_stops_the_pass_before_a_model_is_chosen() {
    // Only a cloud model is registered, and the material is restricted. The gateway
    // refuses to build the capsule, and the pass ends there rather than routing to
    // something that would then have to refuse it again.
    let (mut vault, refs) = vault_with(&[(
        "vault:notes/medical",
        PrivacyGrade::Restricted,
        "the operator's medical history",
    )]);
    let models = registry(vec![model("cloud.big", Locality::Cloud, 100)]);

    let observation = Observation::new(
        Trigger::Timer {
            schedule: "daily".into(),
        },
        "task-1",
        "summarise",
        T0,
    )
    .needing(refs[0].clone())
    // Graded for the cloud, while the entry itself is restricted: the caller was wrong,
    // and the gateway is where that is caught.
    .graded(PrivacyGrade::Internal);

    let pass = cerebra()
        .consider(&mut vault, &models, &observation, TraceId::new(), T0)
        .unwrap();

    assert!(pass.intent.is_silent());
    match &pass.trace.last(Stage::Contextualize).unwrap().outcome {
        StageOutcome::Refused { reason } => {
            assert_eq!(reason, "context_undisclosable_at_destination")
        }
        other => panic!("expected a refusal, got {other:?}"),
    }
    assert!(
        pass.trace.last(Stage::Reason).is_none(),
        "no model should have been consulted"
    );
}

#[test]
fn restricted_work_with_only_a_cloud_model_refuses_rather_than_routes() {
    // The gateway permits a local capsule; the router then finds nothing that may see
    // it. Both refusals are real and they are recorded at different stages.
    let (mut vault, refs) = vault_with(&[(
        "vault:notes/medical",
        PrivacyGrade::Restricted,
        "the operator's medical history",
    )]);
    let models = registry(vec![model("cloud.big", Locality::Cloud, 100)]);

    let observation = Observation::new(
        Trigger::Timer {
            schedule: "daily".into(),
        },
        "task-1",
        "summarise",
        T0,
    )
    .needing(refs[0].clone())
    .graded(PrivacyGrade::Restricted);

    let pass = cerebra()
        .consider(&mut vault, &models, &observation, TraceId::new(), T0)
        .unwrap();

    assert!(pass.intent.is_silent());
    assert!(matches!(
        pass.trace.last(Stage::Contextualize).unwrap().outcome,
        StageOutcome::Proceeded
    ));
    match &pass.trace.last(Stage::Reason).unwrap().outcome {
        StageOutcome::Refused { reason } => assert_eq!(reason, "no_model_permitted"),
        other => panic!("expected a routing refusal, got {other:?}"),
    }
}

#[test]
fn entities_are_pseudonymised_when_the_capsule_is_bound_for_the_cloud() {
    let (mut vault, refs) = vault_with(&[(
        "vault:notes/deal",
        PrivacyGrade::Internal,
        "John Smith signed it",
    )]);
    let models = registry(vec![model("cloud.big", Locality::Cloud, 100)]);
    let cerebra = Cerebra::new(
        Router::default(),
        ContextGateway::new().knowing(Entity::new(EntityKind::Person, "John Smith")),
        CerebraConfig::default(),
    );

    let observation = Observation::new(
        Trigger::Timer {
            schedule: "daily".into(),
        },
        "task-1",
        "summarise",
        T0,
    )
    .needing(refs[0].clone())
    .graded(PrivacyGrade::Internal);

    let pass = cerebra
        .consider(&mut vault, &models, &observation, TraceId::new(), T0)
        .unwrap();

    let capsule = pass.intent.capsule().expect("should have proceeded");
    let rendered = capsule.render("task-1", T0).unwrap();
    assert!(!rendered.contains("John Smith"));
    assert!(rendered.contains("PERSON_1"));
    // And the answer comes back into the real name locally.
    assert_eq!(
        capsule.resolve("PERSON_1 signed it", T0).unwrap(),
        "John Smith signed it"
    );
}

#[test]
fn the_capsule_is_bound_to_the_task_that_caused_the_pass() {
    let (mut vault, refs) = vault_with(&[("vault:notes/one", PrivacyGrade::Internal, "something")]);
    let models = registry(vec![model("local.qwen3", Locality::Local, 60)]);

    let observation = Observation::new(
        Trigger::Timer {
            schedule: "daily".into(),
        },
        "task-alpha",
        "summarise",
        T0,
    )
    .needing(refs[0].clone());

    let pass = cerebra()
        .consider(&mut vault, &models, &observation, TraceId::new(), T0)
        .unwrap();
    let capsule = pass.intent.capsule().unwrap();

    assert!(capsule.render("task-alpha", T0).is_ok());
    assert!(
        capsule.render("task-beta", T0).is_err(),
        "context for one task is not context for another"
    );
}

#[test]
fn an_over_long_goal_label_fails_before_the_vault_is_opened() {
    // A caller pasting an utterance into the label is a disclosure into the trace. It
    // must fail, and it must fail before anything private has been read.
    let (mut vault, _) = vault_with(&[]);
    let models = registry(vec![model("local.qwen3", Locality::Local, 60)]);
    let utterance = "tell me what the lawyer said about the settlement and whether I \
                     should sign before Friday and also what I still owe on the invoice";

    let observation = Observation::new(Trigger::Utterance, "task-1", utterance, T0);

    assert!(cerebra()
        .consider(&mut vault, &models, &observation, TraceId::new(), T0)
        .is_err());
    assert!(vault.audit_log().is_empty());
}

#[test]
fn a_trace_from_a_real_pass_carries_no_content() {
    let (mut vault, refs) = vault_with(&[(
        "vault:notes/deal",
        PrivacyGrade::Internal,
        "John Smith signed the settlement",
    )]);
    let models = registry(vec![model("local.qwen3", Locality::Local, 60)]);

    let observation = Observation::new(Trigger::Utterance, "task-1", "draft-reply", T0)
        .needing(refs[0].clone())
        .graded(PrivacyGrade::Internal)
        .with_floor(finished_asking());

    let pass = cerebra()
        .consider(&mut vault, &models, &observation, TraceId::new(), T0)
        .unwrap();

    let serialised = serde_json::to_string(&pass.trace).unwrap();
    for forbidden in ["John Smith", "settlement", "signed"] {
        assert!(!serialised.contains(forbidden), "trace leaked {forbidden}");
    }
    // It still records the shape of what happened.
    assert!(serialised.contains("vault:notes/deal"));
    assert!(serialised.contains("local.qwen3"));
}
