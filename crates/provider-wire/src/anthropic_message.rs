use crate::{
    build_request, ChatCompletion, ChatStopReason, PrivacyControls, Protocol, ProviderConfig,
    ProviderConfigError, Request, Result, RetryPolicy, TimeoutPolicy, TokenUsage, ToolArguments,
    ToolProposal, WireError,
};
use reqwest::Url;
use serde_json::Value;
use std::{collections::BTreeSet, net::IpAddr, time::Duration};

const BASE_URL: &str = "https://api.anthropic.com/v1/";
const MAX_RESPONSE_BYTES: usize = 32 * 1024 * 1024;
const MAX_TEXT_BYTES: usize = 8 * 1024 * 1024;

impl ProviderConfig {
    pub fn anthropic(id: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            protocol: Protocol::AnthropicMessages,
            base_url: BASE_URL.into(),
            model: model.into(),
            privacy: PrivacyControls {
                allow_model_requests: true,
                ..PrivacyControls::default()
            },
            timeouts: TimeoutPolicy::default(),
            retry: RetryPolicy::default(),
        }
    }

    pub fn anthropic_endpoint(&self) -> std::result::Result<Url, ProviderConfigError> {
        validate_config(self)?;
        let mut base = Url::parse(&self.base_url).map_err(|_| ProviderConfigError::InvalidUrl)?;
        let path = format!("{}/", base.path().trim_end_matches('/'));
        base.set_path(&path);
        base.join("messages").map_err(|_| ProviderConfigError::InvalidUrl)
    }
}

fn validate_config(config: &ProviderConfig) -> std::result::Result<(), ProviderConfigError> {
    if config.protocol != Protocol::AnthropicMessages {
        return Err(ProviderConfigError::UnsupportedProtocol);
    }
    if !config.privacy.allow_model_requests {
        return Err(ProviderConfigError::NetworkDisabled);
    }
    if config.id.is_empty()
        || config.id.len() > 128
        || !config.id.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.')
        })
    {
        return Err(ProviderConfigError::InvalidId);
    }
    if config.model.trim().is_empty()
        || config.model.len() > 256
        || config.model.chars().any(char::is_control)
    {
        return Err(ProviderConfigError::InvalidModel);
    }
    let min = Duration::from_millis(100);
    if config.timeouts.connect < min
        || config.timeouts.idle < min
        || config.timeouts.total < config.timeouts.connect
        || config.timeouts.total < config.timeouts.idle
        || config.timeouts.total > Duration::from_secs(60 * 60)
    {
        return Err(ProviderConfigError::InvalidTimeout);
    }
    if !(1..=3).contains(&config.retry.max_attempts)
        || config.retry.base_delay > config.retry.max_delay
        || config.retry.max_delay > Duration::from_secs(30)
    {
        return Err(ProviderConfigError::InvalidRetry);
    }
    let url = Url::parse(&config.base_url).map_err(|_| ProviderConfigError::InvalidUrl)?;
    if url.cannot_be_a_base()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(ProviderConfigError::InvalidUrl);
    }
    let mut normalized = url.clone();
    normalized.set_path(&format!("{}/", url.path().trim_end_matches('/')));
    if normalized.as_str() != BASE_URL && !config.privacy.allow_custom_endpoints {
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
    let mut body = build_request(Protocol::AnthropicMessages, request)?;
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
    if input.len() > MAX_RESPONSE_BYTES {
        return Err(WireError::LimitExceeded);
    }
    if allowed_names.len() > ToolArguments::MAX_CALLS {
        return Err(WireError::InvalidRequest);
    }
    let input = std::str::from_utf8(input).map_err(|_| WireError::InvalidUtf8)?;
    let value: Value = serde_json::from_str(input).map_err(|_| WireError::InvalidResponse)?;
    let object = value.as_object().ok_or(WireError::InvalidResponse)?;
    if object.get("type").and_then(Value::as_str) != Some("message")
        || object.get("role").and_then(Value::as_str) != Some("assistant")
    {
        return Err(WireError::InvalidResponse);
    }
    let mut text = String::new();
    let mut tools = Vec::new();
    for block in object
        .get("content")
        .and_then(Value::as_array)
        .ok_or(WireError::InvalidResponse)?
    {
        match block.get("type").and_then(Value::as_str) {
            Some("text") => {
                let delta = block.get("text").and_then(Value::as_str).ok_or(WireError::InvalidResponse)?;
                if text.len().checked_add(delta.len()).is_none_or(|size| size > MAX_TEXT_BYTES) {
                    return Err(WireError::LimitExceeded);
                }
                text.push_str(delta);
            }
            Some("tool_use") => {
                let id = block.get("id").and_then(Value::as_str).ok_or(WireError::InvalidArguments)?;
                let name = block.get("name").and_then(Value::as_str).ok_or(WireError::InvalidArguments)?;
                let arguments = block.get("input").filter(|value| value.is_object()).ok_or(WireError::InvalidArguments)?;
                if !allowed_names.contains(name) || id.is_empty() || id.chars().any(char::is_control) {
                    return Err(WireError::InvalidArguments);
                }
                tools.push(ToolProposal { id: id.into(), name: name.into(), arguments: arguments.clone() });
            }
            _ => return Err(WireError::UnsupportedResponse),
        }
    }
    let reason = match object.get("stop_reason").and_then(Value::as_str) {
        Some("tool_use") if !tools.is_empty() => ChatStopReason::ToolCalls,
        Some("end_turn" | "stop_sequence") if tools.is_empty() => ChatStopReason::Stop,
        Some("max_tokens" | "pause_turn" | "refusal") => return Err(WireError::IncompleteResponse),
        _ => return Err(WireError::InvalidResponse),
    };
    let usage = object.get("usage").and_then(Value::as_object).ok_or(WireError::InvalidResponse)?;
    let input_tokens = usage.get("input_tokens").and_then(Value::as_u64).ok_or(WireError::InvalidResponse)?;
    let output_tokens = usage.get("output_tokens").and_then(Value::as_u64).ok_or(WireError::InvalidResponse)?;
    let total_tokens = input_tokens.checked_add(output_tokens).ok_or(WireError::InvalidResponse)?;
    Ok(AnthropicResponse {
        text,
        completion: ChatCompletion {
            reason,
            tools,
            usage: Some(TokenUsage { input_tokens, output_tokens, total_tokens }),
        },
    })
}
