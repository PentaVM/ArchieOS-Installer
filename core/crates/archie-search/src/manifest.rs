//! What a search provider is allowed to be, written down before it is reached.
//!
//! The same bargain the module, provider and MCP registries make: a provider that cannot
//! be described here is one Archie will not query, and an operator can read the file and
//! see every place their node sends search terms.
//!
//! # Retention is part of the clearance
//!
//! A manifest's `max_privacy_grade` is only honoured if the provider's retention posture
//! supports it. A cloud provider with [`DataRetention::Unknown`] — the default, and the
//! honest value until someone checks the account — may be cleared for public queries and
//! nothing more. Declaring `internal` without `retention = "zero-verified"` is a
//! validation error, not a setting that quietly takes effect. Because every grade below
//! a validated ceiling is admitted by the same retention posture, the gate needs only
//! the ceiling.

use archie_types::{DataRetention, Locality, PrivacyGrade};
use archie_vault::VaultRef;
use reqwest::Url;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

/// The most results a single Tavily query may ask for.
pub const PROVIDER_MAX_RESULTS: u8 = 20;

const MAX_TIMEOUT_MS: u32 = 120_000;
const MAX_RESPONSE_BYTES: u32 = 8 * 1_048_576;

/// Which API a provider speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SearchProviderKind {
    /// Tavily's search API, `POST /search`.
    Tavily,
}

/// One search provider, as the operator declared it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SearchProviderManifest {
    pub id: String,
    pub display_name: String,
    pub kind: SearchProviderKind,
    /// Base URL. The client appends the operation path.
    pub endpoint: String,

    /// The NAME of the API key, never the key. Resolved through the vault's broker at
    /// the moment of sending.
    pub secret_handle: VaultRef,

    /// The most sensitive query this provider may receive. Defaults to public.
    #[serde(default = "public_grade")]
    pub max_privacy_grade: PrivacyGrade,

    /// What the provider does with queries. Unknown until verified for this account.
    #[serde(default)]
    pub retention: DataRetention,

    /// The most results one query may request, at most [`PROVIDER_MAX_RESULTS`].
    #[serde(default = "default_max_results")]
    pub max_results: u8,

    /// Whether `advanced` depth may be requested. It costs twice as much per query.
    #[serde(default)]
    pub allow_advanced_depth: bool,

    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u32,
    #[serde(default = "default_max_response_bytes")]
    pub max_response_bytes: u32,

    /// Registered but switched off is distinct from absent.
    #[serde(default)]
    pub enabled: bool,
}

fn public_grade() -> PrivacyGrade {
    PrivacyGrade::Public
}

fn default_max_results() -> u8 {
    10
}

fn default_timeout_ms() -> u32 {
    20_000
}

fn default_max_response_bytes() -> u32 {
    1_048_576
}

impl SearchProviderManifest {
    /// Tavily's public endpoint with the conservative defaults: public queries only,
    /// retention unknown, basic depth, and the provider's own result ceiling.
    pub fn tavily(secret_handle: VaultRef) -> Self {
        Self {
            id: "tavily".into(),
            display_name: "Tavily".into(),
            kind: SearchProviderKind::Tavily,
            endpoint: "https://api.tavily.com".into(),
            secret_handle,
            max_privacy_grade: PrivacyGrade::Public,
            retention: DataRetention::Unknown,
            max_results: PROVIDER_MAX_RESULTS,
            allow_advanced_depth: false,
            timeout_ms: default_timeout_ms(),
            max_response_bytes: default_max_response_bytes(),
            enabled: true,
        }
    }

