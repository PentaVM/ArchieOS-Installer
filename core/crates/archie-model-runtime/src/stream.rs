//! Answers that arrive while they are being written, and the tools a model asks for.
//!
//! A voice cannot wait for a whole answer. The first sentence has to be on its way to the
//! speaker while the second is still being generated, or the pause after every question
//! is the length of the longest answer. So the provider is asked to stream, and what it
//! streams is assembled here into three things only: words, a tool the model wants, and
//! the end.
//!
//! Two things are deliberately never passed on. A model that reasons before it answers
//! streams that reasoning alongside the answer; it is scaffolding, it is not addressed to
//! anyone, and a voice that read it aloud would be saying the model's private notes. And a
//! tool call arrives in fragments — the name in one chunk, the arguments spread over
//! several — so it is held until it is whole: half an argument is not a smaller request,
//! it is a different one.
//!
//! The parser is pure and fed bytes, so every property here is tested against streams
//! recorded from the real provider rather than against a guess at what it sends.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A tool a model may ask for, described the way the provider expects.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    /// A JSON Schema object for the arguments.
    pub parameters: Value,
}

/// A tool the model asked for, whole.
///
/// The arguments are the model's text, unparsed. Whether they are well formed, and whether
/// the call may happen at all, is decided by whoever receives it — never by the transport.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

/// One message of a conversation that can include tools.
#[derive(Debug, Clone, PartialEq)]
pub enum ToolChatMessage {
    System(String),
    User(String),
    /// What the model said, and the tools it asked for in the same breath.
    Assistant {
        content: Option<String>,
        tool_calls: Vec<ToolCall>,
    },
    /// What a tool returned, answering one call by its id.
    Tool {
        call_id: String,
        content: String,
    },
}

impl ToolChatMessage {
    fn to_json(&self) -> Value {
        match self {
            Self::System(content) => serde_json::json!({"role": "system", "content": content}),
            Self::User(content) => serde_json::json!({"role": "user", "content": content}),
            Self::Assistant {
                content,
                tool_calls,
            } => {
                let mut message = serde_json::json!({"role": "assistant", "content": content});
                if !tool_calls.is_empty() {
                    message["tool_calls"] = tool_calls
                        .iter()
                        .map(|call| {
                            serde_json::json!({
                                "id": call.id,
                                "type": "function",
                                "function": {"name": call.name, "arguments": call.arguments}
                            })
                        })
                        .collect();
                }
                message
            }
            Self::Tool { call_id, content } => {
                serde_json::json!({"role": "tool", "tool_call_id": call_id, "content": content})
            }
        }
    }
}

/// A streamed request that may offer tools.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolChatRequest {
    /// The provider's name for the model.
    pub model: String,
    pub messages: Vec<ToolChatMessage>,
    pub tools: Vec<ToolDefinition>,
    pub max_tokens: u32,
    pub temperature: f32,
}

/// The most messages, tools and output one streamed request may carry.
pub const MAX_STREAM_MESSAGES: usize = 64;
pub const MAX_STREAM_TOOLS: usize = 32;
pub const MAX_STREAM_TOKENS: u32 = 4096;

impl ToolChatRequest {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.model.trim().is_empty() {
            return Err("no model named");
        }
        if self.messages.is_empty() || self.messages.len() > MAX_STREAM_MESSAGES {
            return Err("no messages, or more than a streamed request may carry");
        }
        if self.tools.len() > MAX_STREAM_TOOLS {
            return Err("more tools than a streamed request may offer");
        }
        if self.max_tokens == 0 || self.max_tokens > MAX_STREAM_TOKENS {
            return Err("output budget outside what a streamed request may ask for");
        }
        if !(0.0..=2.0).contains(&self.temperature) {
            return Err("temperature outside 0 to 2");
        }
        Ok(())
    }

    /// The body sent to the provider. Always streaming: this request exists to stream.
    pub fn body(&self) -> Value {
        let mut body = serde_json::json!({
            "model": self.model,
            "messages": self.messages.iter().map(ToolChatMessage::to_json).collect::<Vec<_>>(),
            "max_tokens": self.max_tokens,
            "temperature": self.temperature,
            "stream": true,
        });
        if !self.tools.is_empty() {
            body["tools"] = self
                .tools
                .iter()
                .map(|tool| {
                    serde_json::json!({
                        "type": "function",
                        "function": {
                            "name": tool.name,
                            "description": tool.description,
                            "parameters": tool.parameters,
                        }
                    })
                })
                .collect();
        }
        body
    }
}

