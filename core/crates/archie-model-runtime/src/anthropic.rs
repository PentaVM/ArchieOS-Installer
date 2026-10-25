//! Native Messages API mapping. No subscription tokens or account storage are read.
use crate::stream::{SseError, MAX_EVENT_LINE};
use crate::{
    ChatChoice, ChatMessage, ChatRequest, ChatResponse, MessageRole, RuntimeError, StreamEvent,
    TokenUsage, ToolCall, ToolChatMessage, ToolChatRequest,
};
use serde_json::{json, Value};
use std::collections::BTreeMap;

pub(crate) fn chat_body(request: &ChatRequest) -> Result<Value, RuntimeError> {
    let mut system = Vec::new();
    let mut messages = Vec::new();
    for message in &request.messages {
        match message.role {
            MessageRole::System => system.push(message.content.as_str()),
            MessageRole::User => messages.push(json!({"role":"user","content":message.content})),
            MessageRole::Assistant => {
                messages.push(json!({"role":"assistant","content":message.content}))
            }
        }
    }
    if messages.is_empty() {
        return Err(RuntimeError::InvalidRequest);
    }
    let mut body = json!({"model":request.model,"messages":messages,"max_tokens":request.max_tokens.unwrap_or(1024),"stream":false});
    if !system.is_empty() {
        body["system"] = json!(system.join("\n\n"));
    }
    if let Some(temperature) = request.temperature {
        body["temperature"] = json!(temperature.min(1.0));
    }
    Ok(body)
}

pub(crate) fn tool_body(request: &ToolChatRequest) -> Result<Value, RuntimeError> {
    let mut system = Vec::new();
    let mut messages = Vec::new();
    for message in &request.messages {
        match message {
            ToolChatMessage::System(text) => system.push(text.as_str()),
            ToolChatMessage::User(text) => messages.push(json!({"role":"user","content":text})),
            // Not a model that hears: what was said, as text.
            ToolChatMessage::UserHeard { text, .. } => {
                messages.push(json!({"role":"user","content":text}))
            }
            ToolChatMessage::Assistant {
                content,
                tool_calls,
            } => {
                let mut blocks = Vec::new();
                if let Some(text) = content {
                    blocks.push(json!({"type":"text","text":text}));
                }
                for call in tool_calls {
                    let input: Value = serde_json::from_str(&call.arguments)?;
                    if !input.is_object() || call.id.is_empty() || call.name.is_empty() {
                        return Err(RuntimeError::InvalidRequest);
                    }
                    blocks.push(
                        json!({"type":"tool_use","id":call.id,"name":call.name,"input":input}),
                    );
                }
                if blocks.is_empty() {
                    return Err(RuntimeError::InvalidRequest);
                }
                messages.push(json!({"role":"assistant","content":blocks}));
            }
            ToolChatMessage::Tool { call_id, content } => {
                if call_id.is_empty() {
                    return Err(RuntimeError::InvalidRequest);
                }
                messages.push(json!({"role":"user","content":[{"type":"tool_result","tool_use_id":call_id,"content":content}]}));
            }
        }
    }
    if messages.is_empty() {
        return Err(RuntimeError::InvalidRequest);
    }
    let mut body = json!({"model":request.model,"messages":messages,"max_tokens":request.max_tokens,"temperature":request.temperature.min(1.0),"stream":true});
    if !system.is_empty() {
        body["system"] = json!(system.join("\n\n"));
    }
    if !request.tools.is_empty() {
        if request.tools.iter().any(|t| !t.parameters.is_object()) {
            return Err(RuntimeError::InvalidRequest);
        }
        body["tools"] = request
            .tools
            .iter()
            .map(|t| json!({"name":t.name,"description":t.description,"input_schema":t.parameters}))
            .collect();
    }
    Ok(body)
}