    /// The endpoint, parsed and checked. The path always ends in `/`, so joining an
    /// operation name appends to it rather than replacing its last segment.
    pub fn endpoint_url(&self) -> Result<Url, ManifestError> {
        let mut url =
            Url::parse(&self.endpoint).map_err(|_| ManifestError::Endpoint("not a valid URL"))?;
        if url.cannot_be_a_base() || url.host_str().is_none() {
            return Err(ManifestError::Endpoint("must name a host"));
        }
        if !url.username().is_empty() || url.password().is_some() {
            // Credentials in a URL end up in logs, shell history and error messages.
            return Err(ManifestError::Endpoint("must not carry credentials"));
        }
        if url.query().is_some() || url.fragment().is_some() {
            return Err(ManifestError::Endpoint(
                "must not carry a query or fragment",
            ));
        }
        match url.scheme() {
            "https" => {}
            // Plaintext is acceptable only where it never reaches a wire.
            "http" if is_loopback(&url) => {}
            _ => {
                return Err(ManifestError::Endpoint(
                    "a non-loopback provider must be reached over https",
                ))
            }
        }
        if !url.path().ends_with('/') {
            let path = format!("{}/", url.path());
            url.set_path(&path);
        }
        Ok(url)
    }

    /// Where queries to this provider physically go.
    pub fn locality(&self) -> Locality {
        match Url::parse(&self.endpoint) {
            Ok(url) if is_loopback(&url) => Locality::Local,
            _ => Locality::Cloud,
        }
    }

    /// Whether a query graded `grade` may be sent here: the operator's ceiling, and the
    /// locality the grade permits.
    pub fn permits_grade(&self, grade: PrivacyGrade) -> bool {
        grade <= self.max_privacy_grade && grade.permits(self.locality())
    }

    /// Whether this provider's retention posture admits a query graded `grade`.
    ///
    /// Retention only matters when the query reaches a third party, and public queries
    /// may go anywhere. Everything else needs verified zero retention.
    pub fn retention_admits(&self, grade: PrivacyGrade) -> bool {
        grade == PrivacyGrade::Public
            || !self.locality().discloses_to_third_party()
            || self.retention.admits_private_disclosure()
    }

    pub fn validate(&self) -> Result<(), ManifestError> {
        if self.id.is_empty()
            || self.id.len() > 64
            || !self
                .id
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"-_.".contains(&b))
        {
            return Err(ManifestError::Id);
        }
        if self.display_name.trim().is_empty() || self.display_name.chars().count() > 128 {
            return Err(ManifestError::DisplayName);
        }
        self.endpoint_url()?;
        if self.max_privacy_grade == PrivacyGrade::Secret {
            return Err(ManifestError::SecretQueries);
        }
        if !self.max_privacy_grade.permits(self.locality()) {
            return Err(ManifestError::GradeExceedsLocality {
                grade: self.max_privacy_grade,
                locality: self.locality(),
            });
        }
        if !self.retention_admits(self.max_privacy_grade) {
            return Err(ManifestError::RetentionUnverified {
                grade: self.max_privacy_grade,
            });
        }
        if self.max_results == 0 || self.max_results > PROVIDER_MAX_RESULTS {
            return Err(ManifestError::MaxResults);
        }
        if self.timeout_ms == 0 || self.timeout_ms > MAX_TIMEOUT_MS {
            return Err(ManifestError::Timeout);
        }
        if self.max_response_bytes == 0 || self.max_response_bytes > MAX_RESPONSE_BYTES {
            return Err(ManifestError::ResponseBudget);
        }
        Ok(())
    }
}