/// What a stream says, once assembled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamEvent {
    /// Words of the answer, as they arrive. May be part of a word.
    Text(String),
    /// A tool the model asked for, with all of its arguments.
    ToolCall(ToolCall),
    /// The answer ended, and why: `stop`, `length`, `tool_calls`.
    Finished { reason: Option<String> },
}

/// Turns the bytes of a server-sent event stream into [`StreamEvent`]s.
///
/// Fed bytes in whatever pieces the network delivers them; a line cut across two reads is
/// held until it is whole. Tool calls are held until the stream says why it finished,
/// because their arguments arrive in fragments and are only a request once complete.
#[derive(Debug, Default)]
pub struct SseAssembler {
    pending: Vec<u8>,
    calls: Vec<PartialCall>,
    finished: bool,
}

#[derive(Debug, Default, Clone)]
struct PartialCall {
    id: String,
    name: String,
    arguments: String,
}

/// The longest single event line the assembler will hold. A provider that sends one line
/// longer than this is not streaming an answer, and the line is refused rather than kept.
pub const MAX_EVENT_LINE: usize = 256 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SseError {
    /// One event line was longer than an answer's line can be.
    LineTooLong,
    /// A `data:` line that was not JSON.
    Malformed(String),
}

impl SseAssembler {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether the stream has said it is finished.
    pub fn is_finished(&self) -> bool {
        self.finished
    }

    /// Takes the next piece of the stream and returns what it completed.
    pub fn feed(&mut self, bytes: &[u8]) -> Result<Vec<StreamEvent>, SseError> {
        self.pending.extend_from_slice(bytes);
        let mut events = Vec::new();
        while let Some(end) = self.pending.iter().position(|byte| *byte == b'\n') {
            let line: Vec<u8> = self.pending.drain(..=end).collect();
            self.line(&line, &mut events)?;
        }
        if self.pending.len() > MAX_EVENT_LINE {
            return Err(SseError::LineTooLong);
        }
        Ok(events)
    }

    /// The stream closed. Whatever was held is released as it stands.
    pub fn close(&mut self) -> Result<Vec<StreamEvent>, SseError> {
        let mut events = Vec::new();
        if !self.pending.is_empty() {
            let line = std::mem::take(&mut self.pending);
            self.line(&line, &mut events)?;
        }
        if !self.finished {
            self.release_calls(&mut events);
            events.push(StreamEvent::Finished { reason: None });
            self.finished = true;
        }
        Ok(events)
    }

