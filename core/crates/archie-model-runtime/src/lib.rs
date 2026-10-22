//! Provider-neutral inference transport.
//!
//! Routing decides which model may serve a task. This crate is the network boundary:
//! it verifies that decision again, obtains credentials only at call time, and bounds
//! all provider-controlled response data before allocation can grow without limit.

use archie_types::{Locality, ModelDescriptor, PrivacyGrade, ProviderKind};
use base64::Engine;
use futures_util::StreamExt;
use reqwest::{Client, StatusCode, Url};
use serde::{Deserialize, Serialize};
use std::env;
use std::time::Duration;
use thiserror::Error;

pub mod stream;
pub use stream::{
    SseAssembler, SseError, StreamEvent, StreamUnveiler, ToolCall, ToolChatMessage,
    ToolChatRequest, ToolDefinition,
};

mod anthropic;
pub mod provider;
pub mod routed;

/// Nebius Token Factory's OpenAI-compatible API.
pub const NEBIUS_BASE_URL: &str = "https://api.tokenfactory.nebius.com/v1/";

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(90);
const DEFAULT_MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;
const MAX_MESSAGES: usize = 128;
const MAX_CONTENT_BYTES: usize = 2 * 1024 * 1024;
const MAX_VISION_IMAGE_BYTES: usize = 8 * 1024 * 1024;
const DEFAULT_MAX_RETRIES: usize = 3;
const BASE_RETRY_BACKOFF: Duration = Duration::from_millis(250);
const MAX_RETRY_BACKOFF: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderFamily {
    NvidiaNim,
    NebiusTokenFactory,
    OpenAiCompatible,
    Anthropic,
}

