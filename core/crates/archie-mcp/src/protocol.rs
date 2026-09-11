//! The MCP wire format: JSON-RPC 2.0 envelopes and the method payloads Archie uses.
//!
//! Transport-free on purpose. These types are the part that has to be exactly right,
//! and they are testable without a process, a socket or a server — so they are tested
//! that way, and the transports that will carry them do not get to be the reason a
//! parsing rule went unexercised.
//!
//! # Everything inbound is bounded
//!
//! A server is a foreign program. Its `tools/list` can name ten thousand tools, its
//! results can be arbitrarily long, and a client that treats either as trustworthy
//! turns an external process into a local memory-exhaustion primitive. Every inbound
//! collection here has a ceiling, and exceeding it is an error rather than a truncation.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The protocol revision this client speaks.
pub const PROTOCOL_VERSION: &str = "2025-06-18";

/// Ceilings on what a server may send back.
pub mod limits {
    pub const MAX_TOOLS: usize = 256;
    pub const MAX_RESOURCES: usize = 1024;
    pub const MAX_CONTENT_BLOCKS: usize = 64;
    pub const MAX_TOOL_NAME_LEN: usize = 128;
    pub const MAX_TEXT_LEN: usize = 262_144;
}

/// A JSON-RPC 2.0 request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: u64,
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

impl JsonRpcRequest {
    pub fn new(id: u64, method: impl Into<String>, params: Option<Value>) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id,
            method: method.into(),
            params,
        }
    }
}

/// A JSON-RPC 2.0 response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl JsonRpcResponse {
    /// The result, having checked that this response answers `expected_id` and is not
    /// an error.
    ///
    /// The id check is not ceremony: a response correlated to the wrong request would
    /// hand one task's tool output to another, and on a multiplexed transport that is a
    /// live possibility rather than a theoretical one.
    pub fn result_for(&self, expected_id: u64) -> Result<&Value, ProtocolError> {
        if self.jsonrpc != "2.0" {
            return Err(ProtocolError::Version(self.jsonrpc.clone()));
        }
        if self.id != expected_id {
            return Err(ProtocolError::MismatchedId {
                expected: expected_id,
                received: self.id,
            });
        }
        if let Some(error) = &self.error {
            return Err(ProtocolError::Server {
                code: error.code,
                message: error.message.clone(),
            });
        }
        self.result.as_ref().ok_or(ProtocolError::EmptyResult)
    }
}

/// What the client tells the server about itself during `initialize`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InitializeParams {
    #[serde(rename = "protocolVersion")]
    pub protocol_version: String,
    pub capabilities: ClientCapabilities,
    #[serde(rename = "clientInfo")]
    pub client_info: Implementation,
}

impl Default for InitializeParams {
    fn default() -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION.into(),
            capabilities: ClientCapabilities::default(),
            client_info: Implementation {
                name: "archie".into(),
                version: env!("CARGO_PKG_VERSION").into(),
            },
        }
    }
}

/// Deliberately minimal: Archie advertises no sampling capability.
///
/// `sampling` would let a server ask Archie's models to generate text on its behalf.
/// That inverts the authority relationship this crate exists to hold — the server would
/// be driving inference rather than answering a call — so it is not offered.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ClientCapabilities {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub roots: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Implementation {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InitializeResult {
    #[serde(rename = "protocolVersion")]
    pub protocol_version: String,
    #[serde(default)]
    pub capabilities: Value,
    #[serde(rename = "serverInfo", default)]
    pub server_info: Option<Implementation>,
}

impl InitializeResult {
    /// Whether the server speaks a revision this client can talk to.
    pub fn negotiated(&self) -> Result<(), ProtocolError> {
        if self.protocol_version == PROTOCOL_VERSION {
            Ok(())
        } else {
            Err(ProtocolError::UnsupportedVersion(
                self.protocol_version.clone(),
            ))
        }
    }
}

/// One tool, as the server describes itself.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolDescriptor {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(rename = "inputSchema", default)]
    pub input_schema: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolsListResult {
    pub tools: Vec<ToolDescriptor>,
    #[serde(rename = "nextCursor", default)]
    pub next_cursor: Option<String>,
}

