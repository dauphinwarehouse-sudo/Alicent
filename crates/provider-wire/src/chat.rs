//! Single-choice, text/client-tool OpenAI Chat streaming. No transport or execution.
use crate::{
    json::parse_json,
    tools::{valid_id, valid_name},
    Result, SseDecoder, SseFrame, ToolArguments, ToolProposal, WireError,
};
use serde_json::{Map, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};

#[derive(Clone, PartialEq, Eq)]
pub enum ChatEvent {
    /// Provisional display text. Not evidence that the response completed successfully.
    TextDelta(String),
}

impl fmt::Debug for ChatEvent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TextDelta(text) => formatter
                .debug_struct("TextDelta")
                .field("bytes", &text.len())
                .finish(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatStopReason {
    Stop,
    ToolCalls,
}

#[derive(Clone, PartialEq)]
pub struct ChatCompletion {
    pub reason: ChatStopReason,
    /// Still untrusted: schema validation, scopes and approval belong to the future runtime.
    pub tools: Vec<ToolProposal>,
    /// None means unknown, not zero. Includes provider-reported totals only.
    pub usage: Option<TokenUsage>,
}

impl fmt::Debug for ChatCompletion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ChatCompletion")
            .field("reason", &self.reason)
            .field("tool_count", &self.tools.len())
            .field("usage", &self.usage)
            .finish()
    }
}

struct CallHeader {
    id: String,
    name: String,
}

/// `push` only yields provisional text. `finish` alone can return tool proposals,
/// after a successful finish_reason, [DONE], and a clean SSE boundary.
/// On any error, discard the response; this decoder cannot be resumed or retried.
pub struct ChatDecoder {
    sse: SseDecoder,
    arguments: ToolArguments,
    calls: BTreeMap<u32, CallHeader>,
    allowed_names: BTreeSet<String>,
    response_id: Option<String>,
    model: Option<String>,
    reason: Option<ChatStopReason>,
    usage: Option<TokenUsage>,
    done: bool,
    closed: bool,
}

fn object(value: &Value) -> Result<&Map<String, Value>> {
    value.as_object().ok_or(WireError::InvalidResponse)
}

fn optional_string<'a>(value: &'a Map<String, Value>, key: &str) -> Result<Option<&'a str>> {
    value
        .get(key)
        .map(|value| value.as_str().ok_or(WireError::InvalidResponse))
        .transpose()
}

fn required_string<'a>(value: &'a Map<String, Value>, key: &str) -> Result<&'a str> {
    optional_string(value, key)?.ok_or(WireError::InvalidResponse)
}

fn usage(value: &Value) -> Result<TokenUsage> {
    let value = object(value)?;
    let number = |key: &str| {
        value
            .get(key)
            .and_then(Value::as_u64)
            .ok_or(WireError::InvalidResponse)
    };
    let result = TokenUsage {
        input_tokens: number("prompt_tokens")?,
        output_tokens: number("completion_tokens")?,
        total_tokens: number("total_tokens")?,
    };
    if result.input_tokens.checked_add(result.output_tokens) != Some(result.total_tokens) {
        return Err(WireError::InvalidResponse);
    }
    Ok(result)
}

impl ChatDecoder {
    pub fn new(allowed_names: BTreeSet<String>) -> Result<Self> {
        if allowed_names.len() > ToolArguments::MAX_CALLS
            || allowed_names.iter().any(|name| !valid_name(name))
        {
            return Err(WireError::InvalidRequest);
        }
        Ok(Self {
            sse: SseDecoder::default(),
            arguments: ToolArguments::default(),
            calls: BTreeMap::new(),
            allowed_names,
            response_id: None,
            model: None,
            reason: None,
            usage: None,
            done: false,
            closed: false,
        })
    }

    pub fn push(&mut self, chunk: &[u8]) -> Result<Vec<ChatEvent>> {
        if self.closed {
            return Err(WireError::Closed);
        }
        let result = self.consume(chunk);
        if result.is_err() {
            self.cancel();
        }
        result
    }

    fn consume(&mut self, chunk: &[u8]) -> Result<Vec<ChatEvent>> {
        let mut output = Vec::new();
        for frame in self.sse.push(chunk)? {
            self.frame(frame, &mut output)?;
        }
        Ok(output)
    }

    fn frame(&mut self, frame: SseFrame, output: &mut Vec<ChatEvent>) -> Result<()> {
        if self.done {
            return Err(WireError::InvalidResponse);
        }
        if frame.event == "error" {
            return Err(WireError::ProviderFailure);
        }
        if frame.event != "message" {
            return Err(WireError::InvalidResponse);
        }
        if frame.data == "[DONE]" {
            if self.reason.is_none() {
                return Err(WireError::TruncatedStream);
            }
            self.done = true;
            return Ok(());
        }
        let value = parse_json(&frame.data).map_err(|_| WireError::InvalidResponse)?;
        let value = object(&value)?;
        if value.contains_key("error") {
            return Err(WireError::ProviderFailure);
        }
        if required_string(value, "object")? != "chat.completion.chunk" {
            return Err(WireError::InvalidResponse);
        }
        let id = required_string(value, "id")?;
        let model = required_string(value, "model")?;
        if !valid_id(id) || !valid_id(model) {
            return Err(WireError::InvalidResponse);
        }
        if self.response_id.as_deref().is_some_and(|old| old != id)
            || self.model.as_deref().is_some_and(|old| old != model)
        {
            return Err(WireError::InvalidResponse);
        }
        self.response_id.get_or_insert_with(|| id.into());
        self.model.get_or_insert_with(|| model.into());
        let choices = value
            .get("choices")
            .and_then(Value::as_array)
            .ok_or(WireError::InvalidResponse)?;
        if choices.len() > 1 {
            return Err(WireError::UnsupportedResponse);
        }
        let reported_usage = value.get("usage").filter(|value| !value.is_null());
        if let Some(choice) = choices.first() {
            if self.reason.is_some() {
                return Err(WireError::InvalidResponse);
            }
            self.choice(choice, output)?;
        } else if self.reason.is_none() || reported_usage.is_none() {
            return Err(WireError::InvalidResponse);
        }
        if let Some(value) = reported_usage {
            if self.reason.is_none() || self.usage.is_some() {
                return Err(WireError::InvalidResponse);
            }
            self.usage = Some(usage(value)?);
        }
        Ok(())
    }

