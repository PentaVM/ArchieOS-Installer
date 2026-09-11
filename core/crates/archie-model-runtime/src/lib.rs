//! Provider-neutral inference transport.
//!
//! Routing decides which model may serve a task. This crate is the network boundary:
//! it verifies that decision again, obtains credentials only at call time, and bounds
//! all provider-controlled response data before allocation can grow without limit.

use archie_types::{Locality, ModelDescriptor, PrivacyGrade, ProviderKind};
use futures_util::StreamExt;
use reqwest::{Client, StatusCode, Url};
use serde::{Deserialize, Serialize};
use std::env;
use std::time::Duration;
use thiserror::Error;

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(90);
const DEFAULT_MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;
const MAX_MESSAGES: usize = 128;
const MAX_CONTENT_BYTES: usize = 2 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderFamily {
    NvidiaNim,
    NebiusTokenFactory,
    OpenAiCompatible,
}

impl ProviderFamily {
    fn accepts(self, kind: &ProviderKind) -> bool {
        matches!(
            (self, kind),
            (Self::NvidiaNim, ProviderKind::NvidiaNim)
                | (Self::NebiusTokenFactory, ProviderKind::NebiusTokenFactory)
                | (Self::OpenAiCompatible, ProviderKind::OpenAiCompatible)
        )
    }
}

#[derive(Debug, Clone)]
pub struct ProviderEndpoint {
    family: ProviderFamily,
    base_url: Url,
    locality: Locality,
    credential_env: Option<String>,
    timeout: Duration,
    max_response_bytes: usize,
}

impl ProviderEndpoint {
    pub fn nvidia_nim(base_url: &str, locality: Locality) -> Result<Self, RuntimeError> {
        let credential = (locality == Locality::Cloud).then(|| "NVIDIA_API_KEY".to_string());
        Self::new(ProviderFamily::NvidiaNim, base_url, locality, credential)
    }

    pub fn nebius(base_url: &str) -> Result<Self, RuntimeError> {
        Self::new(
            ProviderFamily::NebiusTokenFactory,
            base_url,
            Locality::Cloud,
            Some("NEBIUS_TOKEN_FACTORY_KEY".to_string()),
        )
    }

    pub fn openai_compatible(
        base_url: &str,
        locality: Locality,
        credential_env: Option<String>,
    ) -> Result<Self, RuntimeError> {
        Self::new(
            ProviderFamily::OpenAiCompatible,
            base_url,
            locality,
            credential_env,
        )
    }

    fn new(
        family: ProviderFamily,
        base_url: &str,
        locality: Locality,
        credential_env: Option<String>,
    ) -> Result<Self, RuntimeError> {
        let mut base_url = Url::parse(base_url).map_err(|_| RuntimeError::InvalidEndpoint)?;
        if base_url.username() != ""
            || base_url.password().is_some()
            || base_url.query().is_some()
            || base_url.fragment().is_some()
        {
            return Err(RuntimeError::InvalidEndpoint);
        }

        let loopback = base_url
            .host_str()
            .is_some_and(|host| matches!(host, "localhost" | "127.0.0.1" | "::1"));
        match base_url.scheme() {
            "https" => {}
            "http" if locality == Locality::Local && loopback => {}
            _ => return Err(RuntimeError::InsecureEndpoint),
        }
        if locality == Locality::Cloud && credential_env.is_none() {
            return Err(RuntimeError::CloudCredentialRequired);
        }
        if credential_env.as_deref().is_some_and(str::is_empty) {
            return Err(RuntimeError::InvalidCredentialEnvironment);
        }
        if !base_url.path().ends_with('/') {
            let path = format!("{}/", base_url.path());
            base_url.set_path(&path);
        }

        Ok(Self {
            family,
            base_url,
            locality,
            credential_env,
            timeout: DEFAULT_TIMEOUT,
            max_response_bytes: DEFAULT_MAX_RESPONSE_BYTES,
        })
    }

    pub fn with_limits(
        mut self,
        timeout: Duration,
        max_response_bytes: usize,
    ) -> Result<Self, RuntimeError> {
        if timeout.is_zero() || max_response_bytes == 0 {
            return Err(RuntimeError::InvalidLimits);
        }
        self.timeout = timeout;
        self.max_response_bytes = max_response_bytes;
        Ok(self)
    }

    pub fn family(&self) -> ProviderFamily {
        self.family
    }

    pub fn locality(&self) -> Locality {
        self.locality
    }

