//! Shared provider catalog. Identifiers are configured/discovered, never model entitlements.
use crate::{InferenceClient, ProviderEndpoint, RemoteModel, RuntimeError};
use archie_registry::{ModelRegistry, ModelRegistryError};
use archie_types::{
    Availability, Capability, DataRetention, Locality, ModelDescriptor, ModelId, PrivacyGrade,
    ProviderKind, TokenCost,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProviderProtocol {
    OpenAiCompatible,
    Anthropic,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderSpec {
    pub id: String,
    pub protocol: ProviderProtocol,
    pub base_url: String,
    pub locality: Locality,
    pub credential_env: Option<String>,
    #[serde(default)]
    pub models: Vec<String>,
}

#[derive(Debug, Error)]
pub enum ProviderConfigError {
    #[error("provider id or model id is invalid")]
    InvalidId,
    #[error("provider endpoint is invalid: {0}")]
    Endpoint(#[from] RuntimeError),
    #[error("provider config I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("provider config is invalid: {0}")]
    Decode(#[from] toml::de::Error),
    #[error("provider protocol needs its native transport")]
    UnsupportedProtocol,
    #[error("model catalog is invalid: {0}")]
    Registry(#[from] ModelRegistryError),
}

impl ProviderSpec {
    pub fn validate(&self) -> Result<(), ProviderConfigError> {
        let valid = |value: &str| {
            !value.is_empty()
                && value
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/'))
        };
        if !valid(&self.id)
            || self.id.len() > 128
            || self.models.len() > 1024
            || self.models.iter().any(|model| !valid_model_id(model))
            || self.models.iter().collect::<BTreeSet<_>>().len() != self.models.len()
        {
            return Err(ProviderConfigError::InvalidId);
        }
        // A local wire address does not change a known provider's inference owner.
        if matches!(
            self.id.to_ascii_lowercase().as_str(),
            "openai" | "nebius" | "cerebras" | "openrouter" | "anthropic"
        ) && self.locality != Locality::Cloud
        {
            return Err(RuntimeError::LocalityMismatch.into());
        }
        self.transport_endpoint()?;
        Ok(())
    }

    pub fn openai_endpoint(&self) -> Result<ProviderEndpoint, ProviderConfigError> {
        self.validate()?;
        if self.protocol != ProviderProtocol::OpenAiCompatible {
            return Err(ProviderConfigError::UnsupportedProtocol);
        }
        Ok(ProviderEndpoint::openai_compatible(
            &self.base_url,
            self.locality,
            self.credential_env.clone(),
        )?)
    }

    fn transport_endpoint(&self) -> Result<ProviderEndpoint, ProviderConfigError> {
        Ok(match self.protocol {
            ProviderProtocol::OpenAiCompatible => ProviderEndpoint::openai_compatible(
                &self.base_url,
                self.locality,
                self.credential_env.clone(),
            )?,
            ProviderProtocol::Anthropic => {
                if self.locality != Locality::Cloud {
                    return Err(RuntimeError::LocalityMismatch.into());
                }
                ProviderEndpoint::anthropic(&self.base_url, self.credential_env.clone())?
            }
        })
    }

    pub fn endpoint(&self) -> Result<ProviderEndpoint, ProviderConfigError> {
        self.validate()?;
        self.transport_endpoint()
    }

    /// Discovery does not verify entitlement or claim a successful inference turn.
    pub async fn discover(&self) -> Result<Vec<RemoteModel>, ProviderConfigError> {
        Ok(InferenceClient::new(self.endpoint()?)?.models().await?.data)
    }

    /// Merge configured/discovered names into conservative text-only registrations.
    /// The context/output values are operator budgets, not measured model capacities.
    /// Unknown cost is charged conservatively rather than advertised as free.
    pub fn descriptors(
        &self,
        discovered: &[RemoteModel],
    ) -> Result<Vec<ModelDescriptor>, ProviderConfigError> {
        self.validate()?;
        if discovered.len() > 1024 {
            return Err(ProviderConfigError::InvalidId);
        }
        let mut ids = BTreeSet::new();
        for id in self
            .models
            .iter()
            .map(String::as_str)
            .chain(discovered.iter().map(|m| m.id.as_str()))
        {
            if !valid_model_id(id) {
                return Err(ProviderConfigError::InvalidId);
            }
            ids.insert(id);
        }
        ids.into_iter()
            .map(|remote| {
                let mut digest = Sha256::new();
                digest.update(self.id.as_bytes());
                digest.update([0]);
                digest.update(remote.as_bytes());
                let id = ModelId::parse(format!("catalog.m{:x}", digest.finalize()))
                    .map_err(|_| ProviderConfigError::InvalidId)?;
                // A local server that forwards a model to a cloud service (Ollama's
                // `*-cloud` tags) does not make that model local.
                let locality = if forwards_to_cloud(remote) {
                    Locality::Cloud
                } else {
                    self.locality
                };
                let mut labels = BTreeMap::from([
                    ("provider-id".into(), self.id.clone()),
                    ("provider-model".into(), remote.to_string()),
                    ("base-url".into(), self.base_url.clone()),
                    ("cost-status".into(), "unknown".into()),
                    ("context-status".into(), "unverified-budget".into()),
                    ("capabilities-status".into(), "text-only-unverified".into()),
                    ("availability-status".into(), "configured-unverified".into()),
                ]);
                if let Some(name) = &self.credential_env {
                    labels.insert("credential-env".into(), name.clone());
                }
                Ok(ModelDescriptor {
                    id,
                    display_name: format!("{}: {remote}", self.id),
                    provider: match self.protocol {
                        ProviderProtocol::OpenAiCompatible => {
                            match self.id.to_ascii_lowercase().as_str() {
                                "openai" => ProviderKind::OpenAi,
                                "nebius" => ProviderKind::NebiusTokenFactory,
                                _ => ProviderKind::OpenAiCompatible,
                            }
                        }
                        ProviderProtocol::Anthropic => ProviderKind::Anthropic,
                    },
                    locality,
                    node: None,
                    context_window: 4096,
                    max_output_tokens: 1024,
                    capabilities: vec![Capability::known("cognition.response")],
                    max_privacy_grade: match locality {
                        Locality::Local => PrivacyGrade::Restricted,
                        Locality::Fabric => PrivacyGrade::Sensitive,
                        Locality::Cloud => PrivacyGrade::Public,
                    },
                    retention: DataRetention::Unknown,
                    availability: Availability::Ready,
                    cost: TokenCost {
                        // TOML integers are signed; remain serializable to a manifest.
                        input_micros_per_mtok: i64::MAX as u64,
                        output_micros_per_mtok: i64::MAX as u64,
                    },
                    latency_ema_ms: None,
                    quality_tier: 0,
                    required_accelerator_bytes: None,
                    labels,
                })
            })
            .collect()
    }
}

/// Ollama names the models it forwards to ollama.com with a `cloud` tag, as in
/// `gpt-oss:120b-cloud` or `glm-4.6:cloud`.
fn forwards_to_cloud(remote: &str) -> bool {
    let tag = remote
        .rsplit_once(':')
        .map_or("", |(_, tag)| tag)
        .to_ascii_lowercase();
    tag == "cloud" || tag.ends_with("-cloud")
}

fn valid_model_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && value.is_ascii()
        && !value.chars().any(|c| c.is_whitespace() || c.is_control())
}

/// Load operator manifests and configured provider names. Network discovery is explicit.
pub fn load_catalog(
    models: impl AsRef<Path>,
    providers: impl AsRef<Path>,
) -> Result<ModelRegistry, ProviderConfigError> {
    let mut registry = ModelRegistry::load_from_dir(models)?;
    for provider in load_providers(providers.as_ref())?.into_values() {
        for descriptor in provider.descriptors(&[])? {
            registry.register(descriptor)?;
        }
    }
    Ok(registry)
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct ProviderFile {
    #[serde(default)]
    provider: Vec<ProviderSpec>,
}

pub fn save_provider(path: &Path, provider: ProviderSpec) -> Result<(), ProviderConfigError> {
    provider.validate()?;
    let mut providers = load_providers(path)?;
    providers.insert(provider.id.clone(), provider);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let encoded = toml::to_string_pretty(&ProviderFile {
        provider: providers.into_values().collect(),
    })
    .map_err(|_| ProviderConfigError::InvalidId)?;
    std::fs::write(path, encoded)?;
    Ok(())
}

pub fn load_providers(path: &Path) -> Result<BTreeMap<String, ProviderSpec>, ProviderConfigError> {
    if !path.exists() {
        return Ok(BTreeMap::new());
    }
    let file: ProviderFile = toml::from_str(&std::fs::read_to_string(path)?)?;
    let mut providers = BTreeMap::new();
    for provider in file.provider {
        provider.validate()?;
        if providers.insert(provider.id.clone(), provider).is_some() {
            return Err(ProviderConfigError::InvalidId);
        }
    }
    Ok(providers)
}

pub fn builtin_provider(id: &str) -> Option<ProviderSpec> {
    let (protocol, base_url, credential_env) = match id {
        "nebius" => (
            ProviderProtocol::OpenAiCompatible,
            "https://api.tokenfactory.nebius.com/v1/",
            "NEBIUS_TOKEN_FACTORY_KEY",
        ),
        "cerebras" => (
            ProviderProtocol::OpenAiCompatible,
            "https://api.cerebras.ai/v1/",
            "CEREBRAS_API_KEY",
        ),
        "openai" => (
            ProviderProtocol::OpenAiCompatible,
            "https://api.openai.com/v1/",
            "OPENAI_API_KEY",
        ),
        "openrouter" => (
            ProviderProtocol::OpenAiCompatible,
            "https://openrouter.ai/api/v1/",
            "OPENROUTER_API_KEY",
        ),
        "anthropic" => (
            ProviderProtocol::Anthropic,
            "https://api.anthropic.com/v1/",
            "ANTHROPIC_API_KEY",
        ),
        "ollama" => (
            ProviderProtocol::OpenAiCompatible,
            "http://127.0.0.1:11434/v1/",
            "",
        ),
        "vllm" => (
            ProviderProtocol::OpenAiCompatible,
            "http://127.0.0.1:8000/v1/",
            "",
        ),
        _ => return None,
    };
    Some(ProviderSpec {
        id: id.into(),
        protocol,
        base_url: base_url.into(),
        locality: if credential_env.is_empty() {
            Locality::Local
        } else {
            Locality::Cloud
        },
        credential_env: (!credential_env.is_empty()).then(|| credential_env.into()),
        models: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cloud_requires_https_and_runtime_secret() {
        let mut provider = builtin_provider("cerebras").unwrap();
        assert!(provider.validate().is_ok());
        provider.base_url = "http://cerebras.invalid/v1".into();
        assert!(provider.validate().is_err());
        provider.base_url = "https://cerebras.invalid/v1".into();
        provider.credential_env = None;
        assert!(provider.validate().is_err());
    }

    #[test]
    fn local_backends_never_require_a_secret() {
        for id in ["ollama", "vllm"] {
            let provider = builtin_provider(id).unwrap();
            assert_eq!(provider.locality, Locality::Local);
            assert!(provider.credential_env.is_none());
            assert!(provider.validate().is_ok());
        }
    }

    /// Ollama lists the cloud models a user pulled (`gpt-oss:120b-cloud`) beside local ones
    /// and forwards them to ollama.com. A loopback address does not make them local, so they
    /// never see more than public text (review of 2026-10-22).
    #[test]
    fn a_cloud_model_behind_a_local_server_is_not_local() {
        let mut provider = builtin_provider("ollama").unwrap();
        provider.models = vec![
            "llama3.2:3b".into(),
            "gpt-oss:120b-cloud".into(),
            "glm-4.6:cloud".into(),
        ];
        for model in provider.descriptors(&[]).unwrap() {
            let remote = model.labels["provider-model"].as_str();
            if remote == "llama3.2:3b" {
                assert_eq!(model.locality, Locality::Local);
                assert_eq!(model.max_privacy_grade, PrivacyGrade::Restricted);
            } else {
                assert_eq!(model.locality, Locality::Cloud, "{remote}");
                assert_eq!(model.max_privacy_grade, PrivacyGrade::Public, "{remote}");
                assert!(
                    crate::ProviderEndpoint::for_model(&model).is_ok_and(|e| !e.is_configured())
                        || crate::ProviderEndpoint::for_model(&model).is_err()
                );
            }
        }
    }

    #[test]
    fn models_are_discovered_or_configured_not_hardcoded() {
        for id in [
            "nebius",
            "cerebras",
            "openai",
            "anthropic",
            "openrouter",
            "ollama",
            "vllm",
        ] {
            assert!(builtin_provider(id).unwrap().models.is_empty());
        }
    }

    #[test]
    fn anthropic_cannot_receive_openai_wire_messages() {
        let provider = builtin_provider("anthropic").unwrap();
        assert!(provider.validate().is_ok());
        assert!(matches!(
            provider.openai_endpoint(),
            Err(ProviderConfigError::UnsupportedProtocol)
        ));
    }

    #[test]
    fn provider_config_round_trips_without_secret_values() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("providers.toml");
        let provider = builtin_provider("nebius").unwrap();
        save_provider(&path, provider).unwrap();
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(raw.contains("NEBIUS_TOKEN_FACTORY_KEY"));
        assert!(!raw.contains("Bearer "));
        assert_eq!(load_providers(&path).unwrap().len(), 1);
    }
}
