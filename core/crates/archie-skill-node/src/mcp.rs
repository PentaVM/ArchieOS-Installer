//! The Model Context Protocol over stdio, as much of it as a tool server needs: initialize,
//! the tool list, tool calls and ping. An agent reaches it through SSH (`ssh person@machine
//! archie-skill-node mcp`), so the node opens no port of its own.
//!
//! This is not archie-mcp-server's server on purpose: a GPU machine that trains skills
//! should build this binary without the Stage, the bridges and the Archie daemon.

use crate::hub::Hub;
use crate::tools::{tools, Node};
use serde_json::{json, Map, Value};

pub const SUPPORTED_VERSIONS: [&str; 4] = ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];
pub const MAX_MESSAGE_BYTES: usize = 1024 * 1024;

/// One JSON-RPC message in, at most one out (notifications get none).
pub fn handle<H: Hub>(node: &mut Node<H>, line: &str) -> Option<String> {
    let message: Value = match serde_json::from_str(line) {
        Ok(value) => value,
        Err(_) => return Some(error(Value::Null, -32700, "not JSON")),
    };
    let id = message.get("id").cloned();
    let method = message.get("method").and_then(Value::as_str).unwrap_or("");
    let id = id?; // a notification: nothing is answered
    let result = match method {
        "initialize" => {
            let asked = message.pointer("/params/protocolVersion").and_then(Value::as_str).unwrap_or("");
            let version = SUPPORTED_VERSIONS.iter().find(|v| **v == asked).copied().unwrap_or(SUPPORTED_VERSIONS[0]);
            json!({"protocolVersion": version, "capabilities": {"tools": {}},
                   "serverInfo": {"name": "archie-skill-node", "version": env!("CARGO_PKG_VERSION")},
                   "instructions": "An IRIS skill node: one person's GPU training LoRA skill adapters for the shared \
                                    board. Start with skill_board, pick the most needed work this node can do, claim \
                                    it, train or check it, and publish or record the result. Credits go to this node's \
                                    person."})
        }
        "ping" => json!({}),
        "tools/list" => json!({"tools": tools().into_iter().map(|tool| json!({
            "name": tool.name, "description": tool.description, "inputSchema": tool.schema})).collect::<Vec<_>>()}),
        "tools/call" => {
            let name = message.pointer("/params/name").and_then(Value::as_str).unwrap_or("");
            if !tools().iter().any(|tool| tool.name == name) {
                return Some(error(id, -32602, &format!("unknown tool {name}")));
            }
            let arguments = message.pointer("/params/arguments").and_then(Value::as_object).cloned().unwrap_or_else(Map::new);
            match node.call(name, &arguments) {
                Ok(value) => json!({"content": [{"type": "text", "text": serde_json::to_string_pretty(&value).unwrap_or_default()}],
                                    "isError": false}),
                Err(message) => json!({"content": [{"type": "text", "text": message}], "isError": true}),
            }
        }
        _ => return Some(error(id, -32601, &format!("method {method} is not offered"))),
    };
    Some(json!({"jsonrpc": "2.0", "id": id, "result": result}).to_string())
}

fn error(id: Value, code: i64, message: &str) -> String {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}}).to_string()
}
