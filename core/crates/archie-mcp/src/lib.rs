//! # archie-mcp
//!
//! Model Context Protocol, entered through the capability model rather than around it.
//!
//! # The shape of the rule
//!
//! MCP's usual integration is: give the model a tool list, let it emit a tool call, run
//! it. That is a fine design for a chat application and the wrong one for a system that
//! can reach an operator's accounts and devices, because it makes a language model's
//! output the authorising step.
//!
//! Here the path is
//!
//! ```text
//! Cerebra → capability registry → policy → MCP client → server
//! ```
//!
//! and it is enforced by construction: [`AuthorizedCall`] has no public constructor, so
//! a transport cannot be handed anything the gate did not produce. A model's suggestion
//! is a [`ToolProposal`], which nothing executes.
//!
//! # Three layers
//!
//! * [`manifest`] — what the operator declared. A server that cannot be described here
//!   is one Archie will not call, and `tools/list` is the server describing itself
//!   rather than granting itself anything.
//! * [`protocol`] — JSON-RPC 2.0 and the MCP payloads, transport-free and bounded, so
//!   that a foreign process cannot become a memory-exhaustion primitive.
//! * [`gate`] — admission, and the receipt that may be written down about it.
//!
//! # Credentials
//!
//! A server's manifest names a [`archie_vault::VaultRef`], never a credential.
//! [`AuthorizedCall::credential_ref`] carries that name to the transport, which brokers
//! it at the moment of sending. Nothing in this crate can produce the secret itself.
//!
//! * [`transport`] — streamable HTTP, which re-checks the parts of the gate's decision
//!   that are about where bytes go, and brokers a credential per call.
//!
//! # Not yet here
//!
//! The `stdio` transport. These types carry over it unchanged, and this crate ships no
//! stub resembling one — a fake transport returning plausible results is
//! indistinguishable from a working integration right up until the demo.

#![forbid(unsafe_code)]

pub mod gate;
pub mod manifest;
pub mod protocol;
pub mod transport;

pub use gate::{
    Admission, AuthorizedCall, CallReceipt, CallerAuthority, McpGate, Refusal, ToolProposal,
    MAX_ARGUMENT_BYTES,
};
pub use manifest::{ManifestError, McpRegistry, McpServerManifest, McpTransport, ToolAllowlist};
pub use protocol::{
    limits, methods, CallToolParams, CallToolResult, ClientCapabilities, Content, Implementation,
    InitializeParams, InitializeResult, JsonRpcError, JsonRpcRequest, JsonRpcResponse,
    ProtocolError, ReadResourceParams, ResourceDescriptor, ResourcesListResult, ToolDescriptor,
    ToolsListResult, PROTOCOL_VERSION,
};
pub use transport::{HttpTransport, TransportError};
