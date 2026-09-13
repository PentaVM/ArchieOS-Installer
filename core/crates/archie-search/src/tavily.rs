//! The Tavily client.
//!
//! # This is the network boundary, so it checks again
//!
//! [`crate::SearchGate`] already decided this query may be sent. This module re-checks
//! the parts of that decision that are about where the bytes go: that the search was
//! authorised for this provider and this endpoint, that the endpoint is HTTPS unless it
//! is loopback, and that the key it was handed is the one the manifest names. A client
//! that trusts its caller is one refactor away from sending one provider's key to another.
//!
//! # The key is brokered per call
//!
//! [`TavilyClient::search`] consumes a [`SealedCredential`], which cannot be cloned or
//! stored, and writes it into the `Authorization` header and nowhere else — not an error,
//! not `Debug`, not a receipt.
//!
//! # Redirects are not followed
//!
//! The checks above were made against one URL. A 307 or 308 would re-post the body — the
//! query — to wherever the response names, after every check had passed, so a redirect
//! comes back as [`SearchError::Redirected`].

use crate::gate::AuthorizedSearch;
use crate::manifest::{ManifestError, SearchProviderKind, SearchProviderManifest};
use crate::results::{self, RawResult, SearchResults};
use archie_vault::SealedCredential;
use futures_util::StreamExt;
use reqwest::{Client, StatusCode, Url};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;

const SEARCH_PATH: &str = "search";
const MAX_ERROR_DETAIL_CHARS: usize = 256;

/// One Tavily endpoint, reachable.
pub struct TavilyClient {
    client: Client,
    provider_id: String,
    endpoint: Url,
    max_response_bytes: usize,
}

impl std::fmt::Debug for TavilyClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TavilyClient")
            .field("provider_id", &self.provider_id)
            .field("endpoint", &self.endpoint.as_str())
            .field("max_response_bytes", &self.max_response_bytes)
            .finish_non_exhaustive()
    }
}

/// The request body. Every option Archie does not use is written out as `false`, so a
/// change in the provider's defaults cannot switch it on.
#[derive(Serialize)]
struct WireRequest<'a> {
    query: &'a str,
    search_depth: &'static str,
    topic: &'static str,
    max_results: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    time_range: Option<&'static str>,
    #[serde(skip_serializing_if = "no_domains")]
    include_domains: &'a [String],
    #[serde(skip_serializing_if = "no_domains")]
    exclude_domains: &'a [String],
    include_answer: bool,
    include_raw_content: bool,
    include_images: bool,
    include_image_descriptions: bool,
    include_favicon: bool,
    auto_parameters: bool,
}

fn no_domains(domains: &&[String]) -> bool {
    domains.is_empty()
}

#[derive(Deserialize)]
struct WireResponse {
    results: Vec<WireResult>,
    #[serde(default)]
    response_time: Option<Value>,
}

#[derive(Deserialize)]
struct WireResult {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    score: Option<f64>,
    #[serde(default)]
    published_date: Option<String>,
}

impl TavilyClient {
    /// Open a client for a Tavily manifest.
    ///
    /// Validates the manifest rather than assuming a registered one: this is a public
    /// constructor, and "it must already have been validated" is not a guarantee a public
    /// constructor gets to make.
    pub fn connect(manifest: &SearchProviderManifest) -> Result<Self, SearchError> {
        manifest.validate()?;
        if manifest.kind != SearchProviderKind::Tavily {
            return Err(SearchError::WrongKind);
        }
        let endpoint = manifest.endpoint_url()?;
        let client = Client::builder()
            .timeout(Duration::from_millis(manifest.timeout_ms.into()))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(SearchError::Http)?;
        Ok(Self {
            client,
            provider_id: manifest.id.clone(),
            endpoint,
            max_response_bytes: manifest.max_response_bytes as usize,
        })
    }

    pub fn provider_id(&self) -> &str {
        &self.provider_id
    }

