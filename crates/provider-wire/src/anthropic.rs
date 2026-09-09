use crate::{
    json::parse_json,
    tools::{valid_id, valid_name},
    ChatCompletion, ChatEvent, ChatStopReason, PrivacyControls, ProviderConfig,
    ProviderConfigError, Request, Result, RetryPolicy, SseDecoder, SseFrame, TimeoutPolicy,
    TokenUsage, ToolArguments, WireError,
};
use serde_json::{Map, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    net::IpAddr,
};

const ANTHROPIC_BASE_URL: &str = "https://api.anthropic.com/v1/";
const MAX_RESPONSE_BYTES: usize = SseDecoder::MAX_STREAM;
const MAX_TEXT_BYTES: usize = 8 * 1024 * 1024;
const MAX_CONTENT_BLOCKS: usize = 4096;

impl ProviderConfig {
    pub fn anthropic(id: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            protocol: crate::Protocol::AnthropicMessages,
            base_url: ANTHROPIC_BASE_URL.into(),
            model: model.into(),
            privacy: PrivacyControls {
                allow_model_requests: true,
                ..PrivacyControls::default()
            },
            timeouts: TimeoutPolicy::default(),
            retry: RetryPolicy::default(),
        }
    }

    pub fn anthropic_endpoint(&self) -> std::result::Result<reqwest::Url, ProviderConfigError> {
        validate_anthropic_config(self)?;
        let mut base = reqwest::Url::parse(&self.base_url)
            .map_err(|_| ProviderConfigError::InvalidUrl)?;
        let mut path = base.path().trim_end_matches('/').to_owned();
        path.push('/');
        base.set_path(&path);
        base.join("messages")
            .map_err(|_| ProviderConfigError::InvalidUrl)
    }
}

fn validate_anthropic_config(config: &ProviderConfig) -> std::result::Result<(), ProviderConfigError> {
    if config.protocol != crate::Protocol::AnthropicMessages {
        return Err(ProviderConfigError::UnsupportedProtocol);
    }
    if !config.privacy.allow_model_requests {
        return Err(ProviderConfigError::NetworkDisabled);
    }
    if config.id.is_empty()
        || config.id.len() > 128
        || !config
            .id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(ProviderConfigError::InvalidId);
    }
    if config.model.trim().is_empty()
        || config.model.len() > 256
        || config.model.chars().any(char::is_control)
    {
        return Err(ProviderConfigError::InvalidModel);
    }
    let min = std::time::Duration::from_millis(100);
    if config.timeouts.connect < min
        || config.timeouts.idle < min
        || config.timeouts.total < config.timeouts.connect
        || config.timeouts.total < config.timeouts.idle
        || config.timeouts.total > std::time::Duration::from_secs(60 * 60)
    {
        return Err(ProviderConfigError::InvalidTimeout);
    }
    if !(1..=3).contains(&config.retry.max_attempts)
        || config.retry.base_delay > config.retry.max_delay
        || config.retry.max_delay > std::time::Duration::from_secs(30)
    {
        return Err(ProviderConfigError::InvalidRetry);
    }
    let url = reqwest::Url::parse(&config.base_url).map_err(|_| ProviderConfigError::InvalidUrl)?;
    if url.cannot_be_a_base()
        || url.username() != ""
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(ProviderConfigError::InvalidUrl);
    }
    let mut normalized = url.clone();
    let mut path = normalized.path().trim_end_matches('/').to_owned();
    path.push('/');
    normalized.set_path(&path);
    let official = normalized.as_str() == ANTHROPIC_BASE_URL;
    if !official && !config.privacy.allow_custom_endpoints {
        return Err(ProviderConfigError::CustomEndpointDisabled);
    }
    match url.scheme() {
        "https" => Ok(()),
        "http"
            if config.privacy.allow_http_loopback
                && url.host_str().is_some_and(|host| {
                    host == "localhost"
                        || host
                            .trim_matches(['[', ']'])
                            .parse::<IpAddr>()
                            .is_ok_and(|address| address.is_loopback())
                }) =>
        {
            Ok(())
        }
        _ => Err(ProviderConfigError::InvalidUrl),
    }
}

pub fn build_anthropic_request(request: &Request, stream: bool) -> Result<Value> {
    let mut body = crate::build_request(crate::Protocol::AnthropicMessages, request)?;
    body["stream"] = Value::Bool(stream);
    Ok(body)
}

#[derive(Debug, Clone, PartialEq)]
pub struct AnthropicResponse {
    pub text: String,
    pub completion: ChatCompletion,
}

