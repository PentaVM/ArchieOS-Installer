//! The provider registry.
//!
//! A provider is one upstream data source, described declaratively. Adding a feed means
//! writing a manifest; a feed that cannot be described here is a feed Archie will not
//! fetch. That is the same bargain the module system makes, for the same reason: an
//! operator has to be able to see, enable and disable every place their node reaches out
//! to, and code buried in a worker is not visible.
//!
//! # Two kinds of state, deliberately separate
//!
//! [`ProviderStatus`] answers *can this provider load on this node* — a question about
//! capabilities, credentials and configuration, answerable at boot without touching the
//! network. [`ProviderHealth`] answers *how is it doing right now* — rate limits,
//! errors, staleness. Conflating them makes an unconfigured provider look identical to a
//! broken one, and those need different responses from the operator.
//!
//! # On-disk format
//!
//! Manifests are TOML, matching the module system and hand-editable by an operator.
//! `protocols/provider.schema.json` describes the same data model in a
//! language-independent way for native workers and any external tooling; a schema
//! describes a shape, not a file syntax.

use archie_types::{Capability, CapabilitySet};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ProviderRegistryError {
    #[error("cannot read provider directory {path}: {source}")]
    Directory {
        path: String,
        #[source]
        source: std::io::Error,
    },

    #[error("provider manifest {path} is not valid TOML: {source}")]
    Parse {
        path: String,
        #[source]
        source: toml::de::Error,
    },

    #[error("duplicate provider id {0}")]
    Duplicate(String),
}

/// How a provider is reached, and under what terms.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Access {
    pub transport: Transport,
    pub authentication: Authentication,
    /// The NAME of the credential this provider needs, never the credential.
    ///
    /// Resolved at fetch time from the node's secret store. A manifest can therefore be
    /// committed, shared and reviewed without carrying anything sensitive.
    #[serde(default)]
    pub secret_handle: Option<String>,
    pub access_class: AccessClass,
    #[serde(default)]
    pub terms_url: Option<String>,
    #[serde(default)]
    pub attribution: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Transport {
    Https,
    Websocket,
    Mqtt,
    File,
    S3,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Authentication {
    None,
    ApiKey,
    Oauth2,
    Basic,
    ClientCertificate,
}

impl Authentication {
    pub fn needs_secret(&self) -> bool {
        !matches!(self, Self::None)
    }
}

/// What Archie may do with the records. Carried per provider because a single
/// organisation often mixes terms across its endpoints.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AccessClass {
    Public,
    AttributionRequired,
    ResearchOnly,
    Licensed,
    /// May be used internally but never rendered to a shared display or exported.
    Restricted,
}

impl AccessClass {
    /// Whether records from this provider may appear on the observatory, which is a
    /// shared surface.
    pub fn may_be_displayed(&self) -> bool {
        !matches!(self, Self::Restricted)
    }

