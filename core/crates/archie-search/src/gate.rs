//! The admission gate every query passes through.
//!
//! # Why this is a type and not a check
//!
//! [`AuthorizedSearch`] has private fields and no public constructor. The only way to
//! obtain one is [`SearchGate::authorize`], and [`crate::TavilyClient::search`] takes an
//! `AuthorizedSearch` rather than a string. A caller holding a model's suggested query
//! has a [`SearchProposal`], and the only thing it can do with it is submit it here.
//!
//! # What is graded
//!
//! Everything the provider will receive: the query text and the domain filters. The
//! caller grades them — normally from the context capsule the words came out of — and
//! the gate holds that grade against the provider's validated clearance. A refusal never
//! repeats the query or a domain, because a refusal is exactly what ends up in a log.

use crate::manifest::{SearchProviderManifest, SearchRegistry, PROVIDER_MAX_RESULTS};
use archie_types::capability::well_known;
use archie_types::{Capability, CapabilitySet, PrivacyGrade};
use archie_vault::VaultRef;
use reqwest::Url;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// The longest query admitted, in characters. Tavily's own guidance is to stay under it,
/// and a query that long is usually a prompt pasted where a search belongs.
pub const MAX_QUERY_CHARS: usize = 1_500;

/// Include and exclude filters together. Far below what Tavily accepts: every domain is
/// disclosed alongside the query, and a long list describes whoever wrote it.
pub const MAX_DOMAIN_FILTERS: usize = 32;

const MAX_DOMAIN_CHARS: usize = 253;

/// How hard the provider searches. `advanced` costs two credits, the rest one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SearchDepth {
    UltraFast,
    Fast,
    #[default]
    Basic,
    Advanced,
}

impl SearchDepth {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::UltraFast => "ultra-fast",
            Self::Fast => "fast",
            Self::Basic => "basic",
            Self::Advanced => "advanced",
        }
    }

    /// Tavily credits one query at this depth costs.
    pub fn credits(&self) -> u8 {
        match self {
            Self::Advanced => 2,
            Self::UltraFast | Self::Fast | Self::Basic => 1,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SearchTopic {
    #[default]
    General,
    News,
    Finance,
}

impl SearchTopic {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::General => "general",
            Self::News => "news",
            Self::Finance => "finance",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TimeRange {
    Day,
    Week,
    Month,
    Year,
}

impl TimeRange {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Day => "day",
            Self::Week => "week",
            Self::Month => "month",
            Self::Year => "year",
        }
    }
}

/// What a query asks the provider to do, beyond the words.
///
/// The domain filters are graded with the query, so `Debug` counts them.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchOptions {
    pub depth: SearchDepth,
    pub topic: SearchTopic,
    pub max_results: u8,
    pub time_range: Option<TimeRange>,
    pub include_domains: Vec<String>,
    pub exclude_domains: Vec<String>,
}

impl std::fmt::Debug for SearchOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SearchOptions")
            .field("depth", &self.depth)
            .field("topic", &self.topic)
            .field("max_results", &self.max_results)
            .field("time_range", &self.time_range)
            .field("include_domains", &self.include_domains.len())
            .field("exclude_domains", &self.exclude_domains.len())
            .finish()
    }
}

impl Default for SearchOptions {
    fn default() -> Self {
        Self {
            depth: SearchDepth::Basic,
            topic: SearchTopic::General,
            max_results: 5,
            time_range: None,
            include_domains: Vec::new(),
            exclude_domains: Vec::new(),
        }
    }
}

/// A search somebody — usually a model — has proposed. Carries no authority.
///
/// Its `Debug` output measures the query and counts the filters instead of printing
/// them: a proposal is exactly the value that ends up in a log line when it is refused.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchProposal {
    pub provider_id: String,
    pub query: String,
    pub options: SearchOptions,
    /// The sensitivity of the query and its filters, as assessed by the caller that
    /// assembled them.
    pub query_grade: PrivacyGrade,
    pub task_id: String,
    pub trace_id: String,
}

impl SearchProposal {
    pub fn new(
        provider_id: impl Into<String>,
        query: impl Into<String>,
        query_grade: PrivacyGrade,
        task_id: impl Into<String>,
        trace_id: impl Into<String>,
    ) -> Self {
        Self {
            provider_id: provider_id.into(),
            query: query.into(),
            options: SearchOptions::default(),
            query_grade,
            task_id: task_id.into(),
            trace_id: trace_id.into(),
        }
    }

    pub fn with_options(mut self, options: SearchOptions) -> Self {
        self.options = options;
        self
    }
}

