use archie_cerebra::pillars::{Pillar, PillarContext};
use archie_cerebra::{Cerebra, CerebraConfig, Observation, Trigger};
use archie_registry::ModelRegistry;
use archie_router::Router;
use archie_types::{
    Availability, Capability, DataRetention, Locality, ModelDescriptor, ModelId, PrivacyGrade,
    ProviderKind, TokenCost, TraceId,
};
use archie_vault::{ContextDomain, ContextGateway, EntryClass, Vault, VaultKey, VaultRef};
use std::collections::BTreeMap;

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

#[test]
fn all_pillars_reach_the_existing_loop_as_scoped_local_context() {
    let domain = ContextDomain::work("studio", "house").unwrap();
    let mut context = PillarContext::new(domain.clone(), "cad-task").unwrap();
    let mut vault = Vault::open(&VaultKey::from_bytes([7; 32])).unwrap();
    for (pillar, name) in [
        (Pillar::Sentry, "environment"),
        (Pillar::Wanda, "manager"),
        (Pillar::Vault, "brief"),
        (Pillar::Iris, "document"),
        (Pillar::Forge, "plan"),
        (Pillar::VerifiedOutcome, "receipt"),
    ] {
        let reference = domain.reference(name).unwrap();
        vault
            .store(
                "test",
                reference.clone(),
                EntryClass::PrivateFact,
                PrivacyGrade::Restricted,
                "private evidence",
                100,
                None,
            )
            .unwrap();
        context.include(pillar, reference).unwrap();
    }
    let observation = context.observation(
        Trigger::SystemEvent {
            source: "enterprise_bridge".into(),
        },
        "prepare-cad",
        101,
    );
    assert_eq!(observation.privacy, PrivacyGrade::Restricted);
    let mut models = ModelRegistry::new();
    models.register(local_model()).unwrap();
    let pass = Cerebra::new(
        Router::default(),
        ContextGateway::new(),
        CerebraConfig::default(),
    )
    .consider(&mut vault, &models, &observation, TraceId::new(), 101)
    .unwrap();
    let capsule = pass
        .intent
        .capsule()
        .expect("scoped context should route locally");
    assert_eq!(capsule.receipt().domain, Some(domain));
    assert_eq!(capsule.entries().len(), 6);
    assert_eq!(capsule.destination(), Locality::Local);
    assert!(!format!("{context:?}").contains("private evidence"));
}

#[test]
fn work_never_absorbs_personal_or_another_project() {
    let mut context =
        PillarContext::new(ContextDomain::work("studio", "house").unwrap(), "task").unwrap();
    for path in ["personal/owner/diary", "work/studio/other/brief"] {
        assert!(context
            .include(Pillar::Wanda, VaultRef::from_path(path).unwrap())
            .is_err());
    }
    assert!(PillarContext::new(ContextDomain::personal("owner").unwrap(), "").is_err());
}

#[test]
fn continuous_observation_cannot_grow_context_without_bound() {
    let domain = ContextDomain::personal("owner").unwrap();
    let mut context = PillarContext::new(domain.clone(), "task").unwrap();
    for index in 0..archie_cerebra::pillars::MAX_CONTEXT_REFERENCES {
        context
            .include(
                Pillar::Sentry,
                domain.reference(&format!("event-{index}")).unwrap(),
            )
            .unwrap();
    }
    let before = context.references().len();
    assert!(context
        .include(Pillar::Iris, domain.reference("overflow").unwrap())
        .is_err());
    context
        .include(Pillar::Wanda, domain.reference("event-0").unwrap())
        .unwrap();
    assert_eq!(context.references().len(), before);
}

#[test]
fn a_forged_observation_is_rejected_before_vault_access() {
    let mut vault = Vault::open(&VaultKey::from_bytes([7; 32])).unwrap();
    let observation = Observation::new(
        Trigger::SystemEvent {
            source: "test".into(),
        },
        "task",
        "cad",
        101,
    )
    .in_domain(ContextDomain::work("studio", "house").unwrap())
    .needing(VaultRef::parse("vault:personal/owner/diary").unwrap());
    let pass = Cerebra::new(
        Router::default(),
        ContextGateway::new(),
        CerebraConfig::default(),
    )
    .consider(
        &mut vault,
        &ModelRegistry::new(),
        &observation,
        TraceId::new(),
        101,
    )
    .unwrap();
    assert!(pass.intent.is_silent());
    assert!(vault.audit_log().is_empty());
}