    /// Whether an attribution string must be rendered wherever these records appear.
    pub fn requires_attribution(&self) -> bool {
        matches!(self, Self::AttributionRequired)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BackoffStrategy {
    Exponential,
    Linear,
    Fixed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Backoff {
    pub strategy: BackoffStrategy,
    pub initial_seconds: u32,
    pub max_seconds: u32,
    #[serde(default)]
    pub jitter: bool,
}

impl Default for Backoff {
    fn default() -> Self {
        Self {
            strategy: BackoffStrategy::Exponential,
            initial_seconds: 5,
            max_seconds: 900,
            // Jitter on by default: without it, every node in a fabric that lost a feed
            // retries in lockstep and arrives as a thundering herd.
            jitter: true,
        }
    }
}

impl Backoff {
    /// Delay before attempt `attempt` (0-based), before jitter.
    pub fn delay_seconds(&self, attempt: u32) -> u32 {
        let raw = match self.strategy {
            BackoffStrategy::Fixed => self.initial_seconds,
            BackoffStrategy::Linear => self
                .initial_seconds
                .saturating_mul(attempt.saturating_add(1)),
            BackoffStrategy::Exponential => self
                .initial_seconds
                .saturating_mul(2u32.saturating_pow(attempt.min(16))),
        };
        raw.min(self.max_seconds)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Quota {
    pub requests: u32,
    pub per_seconds: u32,
    #[serde(default)]
    pub burst: Option<u32>,
}

impl Quota {
    /// The fastest sustainable poll interval this quota permits.
    ///
    /// A quota is a hard limit, not a target: exceeding one is how an operator loses
    /// access to a feed they depend on.
    pub fn min_interval_seconds(&self) -> u32 {
        if self.requests == 0 {
            return u32::MAX;
        }
        self.per_seconds.div_ceil(self.requests).max(1)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cadence {
    pub poll_seconds: u32,
    /// Floor imposed by the provider's terms. Never polled faster, whatever the
    /// configuration asks for.
    #[serde(default)]
    pub min_poll_seconds: Option<u32>,
    #[serde(default)]
    pub quota: Option<Quota>,
    #[serde(default)]
    pub backoff: Backoff,
}

impl Cadence {
    /// The interval actually used, respecting the terms floor and the quota.
    ///
    /// Configuration can only ever make Archie *slower* than the provider permits.
    pub fn effective_poll_seconds(&self) -> u32 {
        let mut interval = self.poll_seconds.max(1);
        if let Some(floor) = self.min_poll_seconds {
            interval = interval.max(floor);
        }
        if let Some(quota) = &self.quota {
            interval = interval.max(quota.min_interval_seconds());
        }
        interval
    }

    /// Whether the configured cadence would have breached the provider's terms.
    ///
    /// Reported rather than silently corrected: an operator who wrote a too-fast poll
    /// should find out, even though the scheduler protects them from it.
    pub fn is_clamped(&self) -> bool {
        self.effective_poll_seconds() > self.poll_seconds.max(1)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum GeographicCoverage {
    Global,
    Regional,
    National,
    Local,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Coverage {
    #[serde(default)]
    pub geographic: Option<GeographicCoverage>,
    #[serde(default)]
    pub regions: Vec<String>,
    /// Documented blind spots, rendered alongside the layer.
    ///
    /// This is the field that stops an absence of data being read as an absence of
    /// events — "ADS-B is sparse over open ocean" is the difference between an empty
    /// ocean and a quiet one.
    #[serde(default)]
    pub known_gaps: Vec<String>,
    #[serde(default)]
    pub latency_seconds: Option<u32>,
}

/// The normalized record types a provider can yield.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordKind {
    Observation,
    Entity,
    Track,
    Event,
    Claim,
    Alert,
    Metric,
    TimeSeries,
    Vulnerability,
    CyberIndicator,
    NetworkPrefix,
    AutonomousSystem,
    MarketInstrument,
    PersonPublicProfile,
    Organization,
}

/// A declared upstream data source.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderManifest {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub homepage: Option<String>,
    /// The Archie capability this provider satisfies. A node without it will not load
    /// the provider, which is how an operator turns whole domains off.
    pub capability: Capability,
    pub access: Access,
    pub cadence: Cadence,
    #[serde(default)]
    pub coverage: Coverage,
    pub records: Vec<RecordKind>,
    #[serde(default)]
    pub notes: Option<String>,
}

impl ProviderManifest {
    /// Static validation: everything checkable without a node or a network.
    pub fn validate(&self) -> Result<(), String> {
        if self.id.is_empty() {
            return Err("id must not be empty".into());
        }
        if !self.id.split('.').all(|segment| {
            !segment.is_empty()
                && segment.starts_with(|c: char| c.is_ascii_lowercase())
                && segment
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
        }) {
            return Err(format!("id {:?} must be a dotted lowercase name", self.id));
        }
        if self.title.trim().is_empty() {
            return Err("title must not be empty".into());
        }
        if self.records.is_empty() {
            return Err("a provider must declare at least one record kind".into());
        }
        if self.cadence.poll_seconds == 0 {
            return Err("poll_seconds must be at least 1".into());
        }

        // The rule that keeps credentials out of manifests: a provider that
        // authenticates must NAME the secret it needs. Without this a manifest can
        // depend on an ambient credential nobody documented.
        if self.access.authentication.needs_secret() && self.access.secret_handle.is_none() {
            return Err(format!(
                "provider {} authenticates with {:?} but names no secret_handle",
                self.id, self.access.authentication
            ));
        }
        if !self.access.authentication.needs_secret() && self.access.secret_handle.is_some() {
            return Err(format!(
                "provider {} needs no authentication but names a secret_handle",
                self.id
            ));
        }

        if self.access.access_class.requires_attribution() && self.access.attribution.is_none() {
            return Err(format!(
                "provider {} requires attribution but supplies no attribution string",
                self.id
            ));
        }

        if let Some(backoff) = Some(&self.cadence.backoff) {
            if backoff.max_seconds < backoff.initial_seconds {
                return Err("backoff max_seconds must not be below initial_seconds".into());
            }
        }
        Ok(())
    }
}

/// Whether a provider can load on this node, and why not when it cannot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ProviderStatus {
    /// Loadable. Says nothing about whether it is currently answering.
    Available,
    /// The node does not advertise the capability this provider satisfies.
    CapabilityUnavailable { missing: Capability },
    /// The capability family needs an explicit human grant that was not given.
    NotAuthorized { family: String },
    /// The named secret is not present in the node's secret store. The ordinary state of
    /// an optional provider on a node whose operator has not configured it — not an error.
    NoCredentials { secret_handle: String },
    /// Turned off by configuration.
    Disabled,
    /// The manifest itself is wrong.
    Invalid { reason: String },
}

impl ProviderStatus {
    pub fn is_available(&self) -> bool {
        matches!(self, Self::Available)
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Available => "available",
            Self::CapabilityUnavailable { .. } => "no-capability",
            Self::NotAuthorized { .. } => "not-authorized",
            Self::NoCredentials { .. } => "no-credentials",
            Self::Disabled => "disabled",
            Self::Invalid { .. } => "invalid",
        }
    }

    pub fn explain(&self) -> String {
        match self {
            Self::Available => "available".into(),
            Self::CapabilityUnavailable { missing } => {
                format!("node does not provide {missing}")
            }
            Self::NotAuthorized { family } => {
                format!("the {family} capability family requires an explicit operator grant")
            }
            Self::NoCredentials { secret_handle } => {
                format!("secret {secret_handle:?} is not configured")
            }
            Self::Disabled => "disabled by configuration".into(),
            Self::Invalid { reason } => format!("invalid manifest: {reason}"),
        }
    }
}

/// Runtime health, distinct from whether the provider could load at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderHealth {
    Ok,
    RateLimited,
    Degraded,
    Failed,
    /// Loaded but not yet polled.
    Pending,
}

impl ProviderHealth {
    /// Whether records from this provider should be treated as current.
    pub fn data_is_current(&self) -> bool {
        matches!(self, Self::Ok)
    }
}

/// What the node needs in order to decide which providers can run.
#[derive(Debug, Clone)]
pub struct ProviderContext {
    pub capabilities: CapabilitySet,
    /// Secret handles the node's store actually holds. Names only; no values ever reach
    /// this type.
    pub available_secrets: BTreeSet<String>,
    /// Capability families the operator has explicitly granted.
    pub authorized_families: BTreeSet<String>,
    pub disabled: BTreeSet<String>,
}

impl ProviderContext {
    pub fn new(capabilities: CapabilitySet) -> Self {
        Self {
            capabilities,
            available_secrets: BTreeSet::new(),
            authorized_families: BTreeSet::new(),
            disabled: BTreeSet::new(),
        }
    }

    pub fn with_secret(mut self, handle: impl Into<String>) -> Self {
        self.available_secrets.insert(handle.into());
        self
    }

    pub fn authorize_family(mut self, family: impl Into<String>) -> Self {
        self.authorized_families.insert(family.into());
        self
    }

    pub fn disable(mut self, id: impl Into<String>) -> Self {
        self.disabled.insert(id.into());
        self
    }
}

/// The resolved plan: what will run, and an explanation for everything that will not.
#[derive(Debug, Clone, PartialEq)]
pub struct ProviderPlan {
    pub statuses: BTreeMap<String, ProviderStatus>,
    /// Ids that will actually poll, in id order.
    pub active: Vec<String>,
}

impl ProviderPlan {
    pub fn is_available(&self, id: &str) -> bool {
        self.statuses
            .get(id)
            .is_some_and(ProviderStatus::is_available)
    }

    /// Everything that will not run, each with a reason.
    pub fn excluded(&self) -> Vec<(&String, String)> {
        self.statuses
            .iter()
            .filter(|(_, status)| !status.is_available())
            .map(|(id, status)| (id, status.explain()))
            .collect()
    }
}

/// Installed provider manifests.
#[derive(Debug, Clone, Default)]
pub struct ProviderRegistry {
    manifests: BTreeMap<String, ProviderManifest>,
}

impl ProviderRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, manifest: ProviderManifest) -> Result<(), ProviderRegistryError> {
        if self.manifests.contains_key(&manifest.id) {
            return Err(ProviderRegistryError::Duplicate(manifest.id.clone()));
        }
        self.manifests.insert(manifest.id.clone(), manifest);
        Ok(())
    }

    /// Load every `*.toml` manifest under `dir`, descending into family directories.
    ///
    /// A missing directory yields an empty registry: a node with no providers is a valid
    /// offline node. A malformed manifest is an error, because a typo that silently
    /// disabled a feed would present as an absence of world events.
    pub fn load_from_dir(dir: impl AsRef<Path>) -> Result<Self, ProviderRegistryError> {
        let dir = dir.as_ref();
        let mut registry = Self::new();
        if !dir.exists() {
            return Ok(registry);
        }

        let mut paths = Vec::new();
        collect_toml(dir, &mut paths, 0)?;
        paths.sort();

        for path in paths {
            let text = std::fs::read_to_string(&path).map_err(|source| {
                ProviderRegistryError::Directory {
                    path: path.display().to_string(),
                    source,
                }
            })?;
            let manifest: ProviderManifest =
                toml::from_str(&text).map_err(|source| ProviderRegistryError::Parse {
                    path: path.display().to_string(),
                    source,
                })?;
            registry.register(manifest)?;
        }
        Ok(registry)
    }

    pub fn get(&self, id: &str) -> Option<&ProviderManifest> {
        self.manifests.get(id)
    }

    pub fn manifests(&self) -> impl Iterator<Item = &ProviderManifest> {
        self.manifests.values()
    }

    pub fn len(&self) -> usize {
        self.manifests.len()
    }

    pub fn is_empty(&self) -> bool {
        self.manifests.is_empty()
    }

    /// Providers satisfying a capability.
    pub fn providing(&self, capability: &Capability) -> Vec<&ProviderManifest> {
        self.manifests
            .values()
            .filter(|m| m.capability.satisfies(capability))
            .collect()
    }

    /// Decide which providers can run on this node.
    ///
    /// Checks run cheapest-first and most-decisive-first: an invalid manifest is invalid
    /// everywhere, a missing capability cannot be fixed with a credential, and an
    /// ungranted family must not be reported as merely missing a secret.
    pub fn resolve(&self, context: &ProviderContext) -> ProviderPlan {
        let mut statuses = BTreeMap::new();

        for manifest in self.manifests.values() {
            let status = self.evaluate(manifest, context);
            statuses.insert(manifest.id.clone(), status);
        }

        let active = statuses
            .iter()
            .filter(|(_, status)| status.is_available())
            .map(|(id, _)| id.clone())
            .collect();

        ProviderPlan { statuses, active }
    }

    fn evaluate(&self, manifest: &ProviderManifest, context: &ProviderContext) -> ProviderStatus {
        if let Err(reason) = manifest.validate() {
            return ProviderStatus::Invalid { reason };
        }
        if context.disabled.contains(&manifest.id) {
            return ProviderStatus::Disabled;
        }
        if !context.capabilities.provides(&manifest.capability) {
            return ProviderStatus::CapabilityUnavailable {
                missing: manifest.capability.clone(),
            };
        }

        let family = manifest.capability.family();
        if archie_types::capability::family_requires_authorization(family)
            && !context.authorized_families.contains(family)
        {
            return ProviderStatus::NotAuthorized {
                family: family.to_string(),
            };
        }

        if let Some(handle) = &manifest.access.secret_handle {
            if !context.available_secrets.contains(handle) {
                return ProviderStatus::NoCredentials {
                    secret_handle: handle.clone(),
                };
            }
        }

        ProviderStatus::Available
    }
}

fn collect_toml(
    dir: &Path,
    out: &mut Vec<std::path::PathBuf>,
    depth: usize,
) -> Result<(), ProviderRegistryError> {
    const MAX_DEPTH: usize = 8;
    if depth > MAX_DEPTH {
        return Ok(());
    }
    let entries = std::fs::read_dir(dir).map_err(|source| ProviderRegistryError::Directory {
        path: dir.display().to_string(),
        source,
    })?;
    for entry in entries {
        let entry = entry.map_err(|source| ProviderRegistryError::Directory {
            path: dir.display().to_string(),
            source,
        })?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|source| ProviderRegistryError::Directory {
                path: path.display().to_string(),
                source,
            })?;
        if file_type.is_dir() {
            collect_toml(&path, out, depth + 1)?;
        } else if path.extension().and_then(|e| e.to_str()) == Some("toml") {
            out.push(path);
        }
    }
    Ok(())
}