fn is_loopback(url: &Url) -> bool {
    match url.host_str() {
        Some("localhost") => true,
        Some(host) => host
            .trim_start_matches('[')
            .trim_end_matches(']')
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback()),
        None => false,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ManifestError {
    #[error("provider id must be 1..=64 characters of [a-z0-9-_.]")]
    Id,
    #[error("display name must be 1..=128 characters")]
    DisplayName,
    #[error("invalid endpoint: {0}")]
    Endpoint(&'static str),
    #[error("no search provider may be cleared for secret queries")]
    SecretQueries,
    #[error("declared grade {grade} cannot be honoured at {locality:?}")]
    GradeExceedsLocality {
        grade: PrivacyGrade,
        locality: Locality,
    },
    #[error("declared grade {grade} needs retention = \"zero-verified\" for a third party")]
    RetentionUnverified { grade: PrivacyGrade },
    #[error("max_results must be 1..={PROVIDER_MAX_RESULTS}")]
    MaxResults,
    #[error("timeout must be 1..={MAX_TIMEOUT_MS} ms")]
    Timeout,
    #[error("response budget must be 1..={MAX_RESPONSE_BYTES} bytes")]
    ResponseBudget,
    #[error("duplicate provider id {0}")]
    Duplicate(String),
    #[error("manifest is not valid: {0}")]
    Parse(String),
    #[error("cannot read provider directory: {0}")]
    Directory(String),
}

/// Every search provider this node knows about.
#[derive(Debug, Default, Clone)]
pub struct SearchRegistry {
    providers: BTreeMap<String, SearchProviderManifest>,
}

impl SearchRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a provider. Only a manifest that validates is ever held, which is what
    /// lets the gate rely on it.
    pub fn register(&mut self, manifest: SearchProviderManifest) -> Result<(), ManifestError> {
        manifest.validate()?;
        if self.providers.contains_key(&manifest.id) {
            return Err(ManifestError::Duplicate(manifest.id));
        }
        self.providers.insert(manifest.id.clone(), manifest);
        Ok(())
    }

    pub fn get(&self, id: &str) -> Option<&SearchProviderManifest> {
        self.providers.get(id)
    }

    pub fn iter(&self) -> impl Iterator<Item = &SearchProviderManifest> {
        self.providers.values()
    }

    pub fn len(&self) -> usize {
        self.providers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.providers.is_empty()
    }

    /// Parse and validate one TOML manifest.
    pub fn parse_manifest(toml_text: &str) -> Result<SearchProviderManifest, ManifestError> {
        let manifest: SearchProviderManifest =
            toml::from_str(toml_text).map_err(|e| ManifestError::Parse(e.message().into()))?;
        manifest.validate()?;
        Ok(manifest)
    }

    /// Load every `*.toml` directly inside `dir`, in path order.
    ///
    /// A directory that does not exist is an empty registry: no providers configured is
    /// an honest state. A manifest that fails to parse or validate fails the load, since
    /// skipping it would present a typo as a missing feature.
    pub fn load_from_dir(dir: impl AsRef<Path>) -> Result<Self, ManifestError> {
        let dir = dir.as_ref();
        let entries = match std::fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::new()),
            Err(e) => return Err(ManifestError::Directory(format!("{}: {e}", dir.display()))),
        };
        let mut paths: Vec<_> = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.extension().and_then(|e| e.to_str()) == Some("toml"))
            .collect();
        paths.sort();

        let mut registry = Self::new();
        for path in paths {
            let text = std::fs::read_to_string(&path)
                .map_err(|e| ManifestError::Directory(format!("{}: {e}", path.display())))?;
            registry.register(Self::parse_manifest(&text)?)?;
        }
        Ok(registry)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tavily() -> SearchProviderManifest {
        SearchProviderManifest::tavily(VaultRef::parse("vault:tavily/main").unwrap())
    }

    #[test]
    fn the_default_tavily_manifest_is_valid_and_public_only() {
        let manifest = tavily();
        assert!(manifest.validate().is_ok());
        assert_eq!(manifest.locality(), Locality::Cloud);
        assert!(manifest.permits_grade(PrivacyGrade::Public));
        assert!(!manifest.permits_grade(PrivacyGrade::Internal));
    }

    #[test]
    fn the_endpoint_gains_a_trailing_slash_so_joins_append() {
        let mut manifest = tavily();
        manifest.endpoint = "http://127.0.0.1:8080/v1".into();
        let url = manifest.endpoint_url().unwrap();
        assert_eq!(url.join("search").unwrap().path(), "/v1/search");
    }

    #[test]
    fn a_remote_provider_must_use_https() {
        let mut manifest = tavily();
        manifest.endpoint = "http://api.tavily.com".into();
        assert_eq!(
            manifest.validate(),
            Err(ManifestError::Endpoint(
                "a non-loopback provider must be reached over https"
            ))
        );
        manifest.endpoint = "http://[::1]:9000".into();
        assert!(
            manifest.validate().is_ok(),
            "loopback plaintext never reaches a wire"
        );
        assert_eq!(manifest.locality(), Locality::Local);
    }

    #[test]
    fn a_hostname_that_merely_contains_localhost_is_not_loopback() {
        let mut manifest = tavily();
        manifest.endpoint = "http://localhost.evil.example".into();
        assert!(manifest.validate().is_err());
        manifest.endpoint = "https://localhost.evil.example".into();
        assert_eq!(manifest.locality(), Locality::Cloud);
    }

    #[test]
    fn an_endpoint_may_not_carry_credentials_queries_or_fragments() {
        for endpoint in [
            "https://user:pass@api.tavily.com",
            "https://api.tavily.com/?api_key=tvly-x",
            "https://api.tavily.com/#x",
        ] {
            let mut manifest = tavily();
            manifest.endpoint = endpoint.into();
            assert!(
                matches!(manifest.validate(), Err(ManifestError::Endpoint(_))),
                "{endpoint} should be refused"
            );
        }
    }

    #[test]
    fn an_unverified_cloud_provider_cannot_be_cleared_above_public() {
        let mut manifest = tavily();
        manifest.max_privacy_grade = PrivacyGrade::Internal;
        assert_eq!(
            manifest.validate(),
            Err(ManifestError::RetentionUnverified {
                grade: PrivacyGrade::Internal
            })
        );
        manifest.retention = DataRetention::ZeroVerified;
        assert!(manifest.validate().is_ok());
        assert!(manifest.permits_grade(PrivacyGrade::Internal));
        assert!(!manifest.permits_grade(PrivacyGrade::Sensitive));
    }

    #[test]
    fn no_provider_may_be_cleared_for_secret_or_beyond_its_locality() {
        let mut manifest = tavily();
        manifest.max_privacy_grade = PrivacyGrade::Secret;
        assert_eq!(manifest.validate(), Err(ManifestError::SecretQueries));
        manifest.max_privacy_grade = PrivacyGrade::Sensitive;
        manifest.retention = DataRetention::ZeroVerified;
        assert!(matches!(
            manifest.validate(),
            Err(ManifestError::GradeExceedsLocality { .. })
        ));
    }

    #[test]
    fn limits_are_bounded() {
        let mut manifest = tavily();
        manifest.max_results = 0;
        assert_eq!(manifest.validate(), Err(ManifestError::MaxResults));
        manifest.max_results = PROVIDER_MAX_RESULTS + 1;
        assert_eq!(manifest.validate(), Err(ManifestError::MaxResults));
        manifest.max_results = 5;
        manifest.timeout_ms = 0;
        assert_eq!(manifest.validate(), Err(ManifestError::Timeout));
        manifest.timeout_ms = 1_000;
        manifest.max_response_bytes = MAX_RESPONSE_BYTES + 1;
        assert_eq!(manifest.validate(), Err(ManifestError::ResponseBudget));
    }

    #[test]
    fn a_misspelt_field_is_an_error_rather_than_a_silent_default() {
        let text = r#"
id = "tavily"
display_name = "Tavily"
kind = "tavily"
endpoint = "https://api.tavily.com"
secret_handle = "vault:tavily/main"
enable = true
"#;
        assert!(matches!(
            SearchRegistry::parse_manifest(text),
            Err(ManifestError::Parse(_))
        ));
    }

    #[test]
    fn a_minimal_manifest_is_disabled_and_public_until_someone_says_otherwise() {
        let text = r#"
id = "tavily"
display_name = "Tavily"
kind = "tavily"
endpoint = "https://api.tavily.com"
secret_handle = "vault:tavily/main"
"#;
        let manifest = SearchRegistry::parse_manifest(text).unwrap();
        assert!(!manifest.enabled);
        assert_eq!(manifest.max_privacy_grade, PrivacyGrade::Public);
        assert_eq!(manifest.retention, DataRetention::Unknown);
        assert!(!manifest.allow_advanced_depth);
    }

    #[test]
    fn a_registry_refuses_duplicates_and_invalid_manifests() {
        let mut registry = SearchRegistry::new();
        registry.register(tavily()).unwrap();
        assert_eq!(
            registry.register(tavily()),
            Err(ManifestError::Duplicate("tavily".into()))
        );
        let mut broken = tavily();
        broken.id = "Tavily Search".into();
        assert_eq!(registry.register(broken), Err(ManifestError::Id));
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn a_missing_directory_is_an_empty_registry() {
        let registry =
            SearchRegistry::load_from_dir("definitely/not/a/providers/directory").unwrap();
        assert!(registry.is_empty());
    }
}
