use archie_model_runtime::{ProviderEndpoint, RuntimeError};
use archie_types::Locality;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
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
}

impl ProviderSpec {
    pub fn validate(&self) -> Result<(), ProviderConfigError> {
        let valid = |value: &str| {
            !value.is_empty()
                && value
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/'))
        };
        if !valid(&self.id) || self.models.iter().any(|model| !valid(model)) {
            return Err(ProviderConfigError::InvalidId);
        }
        ProviderEndpoint::openai_compatible(
            &self.base_url,
            self.locality,
            self.credential_env.clone(),
        )?;
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
