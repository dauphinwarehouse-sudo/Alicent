//! Strict Anthropic Messages SSE normalization for text and client tool use.
use crate::{json::parse_json, tools::{valid_id, valid_name}, ChatCompletion, ChatEvent, ChatStopReason, Result, SseDecoder, SseFrame, TokenUsage, ToolArguments, WireError};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};

const MAX_CONTENT_BLOCKS: usize = 128;

enum ContentBlock {
    Text,
    Tool { initial_input: String, has_delta: bool },
}

/// Messages streams remain provisional until `message_stop` and a clean SSE boundary.
/// Tool proposals are released only by [`AnthropicDecoder::finish`].
pub struct AnthropicDecoder {
    sse: SseDecoder,
    allowed_names: BTreeSet<String>,
    arguments: ToolArguments,
    blocks: BTreeMap<u32, ContentBlock>,
    seen_blocks: BTreeSet<u32>,
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    reason: Option<ChatStopReason>,
    started: bool,
    completed: bool,
    closed: bool,
    tool_count: usize,
}

impl AnthropicDecoder {
    pub fn new(allowed_names: BTreeSet<String>) -> Result<Self> {
        if allowed_names.len() > ToolArguments::MAX_CALLS || allowed_names.iter().any(|name| !valid_name(name)) {
            return Err(WireError::InvalidRequest);
        }
        Ok(Self {
            sse: SseDecoder::default(), allowed_names, arguments: ToolArguments::default(),
            blocks: BTreeMap::new(), seen_blocks: BTreeSet::new(), input_tokens: None,
            output_tokens: None, reason: None, started: false, completed: false,
            closed: false, tool_count: 0,
        })
    }

    pub fn push(&mut self, chunk: &[u8]) -> Result<Vec<ChatEvent>> {
        if self.closed { return Err(WireError::Closed); }
        let result = self.consume(chunk);
        if result.is_err() { self.cancel(); }
        result
    }

    fn consume(&mut self, chunk: &[u8]) -> Result<Vec<ChatEvent>> {
        let mut events = Vec::new();
        for frame in self.sse.push(chunk)? { self.frame(frame, &mut events)?; }
        Ok(events)
    }

    fn frame(&mut self, frame: SseFrame, events: &mut Vec<ChatEvent>) -> Result<()> {
        if self.completed { return Err(WireError::InvalidResponse); }
        if frame.event == "error" { return Err(WireError::ProviderFailure); }
        let value = parse_json(&frame.data).map_err(|_| WireError::InvalidResponse)?;
        let object = as_object(&value)?;
        if object.contains_key("error") { return Err(WireError::ProviderFailure); }
        let kind = string(object, "type")?;
        if frame.event != kind { return Err(WireError::InvalidResponse); }
        match kind {
            "message_start" => self.message_start(object),
            "content_block_start" => self.block_start(object),
            "content_block_delta" => self.block_delta(object, events),
            "content_block_stop" => self.block_stop(object),
            "message_delta" => self.message_delta(object),
            "message_stop" => self.message_stop(),
            "ping" if self.started => Ok(()),
            "error" => Err(WireError::ProviderFailure),
            _ => Err(WireError::UnsupportedResponse),
        }
    }

    fn message_start(&mut self, object: &Map<String, Value>) -> Result<()> {
        if self.started { return Err(WireError::InvalidResponse); }
        let message = object_of(object, "message")?;
        if string(message, "type")? != "message" || string(message, "role")? != "assistant" {
            return Err(WireError::InvalidResponse);
        }
        if !valid_id(string(message, "id")?) || !valid_id(string(message, "model")?) {
            return Err(WireError::InvalidResponse);
        }
        self.input_tokens = Some(number(object_of(message, "usage")?, "input_tokens")?);
        self.started = true;
        Ok(())
    }

    fn block_start(&mut self, object: &Map<String, Value>) -> Result<()> {
        if !self.started || self.reason.is_some() { return Err(WireError::InvalidResponse); }
        if self.seen_blocks.len() >= MAX_CONTENT_BLOCKS { return Err(WireError::LimitExceeded); }
        let index = index(object)?;
        if !self.seen_blocks.insert(index) { return Err(WireError::InvalidResponse); }
        let block = object_of(object, "content_block")?;
        let state = match string(block, "type")? {
            "text" => {
                if string(block, "text")? != "" { return Err(WireError::InvalidResponse); }
                ContentBlock::Text
            }
            "tool_use" => {
                if self.tool_count >= ToolArguments::MAX_CALLS { return Err(WireError::LimitExceeded); }
                let id = string(block, "id")?;
                let name = string(block, "name")?;
                if !self.allowed_names.contains(name) { return Err(WireError::InvalidArguments); }
                let input = block.get("input").ok_or(WireError::InvalidResponse)?;
                if !input.is_object() { return Err(WireError::InvalidResponse); }
                self.arguments.start(index, id, name)?;
                self.tool_count += 1;
                ContentBlock::Tool {
                    initial_input: serde_json::to_string(input).map_err(|_| WireError::InvalidResponse)?,
                    has_delta: false,
                }
            }
            _ => return Err(WireError::UnsupportedResponse),
        };
        self.blocks.insert(index, state);
        Ok(())
    }

