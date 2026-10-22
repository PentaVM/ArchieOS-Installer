use crate::{types::EventSource, WandaError};
use archie_vault::ContextDomain;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DomainBinding {
    pub domain: ContextDomain,
    pub graph_path: PathBuf,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceBinding {
    pub device_id: String,
    pub domain: ContextDomain,
    pub source: EventSource,
    pub token_file: Option<PathBuf>,
    pub token_env: Option<String>,
    #[serde(default)]
    pub allow_presumed_owner: bool,
    #[serde(default = "enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub chat_domains: BTreeMap<String, ContextDomain>,
}
fn enabled() -> bool {
    true
}
fn quality() -> u8 {
    50
}
fn deep_quality() -> u8 {
    80
}
fn retention() -> u16 {
    168
}
fn cpu() -> String {
    "cpu".into()
}
fn half() -> f32 {
    0.5
}
fn owner_label() -> String {
    "Yo".into()
}

/// Wanda's local expert (LNCM). Optional: without it Wanda works as before.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LncmBinding {
    /// A checkpoint directory written by `archie-wanda-lncm`.
    pub dir: PathBuf,
    /// `cpu`, or `cuda:N` when built with CUDA.
    #[serde(default = "cpu")]
    pub device: String,
    #[serde(default)]
    pub lang: Option<archie_wanda_lncm::Lang>,
    /// Adapter per context domain; a domain without one uses the shared weights.
    #[serde(default)]
    pub adapters: BTreeMap<ContextDomain, String>,
    /// Below this probability a question is not treated as social.
    #[serde(default = "half")]
    pub gate_threshold: f32,
    /// Below this probability a chunk of conversation is not interpreted.
    #[serde(default = "half")]
    pub update_threshold: f32,
    /// How the owner is named in the state the expert reads.
    #[serde(default = "owner_label")]
    pub owner_label: String,
    /// Long answers drawn per question, in one batch; one that passes the checks against the
    /// state is kept. More cost more on a CPU, little on a GPU.
    #[serde(default = "one")]
    pub compose_candidates: usize,
}
fn one() -> usize {
    1
}
impl std::fmt::Debug for LncmBinding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("LncmBinding(redacted)")
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WandaConfig {
    pub state_dir: PathBuf,
    pub graph_binary: PathBuf,
    pub models_dir: PathBuf,
    pub operator_token_file: PathBuf,
    pub domains: Vec<DomainBinding>,
    pub sources: Vec<SourceBinding>,
    #[serde(default = "quality")]
    pub quality_tier: u8,
    #[serde(default = "deep_quality")]
    pub deep_quality_tier: u8,
    #[serde(default = "retention")]
    pub retention_hours: u16,
    #[serde(default)]
    pub lncm: Option<LncmBinding>,
    /// Queries answered at once (1..=8). Match the local model server's slots
    /// (llama-server `--parallel`); further queries wait their turn for a few seconds.
    #[serde(default = "parallel_queries")]
    pub parallel_queries: u8,
}
fn parallel_queries() -> u8 {
    1
}
impl std::fmt::Debug for WandaConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("WandaConfig(redacted)")
    }
}
impl std::fmt::Debug for SourceBinding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SourceBinding(redacted)")
    }
}
impl std::fmt::Debug for DomainBinding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DomainBinding(redacted)")
    }
}
impl WandaConfig {
    pub fn load(path: &Path) -> Result<Self, WandaError> {
        use std::io::Read;
        let file = std::fs::File::open(path)
            .map_err(|_| WandaError::Config("cannot read configuration".into()))?;
        let mut bytes = Vec::new();
        file.take(256 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| WandaError::Storage)?;
        if bytes.len() > 256 * 1024 {
            return Err(WandaError::TooLarge);
        }
        let mut config: Self = toml::from_str(
            std::str::from_utf8(&bytes).map_err(|_| WandaError::Config("invalid UTF-8".into()))?,
        )
        .map_err(|_| WandaError::Config("invalid configuration schema".into()))?;
        let absolute = std::path::absolute(path).map_err(|_| WandaError::Storage)?;
        let base = absolute.parent().ok_or(WandaError::Storage)?;
        fn resolve(base: &Path, path: &mut PathBuf) {
            if path.is_relative() {
                *path = base.join(&*path);
            }
        }
        for path in [
            &mut config.state_dir,
            &mut config.graph_binary,
            &mut config.models_dir,
            &mut config.operator_token_file,
        ] {
            resolve(base, path);
        }
        for binding in &mut config.domains {
            resolve(base, &mut binding.graph_path);
        }
        for source in &mut config.sources {
            if let Some(path) = &mut source.token_file {
                resolve(base, path);
            }
        }
        if let Some(lncm) = &mut config.lncm {
            resolve(base, &mut lncm.dir);
        }
        config.validate()?;
        Ok(config)
    }
    pub fn validate(&self) -> Result<(), WandaError> {
        use std::collections::BTreeSet;
        let invalid =
            || WandaError::Config("inconsistent domain, path, source or resource limits".into());
        if self.domains.is_empty()
            || self.domains.len() > 32
            || self.sources.len() > 128
            || self.quality_tier > 100
            || self.deep_quality_tier > 100
            || self.deep_quality_tier < self.quality_tier
            || !(1..=720).contains(&self.retention_hours)
            || !(1..=8).contains(&self.parallel_queries)
        {
            return Err(invalid());
        }
        let usable = |p: &Path| {
            p.is_absolute()
                && !p
                    .components()
                    .any(|c| matches!(c, std::path::Component::ParentDir))
        };
        if [
            &self.state_dir,
            &self.graph_binary,
            &self.models_dir,
            &self.operator_token_file,
        ]
        .iter()
        .any(|p| !usable(p))
        {
            return Err(invalid());
        }
        let mut domains = BTreeSet::new();
        let mut graphs = BTreeSet::new();
        let mut devices = BTreeSet::new();
        for binding in &self.domains {
            if !usable(&binding.graph_path) || !domains.insert(binding.domain.clone()) {
                return Err(invalid());
            }
            let path = std::fs::canonicalize(&binding.graph_path)
                .unwrap_or_else(|_| binding.graph_path.clone());
            let key = path.to_string_lossy().replace('\\', "/");
            #[cfg(windows)]
            let key = key.to_lowercase();
            if !graphs.insert(key) {
                return Err(invalid());
            }
        }
        if let Some(lncm) = &self.lncm {
            let device_ok = lncm.device == "cpu"
                || lncm
                    .device
                    .strip_prefix("cuda:")
                    .is_some_and(|n| n.parse::<usize>().is_ok());
            if !usable(&lncm.dir)
                || !device_ok
                || !(0.0..=1.0).contains(&lncm.gate_threshold)
                || !(0.0..=1.0).contains(&lncm.update_threshold)
                || !bounded_id(&lncm.owner_label)
                || lncm.owner_label.len() > 128
                || lncm.adapters.iter().any(|(domain, name)| {
                    !domains.contains(domain) || !archie_wanda_lncm::model::adapter_name_valid(name)
                })
            {
                return Err(invalid());
            }
        }
        for source in &self.sources {
            if !bounded_id(&source.device_id)
                || !domains.contains(&source.domain)
                || !devices.insert((source.device_id.clone(), format!("{:?}", source.source)))
                || source.token_file.is_some() == source.token_env.is_some()
                || source.token_file.as_ref().is_some_and(|p| !usable(p))
                || source.token_env.as_ref().is_some_and(|s| {
                    s.is_empty()
                        || s.len() > 128
                        || !s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
                })
                || source.chat_domains.len() > 1024
                || source.chat_domains.iter().any(|(chat, domain)| {
                    crate::chat::routing_key(chat).is_none() || !domains.contains(domain)
                })
            {
                return Err(invalid());
            }
        }
        Ok(())
    }
}
pub(crate) fn bounded_id(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= 192 && !value.chars().any(char::is_control)
}
