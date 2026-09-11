//! The streamable HTTP transport.
//!
//! # This is the network boundary, so it checks again
//!
//! [`crate::McpGate`] already decided this call may happen. This module verifies the
//! parts of that decision that are about *where the bytes go* a second time, for the
//! same reason `archie-model-runtime` does: the gate ran against a manifest, and
//! between then and now the only thing that matters is the socket. A transport that
//! trusts its caller is one refactor away from being handed a URL nobody checked.
//!
//! # Credentials are brokered per call, not held
//!
//! Every method takes an `Option<SealedCredential>` and consumes it. A
//! [`archie_vault::SealedCredential`] cannot be cloned or stored, so there is no way to
//! obtain one early and keep it: the caller must go to the vault's broker immediately
//! before sending, which is the whole point of the broker existing. The credential is
//! written into an `Authorization` header and into nothing else — not the error types,
//! not `Debug`, not the trace.
//!
//! # Both response shapes
//!
//! Streamable HTTP servers answer a single JSON-RPC request either as
//! `application/json` or as a `text/event-stream` carrying one message. Supporting only
//! the first works against most servers and fails against some, which is the worst
//! version of this to discover during a demo. Both are parsed, and both are bounded.

use crate::gate::AuthorizedCall;
use crate::manifest::{McpServerManifest, McpTransport};
use crate::protocol::{
    limits, methods, CallToolParams, CallToolResult, InitializeParams, InitializeResult,
    JsonRpcRequest, JsonRpcResponse, ProtocolError, ReadResourceParams, ResourcesListResult,
    ToolsListResult, PROTOCOL_VERSION,
};
use archie_types::Locality;
use archie_vault::SealedCredential;
use futures_util::StreamExt;
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, CONTENT_TYPE};
use reqwest::{Client, Url};
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;

/// The header a streamable HTTP server uses to bind subsequent requests to a session.
const SESSION_HEADER: &str = "Mcp-Session-Id";
/// The header the spec requires a client to echo after initialising.
const VERSION_HEADER: &str = "MCP-Protocol-Version";

/// One MCP server, reachable.
pub struct HttpTransport {
    client: Client,
    url: Url,
    locality: Locality,
    max_response_bytes: usize,
    /// Assigned by the server during `initialize`, echoed on everything after.
    session_id: Mutex<Option<String>>,
    initialised: Mutex<bool>,
    next_id: AtomicU64,
}

impl std::fmt::Debug for HttpTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpTransport")
            .field("url", &self.url.as_str())
            .field("locality", &self.locality)
            .field("max_response_bytes", &self.max_response_bytes)
            .finish_non_exhaustive()
    }
}

impl HttpTransport {
    /// Open a transport for a manifest that describes an HTTP server.
    ///
    /// Validates the manifest rather than assuming a registered one: this is a public
    /// constructor, and "it must already have been validated" is not a guarantee a
    /// public constructor gets to make.
    pub fn connect(manifest: &McpServerManifest) -> Result<Self, TransportError> {
        manifest
            .validate()
            .map_err(|e| TransportError::Manifest(e.to_string()))?;
        let url = match &manifest.transport {
            McpTransport::StreamableHttp { url } => {
                Url::parse(url).map_err(|_| TransportError::InvalidUrl)?
            }
            McpTransport::Stdio { .. } => return Err(TransportError::NotHttp),
        };
        if url.scheme() != "https" && manifest.locality() != Locality::Local {
            // Defence in depth: validate() refuses this, and so does the thing that
            // would actually put arguments on a wire.
            return Err(TransportError::InsecureEndpoint);
        }

        let client = Client::builder()
            .timeout(Duration::from_millis(manifest.timeout_ms as u64))
            .build()
            .map_err(TransportError::Http)?;

        Ok(Self {
            client,
            url,
            locality: manifest.locality(),
            max_response_bytes: manifest.max_response_bytes as usize,
            session_id: Mutex::new(None),
            initialised: Mutex::new(false),
            next_id: AtomicU64::new(1),
        })
    }

    pub fn url(&self) -> &Url {
        &self.url
    }

    pub fn locality(&self) -> Locality {
        self.locality
    }

    pub fn session_id(&self) -> Option<String> {
        self.session_id.lock().ok().and_then(|id| id.clone())
    }