impl std::fmt::Debug for SearchProposal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SearchProposal")
            .field("provider_id", &self.provider_id)
            .field("query_chars", &self.query.chars().count())
            .field("options", &self.options)
            .field("query_grade", &self.query_grade)
            .field("task_id", &self.task_id)
            .field("trace_id", &self.trace_id)
            .finish()
    }
}

/// What the caller has been granted, and which providers need a person's say-so.
#[derive(Debug, Clone, Default)]
pub struct SearchAuthority {
    granted: CapabilitySet,
    requires_approval: BTreeSet<String>,
}

impl SearchAuthority {
    pub fn new(granted: CapabilitySet) -> Self {
        Self {
            granted,
            requires_approval: BTreeSet::new(),
        }
    }

    /// Mark a provider as one a person must approve each query to.
    pub fn requiring_approval(mut self, provider_id: &str) -> Self {
        self.requires_approval.insert(provider_id.to_string());
        self
    }

    pub fn grants(&self, capability: &Capability) -> bool {
        self.granted.provides(capability)
    }

    fn needs_approval(&self, provider_id: &str) -> bool {
        self.requires_approval.contains(provider_id)
    }
}

/// A search that has passed every gate. Constructible only by [`SearchGate::authorize`].
///
/// Like the proposal, its `Debug` output never contains the query or the filters.
#[derive(Clone, PartialEq)]
pub struct AuthorizedSearch {
    provider_id: String,
    endpoint: Url,
    query: String,
    options: SearchOptions,
    query_grade: PrivacyGrade,
    credential_ref: VaultRef,
    timeout_ms: u32,
    max_response_bytes: u32,
    task_id: String,
    trace_id: String,
}

impl std::fmt::Debug for AuthorizedSearch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuthorizedSearch")
            .field("provider_id", &self.provider_id)
            .field("endpoint", &self.endpoint.as_str())
            .field("query_chars", &self.query.chars().count())
            .field("options", &self.options)
            .field("query_grade", &self.query_grade)
            .field("credential_ref", &self.credential_ref)
            .field("task_id", &self.task_id)
            .field("trace_id", &self.trace_id)
            .finish_non_exhaustive()
    }
}

impl AuthorizedSearch {
    pub fn provider_id(&self) -> &str {
        &self.provider_id
    }

    pub fn endpoint(&self) -> &Url {
        &self.endpoint
    }

    /// The query as admitted: trimmed, and checked for length and control characters.
    pub fn query(&self) -> &str {
        &self.query
    }

    /// The options as admitted. Domain filters are normalised to lowercase.
    pub fn options(&self) -> &SearchOptions {
        &self.options
    }

    pub fn query_grade(&self) -> PrivacyGrade {
        self.query_grade
    }

    /// The *name* of the key the client must be handed. Never the key.
    pub fn credential_ref(&self) -> &VaultRef {
        &self.credential_ref
    }

    pub fn timeout_ms(&self) -> u32 {
        self.timeout_ms
    }

    pub fn max_response_bytes(&self) -> u32 {
        self.max_response_bytes
    }

    pub fn task_id(&self) -> &str {
        &self.task_id
    }

    pub fn trace_id(&self) -> &str {
        &self.trace_id
    }

    /// The record of this search, for the trace. The query is measured, not included.
    pub fn receipt(&self) -> SearchReceipt {
        SearchReceipt {
            provider_id: self.provider_id.clone(),
            task_id: self.task_id.clone(),
            trace_id: self.trace_id.clone(),
            query_chars: self.query.chars().count(),
            domain_filters: self.options.include_domains.len() + self.options.exclude_domains.len(),
            depth: self.options.depth,
            topic: self.options.topic,
            max_results: self.options.max_results,
            grade: self.query_grade,
            credits: self.options.depth.credits(),
            brokered_credential: self.credential_ref.clone(),
        }
    }
}

/// What may be written down about a search.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchReceipt {
    pub provider_id: String,
    pub task_id: String,
    pub trace_id: String,
    /// How long the query was, not what it said.
    pub query_chars: usize,
    pub domain_filters: usize,
    pub depth: SearchDepth,
    pub topic: SearchTopic,
    pub max_results: u8,
    pub grade: PrivacyGrade,
    /// Credits the provider charges for this depth.
    pub credits: u8,
    pub brokered_credential: VaultRef,
}

/// The gate's verdict.
#[derive(Debug, Clone, PartialEq)]
pub enum SearchAdmission {
    Allowed(Box<AuthorizedSearch>),
    /// Legitimate, and the operator wants to be asked first.
    NeedsApproval {
        search: Box<AuthorizedSearch>,
        reason: &'static str,
    },
    Refused(SearchRefusal),
}