impl ProviderFamily {
    fn accepts(self, kind: &ProviderKind) -> bool {
        matches!(
            (self, kind),
            (Self::NvidiaNim, ProviderKind::NvidiaNim)
                | (Self::NebiusTokenFactory, ProviderKind::NebiusTokenFactory)
                | (Self::OpenAiCompatible, ProviderKind::OpenAiCompatible)
                | (Self::OpenAiCompatible, ProviderKind::OpenAi)
                | (Self::OpenAiCompatible, ProviderKind::NebiusTokenFactory)
                | (Self::Anthropic, ProviderKind::Anthropic)
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
    /// Native Anthropic Messages API. The model remains cloud-backed behind a proxy.
    pub fn anthropic(base_url: &str, credential_env: Option<String>) -> Result<Self, RuntimeError> {
        Self::new(
            ProviderFamily::Anthropic,
            base_url,
            Locality::Cloud,
            credential_env,
        )
    }
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
            .is_some_and(|host| matches!(host, "localhost" | "127.0.0.1" | "::1" | "[::1]"));
        if locality == Locality::Local && !loopback {
            return Err(RuntimeError::LocalityMismatch);
        }
        // These are service endpoints, not arbitrary operator-controlled deployments.
        // Generic local/fabric servers still use their explicit operator declaration.
        let known_cloud_host = base_url.host_str().is_some_and(|host| {
            matches!(
                host.trim_end_matches('.').to_ascii_lowercase().as_str(),
                "api.openai.com"
                    | "api.tokenfactory.nebius.com"
                    | "api.cerebras.ai"
                    | "openrouter.ai"
                    | "api.anthropic.com"
            )
        });
        if known_cloud_host && locality != Locality::Cloud {
            return Err(RuntimeError::LocalityMismatch);
        }
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

    /// The endpoint a model's manifest describes, so that whoever holds a descriptor the
    /// router chose can call it without knowing which provider it is.
    ///
    /// Nebius has one address and one credential. NIM and any other OpenAI-compatible
    /// server say where they are in the manifest's `base-url` label, and which environment
    /// variable holds their key in `credential-env`.
    pub fn for_model(descriptor: &ModelDescriptor) -> Result<Self, RuntimeError> {
        let label = |name: &str| descriptor.labels.get(name).map(String::as_str);
        match &descriptor.provider {
            ProviderKind::NebiusTokenFactory => Self::new(
                ProviderFamily::NebiusTokenFactory,
                label("base-url").unwrap_or(NEBIUS_BASE_URL),
                Locality::Cloud,
                Some(
                    label("credential-env")
                        .unwrap_or("NEBIUS_TOKEN_FACTORY_KEY")
                        .to_string(),
                ),
            ),
            ProviderKind::OpenAi => Self::openai_compatible(
                label("base-url").unwrap_or("https://api.openai.com/v1/"),
                Locality::Cloud,
                Some(
                    label("credential-env")
                        .unwrap_or("OPENAI_API_KEY")
                        .to_string(),
                ),
            ),
            ProviderKind::NvidiaNim => Self::nvidia_nim(
                label("base-url").ok_or(RuntimeError::InvalidEndpoint)?,
                descriptor.locality,
            ),
            ProviderKind::OpenAiCompatible => Self::openai_compatible(
                label("base-url").ok_or(RuntimeError::InvalidEndpoint)?,
                descriptor.locality,
                label("credential-env").map(str::to_string),
            ),
            ProviderKind::Anthropic => Self::anthropic(
                label("base-url").unwrap_or("https://api.anthropic.com/v1/"),
                Some(
                    label("credential-env")
                        .unwrap_or("ANTHROPIC_API_KEY")
                        .to_string(),
                ),
            ),
            _ => Err(RuntimeError::ProviderMismatch),
        }
    }

    /// Whether the credential this endpoint needs is present. A cloud model without its
    /// key is registered and cannot be called.
    pub fn is_configured(&self) -> bool {
        self.credential().is_ok()
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VisionMime {
    Png,
    Jpeg,
    Webp,
}

impl VisionMime {
    fn as_str(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
            Self::Webp => "image/webp",
        }
    }

    fn signature_matches(self, bytes: &[u8]) -> bool {
        match self {
            Self::Png => bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
            Self::Jpeg => bytes.starts_with(&[0xff, 0xd8, 0xff]),
            Self::Webp => bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP",
        }
    }
}

/// An encoded keyframe held only for one inference call. It deliberately implements
/// neither `Serialize`, `Debug` nor `Clone`; only the egress client can form the wire body.
pub struct VisionImage<'a> {
    pub mime: VisionMime,
    pub bytes: &'a [u8],
    pub width: u32,
    pub height: u32,
}

/// OpenAI-compatible image/text request used by local vLLM and selected APIs.
/// Context must already have passed the Vault gateway before this type is constructed.
pub struct VisionChatRequest<'a> {
    pub instruction: &'a str,
    pub context: &'a str,
    pub image: VisionImage<'a>,
    pub max_tokens: u32,
}

impl VisionChatRequest<'_> {
    fn validate(&self) -> Result<(), RuntimeError> {
        if self.instruction.is_empty()
            || self.instruction.len() > 16 * 1024
            || self.context.is_empty()
            || self.context.len() > MAX_CONTENT_BYTES
            || self.image.bytes.is_empty()
            || self.image.bytes.len() > MAX_VISION_IMAGE_BYTES
            || self.image.width == 0
            || self.image.height == 0
            || self.image.width > 8_192
            || self.image.height > 8_192
            || !self.image.mime.signature_matches(self.image.bytes)
            || self.max_tokens == 0
            || self.max_tokens > 8_192
        {
            return Err(RuntimeError::InvalidRequest);
        }
        Ok(())
    }
}

