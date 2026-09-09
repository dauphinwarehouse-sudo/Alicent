use crate::{
    json::parse_json,
    tools::{valid_id, valid_name},
    ChatCompletion, ChatEvent, ChatStopReason, Protocol, Result as WireResult, SseDecoder,
    SseFrame, TokenUsage, ToolArguments, WireError,
};
use reqwest::Url;
use serde_json::{Map, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    net::IpAddr,
    time::Duration,
};

const OPENAI_BASE_URL: &str = "https://api.openai.com/v1/";

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct PrivacyControls {
    /// Network transmission of manuscript content must be an explicit user choice.
    pub allow_model_requests: bool,
    /// Custom endpoints can have different retention and training policies.
    pub allow_custom_endpoints: bool,
    /// Plain HTTP is only ever permitted for an explicit loopback development endpoint.
    pub allow_http_loopback: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeoutPolicy {
    pub connect: Duration,
    pub idle: Duration,
    pub total: Duration,
}

impl Default for TimeoutPolicy {
    fn default() -> Self {
        Self {
            connect: Duration::from_secs(10),
            idle: Duration::from_secs(45),
            total: Duration::from_secs(10 * 60),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryPolicy {
    /// Includes the first request. Deliberately capped at three.
    pub max_attempts: u8,
    pub base_delay: Duration,
    pub max_delay: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            base_delay: Duration::from_millis(250),
            max_delay: Duration::from_secs(5),
        }
    }
}

/// Persistable provider metadata. It intentionally cannot contain an API key.
#[derive(Clone, PartialEq, Eq)]
pub struct ProviderConfig {
    pub id: String,
    pub protocol: Protocol,
    pub base_url: String,
    pub model: String,
    pub privacy: PrivacyControls,
    pub timeouts: TimeoutPolicy,
    pub retry: RetryPolicy,
}

impl fmt::Debug for ProviderConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderConfig")
            .field("id", &self.id)
            .field("protocol", &self.protocol)
            .field("base_url", &self.base_url)
            .field("model", &self.model)
            .field("privacy", &self.privacy)
            .field("timeouts", &self.timeouts)
            .field("retry", &self.retry)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ProviderConfigError {
    #[error("model requests are disabled by privacy controls")]
    NetworkDisabled,
    #[error("custom provider endpoints are disabled by privacy controls")]
    CustomEndpointDisabled,
    #[error("provider identifier is invalid")]
    InvalidId,
    #[error("provider model is invalid")]
    InvalidModel,
    #[error("provider URL is invalid")]
    InvalidUrl,
    #[error("provider protocol is not supported by this transport")]
    UnsupportedProtocol,
    #[error("provider timeout policy is invalid")]
    InvalidTimeout,
    #[error("provider retry policy is invalid")]
    InvalidRetry,
}

impl ProviderConfig {
    pub fn openai(id: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            protocol: Protocol::OpenAiResponses,
            base_url: OPENAI_BASE_URL.into(),
            model: model.into(),
            privacy: PrivacyControls {
                allow_model_requests: true,
                ..PrivacyControls::default()
            },
            timeouts: TimeoutPolicy::default(),
            retry: RetryPolicy::default(),
        }
    }

    pub fn endpoint(&self) -> Result<Url, ProviderConfigError> {
        self.validate()?;
        let mut base = Url::parse(&self.base_url).map_err(|_| ProviderConfigError::InvalidUrl)?;
        let mut path = base.path().trim_end_matches('/').to_owned();
        path.push('/');
        base.set_path(&path);
        base.join(match self.protocol {
            Protocol::OpenAiChat => "chat/completions",
            Protocol::OpenAiResponses => "responses",
            Protocol::AnthropicMessages => {
                return Err(ProviderConfigError::UnsupportedProtocol);
            }
        })
        .map_err(|_| ProviderConfigError::InvalidUrl)
    }

    pub fn validate(&self) -> Result<(), ProviderConfigError> {
        if !self.privacy.allow_model_requests {
            return Err(ProviderConfigError::NetworkDisabled);
        }
        if !valid_identifier(&self.id, 128) {
            return Err(ProviderConfigError::InvalidId);
        }
        if self.model.trim().is_empty()
            || self.model.len() > 256
            || self.model.chars().any(char::is_control)
        {
            return Err(ProviderConfigError::InvalidModel);
        }
        if !matches!(
            self.protocol,
            Protocol::OpenAiChat | Protocol::OpenAiResponses
        ) {
            return Err(ProviderConfigError::UnsupportedProtocol);
        }
        validate_timeouts(self.timeouts)?;
        validate_retry(self.retry)?;

        let url = Url::parse(&self.base_url).map_err(|_| ProviderConfigError::InvalidUrl)?;
        if url.cannot_be_a_base()
            || url.username() != ""
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(ProviderConfigError::InvalidUrl);
        }
        let official = normalized_origin_and_path(&url) == OPENAI_BASE_URL;
        if !official && !self.privacy.allow_custom_endpoints {
            return Err(ProviderConfigError::CustomEndpointDisabled);
        }
        match url.scheme() {
            "https" => {}
            "http" if self.privacy.allow_http_loopback && is_loopback(&url) => {}
            _ => return Err(ProviderConfigError::InvalidUrl),
        }
        Ok(())
    }
}

