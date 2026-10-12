//! Client for the owner's local Wanda service. A private voice tool cannot
//! redirect this request to a public endpoint or follow an HTTP redirect.
use crate::{types::QueryResponse, WandaError, MAX_REQUEST_BYTES};
use reqwest::{
    header::{HeaderValue, AUTHORIZATION},
    Client, Url,
};
use std::{io::Read, path::Path, time::Duration};

pub struct WandaClient {
    base: Url,
    authorization: HeaderValue,
    client: Client,
}
impl std::fmt::Debug for WandaClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("WandaClient(redacted)")
    }
}
impl WandaClient {
    pub fn from_env() -> Result<Option<Self>, WandaError> {
        let endpoint = match std::env::var("ARCHIE_WANDA_ENDPOINT") {
            Ok(value) if !value.is_empty() => value,
            Ok(_) | Err(std::env::VarError::NotPresent) => return Ok(None),
            Err(_) => return Err(WandaError::Transport),
        };
        let token_path =
            std::env::var_os("ARCHIE_WANDA_TOKEN_FILE").ok_or(WandaError::Unauthorized)?;
        let file =
            std::fs::File::open(Path::new(&token_path)).map_err(|_| WandaError::Unauthorized)?;
        if !file
            .metadata()
            .map_err(|_| WandaError::Unauthorized)?
            .is_file()
        {
            return Err(WandaError::Unauthorized);
        }
        let mut bytes = Vec::new();
        file.take(4097)
            .read_to_end(&mut bytes)
            .map_err(|_| WandaError::Unauthorized)?;
        if bytes.len() > 4096 {
            return Err(WandaError::Unauthorized);
        }
        let token = std::str::from_utf8(&bytes)
            .map_err(|_| WandaError::Unauthorized)?
            .trim();
        Self::new(&endpoint, token).map(Some)
    }
    pub fn new(endpoint: &str, token: &str) -> Result<Self, WandaError> {
        let base = Url::parse(endpoint).map_err(|_| WandaError::Transport)?;
        let host = base.host_str().unwrap_or_default().trim_matches(['[', ']']);
        if !matches!(base.scheme(), "http" | "https")
            || !matches!(host, "localhost" | "127.0.0.1" | "::1")
            || !base.username().is_empty()
            || base.password().is_some()
            || base.query().is_some()
            || base.fragment().is_some()
            || base.path() != "/"
        {
            return Err(WandaError::Transport);
        }
        if !(16..=4096).contains(&token.len()) || !token.bytes().all(|b| b.is_ascii_graphic()) {
            return Err(WandaError::Unauthorized);
        }
        let mut authorization = HeaderValue::from_str(&format!("Bearer {token}"))
            .map_err(|_| WandaError::Unauthorized)?;
        authorization.set_sensitive(true);
        let client = Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(2))
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|_| WandaError::Transport)?;
        Ok(Self {
            base,
            authorization,
            client,
        })
    }
    pub async fn query(
        &self,
        domain: &str,
        question: &str,
        deep: bool,
    ) -> Result<QueryResponse, WandaError> {
        let _: archie_vault::ContextDomain = domain
            .to_string()
            .try_into()
            .map_err(|_| WandaError::UnknownDomain)?;
        if question.trim().is_empty() || question.chars().count() > 2000 {
            return Err(WandaError::TooLarge);
        }
        let body = serde_json::to_vec(
            &serde_json::json!({"domain":domain,"question":question,"deep":deep}),
        )
        .map_err(|_| WandaError::Transport)?;
        let mut response = self
            .client
            .post(
                self.base
                    .join("v1/query")
                    .map_err(|_| WandaError::Transport)?,
            )
            .header(AUTHORIZATION, self.authorization.clone())
            .header("Content-Type", "application/json")
            .body(body)
            .send()
            .await
            .map_err(|_| WandaError::Transport)?;
        if !response.status().is_success() {
            return Err(WandaError::Transport);
        }
        if !response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.split(';').next() == Some("application/json"))
            || response
                .content_length()
                .is_some_and(|n| n > MAX_REQUEST_BYTES as u64)
        {
            return Err(WandaError::TooLarge);
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| WandaError::Transport)? {
            if bytes.len().saturating_add(chunk.len()) > MAX_REQUEST_BYTES {
                return Err(WandaError::TooLarge);
            }
            bytes.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&bytes).map_err(|_| WandaError::InvalidAdvice)
    }
}