impl ChatRequest {
    pub fn validate(&self) -> Result<(), RuntimeError> {
        if self.model.trim().is_empty()
            || self.model.len() > 256
            || self.messages.is_empty()
            || self.messages.len() > MAX_MESSAGES
            || self.stream
            || self.messages.iter().any(|message| {
                message.content.is_empty() || message.content.len() > MAX_CONTENT_BYTES
            })
            || self
                .messages
                .iter()
                .fold(0_usize, |n, m| n.saturating_add(m.content.len()))
                > MAX_CONTENT_BYTES
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
    #[error("inference exceeded its total deadline")]
    DeadlineExceeded,
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
    #[error("native provider protocol error: {0}")]
    NativeProtocol(&'static str),
    #[error("provider transport failed: {0}")]
    Transport(#[from] reqwest::Error),
    #[error("provider returned invalid JSON: {0}")]
    Decode(#[from] serde_json::Error),
    #[error("streamed request refused before sending: {0}")]
    StreamRequest(&'static str),
    #[error("provider stream could not be read: {0:?}")]
    Stream(stream::SseError),
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
            // A provider may not redirect private context to an endpoint that was
            // never checked by the router/egress gate (including 307/308 POST replay).
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        Ok(Self { endpoint, client })
    }

    pub async fn ready(&self) -> Result<bool, RuntimeError> {
        self.with_deadline(self.ready_inner()).await
    }

    async fn ready_inner(&self) -> Result<bool, RuntimeError> {
        let response = self
            .send::<()>(self.endpoint.url("health/ready")?, None)
            .await?;
        Ok(response.status().is_success())
    }

    pub async fn models(&self) -> Result<ModelList, RuntimeError> {
        self.with_deadline(self.models_inner()).await
    }

    async fn models_inner(&self) -> Result<ModelList, RuntimeError> {
        let response = self.send::<()>(self.endpoint.url("models")?, None).await?;
        let status = response.status();
        let bytes = self.bounded_body(response).await?;
        if !status.is_success() {
            return Err(RuntimeError::HttpStatus(status));
        }
        Ok(serde_json::from_slice(&bytes)?)
    }

    pub async fn chat(
        &self,
        descriptor: &ModelDescriptor,
        privacy: PrivacyGrade,
        request: &ChatRequest,
    ) -> Result<ChatResponse, RuntimeError> {
        self.with_deadline(self.chat_inner(descriptor, privacy, request))
            .await
    }

    async fn chat_inner(
        &self,
        descriptor: &ModelDescriptor,
        privacy: PrivacyGrade,
        request: &ChatRequest,
    ) -> Result<ChatResponse, RuntimeError> {
        request.validate()?;
        self.authorize(descriptor, privacy)?;
        // The provider's own name for the model, as the streaming call already requires: a
        // Nebius model is `nvidia/nemotron-3-super-120b-a12b` there and `nebius.nemotron-3-super`
        // here, and only the first means anything to Nebius.
        if request.model != provider_model_id(descriptor)? {
            return Err(RuntimeError::ProviderMismatch);
        }
        if self.endpoint.family == ProviderFamily::Anthropic {
            let body = anthropic::chat_body(request)?;
            let response = self
                .send(self.endpoint.url("messages")?, Some(&body))
                .await?;
            let status = response.status();
            let bytes = self.bounded_body(response).await?;
            if !status.is_success() {
                return Err(RuntimeError::HttpStatus(status));
            }
            return anthropic::decode(&bytes);
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

    pub async fn vision_chat(
        &self,
        descriptor: &ModelDescriptor,
        privacy: PrivacyGrade,
        request: &VisionChatRequest<'_>,
    ) -> Result<ChatResponse, RuntimeError> {
        self.with_deadline(self.vision_chat_inner(descriptor, privacy, request))
            .await
    }

    async fn vision_chat_inner(
        &self,
        descriptor: &ModelDescriptor,
        privacy: PrivacyGrade,
        request: &VisionChatRequest<'_>,
    ) -> Result<ChatResponse, RuntimeError> {
        request.validate()?;
        self.authorize(descriptor, privacy)?;
        let provider_model = provider_model_id(descriptor)?;
        let encoded = base64::engine::general_purpose::STANDARD.encode(request.image.bytes);
        let data_url = format!("data:{};base64,{encoded}", request.image.mime.as_str());
        let native = self.endpoint.family == ProviderFamily::Anthropic;
        let body = if native {
            serde_json::json!({
                "model":provider_model,
                "system":request.instruction,
                "messages":[{"role":"user","content":[
                    {"type":"text","text":request.context},
                    {"type":"image","source":{"type":"base64","media_type":request.image.mime.as_str(),"data":encoded}}
                ]}],
                "max_tokens":request.max_tokens,"temperature":0.0,"stream":false
            })
        } else {
            serde_json::json!({
                "model": provider_model,
                "messages": [
                    {"role": "system", "content": request.instruction},
                    {"role": "user", "content": [
                        {"type": "text", "text": request.context},
                        {"type": "image_url", "image_url": {"url": data_url}}
                    ]}
                ],
                "max_tokens": request.max_tokens,
                "temperature": 0.0,
                "stream": false
            })
        };
        let response = self
            .send(
                self.endpoint.url(if native {
                    "messages"
                } else {
                    "chat/completions"
                })?,
                Some(&body),
            )
            .await?;
        let status = response.status();
        let bytes = self.bounded_body(response).await?;
        if !status.is_success() {
            return Err(RuntimeError::HttpStatus(status));
        }
        if native {
            return anthropic::decode(&bytes);
        }
        let decoded: ChatResponse = serde_json::from_slice(&bytes)?;
        if decoded.choices.is_empty() {
            return Err(RuntimeError::EmptyCompletion);
        }
        Ok(decoded)
    }

    /// Asks for an answer that arrives while it is being written, with tools on offer.
    ///
    /// The same gate as [`InferenceClient::chat`] runs first — the provider has to be the
    /// one the descriptor names, the locality has to match, and the privacy grade has to be
    /// one the model may see — and the same ceiling on how much the provider may send.
    /// Every event is handed to `on_event` as soon as it is whole; the call returns when
    /// the stream finishes, and an error if it could not be read to the end.
    pub async fn chat_stream<F>(
        &self,
        descriptor: &ModelDescriptor,
        privacy: PrivacyGrade,
        request: &stream::ToolChatRequest,
        on_event: F,
    ) -> Result<(), RuntimeError>
    where
        F: FnMut(stream::StreamEvent),
    {
        self.with_deadline(self.chat_stream_inner(descriptor, privacy, request, on_event))
            .await
    }

    async fn chat_stream_inner<F>(
        &self,
        descriptor: &ModelDescriptor,
        privacy: PrivacyGrade,
        request: &stream::ToolChatRequest,
        mut on_event: F,
    ) -> Result<(), RuntimeError>
    where
        F: FnMut(stream::StreamEvent),
    {
        request.validate().map_err(RuntimeError::StreamRequest)?;
        self.authorize(descriptor, privacy)?;
        if request.model != provider_model_id(descriptor)? {
            return Err(RuntimeError::ProviderMismatch);
        }
        let native = self.endpoint.family == ProviderFamily::Anthropic;
        let body = if native {
            anthropic::tool_body(request)?
        } else {
            request.body()
        };
        let response = self
            .send(
                self.endpoint.url(if native {
                    "messages"
                } else {
                    "chat/completions"
                })?,
                Some(&body),
            )
            .await?;
        let status = response.status();
        if !status.is_success() {
            return Err(RuntimeError::HttpStatus(status));
        }
        let mut assembler = stream::SseAssembler::new();
        let mut native_assembler = anthropic::AnthropicSse::new();
        let mut received = 0_usize;
        let mut bytes = response.bytes_stream();
        while let Some(chunk) = bytes.next().await {
            let chunk = chunk?;
            received = received.saturating_add(chunk.len());
            if received > self.endpoint.max_response_bytes {
                return Err(RuntimeError::ResponseTooLarge);
            }
            let events = if native {
                native_assembler.feed(&chunk)
            } else {
                assembler.feed(&chunk)
            }
            .map_err(RuntimeError::Stream)?;
            for event in events {
                on_event(event);
            }
            if if native {
                native_assembler.is_finished()
            } else {
                assembler.is_finished()
            } {
                break;
            }
        }
        let events = if native {
            native_assembler.close()
        } else {
            assembler.close()
        }
        .map_err(RuntimeError::Stream)?;
        for event in events {
            on_event(event);
        }
        Ok(())
    }

    async fn with_deadline<T>(
        &self,
        operation: impl std::future::Future<Output = Result<T, RuntimeError>>,
    ) -> Result<T, RuntimeError> {
        tokio::time::timeout(self.endpoint.timeout, operation)
            .await
            .map_err(|_| RuntimeError::DeadlineExceeded)?
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
        let mut attempt = 0;
        loop {
            let mut request = match body {
                Some(body) => self.client.post(url.clone()).json(body),
                None => self.client.get(url.clone()),
            };
            if let Some(credential) = self.endpoint.credential()? {
                request = if self.endpoint.family == ProviderFamily::Anthropic {
                    request.header("x-api-key", credential)
                } else {
                    request.bearer_auth(credential)
                };
            }
            if self.endpoint.family == ProviderFamily::Anthropic {
                request = request.header("anthropic-version", "2023-06-01");
            }
            let response = request.send().await?;

            let status = response.status();
            // Resilient retry for rate limiting (429) or transient provider outage (502, 503, 504)
            if (status == StatusCode::TOO_MANY_REQUESTS
                || status == StatusCode::SERVICE_UNAVAILABLE
                || status == StatusCode::BAD_GATEWAY
                || status == StatusCode::GATEWAY_TIMEOUT)
                && attempt < DEFAULT_MAX_RETRIES
            {
                attempt += 1;
                // Honor Retry-After header if provided by the upstream provider
                let retry_after_secs = response
                    .headers()
                    .get(reqwest::header::RETRY_AFTER)
                    .and_then(|h| h.to_str().ok())
                    .and_then(|s| s.parse::<u64>().ok());

                let delay = if let Some(secs) = retry_after_secs {
                    Duration::from_secs(secs.min(10))
                } else {
                    // Exponential backoff: base * 2^(attempt - 1) + jitter
                    let mult = 1_u32.checked_shl(attempt as u32 - 1).unwrap_or(8);
                    let exp_ms =
                        (BASE_RETRY_BACKOFF.as_millis() as u64).saturating_mul(mult as u64);
                    let jitter_ms = (std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .subsec_millis()
                        % 100) as u64;
                    let total_ms = exp_ms.saturating_add(jitter_ms);
                    Duration::from_millis(total_ms).min(MAX_RETRY_BACKOFF)
                };

                tokio::time::sleep(delay).await;
                continue;
            }

            return Ok(response);
        }
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

/// Resolves the provider-facing model name from a trusted router descriptor. Archie model
/// ids remain path-safe; provider names may contain slashes or uppercase characters.
pub fn provider_model_id(descriptor: &ModelDescriptor) -> Result<&str, RuntimeError> {
    let value = descriptor
        .labels
        .get("provider-model")
        .map(String::as_str)
        .unwrap_or_else(|| descriptor.id.as_str());
    if value.is_empty()
        || value.len() > 256
        || !value.is_ascii()
        || value
            .chars()
            .any(|character| character.is_control() || character.is_whitespace())
    {
        return Err(RuntimeError::InvalidRequest);
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use archie_types::{Availability, Capability, DataRetention, ModelId, TokenCost};
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
            retention: DataRetention::ZeroVerified,
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
    async fn locate_anything_wire_carries_admitted_context_and_inline_keyframe() {
        let body = r#"{"model":"test.model","choices":[{"message":{"role":"assistant","content":"<box> 100, 200, 700, 800 </box>"},"finish_reason":"stop"}]}"#;
        let (url, received) = one_response(body).await;
        let endpoint = ProviderEndpoint::nvidia_nim(&url, Locality::Local).unwrap();
        let client = InferenceClient::new(endpoint).unwrap();
        let png = b"\x89PNG\r\n\x1a\nsynthetic";
        let request = VisionChatRequest {
            instruction: "Return one box.",
            context: "[vault:ui-target] search button",
            image: VisionImage {
                mime: VisionMime::Png,
                bytes: png,
                width: 32,
                height: 24,
            },
            max_tokens: 128,
        };
        let response = client
            .vision_chat(
                &descriptor(ProviderKind::NvidiaNim, Locality::Local),
                PrivacyGrade::Restricted,
                &request,
            )
            .await
            .unwrap();
        assert_eq!(
            response.choices[0].message.content,
            "<box> 100, 200, 700, 800 </box>"
        );
        let request_text = received.await.unwrap();
        assert!(request_text.contains("[vault:ui-target] search button"));
        assert!(request_text.contains("data:image/png;base64,"));
        assert!(request_text.contains("image_url"));
    }

    #[test]
    fn vision_input_rejects_mime_spoofing_and_unbounded_geometry() {
        let invalid = VisionChatRequest {
            instruction: "Return one box.",
            context: "admitted context",
            image: VisionImage {
                mime: VisionMime::Png,
                bytes: b"not-a-png",
                width: 9_000,
                height: 1,
            },
            max_tokens: 128,
        };
        assert!(matches!(
            invalid.validate(),
            Err(RuntimeError::InvalidRequest)
        ));
    }

    #[test]
    fn provider_model_label_is_bounded_and_cannot_inject_wire_content() {
        let mut selected = descriptor(ProviderKind::NvidiaNim, Locality::Local);
        selected
            .labels
            .insert("provider-model".into(), "nvidia/LocateAnything-3B".into());
        assert_eq!(
            provider_model_id(&selected).unwrap(),
            "nvidia/LocateAnything-3B"
        );
        selected
            .labels
            .insert("provider-model".into(), "bad\nmodel".into());
        assert!(matches!(
            provider_model_id(&selected),
            Err(RuntimeError::InvalidRequest)
        ));
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

    #[tokio::test]
    async fn provider_redirect_cannot_forward_context_to_another_endpoint() {
        let sink = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let source = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base_url = format!("http://{}/v1", source.local_addr().unwrap());
        let location = format!("http://{}/collect", sink.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut socket, _) = source.accept().await.unwrap();
            let mut request = [0; 4096];
            let read = socket.read(&mut request).await.unwrap();
            assert!(read > 0);
            socket.write_all(format!("HTTP/1.1 307 Temporary Redirect\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").as_bytes()).await.unwrap();
        });
        let client =
            InferenceClient::new(ProviderEndpoint::nvidia_nim(&base_url, Locality::Local).unwrap())
                .unwrap();
        let request = ChatRequest {
            model: "test.model".into(),
            messages: vec![ChatMessage {
                role: MessageRole::User,
                content: "synthetic:context".into(),
            }],
            max_tokens: Some(32),
            temperature: Some(0.0),
            stream: false,
        };
        assert!(matches!(
            client
                .chat(
                    &descriptor(ProviderKind::NvidiaNim, Locality::Local),
                    PrivacyGrade::Restricted,
                    &request
                )
                .await,
            Err(RuntimeError::HttpStatus(StatusCode::TEMPORARY_REDIRECT))
        ));
        server.await.unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(100), sink.accept())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn transient_rate_limit_429_is_retried_automatically() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base_url = format!("http://{}/v1", listener.local_addr().unwrap());
        tokio::spawn(async move {
            // First attempt returns 429 Too Many Requests
            if let Ok((mut socket, _)) = listener.accept().await {
                let mut buf = [0; 1024];
                let _ = socket.read(&mut buf).await;
                let _ = socket
                    .write_all(
                        b"HTTP/1.1 429 Too Many Requests\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    )
                    .await;
            }
            // Second attempt succeeds with 200 OK
            if let Ok((mut socket, _)) = listener.accept().await {
                let mut buf = [0; 1024];
                let _ = socket.read(&mut buf).await;
                let body = r#"{"data":[{"id":"test.model"}]}"#;
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = socket.write_all(response.as_bytes()).await;
            }
        });

        let endpoint = ProviderEndpoint::nvidia_nim(&base_url, Locality::Local).unwrap();
        let client = InferenceClient::new(endpoint).unwrap();
        let models = client
            .models()
            .await
            .expect("retry should recover on second attempt");
        assert_eq!(models.data.len(), 1);
    }
}
