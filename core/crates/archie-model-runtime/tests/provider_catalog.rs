use archie_model_runtime::provider::{builtin_provider, load_catalog, save_provider};
use archie_model_runtime::routed::{Question, Routed};
use archie_model_runtime::{ProviderEndpoint, RemoteModel};
use archie_types::{DataRetention, Locality, ModelRole, PrivacyGrade, ProviderKind};
use std::time::Duration;

#[test]
fn configured_ids_become_routable_without_inventing_observations() {
    let temp = tempfile::tempdir().unwrap();
    let providers = temp.path().join("providers.toml");
    let mut provider = builtin_provider("ollama").unwrap();
    provider.models = vec!["org/Qwen:small".into()];
    save_provider(&providers, provider).unwrap();
    let routed = Routed::from_paths(temp.path().join("models"), &providers).unwrap();
    let question = Question {
        role: ModelRole::CognitiveResponse,
        privacy: PrivacyGrade::Restricted,
        system: "Answer clearly",
        user: "Hello",
        image: None,
        max_tokens: 128,
        timeout: Duration::from_secs(1),
    };
    assert_eq!(routed.candidates(&question).unwrap().len(), 1);
    let model = routed.registry().iter().next().unwrap();
    assert_eq!(model.labels["provider-model"], "org/Qwen:small");
    assert_eq!(model.retention, DataRetention::Unknown);
    assert_eq!(model.latency_ema_ms, None);
    assert_eq!(model.quality_tier, 0);
    assert_eq!(model.labels["cost-status"], "unknown");
    assert_eq!(model.labels["context-status"], "unverified-budget");
    assert!(!model.provides(&ModelRole::ReasoningWorker.capability()));
}

#[test]
fn discovery_and_configuration_share_one_collision_safe_catalog() {
    let mut a = builtin_provider("ollama").unwrap();
    a.models = vec!["org/model".into()];
    let a_models = a
        .descriptors(&[
            RemoteModel {
                id: "org/model".into(),
            },
            RemoteModel {
                id: "org-model".into(),
            },
        ])
        .unwrap();
    assert_eq!(a_models.len(), 2);
    assert_ne!(a_models[0].id, a_models[1].id);
    let mut b = a.clone();
    b.id = "another".into();
    let b_models = b.descriptors(&[]).unwrap();
    assert!(a_models.iter().all(|m| m.id != b_models[0].id));
    assert!(a
        .descriptors(&[RemoteModel {
            id: "bad\nmodel".into()
        }])
        .is_err());
}

#[test]
fn anthropic_catalog_uses_native_cloud_transport_and_default_denial_for_private_data() {
    let mut provider = builtin_provider("anthropic").unwrap();
    provider.models = vec!["claude-configured-model".into()];
    let models = provider.descriptors(&[]).unwrap();
    assert_eq!(models[0].provider, ProviderKind::Anthropic);
    assert!(!models[0].permits_privacy(PrivacyGrade::Internal));
    assert!(ProviderEndpoint::for_model(&models[0]).is_ok());
    assert!(provider.endpoint().is_ok());
}

#[test]
fn duplicate_provider_identity_and_conflicting_manifest_are_errors() {
    let temp = tempfile::tempdir().unwrap();
    let providers = temp.path().join("providers.toml");
    let mut provider = builtin_provider("ollama").unwrap();
    provider.models = vec!["model".into()];
    save_provider(&providers, provider.clone()).unwrap();
    let models = temp.path().join("models");
    std::fs::create_dir_all(&models).unwrap();
    std::fs::write(
        models.join("duplicate.toml"),
        toml::to_string(&provider.descriptors(&[]).unwrap()[0]).unwrap(),
    )
    .unwrap();
    assert!(load_catalog(&models, &providers).is_err());
    let config = std::fs::read_to_string(&providers).unwrap();
    std::fs::write(&providers, config.repeat(2)).unwrap();
    assert!(load_catalog(temp.path().join("empty"), &providers).is_err());
}

#[test]
fn reserved_cloud_id_cannot_admit_fabric_or_loopback_local_data() {
    for id in ["openai", "nebius", "cerebras", "openrouter", "anthropic"] {
        let mut provider = builtin_provider(id).unwrap();
        provider.models = vec!["model".into()];
        provider.locality = Locality::Fabric;
        assert!(provider.validate().is_err(), "{id} became Fabric");
        assert!(provider.endpoint().is_err());
        assert!(provider.descriptors(&[]).is_err());
        provider.locality = Locality::Local;
        provider.base_url = "http://127.0.0.1:9999/v1/".into();
        provider.credential_env = None;
        assert!(
            provider.validate().is_err(),
            "{id} became Local through a proxy"
        );
        assert!(provider.descriptors(&[]).is_err());
    }
}

#[test]
fn known_cloud_service_host_cannot_be_hidden_by_a_generic_provider_id() {
    for host in [
        "api.openai.com",
        "API.OPENAI.COM.",
        "api.tokenfactory.nebius.com",
        "api.cerebras.ai",
        "openrouter.ai",
        "api.anthropic.com",
    ] {
        let endpoint = format!("https://{host}/v1/");
        assert!(
            ProviderEndpoint::openai_compatible(&endpoint, Locality::Fabric, None).is_err(),
            "{host} became Fabric"
        );
        let mut provider = builtin_provider("ollama").unwrap();
        provider.id = "operator.alias".into();
        provider.base_url = endpoint;
        provider.locality = Locality::Fabric;
        assert!(provider.validate().is_err());
    }
}

#[test]
fn openai_and_nebius_descriptors_keep_their_fixed_cloud_identity() {
    for (id, kind) in [
        ("openai", ProviderKind::OpenAi),
        ("nebius", ProviderKind::NebiusTokenFactory),
    ] {
        let mut provider = builtin_provider(id).unwrap();
        provider.models = vec!["model".into()];
        let mut descriptor = provider.descriptors(&[]).unwrap().remove(0);
        assert_eq!(descriptor.provider, kind);
        assert!(!descriptor.permits_privacy(PrivacyGrade::Restricted));
        assert!(!descriptor.permits_privacy(PrivacyGrade::Secret));
        assert!(ProviderEndpoint::for_model(&descriptor).is_ok());
        descriptor.locality = Locality::Fabric;
        assert!(archie_registry::ModelRegistry::new()
            .register(descriptor)
            .is_err());
    }
}

#[test]
fn generic_operator_deployments_and_https_cloud_proxy_keep_explicit_locality() {
    let mut provider = builtin_provider("ollama").unwrap();
    provider.id = "operator.deployment".into();
    provider.models = vec!["model".into()];
    assert!(provider.descriptors(&[]).unwrap()[0].permits_privacy(PrivacyGrade::Restricted));
    provider.base_url = "https://worker.fabric.internal/v1/".into();
    provider.locality = Locality::Fabric;
    assert!(provider.descriptors(&[]).unwrap()[0].permits_privacy(PrivacyGrade::Sensitive));
    let mut proxy = builtin_provider("openai").unwrap();
    proxy.models = vec!["model".into()];
    proxy.base_url = "https://127.0.0.1:9999/v1/".into();
    let descriptor = proxy.descriptors(&[]).unwrap().remove(0);
    assert_eq!(descriptor.locality, Locality::Cloud);
    assert!(!descriptor.permits_privacy(PrivacyGrade::Restricted));
    assert!(!descriptor.permits_privacy(PrivacyGrade::Secret));
}