impl SearchAdmission {
    pub fn allowed(&self) -> Option<&AuthorizedSearch> {
        match self {
            Self::Allowed(search) => Some(search),
            _ => None,
        }
    }

    pub fn refusal(&self) -> Option<&SearchRefusal> {
        match self {
            Self::Refused(refusal) => Some(refusal),
            _ => None,
        }
    }
}

/// Why a search did not pass. None of these repeat the query or a domain.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SearchRefusal {
    #[error("no search provider registered as {0}")]
    UnknownProvider(String),
    #[error("search provider {0} is registered but disabled")]
    ProviderDisabled(String),
    #[error("caller lacks the capability {0} a web search requires")]
    CapabilityNotGranted(Capability),
    #[error("a query may never be graded secret")]
    SecretQuery,
    #[error("a query graded {grade} exceeds what {provider_id} may receive")]
    PrivacyExceeded {
        provider_id: String,
        grade: PrivacyGrade,
    },
    #[error("the query is empty")]
    EmptyQuery,
    #[error("the query is {chars} characters, over the {MAX_QUERY_CHARS} permitted")]
    QueryTooLong { chars: usize },
    #[error("the query contains control characters")]
    QueryNotPlainText,
    #[error("{requested} results requested; this provider permits 1..={ceiling}")]
    ResultCount { requested: u8, ceiling: u8 },
    #[error("advanced depth is not enabled for {0}")]
    DepthNotAllowed(String),
    #[error("{count} domain filters given, over the {MAX_DOMAIN_FILTERS} permitted")]
    TooManyDomainFilters { count: usize },
    #[error("a domain filter is not a hostname")]
    InvalidDomain,
    #[error("a domain is both included and excluded")]
    DomainInBothLists,
    #[error("search provider {0} has an unusable endpoint")]
    ProviderMisconfigured(String),
}

impl SearchRefusal {
    /// A stable code for traces. Never a sentence.
    pub fn code(&self) -> &'static str {
        match self {
            Self::UnknownProvider(_) => "search_provider_unknown",
            Self::ProviderDisabled(_) => "search_provider_disabled",
            Self::CapabilityNotGranted(_) => "capability_not_granted",
            Self::SecretQuery => "secret_in_query",
            Self::PrivacyExceeded { .. } => "query_exceeds_provider_grade",
            Self::EmptyQuery => "query_empty",
            Self::QueryTooLong { .. } => "query_too_long",
            Self::QueryNotPlainText => "query_not_plain_text",
            Self::ResultCount { .. } => "result_count_out_of_range",
            Self::DepthNotAllowed(_) => "search_depth_not_allowed",
            Self::TooManyDomainFilters { .. } => "too_many_domain_filters",
            Self::InvalidDomain => "domain_filter_invalid",
            Self::DomainInBothLists => "domain_filter_contradictory",
            Self::ProviderMisconfigured(_) => "search_provider_misconfigured",
        }
    }
}

/// Admits searches, or does not.
#[derive(Debug, Default, Clone)]
pub struct SearchGate;