impl ToolsListResult {
    pub fn validate(&self) -> Result<(), ProtocolError> {
        if self.tools.len() > limits::MAX_TOOLS {
            return Err(ProtocolError::TooMany {
                what: "tools",
                limit: limits::MAX_TOOLS,
            });
        }
        for tool in &self.tools {
            if tool.name.is_empty() || tool.name.len() > limits::MAX_TOOL_NAME_LEN {
                return Err(ProtocolError::ToolName);
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CallToolParams {
    pub name: String,
    #[serde(default)]
    pub arguments: Value,
}

/// One block of a tool's output.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Content {
    Text { text: String },
    Image { data: String, mime_type: String },
    Resource { resource: Value },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CallToolResult {
    #[serde(default)]
    pub content: Vec<Content>,
    /// The server reporting that the *tool* failed, as distinct from the call failing.
    #[serde(rename = "isError", default)]
    pub is_error: bool,
}

impl CallToolResult {
    pub fn validate(&self) -> Result<(), ProtocolError> {
        if self.content.len() > limits::MAX_CONTENT_BLOCKS {
            return Err(ProtocolError::TooMany {
                what: "content blocks",
                limit: limits::MAX_CONTENT_BLOCKS,
            });
        }
        for block in &self.content {
            if let Content::Text { text } = block {
                if text.len() > limits::MAX_TEXT_LEN {
                    return Err(ProtocolError::TooMany {
                        what: "text bytes",
                        limit: limits::MAX_TEXT_LEN,
                    });
                }
            }
        }
        Ok(())
    }

    /// The text blocks joined, for handing to a model.
    pub fn text(&self) -> String {
        self.content
            .iter()
            .filter_map(|c| match c {
                Content::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResourceDescriptor {
    pub uri: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(rename = "mimeType", default)]
    pub mime_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResourcesListResult {
    pub resources: Vec<ResourceDescriptor>,
    #[serde(rename = "nextCursor", default)]
    pub next_cursor: Option<String>,
}

impl ResourcesListResult {
    pub fn validate(&self) -> Result<(), ProtocolError> {
        if self.resources.len() > limits::MAX_RESOURCES {
            return Err(ProtocolError::TooMany {
                what: "resources",
                limit: limits::MAX_RESOURCES,
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReadResourceParams {
    pub uri: String,
}

/// The methods Archie calls. Not an exhaustive list of the protocol's.
pub mod methods {
    pub const INITIALIZE: &str = "initialize";
    pub const TOOLS_LIST: &str = "tools/list";
    pub const TOOLS_CALL: &str = "tools/call";
    pub const RESOURCES_LIST: &str = "resources/list";
    pub const RESOURCES_READ: &str = "resources/read";
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ProtocolError {
    #[error("expected jsonrpc 2.0, got {0:?}")]
    Version(String),
    #[error("response id {received} does not answer request {expected}")]
    MismatchedId { expected: u64, received: u64 },
    #[error("server error {code}: {message}")]
    Server { code: i64, message: String },
    #[error("response carried neither a result nor an error")]
    EmptyResult,
    #[error("server speaks protocol {0}, which this client does not")]
    UnsupportedVersion(String),
    #[error("server returned more {what} than the {limit} permitted")]
    TooMany { what: &'static str, limit: usize },
    #[error("tool name is empty or over-long")]
    ToolName,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_response_for_another_request_is_refused() {
        let response = JsonRpcResponse {
            jsonrpc: "2.0".into(),
            id: 7,
            result: Some(json!({"ok": true})),
            error: None,
        };
        assert!(response.result_for(7).is_ok());
        assert_eq!(
            response.result_for(8).unwrap_err(),
            ProtocolError::MismatchedId {
                expected: 8,
                received: 7
            }
        );
    }

    #[test]
    fn a_server_error_is_an_error_not_an_empty_result() {
        let response = JsonRpcResponse {
            jsonrpc: "2.0".into(),
            id: 1,
            result: None,
            error: Some(JsonRpcError {
                code: -32601,
                message: "method not found".into(),
                data: None,
            }),
        };
        assert!(matches!(
            response.result_for(1).unwrap_err(),
            ProtocolError::Server { code: -32601, .. }
        ));
    }

    #[test]
    fn a_tool_call_serialises_to_the_wire_shape_the_spec_describes() {
        let request = JsonRpcRequest::new(
            1,
            methods::TOOLS_CALL,
            Some(json!({"name": "create_issue", "arguments": {"title": "x"}})),
        );
        let text = serde_json::to_string(&request).unwrap();
        assert!(text.contains(r#""jsonrpc":"2.0""#));
        assert!(text.contains(r#""method":"tools/call""#));
        assert!(text.contains(r#""name":"create_issue""#));
    }

    #[test]
    fn initialize_advertises_no_sampling_capability() {
        // A sampling grant would let the server drive Archie's models. It is absent by
        // construction rather than by omission, so this test fails if it is ever added.
        let text = serde_json::to_string(&InitializeParams::default()).unwrap();
        assert!(!text.contains("sampling"));
        assert!(text.contains(PROTOCOL_VERSION));
    }

    #[test]
    fn a_server_on_another_protocol_revision_is_refused() {
        let result = InitializeResult {
            protocol_version: "1999-01-01".into(),
            capabilities: json!({}),
            server_info: None,
        };
        assert!(matches!(
            result.negotiated().unwrap_err(),
            ProtocolError::UnsupportedVersion(_)
        ));
    }

    #[test]
    fn an_unbounded_tool_list_is_refused_rather_than_truncated() {
        let listing = ToolsListResult {
            tools: (0..limits::MAX_TOOLS + 1)
                .map(|i| ToolDescriptor {
                    name: format!("tool_{i}"),
                    description: None,
                    input_schema: None,
                })
                .collect(),
            next_cursor: None,
        };
        assert!(matches!(
            listing.validate().unwrap_err(),
            ProtocolError::TooMany { what: "tools", .. }
        ));
    }

    #[test]
    fn an_oversized_text_block_is_refused() {
        let result = CallToolResult {
            content: vec![Content::Text {
                text: "x".repeat(limits::MAX_TEXT_LEN + 1),
            }],
            is_error: false,
        };
        assert!(result.validate().is_err());
    }

    #[test]
    fn a_tool_error_is_reported_rather_than_read_as_output() {
        let result: CallToolResult = serde_json::from_value(json!({
            "content": [{"type": "text", "text": "no such repository"}],
            "isError": true
        }))
        .unwrap();
        assert!(result.is_error);
        assert_eq!(result.text(), "no such repository");
    }
}
