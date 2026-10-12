use archie_model_runtime::routed::{Question, Routed};
use archie_registry::ModelRegistry;
use archie_types::{Availability, Capability, DataRetention, Locality, ModelDescriptor, ModelId, ModelRole, PrivacyGrade, ProviderKind, TokenCost};
use std::{collections::BTreeMap, time::Duration};

#[test]
fn deep_reasoning_excludes_models_below_the_requested_quality_floor() {
    let mut registry = ModelRegistry::new();
    for (id, quality) in [("local.small", 40), ("local.large", 90)] {
        registry.register(ModelDescriptor {
            id: ModelId::parse(id).unwrap(), display_name: id.into(), provider: ProviderKind::OpenAiCompatible,
            locality: Locality::Local, node: None, context_window: 8192, max_output_tokens: 2048,
            capabilities: vec![Capability::known("cognition.reasoning")], max_privacy_grade: PrivacyGrade::Restricted,
            retention: DataRetention::ZeroVerified, availability: Availability::Ready, cost: TokenCost::FREE,
            latency_ema_ms: Some(100), quality_tier: quality, required_accelerator_bytes: None,
            labels: BTreeMap::from([("base-url".into(), "http://127.0.0.1:8080/v1/".into())]),
        }).unwrap();
    }
    let routed = Routed::new(registry).requiring_quality(80);
    let question = Question { role: ModelRole::ReasoningWorker, privacy: PrivacyGrade::Restricted,
        system: "fixture instructions", user: "fixture context", image: None, max_tokens: 512, timeout: Duration::from_secs(1) };
    let candidates = routed.candidates(&question).unwrap();
    assert_eq!(candidates, vec![ModelId::parse("local.large").unwrap()], "deep mode cannot admit the smaller quality tier");
}
