//! Strict Anthropic Messages SSE normalization for text and client tool use.
use crate::{json::parse_json, tools::{valid_id, valid_name}, ChatCompletion, ChatEvent, ChatStopReason, Result, SseDecoder, SseFrame, TokenUsage, ToolArguments, WireError};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};

const MAX_CONTENT_BLOCKS: usize = 128;
/// Shared with the regular decoder: both paths must cap assistant text.
pub(crate) const MAX_TEXT_BYTES: usize = 8 * 1024 * 1024;

enum ContentBlock {
    Text,
    Tool { initial_input: String, has_delta: bool },
    /// A block type this build does not model (thinking, server tool use, ...).
    /// Its deltas are dropped instead of failing an otherwise valid response.
    Ignored,
}

enum DeltaTarget {
    Text,
    Tool { accepts: bool },
    Ignored,
}

/// Messages streams remain provisional until `message_stop` and a clean SSE boundary.
/// Tool proposals are released only by [`AnthropicDecoder::finish`].
pub struct AnthropicDecoder {
    sse: SseDecoder,
    allowed_names: BTreeSet<String>,
    arguments: ToolArguments,
    blocks: BTreeMap<u32, ContentBlock>,
    seen_blocks: BTreeSet<u32>,
    text_bytes: usize,
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
            blocks: BTreeMap::new(), seen_blocks: BTreeSet::new(), text_bytes: 0,
            input_tokens: None, output_tokens: None, reason: None, started: false,
            completed: false, closed: false, tool_count: 0,
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
        if frame.event == "error" { return Err(WireError::ProviderFailure); }
        let value = parse_json(&frame.data).map_err(|_| WireError::InvalidResponse)?;
        let object = as_object(&value)?;
        if object.contains_key("error") { return Err(WireError::ProviderFailure); }
        let kind = string(object, "type")?;
        if frame.event != kind { return Err(WireError::InvalidResponse); }
        // Keep-alives are allowed at any point, including before message_start.
        if kind == "ping" { return Ok(()); }
        if self.completed { return Err(WireError::InvalidResponse); }
        match kind {
            "message_start" => self.message_start(object),
            "content_block_start" => self.block_start(object),
            "content_block_delta" => self.block_delta(object, events),
            "content_block_stop" => self.block_stop(object),
            "message_delta" => self.message_delta(object),
            "message_stop" => self.message_stop(),
            // Unknown event types are ignored on purpose: a newly introduced
            // Anthropic event must not invalidate a complete response.
            _ => Ok(()),
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
        self.input_tokens = Some(billable_input_tokens(object_of(message, "usage")?)?);
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
                if !string(block, "text")?.is_empty() { return Err(WireError::InvalidResponse); }
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
            // Thinking, redacted thinking and server-side tool blocks are not
            // surfaced by this build, but they must not abort the stream.
            _ => ContentBlock::Ignored,
        };
        self.blocks.insert(index, state);
        Ok(())
    }

    fn block_delta(&mut self, object: &Map<String, Value>, events: &mut Vec<ChatEvent>) -> Result<()> {
        if !self.started || self.reason.is_some() { return Err(WireError::InvalidResponse); }
        let index = index(object)?;
        let delta = object_of(object, "delta")?;
        let kind = string(delta, "type")?;
        // Resolve the target first so the block borrow ends before mutation.
        let target = match self.blocks.get(&index).ok_or(WireError::InvalidResponse)? {
            ContentBlock::Text => DeltaTarget::Text,
            ContentBlock::Tool { initial_input, has_delta } => DeltaTarget::Tool {
                accepts: *has_delta || initial_input.as_str() == "{}",
            },
            ContentBlock::Ignored => DeltaTarget::Ignored,
        };
        match target {
            DeltaTarget::Text => {
                if kind != "text_delta" { return Ok(()); }
                let text = string(delta, "text")?;
                if text.is_empty() { return Ok(()); }
                self.text_bytes = self.text_bytes.checked_add(text.len()).ok_or(WireError::LimitExceeded)?;
                if self.text_bytes > MAX_TEXT_BYTES { return Err(WireError::LimitExceeded); }
                events.push(ChatEvent::TextDelta(text.into()));
                Ok(())
            }
            DeltaTarget::Tool { accepts } => {
                if kind != "input_json_delta" { return Err(WireError::InvalidResponse); }
                if !accepts { return Err(WireError::InvalidResponse); }
                self.arguments.append(index, string(delta, "partial_json")?)?;
                if let Some(ContentBlock::Tool { has_delta, .. }) = self.blocks.get_mut(&index) {
                    *has_delta = true;
                }
                Ok(())
            }
            DeltaTarget::Ignored => Ok(()),
        }
    }

    fn block_stop(&mut self, object: &Map<String, Value>) -> Result<()> {
        if !self.started || self.reason.is_some() { return Err(WireError::InvalidResponse); }
        let index = index(object)?;
        match self.blocks.remove(&index) {
            Some(ContentBlock::Text) | Some(ContentBlock::Ignored) => Ok(()),
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
        self.reason = Some(stop_reason(string(delta, "stop_reason")?, self.tool_count > 0)?);
        let usage = object_of(object, "usage")?;
        self.output_tokens = Some(number(usage, "output_tokens")?);
        // Anthropic may restate input usage here; keep the largest value seen.
        if usage.contains_key("input_tokens") {
            let updated = billable_input_tokens(usage)?;
            self.input_tokens = Some(self.input_tokens.unwrap_or(0).max(updated));
        }
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
        self.text_bytes = 0;
        self.input_tokens = None;
        self.output_tokens = None;
        self.reason = None;
        self.started = false;
        self.completed = false;
        self.tool_count = 0;
    }
}