fn finish_reason(reason: &str) -> Result<&'static str, RuntimeError> {
    match reason {
        "end_turn" | "stop_sequence" => Ok("stop"),
        "max_tokens" | "model_context_window_exceeded" => Ok("length"),
        "tool_use" => Ok("tool_calls"),
        "refusal" => Ok("refusal"),
        _ => Err(RuntimeError::NativeProtocol(
            "unsupported or missing terminal stop reason",
        )),
    }
}

pub(crate) fn decode(bytes: &[u8]) -> Result<ChatResponse, RuntimeError> {
    let value: Value = serde_json::from_slice(bytes)?;
    if value["type"] != "message" || value["role"] != "assistant" {
        return Err(RuntimeError::NativeProtocol("expected assistant message"));
    }
    let model = value["model"]
        .as_str()
        .filter(|m| !m.is_empty())
        .ok_or(RuntimeError::NativeProtocol("missing model"))?;
    let reason = finish_reason(value["stop_reason"].as_str().unwrap_or_default())?;
    let mut text = String::new();
    for block in value["content"]
        .as_array()
        .ok_or(RuntimeError::NativeProtocol("missing content"))?
    {
        match block["type"].as_str() {
            Some("text") => text.push_str(
                block["text"]
                    .as_str()
                    .ok_or(RuntimeError::NativeProtocol("invalid text block"))?,
            ),
            Some("thinking" | "redacted_thinking") => {}
            _ => {
                return Err(RuntimeError::NativeProtocol(
                    "unsupported content in text completion",
                ))
            }
        }
    }
    if (text.is_empty() && reason != "refusal") || reason == "tool_calls" {
        return Err(RuntimeError::EmptyCompletion);
    }
    let usage = value
        .get("usage")
        .map(|usage| -> Result<TokenUsage, RuntimeError> {
            let input = usage["input_tokens"]
                .as_u64()
                .ok_or(RuntimeError::NativeProtocol("invalid input usage"))?;
            let input = input
                .saturating_add(usage["cache_read_input_tokens"].as_u64().unwrap_or(0))
                .saturating_add(usage["cache_creation_input_tokens"].as_u64().unwrap_or(0));
            let output = usage["output_tokens"]
                .as_u64()
                .ok_or(RuntimeError::NativeProtocol("invalid output usage"))?;
            Ok(TokenUsage {
                prompt_tokens: input,
                completion_tokens: output,
                total_tokens: input.saturating_add(output),
            })
        })
        .transpose()?;
    Ok(ChatResponse {
        model: model.to_string(),
        choices: vec![ChatChoice {
            message: ChatMessage {
                role: MessageRole::Assistant,
                content: text,
            },
            finish_reason: Some(reason.to_string()),
        }],
        usage,
    })
}

enum Block {
    Text,
    Hidden,
    Tool {
        call: ToolCall,
        initial: Value,
        partial: String,
    },
}

#[derive(Default)]
pub(crate) struct AnthropicSse {
    pending: Vec<u8>,
    started: bool,
    finished: bool,
    reason: Option<String>,
    blocks: BTreeMap<u64, Block>,
    calls: Vec<ToolCall>,
}

fn malformed() -> SseError {
    SseError::Malformed("invalid Anthropic stream lifecycle or content".into())
}

impl AnthropicSse {
    pub(crate) fn new() -> Self {
        Self::default()
    }
    pub(crate) fn is_finished(&self) -> bool {
        self.finished
    }

    pub(crate) fn feed(&mut self, bytes: &[u8]) -> Result<Vec<StreamEvent>, SseError> {
        let mut events = Vec::new();
        // Bound the unfinished line before copying provider-controlled bytes.
        for piece in bytes.split_inclusive(|b| *b == b'\n') {
            if self.pending.len().saturating_add(piece.len()) > MAX_EVENT_LINE {
                return Err(SseError::LineTooLong);
            }
            self.pending.extend_from_slice(piece);
            if self.pending.last() == Some(&b'\n') {
                let line = std::mem::take(&mut self.pending);
                self.line(&line, &mut events)?;
            }
        }
        Ok(events)
    }

