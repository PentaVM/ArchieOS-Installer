//! The MCP side: JSON-RPC 2.0 messages in, replies out, one line each.
//!
//! Transport-free, so every rule here is tested without a process. The binary feeds it
//! lines from stdin and writes its replies to stdout.

use crate::toolbox::{Clearance, Effect, ToolSpec, Toolbox};
use base64::Engine as _;
use serde_json::{json, Map, Value};

/// Protocol revisions this server speaks, newest first. The tools it uses — `tools/list`
/// and `tools/call` with text and image content — mean the same in all of them.
pub const SUPPORTED_VERSIONS: [&str; 4] = ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

/// The largest message read. Anything longer is refused rather than parsed.
pub const MAX_MESSAGE_BYTES: usize = 1024 * 1024;

/// The largest tool result returned. A bigger one is an error, not a truncation: cut
/// JSON is a different document.
pub const MAX_RESULT_BYTES: usize = 512 * 1024;

/// The most image bytes one result carries, before base64.
pub const MAX_IMAGE_BYTES: usize = 768 * 1024;

const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;
const NOT_INITIALIZED: i64 = -32002;

const INSTRUCTIONS: &str = "Archie is the agent of an ArchieOS node. These tools read the node's state and, when \
the operator has allowed it, operate applications on the Archie Stage. Actions go through the node's policy and \
a person can take the stage back at any moment; a refusal is an answer, not an error to work around. Results \
never contain more than this server's clearance allows. Call archie_contract once before anything else: it says \
who does what, and how to hand work on the screen to IRIS, the node's own eyes and hands.";

pub struct McpServer<T> {
    toolbox: T,
    clearance: Clearance,
    protocol: Option<&'static str>,
}

impl<T: Toolbox> McpServer<T> {
    pub fn new(toolbox: T, clearance: Clearance) -> Self {
        Self {
            toolbox,
            clearance,
            protocol: None,
        }
    }

    /// The protocol revision agreed with the client, once it has initialised.
    pub fn protocol_version(&self) -> Option<&'static str> {
        self.protocol
    }

    /// One inbound message line. Returns the reply line, or nothing when the message
    /// wants none (a notification, or a response to a request this server never sends).
    pub async fn handle(&mut self, line: &str) -> Option<String> {
        if line.len() > MAX_MESSAGE_BYTES {
            return Some(error(Value::Null, INVALID_REQUEST, "message too large"));
        }
        let Ok(message) = serde_json::from_str::<Value>(line) else {
            return Some(error(Value::Null, PARSE_ERROR, "not JSON"));
        };
        let Value::Object(message) = message else {
            return Some(error(
                Value::Null,
                INVALID_REQUEST,
                "one JSON-RPC object per line; batches are not accepted",
            ));
        };
        let id = message.get("id").cloned();
        if let Some(id) = &id {
            if !(id.is_string() || id.is_i64() || id.is_u64()) {
                return Some(error(
                    Value::Null,
                    INVALID_REQUEST,
                    "an id is a string or an integer",
                ));
            }
        }
        if message.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
            return id.map(|id| error(id, INVALID_REQUEST, "jsonrpc must be \"2.0\""));
        }
        // No method: a response to a request this server never sends. No id: a notification,
        // such as notifications/initialized. Neither gets a reply.
        let method = message.get("method").and_then(Value::as_str)?;
        let id = id?;
        let params = message.get("params").cloned().unwrap_or(Value::Null);
        Some(match self.dispatch(method, params).await {
            Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}).to_string(),
            Err((code, message)) => error(id, code, message),
        })
    }

    async fn dispatch(
        &mut self,
        method: &str,
        params: Value,
    ) -> Result<Value, (i64, &'static str)> {
        match method {
            "initialize" => Ok(self.initialize(&params)),
            "ping" => Ok(json!({})),
            _ if self.protocol.is_none() => Err((NOT_INITIALIZED, "send initialize first")),
            "tools/list" => Ok(self.list()),
            "tools/call" => self.call(params).await,
            // The method name is not echoed: it is whatever the client sent, at any length.
            _ => Err((METHOD_NOT_FOUND, "method not found")),
        }
    }

    fn initialize(&mut self, params: &Value) -> Value {
        let requested = params.get("protocolVersion").and_then(Value::as_str);
        let version = SUPPORTED_VERSIONS
            .iter()
            .copied()
            .find(|v| Some(*v) == requested)
            .unwrap_or(SUPPORTED_VERSIONS[0]);
        self.protocol = Some(version);
        json!({
            "protocolVersion": version,
            "capabilities": {"tools": {"listChanged": false}},
            "serverInfo": {"name": "archie", "title": "Archie", "version": env!("CARGO_PKG_VERSION")},
            "instructions": INSTRUCTIONS,
        })
    }

    /// The tools within clearance. One above it does not exist, as far as a client can tell.
    fn visible(&self) -> Vec<ToolSpec> {
        self.toolbox
            .tools()
            .into_iter()
            .filter(|tool| self.clearance.covers(tool.grade))
            .collect()
    }

    fn list(&self) -> Value {
        let tools: Vec<Value> = self
            .visible()
            .iter()
            .map(|tool| {
                json!({
                    "name": tool.name,
                    "title": tool.title,
                    "description": tool.description,
                    "inputSchema": tool.input_schema,
                    "annotations": {
                        "title": tool.title,
                        "readOnlyHint": tool.effect == Effect::ReadOnly,
                        "destructiveHint": tool.effect == Effect::Acts,
                        "idempotentHint": tool.effect == Effect::ReadOnly,
                        "openWorldHint": false,
                    },
                })
            })
            .collect();
        json!({"tools": tools})
    }

    async fn call(&mut self, params: Value) -> Result<Value, (i64, &'static str)> {
        let Value::Object(mut params) = params else {
            return Err((INVALID_PARAMS, "tools/call takes an object"));
        };
        let Some(Value::String(name)) = params.remove("name") else {
            return Err((INVALID_PARAMS, "tools/call needs a tool name"));
        };
        let arguments = match params.remove("arguments") {
            None | Some(Value::Null) => Map::new(),
            Some(Value::Object(arguments)) => arguments,
            Some(_) => return Err((INVALID_PARAMS, "arguments must be an object")),
        };
        if !self.visible().iter().any(|tool| tool.name == name) {
            return Err((INVALID_PARAMS, "unknown tool"));
        }
        let outcome = self.toolbox.call(&name, arguments).await;
        let image_bytes: usize = outcome.images.iter().map(|i| i.data.len()).sum();
        if outcome.text.len() > MAX_RESULT_BYTES || image_bytes > MAX_IMAGE_BYTES {
            return Ok(json!({
                "content": [{"type": "text", "text": "the result was larger than this server returns"}],
                "isError": true,
            }));
        }
        let mut content = vec![json!({"type": "text", "text": outcome.text})];
        for image in &outcome.images {
            content.push(json!({
                "type": "image",
                "mimeType": image.mime,
                "data": base64::engine::general_purpose::STANDARD.encode(&image.data),
            }));
        }
        Ok(json!({"content": content, "isError": outcome.is_error}))
    }
}

fn error(id: Value, code: i64, message: &str) -> String {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}}).to_string()
}