    fn line(&mut self, raw: &[u8], events: &mut Vec<StreamEvent>) -> Result<(), SseError> {
        let text = String::from_utf8_lossy(raw);
        let text = text.trim_end_matches(['\r', '\n']);
        let Some(data) = text.strip_prefix("data:") else {
            // Comments, event names and blank separators carry nothing for us.
            return Ok(());
        };
        let data = data.trim_start();
        if data == "[DONE]" {
            if !self.finished {
                self.release_calls(events);
                events.push(StreamEvent::Finished { reason: None });
                self.finished = true;
            }
            return Ok(());
        }
        let chunk: Value =
            serde_json::from_str(data).map_err(|error| SseError::Malformed(error.to_string()))?;
        let Some(choice) = chunk["choices"].get(0) else {
            return Ok(());
        };
        let delta = &choice["delta"];
        // The answer, and only the answer. `reasoning_content` is the model's private
        // scaffolding and is dropped here, before anything downstream could speak it.
        if let Some(words) = delta["content"].as_str() {
            if !words.is_empty() {
                events.push(StreamEvent::Text(words.to_string()));
            }
        }
        if let Some(pieces) = delta["tool_calls"].as_array() {
            for piece in pieces {
                let index = piece["index"].as_u64().unwrap_or(0) as usize;
                if index >= 32 {
                    continue;
                }
                if self.calls.len() <= index {
                    self.calls.resize(index + 1, PartialCall::default());
                }
                let call = &mut self.calls[index];
                if let Some(id) = piece["id"].as_str() {
                    call.id.push_str(id);
                }
                if let Some(name) = piece["function"]["name"].as_str() {
                    call.name.push_str(name);
                }
                if let Some(arguments) = piece["function"]["arguments"].as_str() {
                    call.arguments.push_str(arguments);
                }
            }
        }
        if let Some(reason) = choice["finish_reason"].as_str() {
            self.release_calls(events);
            events.push(StreamEvent::Finished {
                reason: Some(reason.to_string()),
            });
            self.finished = true;
        }
        Ok(())
    }

    fn release_calls(&mut self, events: &mut Vec<StreamEvent>) {
        for call in self.calls.drain(..) {
            // A call with no name is not a request for anything.
            if call.name.is_empty() {
                continue;
            }
            events.push(StreamEvent::ToolCall(ToolCall {
                id: call.id,
                name: call.name,
                arguments: call.arguments,
            }));
        }
    }
}

/// Unveils streamed text and tool calls as they arrive.
///
/// Stand-ins may be split across SSE deltas (e.g. `SEC` then `RET_1`).
/// `StreamUnveiler` buffers any trailing suffix that matches a stand-in prefix,
/// restoring all prior text immediately and releasing the remainder once complete
/// or when `Finished` arrives.
#[derive(Debug, Default)]
pub struct StreamUnveiler {
    unveil: Option<archie_vault::veil::Unveil>,
    pending: String,
}

impl StreamUnveiler {
    pub fn new(unveil: Option<archie_vault::veil::Unveil>) -> Self {
        Self {
            unveil,
            pending: String::new(),
        }
    }

    pub fn feed(&mut self, event: StreamEvent) -> Vec<StreamEvent> {
        let Some(unveil) = &self.unveil else {
            return vec![event];
        };
        match event {
            StreamEvent::Text(chunk) => {
                self.pending.push_str(&chunk);
                let suffix_len = self
                    .pending
                    .chars()
                    .rev()
                    .take_while(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || *c == '_')
                    .count();
                if suffix_len == 0 {
                    let out = unveil.restore(&self.pending);
                    self.pending.clear();
                    if out.is_empty() {
                        vec![]
                    } else {
                        vec![StreamEvent::Text(out)]
                    }
                } else {
                    let split_pos = self.pending.len() - suffix_len;
                    let potential_prefix = &self.pending[split_pos..];
                    if unveil.is_alias_prefix(potential_prefix) {
                        let to_emit = &self.pending[..split_pos];
                        let out = unveil.restore(to_emit);
                        self.pending = potential_prefix.to_string();
                        if out.is_empty() {
                            vec![]
                        } else {
                            vec![StreamEvent::Text(out)]
                        }
                    } else {
                        let out = unveil.restore(&self.pending);
                        self.pending.clear();
                        if out.is_empty() {
                            vec![]
                        } else {
                            vec![StreamEvent::Text(out)]
                        }
                    }
                }
            }
            StreamEvent::ToolCall(mut call) => {
                // Inside the strings of JSON arguments, so a value with a quote in it
                // comes back escaped instead of breaking them.
                call.arguments = match serde_json::from_str::<Value>(&call.arguments) {
                    Ok(mut value) => {
                        restore_json(&mut value, unveil);
                        value.to_string()
                    }
                    Err(_) => unveil.restore(&call.arguments),
                };
                vec![StreamEvent::ToolCall(call)]
            }
            StreamEvent::Finished { reason } => {
                let mut events = Vec::new();
                if !self.pending.is_empty() {
                    let out = unveil.restore(&self.pending);
                    self.pending.clear();
                    if !out.is_empty() {
                        events.push(StreamEvent::Text(out));
                    }
                }
                events.push(StreamEvent::Finished { reason });
                events
            }
        }
    }
}