    pub(crate) fn close(&mut self) -> Result<Vec<StreamEvent>, SseError> {
        let mut events = Vec::new();
        if !self.pending.is_empty() {
            let line = std::mem::take(&mut self.pending);
            self.line(&line, &mut events)?;
        }
        if !self.finished {
            return Err(malformed());
        }
        Ok(events)
    }

    fn line(&mut self, line: &[u8], events: &mut Vec<StreamEvent>) -> Result<(), SseError> {
        let text = std::str::from_utf8(line)
            .map_err(|_| malformed())?
            .trim_end_matches(['\r', '\n']);
        let Some(data) = text.strip_prefix("data:") else {
            return Ok(());
        };
        if self.finished {
            return Err(malformed());
        }
        let value: Value = serde_json::from_str(data.trim_start()).map_err(|_| malformed())?;
        let kind = value["type"].as_str().ok_or_else(malformed)?;
        match kind {
            "error" => {
                return Err(SseError::Malformed(
                    "Anthropic returned an error event".into(),
                ))
            }
            "ping" => {}
            "message_start" => {
                if self.started || value["message"]["role"] != "assistant" {
                    return Err(malformed());
                }
                self.started = true;
            }
            "content_block_start" => {
                if !self.started || self.reason.is_some() {
                    return Err(malformed());
                }
                let index = value["index"]
                    .as_u64()
                    .filter(|n| *n < 128)
                    .ok_or_else(malformed)?;
                if self.blocks.contains_key(&index) {
                    return Err(malformed());
                }
                let block = &value["content_block"];
                let block = match block["type"].as_str() {
                    Some("text") => {
                        let text = block["text"].as_str().ok_or_else(malformed)?;
                        if !text.is_empty() {
                            events.push(StreamEvent::Text(text.into()));
                        }
                        Block::Text
                    }
                    Some("thinking" | "redacted_thinking") => Block::Hidden,
                    Some("tool_use") => {
                        if self.calls.len() + self.blocks.len() >= 32 {
                            return Err(malformed());
                        }
                        let id = block["id"]
                            .as_str()
                            .filter(|s| !s.is_empty())
                            .ok_or_else(malformed)?;
                        let name = block["name"]
                            .as_str()
                            .filter(|s| !s.is_empty())
                            .ok_or_else(malformed)?;
                        let initial = block["input"].as_object().ok_or_else(malformed)?;
                        Block::Tool {
                            call: ToolCall {
                                id: id.into(),
                                name: name.into(),
                                arguments: String::new(),
                            },
                            initial: Value::Object(initial.clone()),
                            partial: String::new(),
                        }
                    }
                    _ => return Err(malformed()),
                };
                self.blocks.insert(index, block);
            }
            "content_block_delta" => {
                let index = value["index"].as_u64().ok_or_else(malformed)?;
                let block = self.blocks.get_mut(&index).ok_or_else(malformed)?;
                let delta = &value["delta"];
                match (block, delta["type"].as_str()) {
                    (Block::Text, Some("text_delta")) => {
                        let text = delta["text"].as_str().ok_or_else(malformed)?;
                        if !text.is_empty() {
                            events.push(StreamEvent::Text(text.into()));
                        }
                    }
                    (Block::Tool { partial, .. }, Some("input_json_delta")) => {
                        let text = delta["partial_json"].as_str().ok_or_else(malformed)?;
                        if partial.len().saturating_add(text.len()) > crate::MAX_CONTENT_BYTES {
                            return Err(SseError::LineTooLong);
                        }
                        partial.push_str(text);
                    }
                    (Block::Hidden, Some("thinking_delta" | "signature_delta")) => {}
                    _ => return Err(malformed()),
                }
            }
            "content_block_stop" => {
                let index = value["index"].as_u64().ok_or_else(malformed)?;
                if let Block::Tool {
                    mut call,
                    initial,
                    partial,
                } = self.blocks.remove(&index).ok_or_else(malformed)?
                {
                    let input = if partial.is_empty() {
                        initial
                    } else {
                        serde_json::from_str::<Value>(&partial).map_err(|_| malformed())?
                    };
                    if !input.is_object() {
                        return Err(malformed());
                    }
                    call.arguments = if partial.is_empty() {
                        input.to_string()
                    } else {
                        partial
                    };
                    if self.calls.iter().any(|c| c.id == call.id) {
                        return Err(malformed());
                    }
                    self.calls.push(call);
                }
            }
            "message_delta" => {
                if !self.started || !self.blocks.is_empty() {
                    return Err(malformed());
                }
                if let Some(reason) = value["delta"]["stop_reason"].as_str() {
                    if self.reason.is_some() {
                        return Err(malformed());
                    }
                    self.reason = Some(finish_reason(reason).map_err(|_| malformed())?.into());
                }
            }
            "message_stop" => {
                if !self.started || !self.blocks.is_empty() || self.reason.is_none() {
                    return Err(malformed());
                }
                if !self.calls.is_empty() && self.reason.as_deref() != Some("tool_calls") {
                    return Err(malformed());
                }
                events.extend(self.calls.drain(..).map(StreamEvent::ToolCall));
                events.push(StreamEvent::Finished {
                    reason: self.reason.take(),
                });
                self.finished = true;
            }
            // Upstream may add event types. They cannot establish a terminal status.
            _ => {}
        }
        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::{
        ChatMessage, ChatRequest, MessageRole, StreamEvent, ToolChatMessage, ToolChatRequest,
        ToolDefinition,
    };

    pub(crate) async fn http_fixture(
        body: &'static str,
    ) -> (
        crate::InferenceClient,
        archie_types::ModelDescriptor,
        tokio::sync::oneshot::Receiver<String>,
    ) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (sender, receiver) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            loop {
                let mut chunk = [0; 4096];
                let count = socket.read(&mut chunk).await.unwrap();
                if count == 0 {
                    break;
                }
                bytes.extend_from_slice(&chunk[..count]);
                if let Some(end) = bytes.windows(4).position(|s| s == b"\r\n\r\n") {
                    let header = String::from_utf8_lossy(&bytes[..end]);
                    let length = header
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length: ")
                                .and_then(|v| v.parse::<usize>().ok())
                        })
                        .unwrap_or(0);
                    if bytes.len() >= end + 4 + length {
                        break;
                    }
                }
            }
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            socket.write_all(response.as_bytes()).await.unwrap();
            sender.send(String::from_utf8(bytes).unwrap()).unwrap();
        });
        // Test-only HTTP endpoint construction. The public native constructor always
        // requires HTTPS + cloud locality + a credential source.
        let endpoint = crate::ProviderEndpoint {
            family: crate::ProviderFamily::Anthropic,
            base_url: reqwest::Url::parse(&format!("http://{address}/v1/")).unwrap(),
            locality: archie_types::Locality::Cloud,
            credential_env: None,
            timeout: std::time::Duration::from_secs(1),
            max_response_bytes: 8192,
        };
        let mut provider = crate::provider::builtin_provider("anthropic").unwrap();
        provider.models = vec!["m".into()];
        let model = provider.descriptors(&[]).unwrap().remove(0);
        (
            crate::InferenceClient::new(endpoint).unwrap(),
            model,
            receiver,
        )
    }

    #[tokio::test]
    async fn native_http_json_uses_messages_endpoint_and_required_version_header() {
        let (client, model, observed) = http_fixture(r#"{"type":"message","role":"assistant","model":"m","content":[{"type":"text","text":"ready"}],"stop_reason":"end_turn","usage":{"input_tokens":2,"output_tokens":1}}"#).await;
        let request = ChatRequest {
            model: "m".into(),
            messages: vec![ChatMessage {
                role: MessageRole::User,
                content: "Hello".into(),
            }],
            max_tokens: Some(128),
            temperature: Some(0.2),
            stream: false,
            reply_schema: None,
        };
        let response = client
            .chat(&model, archie_types::PrivacyGrade::Public, &request)
            .await
            .unwrap();
        assert_eq!(response.choices[0].message.content, "ready");
        let wire = observed.await.unwrap();
        assert!(wire.starts_with("POST /v1/messages HTTP/1.1\r\n"));
        assert!(wire
            .to_ascii_lowercase()
            .contains("anthropic-version: 2023-06-01"));
        assert!(!wire.contains("chat/completions"));
        let body: Value = serde_json::from_str(wire.split_once("\r\n\r\n").unwrap().1).unwrap();
        assert_eq!(body["messages"][0]["role"], "user");
        assert_eq!(body["max_tokens"], 128);
    }

    #[tokio::test]
    async fn native_http_stream_passes_only_answer_text_and_terminal_event() {
        let (client, model, observed) = http_fixture(concat!(
            "data: {\"type\":\"message_start\",\"message\":{\"role\":\"assistant\"}}\n\n",
            "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"ready\"}}\n\n",
            "data: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
            "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"}}\n\n",
            "data: {\"type\":\"message_stop\"}\n\n",
        )).await;
        let request = ToolChatRequest {
            model: "m".into(),
            messages: vec![ToolChatMessage::User("Hello".into())],
            tools: vec![],
            max_tokens: 128,
            temperature: 0.2,
        };
        let mut events = Vec::new();
        client
            .chat_stream(
                &model,
                archie_types::PrivacyGrade::Public,
                &request,
                |event| events.push(event),
            )
            .await
            .unwrap();
        assert_eq!(
            events,
            vec![
                StreamEvent::Text("ready".into()),
                StreamEvent::Finished {
                    reason: Some("stop".into())
                }
            ]
        );
        assert!(observed.await.unwrap().starts_with("POST /v1/messages "));
    }

    #[test]
    fn messages_use_anthropic_system_and_tool_result_blocks() {
        let request = ToolChatRequest {
            model: "configured-claude".into(),
            messages: vec![
                ToolChatMessage::System("Be clear".into()),
                ToolChatMessage::User("Hello".into()),
                ToolChatMessage::Tool {
                    call_id: "call1".into(),
                    content: "result".into(),
                },
            ],
            tools: vec![ToolDefinition {
                name: "lookup".into(),
                description: "Look up".into(),
                parameters: serde_json::json!({"type":"object"}),
            }],
            max_tokens: 128,
            temperature: 0.2,
        };
        let body = tool_body(&request).unwrap();
        assert_eq!(body["system"], "Be clear");
        assert_eq!(body["messages"][0]["role"], "user");
        assert_eq!(body["messages"][1]["content"][0]["type"], "tool_result");
        assert_eq!(body["messages"][1]["content"][0]["tool_use_id"], "call1");
        assert_eq!(
            body["tools"][0]["input_schema"],
            serde_json::json!({"type":"object"})
        );
        assert!(body["tools"][0].get("function").is_none());
    }

    #[test]
    fn native_json_response_maps_text_and_observed_usage() {
        let response = decode(br#"{"type":"message","role":"assistant","model":"configured-claude","content":[{"type":"thinking","thinking":"private"},{"type":"text","text":"Hello"}],"stop_reason":"end_turn","usage":{"input_tokens":7,"output_tokens":2}}"#).unwrap();
        assert_eq!(response.choices[0].message.content, "Hello");
        assert_eq!(response.choices[0].finish_reason.as_deref(), Some("stop"));
        assert_eq!(response.usage.unwrap().total_tokens, 9);
        assert!(decode(br#"{"type":"message","role":"assistant","model":"m","content":[{"type":"text","text":"partial"}],"stop_reason":null}"#).is_err());
        assert!(decode(br#"{"type":"error","error":{"message":"failure"}}"#).is_err());
    }

    #[test]
    fn refusal_json_is_a_terminal_response_with_observed_content_and_usage() {
        let response = decode(br#"{"type":"message","role":"assistant","model":"m","content":[{"type":"text","text":"I cannot help with that."}],"stop_reason":"refusal","usage":{"input_tokens":7,"output_tokens":6}}"#).unwrap();
        assert_eq!(
            response.choices[0].message.content,
            "I cannot help with that."
        );
        assert_eq!(
            response.choices[0].finish_reason.as_deref(),
            Some("refusal")
        );
        assert_eq!(response.usage.unwrap().total_tokens, 13);
        let empty = decode(br#"{"type":"message","role":"assistant","model":"m","content":[],"stop_reason":"refusal","usage":{"input_tokens":7,"output_tokens":0}}"#).unwrap();
        assert_eq!(empty.choices[0].finish_reason.as_deref(), Some("refusal"));
    }

    #[test]
    fn split_refusal_sse_keeps_the_terminal_reason_after_the_last_text_block() {
        let input = concat!(
            "data: {\"type\":\"message_start\",\"message\":{\"role\":\"assistant\"}}\n\n",
            "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"I cannot help.\"}}\n\n",
            "data: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
            "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"refusal\"}}\n\n",
            "data: {\"type\":\"message_stop\"}\n\n",
        );
        let mut parser = AnthropicSse::new();
        let mut events = Vec::new();
        for chunk in input.as_bytes().chunks(5) {
            events.extend(parser.feed(chunk).unwrap());
        }
        events.extend(parser.close().unwrap());
        assert_eq!(
            events,
            vec![
                StreamEvent::Text("I cannot help.".into()),
                StreamEvent::Finished {
                    reason: Some("refusal".into())
                }
            ]
        );
    }

    #[test]
    fn split_native_stream_emits_text_and_whole_tool_only_after_terminal_success() {
        let input = concat!(
            "data: {\"type\":\"message_start\",\"message\":{\"role\":\"assistant\"}}\n\n",
            "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
            "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hello\"}}\n\n",
            "data: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
            "data: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"tool_use\",\"id\":\"call1\",\"name\":\"lookup\",\"input\":{}}}\n\n",
            "data: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"q\\\":\\\"Chile\\\"}\"}}\n\n",
            "data: {\"type\":\"content_block_stop\",\"index\":1}\n\n",
            "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\"}}\n\n",
            "data: {\"type\":\"message_stop\"}\n\n",
        );
        let mut parser = AnthropicSse::new();
        let mut events = Vec::new();
        for bytes in input.as_bytes().chunks(7) {
            events.extend(parser.feed(bytes).unwrap());
        }
        events.extend(parser.close().unwrap());
        assert_eq!(events[0], StreamEvent::Text("Hello".into()));
        assert!(
            matches!(&events[1], StreamEvent::ToolCall(call) if call.id == "call1" && call.arguments == "{\"q\":\"Chile\"}")
        );
        assert_eq!(
            events[2],
            StreamEvent::Finished {
                reason: Some("tool_calls".into())
            }
        );
    }

    #[test]
    fn truncated_or_error_stream_never_becomes_a_successful_finish() {
        let mut parser = AnthropicSse::new();
        parser
            .feed(b"data: {\"type\":\"message_start\",\"message\":{\"role\":\"assistant\"}}\n\n")
            .unwrap();
        assert!(parser.close().is_err());
        let mut parser = AnthropicSse::new();
        assert!(parser
            .feed(b"data: {\"type\":\"error\",\"error\":{\"type\":\"overloaded_error\"}}\n\n")
            .is_err());
    }

    #[test]
    fn ordinary_chat_system_messages_never_appear_in_messages_array() {
        let request = ChatRequest {
            model: "m".into(),
            messages: vec![
                ChatMessage {
                    role: MessageRole::System,
                    content: "instructions".into(),
                },
                ChatMessage {
                    role: MessageRole::User,
                    content: "question".into(),
                },
            ],
            max_tokens: Some(12),
            temperature: None,
            stream: false,
            reply_schema: None,
        };
        let body = chat_body(&request).unwrap();
        assert_eq!(body["system"], "instructions");
        assert_eq!(body["messages"].as_array().unwrap().len(), 1);
        assert_eq!(body["max_tokens"], 12);
    }
}