impl SearchGate {
    /// The only way to obtain an [`AuthorizedSearch`].
    ///
    /// Ordered so the most categorical refusals come first, and so that a refusal names
    /// the first rule that bound.
    pub fn authorize(
        registry: &SearchRegistry,
        authority: &SearchAuthority,
        proposal: &SearchProposal,
    ) -> SearchAdmission {
        let refuse = SearchAdmission::Refused;
        let Some(provider) = registry.get(&proposal.provider_id) else {
            return refuse(SearchRefusal::UnknownProvider(proposal.provider_id.clone()));
        };
        if !provider.enabled {
            return refuse(SearchRefusal::ProviderDisabled(provider.id.clone()));
        }
        // Every search reaches outside the machine, so the grant is structural rather
        // than something a manifest could leave out.
        let internet = well_known::network_internet();
        if !authority.grants(&internet) {
            return refuse(SearchRefusal::CapabilityNotGranted(internet));
        }
        if proposal.query_grade == PrivacyGrade::Secret {
            return refuse(SearchRefusal::SecretQuery);
        }
        // The registry holds only validated manifests, and validation already required
        // the retention posture to admit every grade up to this ceiling.
        if !provider.permits_grade(proposal.query_grade) {
            return refuse(SearchRefusal::PrivacyExceeded {
                provider_id: provider.id.clone(),
                grade: proposal.query_grade,
            });
        }

        let query = proposal.query.trim();
        if query.is_empty() {
            return refuse(SearchRefusal::EmptyQuery);
        }
        let chars = query.chars().count();
        if chars > MAX_QUERY_CHARS {
            return refuse(SearchRefusal::QueryTooLong { chars });
        }
        if query.chars().any(char::is_control) {
            return refuse(SearchRefusal::QueryNotPlainText);
        }

        let options = &proposal.options;
        let ceiling = provider.max_results.min(PROVIDER_MAX_RESULTS);
        if options.max_results == 0 || options.max_results > ceiling {
            return refuse(SearchRefusal::ResultCount {
                requested: options.max_results,
                ceiling,
            });
        }
        if options.depth == SearchDepth::Advanced && !provider.allow_advanced_depth {
            return refuse(SearchRefusal::DepthNotAllowed(provider.id.clone()));
        }

        let count = options.include_domains.len() + options.exclude_domains.len();
        if count > MAX_DOMAIN_FILTERS {
            return refuse(SearchRefusal::TooManyDomainFilters { count });
        }
        let Some(include_domains) = normalise_domains(&options.include_domains) else {
            return refuse(SearchRefusal::InvalidDomain);
        };
        let Some(exclude_domains) = normalise_domains(&options.exclude_domains) else {
            return refuse(SearchRefusal::InvalidDomain);
        };
        if include_domains.iter().any(|d| exclude_domains.contains(d)) {
            return refuse(SearchRefusal::DomainInBothLists);
        }

        let Some(search) = admitted(provider, proposal, query, include_domains, exclude_domains)
        else {
            return refuse(SearchRefusal::ProviderMisconfigured(provider.id.clone()));
        };
        if authority.needs_approval(&provider.id) {
            return SearchAdmission::NeedsApproval {
                search: Box::new(search),
                reason: "operator marked this provider as requiring approval",
            };
        }
        SearchAdmission::Allowed(Box::new(search))
    }
}

fn admitted(
    provider: &SearchProviderManifest,
    proposal: &SearchProposal,
    query: &str,
    include_domains: Vec<String>,
    exclude_domains: Vec<String>,
) -> Option<AuthorizedSearch> {
    let endpoint = provider.endpoint_url().ok()?;
    Some(AuthorizedSearch {
        provider_id: provider.id.clone(),
        endpoint,
        query: query.to_string(),
        options: SearchOptions {
            include_domains,
            exclude_domains,
            ..proposal.options.clone()
        },
        query_grade: proposal.query_grade,
        credential_ref: provider.secret_handle.clone(),
        timeout_ms: provider.timeout_ms,
        max_response_bytes: provider.max_response_bytes,
        task_id: proposal.task_id.clone(),
        trace_id: proposal.trace_id.clone(),
    })
}

/// Lowercase each domain and check it is a hostname: dotted labels of `[a-z0-9-]`, none
/// empty, none starting or ending with a hyphen. `None` if any is not.
fn normalise_domains(domains: &[String]) -> Option<Vec<String>> {
    domains
        .iter()
        .map(|domain| {
            let domain = domain.trim().to_ascii_lowercase();
            is_hostname(&domain).then_some(domain)
        })
        .collect()
}

fn is_hostname(domain: &str) -> bool {
    if domain.is_empty() || domain.len() > MAX_DOMAIN_CHARS || !domain.contains('.') {
        return false;
    }
    domain.split('.').all(|label| {
        !label.is_empty()
            && label.len() <= 63
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hostnames_are_dotted_labels_and_nothing_else() {
        for good in ["arxiv.org", "docs.rs", "a-b.example.co.uk", "123.example"] {
            assert!(is_hostname(good), "{good}");
        }
        for bad in [
            "",
            "localhost",
            "https://arxiv.org",
            "arxiv.org/abs",
            "-bad.example",
            "bad-.example",
            "a..b",
            "exa mple.com",
            "ünïcode.example",
        ] {
            assert!(!is_hostname(bad), "{bad}");
        }
    }

    #[test]
    fn domains_are_lowercased_before_they_are_compared() {
        assert_eq!(
            normalise_domains(&[" ArXiv.ORG ".to_string()]),
            Some(vec!["arxiv.org".to_string()])
        );
        assert_eq!(normalise_domains(&["not a domain".to_string()]), None);
    }

    #[test]
    fn depth_costs_match_the_provider_price_list() {
        assert_eq!(SearchDepth::Advanced.credits(), 2);
        for depth in [
            SearchDepth::UltraFast,
            SearchDepth::Fast,
            SearchDepth::Basic,
        ] {
            assert_eq!(depth.credits(), 1);
        }
    }
}