pub fn decode_anthropic_message(
    input: &[u8],
    allowed_names: BTreeSet<String>,
) -> Result<AnthropicResponse> {
    validate_allowed_names(&allowed_names)?;
    if input.len() > MAX_RESPONSE_BYTES {
        return Err(WireError::LimitExceeded);
    }
    let input = std::str::from_utf8(input).map_err(|_| WireError::InvalidUtf8)?;
    let value = parse_json(input).map_err(|_| WireError::InvalidResponse)?;
    let message = object(&value)?;
    validate_message_header(message)?;
    let content = message
        .get("content")
        .and_then(Value::as_array)
        .ok_or(WireError::InvalidResponse)?;
    if content.is_empty() || content.len() > MAX_CONTENT_BLOCKS {
        return Err(WireError::InvalidResponse);
    }

    let mut text = String::new();
    let mut arguments = ToolArguments::default();
    let mut has_tools = false;
    for (index, block) in content.iter().enumerate() {
        let block = object(block)?;
        match string(block, "type")? {
            "text" => append_text(&mut text, string(block, "text")?)?,
            "tool_use" => {
                let id = string(block, "id")?;
                let name = string(block, "name")?;
                if !valid_id(id) || !allowed_names.contains(name) {
                    return Err(WireError::InvalidArguments);
                }
                let input = block.get("input").ok_or(WireError::InvalidResponse)?;
                if !input.is_object() {
                    return Err(WireError::InvalidArguments);
                }
                let encoded = serde_json::to_string(input).map_err(|_| WireError::InvalidArguments)?;
                let index = u32::try_from(index).map_err(|_| WireError::LimitExceeded)?;
                arguments.start(index, id, name)?;
                arguments.append(index, &encoded)?;
                has_tools = true;
            }
            _ => return Err(WireError::UnsupportedResponse),
        }
    }

    let reason = stop_reason(message, has_tools)?;
    let tools = arguments.finish(&allowed_names)?;
    validate_reason(reason, tools.is_empty())?;
    if text.is_empty() && tools.is_empty() {
        return Err(WireError::InvalidResponse);
    }
    Ok(AnthropicResponse {
        text,
        completion: ChatCompletion {
            reason,
            tools,
            usage: Some(usage(message.get("usage").ok_or(WireError::InvalidResponse)?)?),
        },
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BlockKind {
    Text,
    Tool,
}

#[derive(Debug, Clone, Copy)]
struct BlockState {
    kind: BlockKind,
    closed: bool,
}

/// Strict Anthropic Messages SSE normalizer. Tool-use blocks are accumulated
/// but only returned by `finish` after `message_stop` confirms success.
pub struct AnthropicDecoder {
    sse: SseDecoder,
    allowed_names: BTreeSet<String>,
    arguments: ToolArguments,
    blocks: BTreeMap<u32, BlockState>,
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    reason: Option<ChatStopReason>,
    text_bytes: usize,
    started: bool,
    stopped: bool,
    closed: bool,
}

impl AnthropicDecoder {
    pub fn new(allowed_names: BTreeSet<String>) -> Result<Self> {
        validate_allowed_names(&allowed_names)?;
        Ok(Self {
            sse: SseDecoder::default(),
            allowed_names,
            arguments: ToolArguments::default(),
            blocks: BTreeMap::new(),
            input_tokens: None,
            output_tokens: None,
            reason: None,
            text_bytes: 0,
            started: false,
            stopped: false,
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
        let mut events = Vec::new();
        for frame in self.sse.push(chunk)? {
            self.frame(frame, &mut events)?;
        }
        Ok(events)
    }

    fn frame(&mut self, frame: SseFrame, events: &mut Vec<ChatEvent>) -> Result<()> {
        if self.stopped {
            return Err(WireError::InvalidResponse);
        }
        if frame.event == "error" {
            return Err(WireError::ProviderFailure);
        }
        let value = parse_json(&frame.data).map_err(|_| WireError::InvalidResponse)?;
        let event = object(&value)?;
        let kind = string(event, "type")?;
        if frame.event != "message" && frame.event != kind {
            return Err(WireError::InvalidResponse);
        }
        match kind {
            "ping" => Ok(()),
            "message_start" => self.message_start(event),
            "content_block_start" => self.block_start(event, events),
            "content_block_delta" => self.block_delta(event, events),
            "content_block_stop" => self.block_stop(event),
            "message_delta" => self.message_delta(event),
            "message_stop" => self.message_stop(),
            "error" => Err(WireError::ProviderFailure),
            _ => Err(WireError::UnsupportedResponse),
        }
    }

    fn message_start(&mut self, event: &Map<String, Value>) -> Result<()> {
        if self.started || !self.blocks.is_empty() {
            return Err(WireError::InvalidResponse);
        }
        let message = object(event.get("message").ok_or(WireError::InvalidResponse)?)?;
        validate_message_header(message)?;
        if message.get("stop_reason").is_some_and(|value| !value.is_null())
            || message
                .get("content")
                .and_then(Value::as_array)
                .is_none_or(|content| !content.is_empty())
        {
            return Err(WireError::InvalidResponse);
        }
        self.input_tokens = Some(usage_number(
            message.get("usage").ok_or(WireError::InvalidResponse)?,
            "input_tokens",
        )?);
        self.started = true;
        Ok(())
    }

    fn block_start(
        &mut self,
        event: &Map<String, Value>,
        events: &mut Vec<ChatEvent>,
    ) -> Result<()> {
        self.require_started()?;
        let index = index(event)?;
        if self.blocks.len() >= MAX_CONTENT_BLOCKS || self.blocks.contains_key(&index) {
            return Err(WireError::InvalidResponse);
        }
        let block = object(event.get("content_block").ok_or(WireError::InvalidResponse)?)?;
        let kind = match string(block, "type")? {
            "text" => {
                let text = string(block, "text")?;
                self.push_text(text, events)?;
                BlockKind::Text
            }
            "tool_use" => {
                let id = string(block, "id")?;
                let name = string(block, "name")?;
                if !valid_id(id) || !self.allowed_names.contains(name) {
                    return Err(WireError::InvalidArguments);
                }
                let input = block.get("input").ok_or(WireError::InvalidResponse)?;
                if input.as_object().is_none_or(|input| !input.is_empty()) {
                    return Err(WireError::InvalidArguments);
                }
                self.arguments.start(index, id, name)?;
                BlockKind::Tool
            }
            _ => return Err(WireError::UnsupportedResponse),
        };
        self.blocks.insert(
            index,
            BlockState {
                kind,
                closed: false,
            },
        );
        Ok(())
    }

    fn block_delta(
        &mut self,
        event: &Map<String, Value>,
        events: &mut Vec<ChatEvent>,
    ) -> Result<()> {
        self.require_started()?;
        let index = index(event)?;
        let state = self.blocks.get(&index).ok_or(WireError::InvalidResponse)?;
        if state.closed {
            return Err(WireError::InvalidResponse);
        }
        let kind = state.kind;
        let delta = object(event.get("delta").ok_or(WireError::InvalidResponse)?)?;
        match (kind, string(delta, "type")?) {
            (BlockKind::Text, "text_delta") => {
                self.push_text(string(delta, "text")?, events)
            }
            (BlockKind::Tool, "input_json_delta") => {
                self.arguments.append(index, string(delta, "partial_json")?)
            }
            _ => Err(WireError::UnsupportedResponse),
        }
    }

    fn block_stop(&mut self, event: &Map<String, Value>) -> Result<()> {
        self.require_started()?;
        let state = self
            .blocks
            .get_mut(&index(event)?)
            .ok_or(WireError::InvalidResponse)?;
        if state.closed {
            return Err(WireError::InvalidResponse);
        }
        state.closed = true;
        Ok(())
    }

    fn message_delta(&mut self, event: &Map<String, Value>) -> Result<()> {
        self.require_started()?;
        if self.reason.is_some() || self.blocks.values().any(|block| !block.closed) {
            return Err(WireError::InvalidResponse);
        }
        let delta = object(event.get("delta").ok_or(WireError::InvalidResponse)?)?;
        let has_tools = self
            .blocks
            .values()
            .any(|block| block.kind == BlockKind::Tool);
        self.reason = Some(stop_reason(delta, has_tools)?);
        self.output_tokens = Some(usage_number(
            event.get("usage").ok_or(WireError::InvalidResponse)?,
            "output_tokens",
        )?);
        Ok(())
    }

    fn message_stop(&mut self) -> Result<()> {
        self.require_started()?;
        if self.reason.is_none() || self.blocks.values().any(|block| !block.closed) {
            return Err(WireError::TruncatedStream);
        }
        self.stopped = true;
        Ok(())
    }

    fn require_started(&self) -> Result<()> {
        if self.started && !self.stopped {
            Ok(())
        } else {
            Err(WireError::InvalidResponse)
        }
    }

    fn push_text(&mut self, text: &str, events: &mut Vec<ChatEvent>) -> Result<()> {
        self.text_bytes = self
            .text_bytes
            .checked_add(text.len())
            .ok_or(WireError::LimitExceeded)?;
        if self.text_bytes > MAX_TEXT_BYTES {
            return Err(WireError::LimitExceeded);
        }
        if !text.is_empty() {
            events.push(ChatEvent::TextDelta(text.into()));
        }
        Ok(())
    }

    pub fn finish(&mut self) -> Result<ChatCompletion> {
        if self.closed {
            return Err(WireError::Closed);
        }
        self.closed = true;
        self.sse.finish(self.stopped)?;
        if !self.stopped {
            return Err(WireError::TruncatedStream);
        }
        let reason = self.reason.ok_or(WireError::TruncatedStream)?;
        let tools = self.arguments.finish(&self.allowed_names)?;
        validate_reason(reason, tools.is_empty())?;
        let input_tokens = self.input_tokens.ok_or(WireError::InvalidResponse)?;
        let output_tokens = self.output_tokens.ok_or(WireError::InvalidResponse)?;
        let total_tokens = input_tokens
            .checked_add(output_tokens)
            .ok_or(WireError::InvalidResponse)?;
        Ok(ChatCompletion {
            reason,
            tools,
            usage: Some(TokenUsage {
                input_tokens,
                output_tokens,
                total_tokens,
            }),
        })
    }

    pub fn cancel(&mut self) {
        self.closed = true;
        self.sse.cancel();
        self.arguments.cancel();
        self.blocks.clear();
        self.input_tokens = None;
        self.output_tokens = None;
        self.reason = None;
        self.text_bytes = 0;
        self.started = false;
        self.stopped = false;
    }
}

fn validate_allowed_names(allowed_names: &BTreeSet<String>) -> Result<()> {
    if allowed_names.len() > ToolArguments::MAX_CALLS
        || allowed_names.iter().any(|name| !valid_name(name))
    {
        Err(WireError::InvalidRequest)
    } else {
        Ok(())
    }
}

fn validate_message_header(message: &Map<String, Value>) -> Result<()> {
    if string(message, "type")? != "message"
        || string(message, "role")? != "assistant"
        || !valid_id(string(message, "id")?)
        || !valid_id(string(message, "model")?)
    {
        return Err(WireError::InvalidResponse);
    }
    Ok(())
}

fn object(value: &Value) -> Result<&Map<String, Value>> {
    value.as_object().ok_or(WireError::InvalidResponse)
}

fn string<'a>(object: &'a Map<String, Value>, key: &str) -> Result<&'a str> {
    object
        .get(key)
        .and_then(Value::as_str)
        .ok_or(WireError::InvalidResponse)
}

fn index(object: &Map<String, Value>) -> Result<u32> {
    object
        .get("index")
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .ok_or(WireError::InvalidResponse)
}

fn stop_reason(object: &Map<String, Value>, has_tools: bool) -> Result<ChatStopReason> {
    match string(object, "stop_reason")? {
        "end_turn" | "stop_sequence" if !has_tools => Ok(ChatStopReason::Stop),
        "tool_use" if has_tools => Ok(ChatStopReason::ToolCalls),
        "max_tokens" => Err(WireError::IncompleteResponse),
        "refusal" | "pause_turn" => Err(WireError::IncompleteResponse),
        _ => Err(WireError::InvalidResponse),
    }
}

fn validate_reason(reason: ChatStopReason, tools_empty: bool) -> Result<()> {
    match (reason, tools_empty) {
        (ChatStopReason::Stop, true) | (ChatStopReason::ToolCalls, false) => Ok(()),
        _ => Err(WireError::InvalidResponse),
    }
}

fn usage(value: &Value) -> Result<TokenUsage> {
    let input_tokens = usage_number(value, "input_tokens")?;
    let output_tokens = usage_number(value, "output_tokens")?;
    let total_tokens = input_tokens
        .checked_add(output_tokens)
        .ok_or(WireError::InvalidResponse)?;
    Ok(TokenUsage {
        input_tokens,
        output_tokens,
        total_tokens,
    })
}

fn usage_number(value: &Value, key: &str) -> Result<u64> {
    object(value)?
        .get(key)
        .and_then(Value::as_u64)
        .ok_or(WireError::InvalidResponse)
}

fn append_text(output: &mut String, text: &str) -> Result<()> {
    if output
        .len()
        .checked_add(text.len())
        .is_none_or(|bytes| bytes > MAX_TEXT_BYTES)
    {
        return Err(WireError::LimitExceeded);
    }
    output.push_str(text);
    Ok(())
}