    fn block_delta(&mut self, object: &Map<String, Value>, events: &mut Vec<ChatEvent>) -> Result<()> {
        if !self.started || self.reason.is_some() { return Err(WireError::InvalidResponse); }
        let index = index(object)?;
        let delta = object_of(object, "delta")?;
        let block = self.blocks.get_mut(&index).ok_or(WireError::InvalidResponse)?;
        match (block, string(delta, "type")?) {
            (ContentBlock::Text, "text_delta") => {
                let text = string(delta, "text")?;
                if !text.is_empty() { events.push(ChatEvent::TextDelta(text.into())); }
                Ok(())
            }
            (ContentBlock::Tool { initial_input, has_delta }, "input_json_delta") => {
                if !*has_delta && initial_input.as_str() != "{}" { return Err(WireError::InvalidResponse); }
                self.arguments.append(index, string(delta, "partial_json")?)?;
                *has_delta = true;
                Ok(())
            }
            (_, "thinking_delta" | "signature_delta" | "citations_delta") => Err(WireError::UnsupportedResponse),
            _ => Err(WireError::InvalidResponse),
        }
    }

    fn block_stop(&mut self, object: &Map<String, Value>) -> Result<()> {
        if !self.started || self.reason.is_some() { return Err(WireError::InvalidResponse); }
        let index = index(object)?;
        match self.blocks.remove(&index) {
            Some(ContentBlock::Text) => Ok(()),
            Some(ContentBlock::Tool { initial_input, has_delta }) => {
                if !has_delta { self.arguments.append(index, &initial_input)?; }
                Ok(())
            }
            None => Err(WireError::InvalidResponse),
        }
    }

    fn message_delta(&mut self, object: &Map<String, Value>) -> Result<()> {
        if !self.started || !self.blocks.is_empty() || self.reason.is_some() { return Err(WireError::InvalidResponse); }
        let delta = object_of(object, "delta")?;
        self.reason = Some(match string(delta, "stop_reason")? {
            "end_turn" | "stop_sequence" if self.tool_count == 0 => ChatStopReason::Stop,
            "tool_use" if self.tool_count > 0 => ChatStopReason::ToolCalls,
            "max_tokens" | "model_context_window_exceeded" | "refusal" | "pause_turn" => return Err(WireError::IncompleteResponse),
            "end_turn" | "stop_sequence" | "tool_use" => return Err(WireError::InvalidResponse),
            _ => return Err(WireError::UnsupportedResponse),
        });
        self.output_tokens = Some(number(object_of(object, "usage")?, "output_tokens")?);
        Ok(())
    }

    fn message_stop(&mut self) -> Result<()> {
        if !self.started || self.reason.is_none() || !self.blocks.is_empty() { return Err(WireError::TruncatedStream); }
        self.completed = true;
        Ok(())
    }

    pub fn finish(&mut self) -> Result<ChatCompletion> {
        if self.closed { return Err(WireError::Closed); }
        let result = self.complete();
        self.cancel();
        result
    }

    fn complete(&mut self) -> Result<ChatCompletion> {
        self.sse.finish(self.completed)?;
        if !self.completed { return Err(WireError::TruncatedStream); }
        let reason = self.reason.ok_or(WireError::TruncatedStream)?;
        let input_tokens = self.input_tokens.ok_or(WireError::InvalidResponse)?;
        let output_tokens = self.output_tokens.ok_or(WireError::InvalidResponse)?;
        let total_tokens = input_tokens.checked_add(output_tokens).ok_or(WireError::InvalidResponse)?;
        let tools = self.arguments.finish(&self.allowed_names)?;
        Ok(ChatCompletion { reason, tools, usage: Some(TokenUsage { input_tokens, output_tokens, total_tokens }) })
    }

    pub fn cancel(&mut self) {
        self.closed = true;
        self.sse.cancel();
        self.arguments.cancel();
        self.blocks.clear();
        self.seen_blocks.clear();
        self.input_tokens = None;
        self.output_tokens = None;
        self.reason = None;
        self.started = false;
        self.completed = false;
        self.tool_count = 0;
    }
}

fn as_object(value: &Value) -> Result<&Map<String, Value>> { value.as_object().ok_or(WireError::InvalidResponse) }
fn object_of<'a>(object: &'a Map<String, Value>, key: &str) -> Result<&'a Map<String, Value>> { as_object(object.get(key).ok_or(WireError::InvalidResponse)?) }
fn string<'a>(object: &'a Map<String, Value>, key: &str) -> Result<&'a str> { object.get(key).and_then(Value::as_str).ok_or(WireError::InvalidResponse) }
fn number(object: &Map<String, Value>, key: &str) -> Result<u64> { object.get(key).and_then(Value::as_u64).ok_or(WireError::InvalidResponse) }
fn index(object: &Map<String, Value>) -> Result<u32> { object.get("index").and_then(Value::as_u64).and_then(|value| u32::try_from(value).ok()).ok_or(WireError::InvalidResponse) }