    fn choice(&mut self, value: &Value, output: &mut Vec<ChatEvent>) -> Result<()> {
        let value = object(value)?;
        if value.get("index").and_then(Value::as_u64) != Some(0) {
            return Err(WireError::UnsupportedResponse);
        }
        let delta = object(value.get("delta").ok_or(WireError::InvalidResponse)?)?;
        // Do not silently discard reasoning/signature, audio or legacy function-call data.
        for (key, value) in delta {
            if !matches!(key.as_str(), "role" | "content" | "tool_calls" | "refusal")
                && !value.is_null()
            {
                return Err(WireError::UnsupportedResponse);
            }
        }
        if let Some(role) = optional_string(delta, "role")? {
            if role != "assistant" {
                return Err(WireError::InvalidResponse);
            }
        }
        if let Some(value) = delta.get("refusal").filter(|value| !value.is_null()) {
            if !value.as_str().ok_or(WireError::InvalidResponse)?.is_empty() {
                return Err(WireError::IncompleteResponse);
            }
        }
        if let Some(value) = delta.get("content").filter(|value| !value.is_null()) {
            let text = value.as_str().ok_or(WireError::InvalidResponse)?;
            if !text.is_empty() {
                output.push(ChatEvent::TextDelta(text.into()));
            }
        }
        if let Some(value) = delta.get("tool_calls").filter(|value| !value.is_null()) {
            let calls = value.as_array().ok_or(WireError::InvalidResponse)?;
            if calls.len() > ToolArguments::MAX_CALLS {
                return Err(WireError::LimitExceeded);
            }
            let mut indices = BTreeSet::new();
            for call in calls {
                let call = object(call)?;
                let index = call
                    .get("index")
                    .and_then(Value::as_u64)
                    .and_then(|index| u32::try_from(index).ok())
                    .ok_or(WireError::InvalidResponse)?;
                if !indices.insert(index) {
                    return Err(WireError::InvalidResponse);
                }
                self.tool_delta(index, call)?;
            }
        }
        if let Some(reason) = value.get("finish_reason").filter(|value| !value.is_null()) {
            self.reason = Some(match reason.as_str() {
                Some("stop") if self.calls.is_empty() => ChatStopReason::Stop,
                Some("tool_calls") if !self.calls.is_empty() => ChatStopReason::ToolCalls,
                Some("length" | "content_filter") => return Err(WireError::IncompleteResponse),
                Some("function_call") => return Err(WireError::UnsupportedResponse),
                _ => return Err(WireError::InvalidResponse),
            });
        }
        Ok(())
    }

    fn tool_delta(&mut self, index: u32, call: &Map<String, Value>) -> Result<()> {
        let function = object(call.get("function").ok_or(WireError::InvalidResponse)?)?;
        if let Some(kind) = optional_string(call, "type")? {
            if kind != "function" {
                return Err(WireError::UnsupportedResponse);
            }
        }
        if let Some(previous) = self.calls.get(&index) {
            if optional_string(call, "id")?.is_some_and(|id| id != previous.id)
                || optional_string(function, "name")?.is_some_and(|name| name != previous.name)
            {
                return Err(WireError::InvalidResponse);
            }
        } else {
            if required_string(call, "type")? != "function" {
                return Err(WireError::UnsupportedResponse);
            }
            let id = required_string(call, "id")?;
            let name = required_string(function, "name")?;
            if !self.allowed_names.contains(name) {
                return Err(WireError::InvalidArguments);
            }
            self.arguments.start(index, id, name)?;
            self.calls.insert(
                index,
                CallHeader {
                    id: id.into(),
                    name: name.into(),
                },
            );
        }
        if let Some(fragment) = optional_string(function, "arguments")? {
            self.arguments.append(index, fragment)?;
        }
        Ok(())
    }

    pub fn finish(&mut self) -> Result<ChatCompletion> {
        if self.closed {
            return Err(WireError::Closed);
        }
        let result = self.complete();
        self.cancel();
        result
    }

    fn complete(&mut self) -> Result<ChatCompletion> {
        self.sse.finish(self.done)?;
        let reason = self.reason.ok_or(WireError::TruncatedStream)?;
        let tools = self.arguments.finish(&self.allowed_names)?;
        Ok(ChatCompletion {
            reason,
            tools,
            usage: self.usage,
        })
    }

    pub fn cancel(&mut self) {
        self.closed = true;
        self.sse.cancel();
        self.arguments.cancel();
        self.calls.clear();
        self.response_id = None;
        self.model = None;
        self.reason = None;
        self.usage = None;
        self.done = false;
    }
}