fn normalized_origin_and_path(url: &Url) -> String {
    let mut normalized = url.clone();
    normalized.set_query(None);
    normalized.set_fragment(None);
    let mut path = normalized.path().trim_end_matches('/').to_owned();
    path.push('/');
    normalized.set_path(&path);
    normalized.to_string()
}

fn is_loopback(url: &Url) -> bool {
    match url.host_str() {
        Some("localhost") => true,
        Some(host) => host
            .trim_matches(['[', ']'])
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback()),
        None => false,
    }
}

fn valid_identifier(value: &str, max: usize) -> bool {
    !value.is_empty()
        && value.len() <= max
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn validate_timeouts(policy: TimeoutPolicy) -> Result<(), ProviderConfigError> {
    let min = Duration::from_millis(100);
    let max_total = Duration::from_secs(60 * 60);
    if policy.connect < min
        || policy.idle < min
        || policy.total < policy.connect
        || policy.total < policy.idle
        || policy.total > max_total
    {
        return Err(ProviderConfigError::InvalidTimeout);
    }
    Ok(())
}

fn validate_retry(policy: RetryPolicy) -> Result<(), ProviderConfigError> {
    if !(1..=3).contains(&policy.max_attempts)
        || policy.base_delay > policy.max_delay
        || policy.max_delay > Duration::from_secs(30)
    {
        return Err(ProviderConfigError::InvalidRetry);
    }
    Ok(())
}

struct ResponseCall {
    call_id: String,
    name: String,
    has_delta: bool,
}

/// Strict OpenAI Responses SSE normalizer for text and client function calls.
/// Unknown output types, refusals, incomplete responses and provider errors fail closed.
pub struct ResponsesDecoder {
    sse: SseDecoder,
    allowed_names: BTreeSet<String>,
    arguments: ToolArguments,
    calls: BTreeMap<u32, ResponseCall>,
    usage: Option<TokenUsage>,
    completed: bool,
    closed: bool,
}

impl ResponsesDecoder {
    pub fn new(allowed_names: BTreeSet<String>) -> WireResult<Self> {
        if allowed_names.len() > ToolArguments::MAX_CALLS
            || allowed_names.iter().any(|name| !valid_name(name))
        {
            return Err(WireError::InvalidRequest);
        }
        Ok(Self {
            sse: SseDecoder::default(),
            allowed_names,
            arguments: ToolArguments::default(),
            calls: BTreeMap::new(),
            usage: None,
            completed: false,
            closed: false,
        })
    }

    pub fn push(&mut self, chunk: &[u8]) -> WireResult<Vec<ChatEvent>> {
        if self.closed {
            return Err(WireError::Closed);
        }
        let result = self.consume(chunk);
        if result.is_err() {
            self.cancel();
        }
        result
    }

    fn consume(&mut self, chunk: &[u8]) -> WireResult<Vec<ChatEvent>> {
        let mut events = Vec::new();
        for frame in self.sse.push(chunk)? {
            self.frame(frame, &mut events)?;
        }
        Ok(events)
    }

    fn frame(&mut self, frame: SseFrame, events: &mut Vec<ChatEvent>) -> WireResult<()> {
        if self.completed {
            return Err(WireError::InvalidResponse);
        }
        if frame.event == "error" {
            return Err(WireError::ProviderFailure);
        }
        let value = parse_json(&frame.data).map_err(|_| WireError::InvalidResponse)?;
        let object = response_object(&value)?;
        let kind = response_string(object, "type")?;
        if frame.event != "message" && frame.event != kind {
            return Err(WireError::InvalidResponse);
        }
        match kind {
            "response.created"
            | "response.in_progress"
            | "response.output_item.done"
            | "response.content_part.added"
            | "response.content_part.done"
            | "response.output_text.done" => Ok(()),
            "response.output_text.delta" => {
                let delta = response_string(object, "delta")?;
                if !delta.is_empty() {
                    events.push(ChatEvent::TextDelta(delta.into()));
                }
                Ok(())
            }
            "response.output_item.added" => self.start_call(object),
            "response.function_call_arguments.delta" => self.call_delta(object),
            "response.function_call_arguments.done" => self.call_done(object),
            "response.completed" => self.complete_event(object),
            "response.incomplete" => Err(WireError::IncompleteResponse),
            "response.failed" | "error" => Err(WireError::ProviderFailure),
            "response.refusal.delta" | "response.refusal.done" => {
                Err(WireError::IncompleteResponse)
            }
            _ => Err(WireError::UnsupportedResponse),
        }
    }

    fn start_call(&mut self, object: &Map<String, Value>) -> WireResult<()> {
        let item = response_object(object.get("item").ok_or(WireError::InvalidResponse)?)?;
        match response_string(item, "type")? {
            "message" => Ok(()),
            "function_call" => {
                let index = response_index(object)?;
                let call_id = response_string(item, "call_id")?;
                let name = response_string(item, "name")?;
                if self.calls.contains_key(&index)
                    || !valid_id(call_id)
                    || !self.allowed_names.contains(name)
                {
                    return Err(WireError::InvalidArguments);
                }
                self.arguments.start(index, call_id, name)?;
                let arguments = item.get("arguments").and_then(Value::as_str).unwrap_or("");
                if !arguments.is_empty() {
                    self.arguments.append(index, arguments)?;
                }
                self.calls.insert(
                    index,
                    ResponseCall {
                        call_id: call_id.into(),
                        name: name.into(),
                        has_delta: !arguments.is_empty(),
                    },
                );
                Ok(())
            }
            _ => Err(WireError::UnsupportedResponse),
        }
    }

    fn call_delta(&mut self, object: &Map<String, Value>) -> WireResult<()> {
        let index = response_index(object)?;
        let delta = response_string(object, "delta")?;
        let call = self
            .calls
            .get_mut(&index)
            .ok_or(WireError::InvalidArguments)?;
        self.arguments.append(index, delta)?;
        call.has_delta = true;
        Ok(())
    }

    fn call_done(&mut self, object: &Map<String, Value>) -> WireResult<()> {
        let index = response_index(object)?;
        let arguments = response_string(object, "arguments")?;
        let call = self.calls.get(&index).ok_or(WireError::InvalidArguments)?;
        if object
            .get("name")
            .and_then(Value::as_str)
            .is_some_and(|name| name != call.name)
            || object
                .get("call_id")
                .and_then(Value::as_str)
                .is_some_and(|id| id != call.call_id)
        {
            return Err(WireError::InvalidResponse);
        }
        if !call.has_delta {
            self.arguments.append(index, arguments)?;
        }
        Ok(())
    }

    fn complete_event(&mut self, object: &Map<String, Value>) -> WireResult<()> {
        let response = response_object(object.get("response").ok_or(WireError::InvalidResponse)?)?;
        if response_string(response, "status")? != "completed" {
            return Err(WireError::IncompleteResponse);
        }
        if let Some(usage) = response.get("usage").filter(|usage| !usage.is_null()) {
            self.usage = Some(response_usage(usage)?);
        }
        self.completed = true;
        Ok(())
    }

    pub fn finish(&mut self) -> WireResult<ChatCompletion> {
        if self.closed {
            return Err(WireError::Closed);
        }
        self.closed = true;
        self.sse.finish(self.completed)?;
        if !self.completed {
            return Err(WireError::TruncatedStream);
        }
        let tools = self.arguments.finish(&self.allowed_names)?;
        Ok(ChatCompletion {
            reason: if tools.is_empty() {
                ChatStopReason::Stop
            } else {
                ChatStopReason::ToolCalls
            },
            tools,
            usage: self.usage,
        })
    }

    pub fn cancel(&mut self) {
        self.closed = true;
        self.sse.cancel();
        self.arguments.cancel();
        self.calls.clear();
        self.usage = None;
        self.completed = false;
    }
}

fn response_object(value: &Value) -> WireResult<&Map<String, Value>> {
    value.as_object().ok_or(WireError::InvalidResponse)
}

fn response_string<'a>(object: &'a Map<String, Value>, key: &str) -> WireResult<&'a str> {
    object
        .get(key)
        .and_then(Value::as_str)
        .ok_or(WireError::InvalidResponse)
}

fn response_index(object: &Map<String, Value>) -> WireResult<u32> {
    object
        .get("output_index")
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .ok_or(WireError::InvalidResponse)
}

fn response_usage(value: &Value) -> WireResult<TokenUsage> {
    let object = response_object(value)?;
    let number = |key: &str| {
        object
            .get(key)
            .and_then(Value::as_u64)
            .ok_or(WireError::InvalidResponse)
    };
    let usage = TokenUsage {
        input_tokens: number("input_tokens")?,
        output_tokens: number("output_tokens")?,
        total_tokens: number("total_tokens")?,
    };
    if usage.input_tokens.checked_add(usage.output_tokens) != Some(usage.total_tokens) {
        return Err(WireError::InvalidResponse);
    }
    Ok(usage)
}