    /// Run a search the gate authorised, with the key the vault brokered for it.
    pub async fn search(
        &self,
        search: &AuthorizedSearch,
        credential: SealedCredential,
    ) -> Result<SearchResults, SearchError> {
        if search.provider_id() != self.provider_id || search.endpoint() != &self.endpoint {
            return Err(SearchError::ProviderMismatch);
        }
        if credential.reference() != search.credential_ref() {
            return Err(SearchError::CredentialMismatch);
        }
        let url = self
            .endpoint
            .join(SEARCH_PATH)
            .map_err(|_| SearchError::InvalidEndpoint)?;

        let options = search.options();
        let body = WireRequest {
            query: search.query(),
            search_depth: options.depth.as_str(),
            topic: options.topic.as_str(),
            max_results: options.max_results,
            time_range: options.time_range.map(|range| range.as_str()),
            include_domains: &options.include_domains,
            exclude_domains: &options.exclude_domains,
            include_answer: false,
            include_raw_content: false,
            include_images: false,
            include_image_descriptions: false,
            include_favicon: false,
            auto_parameters: false,
        };

        let response = self
            .client
            .post(url)
            // Consumed here and nowhere else.
            .bearer_auth(credential.expose().as_str())
            .json(&body)
            .send()
            .await
            .map_err(SearchError::Http)?;

        let status = response.status();
        let budget = (search.max_response_bytes() as usize).min(self.max_response_bytes);
        let bytes = bounded_body(response, budget).await?;
        if !status.is_success() {
            return Err(status_error(status, &bytes, search.query()));
        }

        let wire: WireResponse = serde_json::from_slice(&bytes)
            .map_err(|e| SearchError::Decoding(bounded(&e.to_string(), search.query())))?;
        let raw = wire
            .results
            .into_iter()
            .map(|r| RawResult {
                title: r.title,
                url: r.url,
                content: r.content,
                score: r.score,
                published_date: r.published_date,
            })
            .collect();
        Ok(results::admit(
            &self.provider_id,
            raw,
            options.max_results.into(),
            wire.response_time.as_ref().and_then(seconds_to_ms),
        ))
    }
}

/// Collect a response without letting the provider choose the allocation.
async fn bounded_body(response: reqwest::Response, budget: usize) -> Result<Vec<u8>, SearchError> {
    if response
        .content_length()
        .is_some_and(|length| length > budget as u64)
    {
        return Err(SearchError::ResponseTooLarge { budget });
    }
    let mut stream = response.bytes_stream();
    let mut output = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(SearchError::Http)?;
        if output.len().saturating_add(chunk.len()) > budget {
            return Err(SearchError::ResponseTooLarge { budget });
        }
        output.extend_from_slice(&chunk);
    }
    Ok(output)
}

fn status_error(status: StatusCode, body: &[u8], query: &str) -> SearchError {
    let detail = || bounded(&String::from_utf8_lossy(body), query);
    match status.as_u16() {
        code @ 300..=399 => SearchError::Redirected { status: code },
        400 => SearchError::Rejected { detail: detail() },
        401 | 403 => SearchError::Unauthorized,
        429 => SearchError::RateLimited,
        432 => SearchError::PlanLimit,
        433 => SearchError::SpendLimit,
        code => SearchError::Status {
            status: code,
            detail: detail(),
        },
    }
}