    /// Negotiate the protocol revision and capture the session.
    ///
    /// A server on another revision is refused rather than talked to hopefully: the
    /// payload shapes this crate parses are the ones that revision defines.
    pub async fn initialize(
        &self,
        credential: Option<SealedCredential>,
    ) -> Result<InitializeResult, TransportError> {
        let params = serde_json::to_value(InitializeParams::default())
            .map_err(|e| TransportError::Encoding(e.to_string()))?;
        let result: InitializeResult = self
            .request(methods::INITIALIZE, Some(params), credential, true)
            .await?;
        result.negotiated()?;
        if let Ok(mut flag) = self.initialised.lock() {
            *flag = true;
        }
        Ok(result)
    }

    /// What the server says it offers. An input to the allowlist, never an authority.
    pub async fn list_tools(
        &self,
        credential: Option<SealedCredential>,
    ) -> Result<ToolsListResult, TransportError> {
        let listing: ToolsListResult = self
            .request(methods::TOOLS_LIST, None, credential, false)
            .await?;
        listing.validate()?;
        Ok(listing)
    }

    /// Run a call the gate authorised.
    ///
    /// Takes an [`AuthorizedCall`], which has no public constructor, so this function
    /// cannot be reached with a tool call a model merely proposed.
    pub async fn call_tool(
        &self,
        call: &AuthorizedCall,
        credential: Option<SealedCredential>,
    ) -> Result<CallToolResult, TransportError> {
        let params = serde_json::to_value(CallToolParams {
            name: call.tool().to_string(),
            arguments: call.arguments().clone(),
        })
        .map_err(|e| TransportError::Encoding(e.to_string()))?;

        // The call carries its own budget, which may be tighter than the manifest's.
        let budget = (call.max_response_bytes() as usize).min(self.max_response_bytes);
        let result: CallToolResult = self
            .request_bounded(methods::TOOLS_CALL, Some(params), credential, false, budget)
            .await?;
        result.validate()?;
        Ok(result)
    }

    pub async fn list_resources(
        &self,
        credential: Option<SealedCredential>,
    ) -> Result<ResourcesListResult, TransportError> {
        let listing: ResourcesListResult = self
            .request(methods::RESOURCES_LIST, None, credential, false)
            .await?;
        listing.validate()?;
        Ok(listing)
    }

    pub async fn read_resource(
        &self,
        uri: &str,
        credential: Option<SealedCredential>,
    ) -> Result<Value, TransportError> {
        let params = serde_json::to_value(ReadResourceParams {
            uri: uri.to_string(),
        })
        .map_err(|e| TransportError::Encoding(e.to_string()))?;
        self.request(methods::RESOURCES_READ, Some(params), credential, false)
            .await
    }

    async fn request<T: DeserializeOwned>(
        &self,
        method: &str,
        params: Option<Value>,
        credential: Option<SealedCredential>,
        initialising: bool,
    ) -> Result<T, TransportError> {
        self.request_bounded(
            method,
            params,
            credential,
            initialising,
            self.max_response_bytes,
        )
        .await
    }

    async fn request_bounded<T: DeserializeOwned>(
        &self,
        method: &str,
        params: Option<Value>,
        credential: Option<SealedCredential>,
        initialising: bool,
        budget: usize,
    ) -> Result<T, TransportError> {
        if !initialising && !self.is_initialised() {
            // Calling before negotiation would mean parsing payloads under a revision
            // nobody agreed on.
            return Err(TransportError::NotInitialised);
        }

        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let envelope = JsonRpcRequest::new(id, method, params);

        let mut request = self
            .client
            .post(self.url.clone())
            .headers(self.headers())
            .json(&envelope);
        if let Some(credential) = credential {
            // Consumed here and nowhere else. `expose` takes ownership, so this is the
            // only place the bytes exist outside the vault.
            request = request.bearer_auth(credential.expose().as_str());
        }

        let response = request.send().await.map_err(TransportError::Http)?;
        let status = response.status();
        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_ascii_lowercase();

        if let Some(session) = response
            .headers()
            .get(SESSION_HEADER)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string)
        {
            if let Ok(mut held) = self.session_id.lock() {
                *held = Some(session);
            }
        }

        let body = self.bounded_body(response, budget).await?;
        if !status.is_success() {
            return Err(TransportError::Status {
                status: status.as_u16(),
                // Bounded, and never the request body — a server that echoes the
                // request would otherwise put arguments into an error string.
                detail: String::from_utf8_lossy(&body[..body.len().min(512)]).to_string(),
            });
        }