fn restore_json(value: &mut Value, unveil: &archie_vault::veil::Unveil) {
    match value {
        Value::String(text) => *text = unveil.restore(text),
        Value::Object(map) => map.values_mut().for_each(|v| restore_json(v, unveil)),
        Value::Array(items) => items.iter_mut().for_each(|v| restore_json(v, unveil)),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(bytes: &[u8], piece: usize) -> Vec<StreamEvent> {
        let mut assembler = SseAssembler::new();
        let mut events = Vec::new();
        for chunk in bytes.chunks(piece.max(1)) {
            events.extend(assembler.feed(chunk).unwrap());
        }
        events.extend(assembler.close().unwrap());
        events
    }

    const TOOL: &[u8] = include_bytes!("../tests/fixtures/nebius-tool-call.sse");
    const TEXT: &[u8] = include_bytes!("../tests/fixtures/nebius-text.sse");
    const REASONING: &[u8] = include_bytes!("../tests/fixtures/nebius-reasoning.sse");

    #[test]
    fn a_tool_call_from_the_real_provider_arrives_whole() {
        let events = run(TOOL, 4096);
        let calls: Vec<&ToolCall> = events
            .iter()
            .filter_map(|event| match event {
                StreamEvent::ToolCall(call) => Some(call),
                _ => None,
            })
            .collect();
        assert_eq!(calls.len(), 1, "one call: {events:?}");
        assert_eq!(calls[0].name, "open_app");
        let arguments: Value = serde_json::from_str(&calls[0].arguments)
            .expect("the arguments, reassembled from fragments, are whole JSON");
        assert!(
            arguments["app"]
                .as_str()
                .is_some_and(|app| app.to_lowercase().contains("firefox")),
            "and they say what was asked: {arguments}"
        );
        assert!(!calls[0].id.is_empty(), "with the id a result must answer");
        assert_eq!(
            events.last(),
            Some(&StreamEvent::Finished {
                reason: Some("tool_calls".into())
            })
        );
    }

    #[test]
    fn it_does_not_matter_how_the_network_cuts_the_bytes() {
        // One byte at a time is the worst a network can do, and must give the same answer.
        assert_eq!(run(TOOL, 1), run(TOOL, 4096));
        assert_eq!(run(TEXT, 1), run(TEXT, 4096));
        assert_eq!(run(TEXT, 7), run(TEXT, 4096));
    }

    #[test]
    fn the_words_of_an_answer_arrive_in_order_and_complete() {
        let said: String = run(TEXT, 64)
            .iter()
            .filter_map(|event| match event {
                StreamEvent::Text(words) => Some(words.as_str()),
                _ => None,
            })
            .collect();
        assert!(
            said.to_lowercase().contains("volc"),
            "the answer is about volcanoes: {said}"
        );
        assert!(
            said.len() > 40,
            "and it is a whole answer, not a fragment: {said}"
        );
    }

    #[test]
    fn a_models_private_reasoning_is_never_passed_on() {
        // The recorded stream carries reasoning chunks; none of them may come out as words.
        let raw = String::from_utf8_lossy(REASONING);
        assert!(
            raw.contains("reasoning"),
            "the fixture really does contain reasoning, or this test proves nothing"
        );
        let said: String = run(REASONING, 64)
            .iter()
            .filter_map(|event| match event {
                StreamEvent::Text(words) => Some(words.clone()),
                _ => None,
            })
            .collect();
        for line in raw
            .lines()
            .filter(|line| line.contains("reasoning_content"))
        {
            if let Some(start) = line.find("\"reasoning_content\":\"") {
                let rest = &line[start + 21..];
                let piece: String = rest.chars().take_while(|c| *c != '"').collect();
                if piece.trim().len() > 6 {
                    assert!(
                        !said.contains(piece.trim()),
                        "reasoning leaked into the spoken answer: {piece:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn a_line_that_never_ends_is_refused() {
        let mut assembler = SseAssembler::new();
        let endless = vec![b'x'; MAX_EVENT_LINE + 1];
        assert_eq!(assembler.feed(&endless), Err(SseError::LineTooLong));
    }

    #[test]
    fn a_request_says_what_it_asks_and_offers() {
        let request = ToolChatRequest {
            model: "nvidia/Nemotron-3_5-Lightning".into(),
            messages: vec![
                ToolChatMessage::System("be brief".into()),
                ToolChatMessage::User("open firefox".into()),
                ToolChatMessage::Assistant {
                    content: None,
                    tool_calls: vec![ToolCall {
                        id: "call_1".into(),
                        name: "open_app".into(),
                        arguments: "{\"app\":\"firefox\"}".into(),
                    }],
                },
                ToolChatMessage::Tool {
                    call_id: "call_1".into(),
                    content: "opened".into(),
                },
            ],
            tools: vec![ToolDefinition {
                name: "open_app".into(),
                description: "Opens an application.".into(),
                parameters: serde_json::json!({"type": "object"}),
            }],
            max_tokens: 200,
            temperature: 0.2,
        };
        request.validate().unwrap();
        let body = request.body();
        assert_eq!(body["stream"], true);
        assert_eq!(
            body["messages"][2]["tool_calls"][0]["function"]["name"],
            "open_app"
        );
        assert_eq!(body["messages"][3]["role"], "tool");
        assert_eq!(body["messages"][3]["tool_call_id"], "call_1");
        assert_eq!(body["tools"][0]["function"]["name"], "open_app");

        let mut empty = request.clone();
        empty.messages.clear();
        assert!(empty.validate().is_err());
        let mut greedy = request;
        greedy.max_tokens = MAX_STREAM_TOKENS + 1;
        assert!(greedy.validate().is_err());
    }

    #[test]
    fn stream_unveiler_restores_split_stand_in_and_tool_call_arguments() {
        let text = "El usuario es juan@example.com y la clave es sk-proj-1234567890abcdef.";
        let veiled = archie_vault::Veil::new().cover(&[text]);
        assert!(!veiled.unveil.is_empty());

        let mut unveiler = StreamUnveiler::new(Some(veiled.unveil));
        // Simulate SSE deltas where stand-ins are split across chunks:
        // "Tu correo es EMAIL_" then "1 y clave " then "SEC" then "RET_1."
        let mut out = Vec::new();
        out.extend(unveiler.feed(StreamEvent::Text("Tu correo es EMAIL_".into())));
        out.extend(unveiler.feed(StreamEvent::Text("1 y clave SEC".into())));
        out.extend(unveiler.feed(StreamEvent::Text("RET_1.".into())));
        out.extend(unveiler.feed(StreamEvent::ToolCall(ToolCall {
            id: "call_1".into(),
            name: "send_email".into(),
            arguments: "{\"to\":\"EMAIL_1\"}".into(),
        })));
        out.extend(unveiler.feed(StreamEvent::Finished { reason: None }));

        let combined: String = out
            .iter()
            .filter_map(|e| match e {
                StreamEvent::Text(t) => Some(t.as_str()),
                _ => None,
            })
            .collect();

        assert_eq!(
            combined,
            "Tu correo es juan@example.com y clave sk-proj-1234567890abcdef."
        );

        let tool_call = out
            .iter()
            .find_map(|e| match e {
                StreamEvent::ToolCall(call) => Some(call),
                _ => None,
            })
            .expect("tool call preserved");

        assert_eq!(tool_call.arguments, "{\"to\":\"juan@example.com\"}");
    }
}