/// Cache reads and cache writes are billed as input, so dropping them would
/// understate the cost of a prompt-cached request.
pub(crate) fn billable_input_tokens(usage: &Map<String, Value>) -> Result<u64> {
    let base = number(usage, "input_tokens")?;
    let creation = optional_number(usage, "cache_creation_input_tokens")?;
    let read = optional_number(usage, "cache_read_input_tokens")?;
    base.checked_add(creation)
        .and_then(|total| total.checked_add(read))
        .ok_or(WireError::InvalidResponse)
}

/// One mapping for both the stream and the regular response: the two must not
/// disagree about which stop reasons are terminal.
pub(crate) fn stop_reason(value: &str, has_tools: bool) -> Result<ChatStopReason> {
    match value {
        "end_turn" | "stop_sequence" if !has_tools => Ok(ChatStopReason::Stop),
        "tool_use" if has_tools => Ok(ChatStopReason::ToolCalls),
        "max_tokens" | "model_context_window_exceeded" | "refusal" | "pause_turn" => Err(WireError::IncompleteResponse),
        "end_turn" | "stop_sequence" | "tool_use" => Err(WireError::InvalidResponse),
        _ => Err(WireError::UnsupportedResponse),
    }
}

fn as_object(value: &Value) -> Result<&Map<String, Value>> { value.as_object().ok_or(WireError::InvalidResponse) }
fn object_of<'a>(object: &'a Map<String, Value>, key: &str) -> Result<&'a Map<String, Value>> { as_object(object.get(key).ok_or(WireError::InvalidResponse)?) }
fn string<'a>(object: &'a Map<String, Value>, key: &str) -> Result<&'a str> { object.get(key).and_then(Value::as_str).ok_or(WireError::InvalidResponse) }
fn number(object: &Map<String, Value>, key: &str) -> Result<u64> { object.get(key).and_then(Value::as_u64).ok_or(WireError::InvalidResponse) }
fn optional_number(object: &Map<String, Value>, key: &str) -> Result<u64> {
    match object.get(key) {
        None | Some(Value::Null) => Ok(0),
        Some(value) => value.as_u64().ok_or(WireError::InvalidResponse),
    }
}
fn index(object: &Map<String, Value>) -> Result<u32> { object.get("index").and_then(Value::as_u64).and_then(|value| u32::try_from(value).ok()).ok_or(WireError::InvalidResponse) }