    pub fn base_url(&self) -> &Url {
        &self.base_url
    }

    fn url(&self, path: &str) -> Result<Url, RuntimeError> {
        self.base_url
            .join(path)
            .map_err(|_| RuntimeError::InvalidEndpoint)
    }

    fn credential(&self) -> Result<Option<String>, RuntimeError> {
        self.credential_env
            .as_deref()
            .map(|name| {
                env::var(name)
                    .ok()
                    .filter(|value| !value.trim().is_empty())
                    .ok_or_else(|| RuntimeError::MissingCredential(name.to_string()))
            })
            .transpose()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MessageRole {
    System,
    User,
    Assistant,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: MessageRole,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(default)]
    pub stream: bool,
}

impl ChatRequest {
    pub fn validate(&self) -> Result<(), RuntimeError> {
        if self.model.trim().is_empty()
            || self.messages.is_empty()
            || self.messages.len() > MAX_MESSAGES
            || self.stream
            || self.messages.iter().any(|message| {
                message.content.is_empty() || message.content.len() > MAX_CONTENT_BYTES
            })
            || self.max_tokens == Some(0)
            || self
                .temperature
                .is_some_and(|value| !value.is_finite() || !(0.0..=2.0).contains(&value))
        {
            return Err(RuntimeError::InvalidRequest);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ModelList {
    #[serde(default)]
    pub data: Vec<RemoteModel>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct RemoteModel {
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct ChatResponse {
    pub model: String,
    #[serde(default)]
    pub choices: Vec<ChatChoice>,
    #[serde(default)]
    pub usage: Option<TokenUsage>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct ChatChoice {
    pub message: ChatMessage,
    #[serde(default)]
    pub finish_reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
pub struct TokenUsage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
}

#[derive(Debug, Error)]
pub enum RuntimeError {
    #[error("provider endpoint is invalid")]
    InvalidEndpoint,
    #[error(
        "remote provider endpoints require HTTPS; loopback HTTP is allowed only for local runtimes"
    )]
    InsecureEndpoint,
    #[error("cloud providers require an environment-backed credential")]
    CloudCredentialRequired,
    #[error("credential environment variable name is invalid")]
    InvalidCredentialEnvironment,
    #[error("provider credential is absent from environment variable {0}")]
    MissingCredential(String),
    #[error("transport limits must be greater than zero")]
    InvalidLimits,
    #[error("inference request is empty, streaming, or outside supported bounds")]
    InvalidRequest,
    #[error("model descriptor does not match the configured provider endpoint")]
    ProviderMismatch,
    #[error("model locality does not match the configured provider endpoint")]
    LocalityMismatch,
    #[error("privacy grade {grade} is not permitted for model {model}")]
    PrivacyDenied { grade: PrivacyGrade, model: String },
    #[error("provider returned HTTP {0}")]
    HttpStatus(StatusCode),
    #[error("provider response exceeded the configured byte budget")]
    ResponseTooLarge,
    #[error("provider response did not contain a completion")]
    EmptyCompletion,
    #[error("provider transport failed: {0}")]
    Transport(#[from] reqwest::Error),
    #[error("provider returned invalid JSON: {0}")]
    Decode(#[from] serde_json::Error),
}

#[derive(Clone)]
pub struct InferenceClient {
    endpoint: ProviderEndpoint,
    client: Client,
}

impl InferenceClient {
    pub fn new(endpoint: ProviderEndpoint) -> Result<Self, RuntimeError> {
        let client = Client::builder()
            .timeout(endpoint.timeout)
            .https_only(false)
            .build()?;
        Ok(Self { endpoint, client })
    }

    pub async fn ready(&self) -> Result<bool, RuntimeError> {
        let response = self
            .send::<()>(self.endpoint.url("health/ready")?, None)
            .await?;
        Ok(response.status().is_success())
    }

    pub async fn models(&self) -> Result<ModelList, RuntimeError> {
        let response = self.send::<()>(self.endpoint.url("models")?, None).await?;
        let bytes = self.bounded_body(response).await?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    pub async fn chat(
        &self,
        descriptor: &ModelDescriptor,
        privacy: PrivacyGrade,
        request: &ChatRequest,
    ) -> Result<ChatResponse, RuntimeError> {
        request.validate()?;
        self.authorize(descriptor, privacy)?;
        if request.model != descriptor.id.as_str() {
            return Err(RuntimeError::ProviderMismatch);
        }
        let response = self
            .send(self.endpoint.url("chat/completions")?, Some(request))
            .await?;
        let status = response.status();
        let bytes = self.bounded_body(response).await?;
        if !status.is_success() {
            return Err(RuntimeError::HttpStatus(status));
        }
        let decoded: ChatResponse = serde_json::from_slice(&bytes)?;
        if decoded.choices.is_empty() {
            return Err(RuntimeError::EmptyCompletion);
        }
        Ok(decoded)
    }

    fn authorize(
        &self,
        descriptor: &ModelDescriptor,
        privacy: PrivacyGrade,
    ) -> Result<(), RuntimeError> {
        if !self.endpoint.family.accepts(&descriptor.provider) {
            return Err(RuntimeError::ProviderMismatch);
        }
        if descriptor.locality != self.endpoint.locality || !descriptor.locality_is_consistent() {
            return Err(RuntimeError::LocalityMismatch);
        }
        if !descriptor.permits_privacy(privacy) {
            return Err(RuntimeError::PrivacyDenied {
                grade: privacy,
                model: descriptor.id.to_string(),
            });
        }
        Ok(())
    }

    async fn send<T: Serialize + ?Sized>(
        &self,
        url: Url,
        body: Option<&T>,
    ) -> Result<reqwest::Response, RuntimeError> {
        let mut request = match body {
            Some(body) => self.client.post(url).json(body),
            None => self.client.get(url),
        };
        if let Some(credential) = self.endpoint.credential()? {
            request = request.bearer_auth(credential);
        }
        Ok(request.send().await?)
    }

    async fn bounded_body(&self, response: reqwest::Response) -> Result<Vec<u8>, RuntimeError> {
        if response
            .content_length()
            .is_some_and(|length| length > self.endpoint.max_response_bytes as u64)
        {
            return Err(RuntimeError::ResponseTooLarge);
        }
        let mut stream = response.bytes_stream();
        let mut output = Vec::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            if output.len().saturating_add(chunk.len()) > self.endpoint.max_response_bytes {
                return Err(RuntimeError::ResponseTooLarge);
            }
            output.extend_from_slice(&chunk);
        }
        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use archie_types::{Availability, Capability, ModelId, TokenCost};
    use std::collections::BTreeMap;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    async fn one_response(
        response_body: &'static str,
    ) -> (String, tokio::sync::oneshot::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (sender, receiver) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = vec![0_u8; 16 * 1024];
            let read = socket.read(&mut request).await.unwrap();
            request.truncate(read);
            let request = String::from_utf8(request).unwrap();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                response_body.len(),
                response_body
            );
            socket.write_all(response.as_bytes()).await.unwrap();
            let _ = sender.send(request);
        });
        (format!("http://{address}/v1"), receiver)
    }

    fn descriptor(provider: ProviderKind, locality: Locality) -> ModelDescriptor {
        ModelDescriptor {
            id: ModelId::parse("test.model").unwrap(),
            display_name: "test".into(),
            provider,
            locality,
            node: None,
            context_window: 8192,
            max_output_tokens: 2048,
            capabilities: vec![Capability::known("compute.inference")],
            max_privacy_grade: PrivacyGrade::Restricted,
            availability: Availability::Ready,
            cost: TokenCost::FREE,
            latency_ema_ms: None,
            quality_tier: 50,
            required_accelerator_bytes: None,
            labels: BTreeMap::new(),
        }
    }

    #[test]
    fn local_nim_allows_loopback_http_without_a_key() {
        let endpoint =
            ProviderEndpoint::nvidia_nim("http://127.0.0.1:8000/v1", Locality::Local).unwrap();
        assert_eq!(endpoint.base_url().as_str(), "http://127.0.0.1:8000/v1/");
        assert_eq!(endpoint.credential().unwrap(), None);
    }

    #[test]
    fn remote_http_and_embedded_credentials_are_rejected() {
        assert!(matches!(
            ProviderEndpoint::nvidia_nim("http://nim.example/v1", Locality::Fabric),
            Err(RuntimeError::InsecureEndpoint)
        ));
        assert!(matches!(
            ProviderEndpoint::nvidia_nim("https://key@nim.example/v1", Locality::Cloud),
            Err(RuntimeError::InvalidEndpoint)
        ));
    }

    #[test]
    fn nebius_is_always_cloud_and_requires_its_key() {
        let endpoint = ProviderEndpoint::nebius("https://api.tokenfactory.nebius.com/v1").unwrap();
        assert_eq!(endpoint.locality(), Locality::Cloud);
        if env::var_os("NEBIUS_TOKEN_FACTORY_KEY").is_none() {
            assert!(matches!(
                endpoint.credential(),
                Err(RuntimeError::MissingCredential(name)) if name == "NEBIUS_TOKEN_FACTORY_KEY"
            ));
        }
    }

    #[test]
    fn cloud_cannot_be_configured_without_a_secret_source() {
        assert!(matches!(
            ProviderEndpoint::openai_compatible("https://models.example/v1", Locality::Cloud, None),
            Err(RuntimeError::CloudCredentialRequired)
        ));
    }

    #[test]
    fn transport_repeats_provider_locality_and_privacy_gates() {
        let endpoint =
            ProviderEndpoint::nvidia_nim("http://localhost:8000/v1", Locality::Local).unwrap();
        let client = InferenceClient::new(endpoint).unwrap();
        let local = descriptor(ProviderKind::NvidiaNim, Locality::Local);
        assert!(client.authorize(&local, PrivacyGrade::Restricted).is_ok());

        let wrong = descriptor(ProviderKind::OpenAiCompatible, Locality::Local);
        assert!(matches!(
            client.authorize(&wrong, PrivacyGrade::Public),
            Err(RuntimeError::ProviderMismatch)
        ));

        let cloud_endpoint =
            ProviderEndpoint::nebius("https://api.tokenfactory.nebius.com/v1").unwrap();
        let cloud_client = InferenceClient::new(cloud_endpoint).unwrap();
        let cloud = descriptor(ProviderKind::NebiusTokenFactory, Locality::Cloud);
        assert!(matches!(
            cloud_client.authorize(&cloud, PrivacyGrade::Sensitive),
            Err(RuntimeError::PrivacyDenied { .. })
        ));
    }

    #[test]
    fn request_validation_is_bounded_and_non_streaming() {
        let valid = ChatRequest {
            model: "test.model".into(),
            messages: vec![ChatMessage {
                role: MessageRole::User,
                content: "hello".into(),
            }],
            max_tokens: Some(100),
            temperature: Some(0.2),
            stream: false,
        };
        assert!(valid.validate().is_ok());
        let mut invalid = valid.clone();
        invalid.stream = true;
        assert!(matches!(
            invalid.validate(),
            Err(RuntimeError::InvalidRequest)
        ));
    }

    #[tokio::test]
    async fn local_nim_chat_uses_the_openai_wire_contract() {
        let body = r#"{"model":"test.model","choices":[{"message":{"role":"assistant","content":"ready"},"finish_reason":"stop"}],"usage":{"prompt_tokens":2,"completion_tokens":1,"total_tokens":3}}"#;
        let (base_url, observed) = one_response(body).await;
        let endpoint = ProviderEndpoint::nvidia_nim(&base_url, Locality::Local).unwrap();
        let client = InferenceClient::new(endpoint).unwrap();
        let request = ChatRequest {
            model: "test.model".into(),
            messages: vec![ChatMessage {
                role: MessageRole::User,
                content: "status".into(),
            }],
            max_tokens: Some(32),
            temperature: Some(0.0),
            stream: false,
        };

        let response = client
            .chat(
                &descriptor(ProviderKind::NvidiaNim, Locality::Local),
                PrivacyGrade::Restricted,
                &request,
            )
            .await
            .unwrap();
        assert_eq!(response.choices[0].message.content, "ready");
        assert_eq!(response.usage.unwrap().total_tokens, 3);

        let request_text = observed.await.unwrap();
        assert!(request_text.starts_with("POST /v1/chat/completions HTTP/1.1"));
        assert!(request_text.contains("\"model\":\"test.model\""));
        assert!(request_text.contains("\"content\":\"status\""));
        assert!(!request_text.to_ascii_lowercase().contains("authorization:"));
    }

    #[tokio::test]
    async fn content_length_over_budget_is_rejected_before_body_collection() {
        let body = r#"{"data":[{"id":"test.model"}]}"#;
        let (base_url, _observed) = one_response(body).await;
        let endpoint = ProviderEndpoint::nvidia_nim(&base_url, Locality::Local)
            .unwrap()
            .with_limits(Duration::from_secs(2), 8)
            .unwrap();
        let client = InferenceClient::new(endpoint).unwrap();
        assert!(matches!(
            client.models().await,
            Err(RuntimeError::ResponseTooLarge)
        ));
    }
}