/// Provider text for an error message: anything shaped like a key and the query blanked
/// out wherever they are echoed, control characters removed, and bounded.
fn bounded(text: &str, query: &str) -> String {
    let text = redact_keys(text);
    let text = if query.is_empty() {
        text
    } else {
        text.replace(query, "[query]")
    };
    text.chars()
        .take(MAX_ERROR_DETAIL_CHARS)
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

/// Blank every `tvly-…` token. By the time a response is read the key has already been
/// consumed into the request, so an echo of it can only be recognised by its shape.
fn redact_keys(text: &str) -> String {
    const PREFIX: &str = "tvly-";
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find(PREFIX) {
        out.push_str(&rest[..at]);
        out.push_str("[key]");
        let tail = &rest[at + PREFIX.len()..];
        let end = tail
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_'))
            .unwrap_or(tail.len());
        rest = &tail[end..];
    }
    out.push_str(rest);
    out
}

/// Tavily reports `response_time` in seconds, as a number or a numeric string.
fn seconds_to_ms(value: &Value) -> Option<u32> {
    let seconds = match value {
        Value::Number(n) => n.as_f64()?,
        Value::String(s) => s.trim().parse::<f64>().ok()?,
        _ => return None,
    };
    // A range comparison is false for NaN and for either infinity.
    (0.0..86_400.0)
        .contains(&seconds)
        .then(|| (seconds * 1000.0).round() as u32)
}

#[derive(Debug, thiserror::Error)]
pub enum SearchError {
    #[error("provider manifest is not usable: {0}")]
    Manifest(#[from] ManifestError),
    #[error("this client speaks Tavily; the manifest names another provider kind")]
    WrongKind,
    #[error("the endpoint cannot be joined with the search path")]
    InvalidEndpoint,
    #[error("this search was authorised for a different provider or endpoint")]
    ProviderMismatch,
    #[error("the brokered credential is not the one the manifest names")]
    CredentialMismatch,
    #[error("http error: {0}")]
    Http(#[source] reqwest::Error),
    #[error("provider answered {status}; redirects are not followed")]
    Redirected { status: u16 },
    #[error("provider rejected the request: {detail}")]
    Rejected { detail: String },
    #[error("provider refused the API key")]
    Unauthorized,
    #[error("provider rate limit exceeded")]
    RateLimited,
    #[error("provider plan limit exceeded")]
    PlanLimit,
    #[error("provider pay-as-you-go limit exceeded")]
    SpendLimit,
    #[error("provider returned {status}: {detail}")]
    Status { status: u16, detail: String },
    #[error("response exceeded the {budget} byte budget")]
    ResponseTooLarge { budget: usize },
    #[error("could not decode the provider's response: {0}")]
    Decoding(String),
}

impl SearchError {
    /// A stable code for traces. Never a sentence.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Manifest(_) => "search_manifest_invalid",
            Self::WrongKind => "search_provider_kind_mismatch",
            Self::InvalidEndpoint => "search_endpoint_invalid",
            Self::ProviderMismatch => "search_provider_mismatch",
            Self::CredentialMismatch => "search_credential_mismatch",
            Self::Http(_) => "search_transport_failed",
            Self::Redirected { .. } => "search_redirect_refused",
            Self::Rejected { .. } => "search_request_rejected",
            Self::Unauthorized => "search_key_refused",
            Self::RateLimited => "search_rate_limited",
            Self::PlanLimit => "search_plan_limit",
            Self::SpendLimit => "search_spend_limit",
            Self::Status { .. } => "search_provider_error",
            Self::ResponseTooLarge { .. } => "search_response_too_large",
            Self::Decoding(_) => "search_response_undecodable",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn response_time_accepts_numbers_and_numeric_strings() {
        assert_eq!(seconds_to_ms(&Value::from(1.25)), Some(1250));
        assert_eq!(seconds_to_ms(&Value::from("0.5")), Some(500));
        assert_eq!(seconds_to_ms(&Value::from(-1.0)), None);
        assert_eq!(seconds_to_ms(&Value::from("soon")), None);
        assert_eq!(seconds_to_ms(&Value::Bool(true)), None);
    }

    #[test]
    fn an_echoed_key_is_blanked_out_of_error_detail() {
        assert_eq!(
            redact_keys("invalid key tvly-dev-AbC_123, try again (tvly-x)"),
            "invalid key [key], try again ([key])"
        );
        assert_eq!(redact_keys("no key here"), "no key here");
        assert_eq!(redact_keys("ends with tvly-"), "ends with [key]");
    }

    #[test]
    fn an_echoed_query_is_blanked_out_of_error_detail() {
        let detail = bounded("query 'my private words' is too long\n", "my private words");
        assert!(!detail.contains("my private words"));
        assert!(detail.contains("[query]"));
        assert!(!detail.contains('\n'));
        assert_eq!(
            bounded(&"x".repeat(10_000), "q").chars().count(),
            MAX_ERROR_DETAIL_CHARS
        );
    }
}