        let envelope: JsonRpcResponse = if content_type.starts_with("text/event-stream") {
            parse_event_stream(&body, id)?
        } else {
            serde_json::from_slice(&body).map_err(|e| TransportError::Decoding(e.to_string()))?
        };

        let result = envelope.result_for(id)?;
        serde_json::from_value(result.clone()).map_err(|e| TransportError::Decoding(e.to_string()))
    }

    fn is_initialised(&self) -> bool {
        self.initialised.lock().map(|f| *f).unwrap_or(false)
    }

    fn headers(&self) -> HeaderMap {
        let mut headers = HeaderMap::new();
        // Streamable HTTP: the server chooses which of the two it answers with.
        headers.insert(
            ACCEPT,
            HeaderValue::from_static("application/json, text/event-stream"),
        );
        if let Ok(value) = HeaderValue::from_str(PROTOCOL_VERSION) {
            headers.insert(VERSION_HEADER, value);
        }
        if let Some(session) = self.session_id() {
            if let Ok(value) = HeaderValue::from_str(&session) {
                headers.insert(SESSION_HEADER, value);
            }
        }
        headers
    }

    /// Collect a response without letting a foreign server choose the allocation.
    ///
    /// Content-Length is checked first so that an over-large response costs one header
    /// read rather than a download, and the running total is checked anyway because a
    /// chunked response does not have to declare one.
    async fn bounded_body(
        &self,
        response: reqwest::Response,
        budget: usize,
    ) -> Result<Vec<u8>, TransportError> {
        if response
            .content_length()
            .is_some_and(|length| length > budget as u64)
        {
            return Err(TransportError::ResponseTooLarge { budget });
        }
        let mut stream = response.bytes_stream();
        let mut output = Vec::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(TransportError::Http)?;
            if output.len().saturating_add(chunk.len()) > budget {
                return Err(TransportError::ResponseTooLarge { budget });
            }
            output.extend_from_slice(&chunk);
        }
        Ok(output)
    }
}

/// Pull the JSON-RPC response for `id` out of an SSE body.
///
/// Frames are separated by a blank line; `data:` lines within a frame concatenate. A
/// frame that is not the answer to this request — a server-initiated notification, say —
/// is skipped rather than mistaken for one.
fn parse_event_stream(body: &[u8], expected_id: u64) -> Result<JsonRpcResponse, TransportError> {
    let text =
        std::str::from_utf8(body).map_err(|_| TransportError::Decoding("not utf-8".into()))?;
    let mut frames = 0usize;
    for frame in text.split("\n\n") {
        frames += 1;
        if frames > limits::MAX_CONTENT_BLOCKS {
            return Err(TransportError::Decoding(
                "event stream carried too many frames".into(),
            ));
        }
        let payload: String = frame
            .lines()
            .filter_map(|line| line.strip_prefix("data:"))
            .map(|value| value.strip_prefix(' ').unwrap_or(value))
            .collect::<Vec<_>>()
            .join("\n");
        if payload.trim().is_empty() {
            continue;
        }
        if let Ok(envelope) = serde_json::from_str::<JsonRpcResponse>(&payload) {
            if envelope.id == expected_id {
                return Ok(envelope);
            }
        }
    }
    Err(TransportError::NoAnswer { expected_id })
}

#[derive(Debug, thiserror::Error)]
pub enum TransportError {
    #[error("manifest is not usable: {0}")]
    Manifest(String),
    #[error("this transport carries streamable HTTP, not stdio")]
    NotHttp,
    #[error("server url is not a valid URL")]
    InvalidUrl,
    #[error("a non-loopback server must be reached over https")]
    InsecureEndpoint,
    #[error("initialize must succeed before any other request")]
    NotInitialised,
    #[error("http error: {0}")]
    Http(#[source] reqwest::Error),
    #[error("server returned {status}: {detail}")]
    Status { status: u16, detail: String },
    #[error("response exceeded the {budget} byte budget")]
    ResponseTooLarge { budget: usize },
    #[error("could not encode request: {0}")]
    Encoding(String),
    #[error("could not decode response: {0}")]
    Decoding(String),
    #[error("event stream carried no answer to request {expected_id}")]
    NoAnswer { expected_id: u64 },
    #[error(transparent)]
    Protocol(#[from] ProtocolError),
}
