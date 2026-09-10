use crate::{
    classify_http, may_retry, CredentialVault, HttpFailure, Protocol, ProviderConfig,
    ProviderConfigError, SseDecoder, VaultError,
};
use bytes::Bytes;
use futures_util::{stream::BoxStream, StreamExt};
use reqwest::{
    header::{
        HeaderMap, HeaderName, HeaderValue, ACCEPT, AUTHORIZATION, CONTENT_TYPE, RETRY_AFTER,
    },
    redirect::Policy,
    Body, Client, StatusCode, Url,
};
use serde_json::Value;
use std::{cmp, fmt, sync::Arc, time::Duration};
use tokio::time::{sleep, timeout_at, Instant};
use tokio_util::sync::CancellationToken;
use zeroize::Zeroizing;

const MAX_RESPONSE_BYTES: usize = SseDecoder::MAX_STREAM;
const MAX_PROBE_RESPONSE_BYTES: usize = 1024 * 1024;
const ANTHROPIC_VERSION: &str = "2023-06-01";

#[derive(Clone, Default)]
pub struct AbortHandle(CancellationToken);

impl AbortHandle {
    pub fn abort(&self) {
        self.0.cancel();
    }

    pub fn is_aborted(&self) -> bool {
        self.0.is_cancelled()
    }
}

impl fmt::Debug for AbortHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AbortHandle")
            .field("aborted", &self.is_aborted())
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum TransportError {
    #[error("provider configuration was rejected")]
    Configuration,
    #[error("provider credential is unavailable")]
    Credential,
    #[error("provider authentication failed")]
    Authentication,
    #[error("provider rate limit was reached")]
    RateLimited,
    #[error("provider rejected the request")]
    InvalidRequest,
    #[error("provider endpoint was not found")]
    NotFound,
    #[error("provider server failed")]
    Server,
    #[error("provider returned an unexpected response")]
    UnexpectedResponse,
    #[error("provider connection failed")]
    Connect,
    #[error("provider request timed out")]
    Timeout,
    #[error("provider request was aborted")]
    Aborted,
    #[error("provider response exceeded the safety limit")]
    ResponseTooLarge,
}

impl From<ProviderConfigError> for TransportError {
    fn from(_: ProviderConfigError) -> Self {
        Self::Configuration
    }
}

impl From<VaultError> for TransportError {
    fn from(_: VaultError) -> Self {
        Self::Credential
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderProbe {
    pub latency: Duration,
}

pub struct ProviderTransport<V> {
    vault: Arc<V>,
}

impl<V> Clone for ProviderTransport<V> {
    fn clone(&self) -> Self {
        Self {
            vault: Arc::clone(&self.vault),
        }
    }
}

impl<V: CredentialVault> ProviderTransport<V> {
    pub fn new(vault: Arc<V>) -> Self {
        Self { vault }
    }

    /// Starts one bounded OpenAI or Anthropic streaming request. Redirects are
    /// disabled so provider credentials can never be forwarded to another origin.
    pub async fn stream(
        &self,
        config: &ProviderConfig,
        body: Value,
        abort: AbortHandle,
    ) -> Result<ProviderStream, TransportError> {
        let endpoint = configured_endpoint(config)?;
        validate_body(config, &body)?;
        let client = build_client(&endpoint, config.timeouts.connect)?;
        let authentication = authentication_headers(self.vault.as_ref(), config)?;
        let encoded_body = serde_json::to_vec(&body).map_err(|_| TransportError::Configuration)?;

        let deadline = Instant::now() + config.timeouts.total;
        let mut attempt = 0_u8;
        loop {
            if abort.is_aborted() {
                return Err(TransportError::Aborted);
            }
            let request = client
                .post(endpoint.clone())
                .headers(authentication.clone())
                .header(ACCEPT, "text/event-stream")
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(encoded_body.clone()));
            let response = tokio::select! {
                _ = abort.0.cancelled() => return Err(TransportError::Aborted),
                result = timeout_at(deadline, request.send()) => {
                    match result {
                        Ok(Ok(response)) => response,
                        Ok(Err(error)) if error.is_connect()
                            && attempt + 1 < config.retry.max_attempts => {
                                let delay = retry_delay(config, attempt, None);
                                wait(delay, deadline, &abort).await?;
                                attempt += 1;
                                continue;
                            }
                        Ok(Err(error)) if error.is_timeout() => return Err(TransportError::Timeout),
                        Ok(Err(_)) => return Err(TransportError::Connect),
                        Err(_) => return Err(TransportError::Timeout),
                    }
                }
            };

            let status = response.status();
            if status.is_success() {
                if !is_event_stream(response.headers().get(CONTENT_TYPE)) {
                    return Err(TransportError::UnexpectedResponse);
                }
                return Ok(ProviderStream {
                    chunks: response.bytes_stream().boxed(),
                    abort,
                    idle: config.timeouts.idle,
                    deadline,
                    received: 0,
                    closed: false,
                });
            }

            let retry_after = response
                .headers()
                .get(RETRY_AFTER)
                .and_then(|value| value.to_str().ok())
                .and_then(parse_retry_after);
            let status_code = status.as_u16();
            drop(response);
            if attempt + 1 < config.retry.max_attempts
                && may_retry(status_code, attempt, false, false)
            {
                let delay = retry_delay(config, attempt, retry_after);
                wait(delay, deadline, &abort).await?;
                attempt += 1;
                continue;
            }
            return Err(map_status(status));
        }
    }

    /// Performs a non-generation capability probe against the provider's
    /// bounded `GET /models` endpoint. Response bodies are validated and
    /// drained only up to a fixed limit, and are never exposed to the caller.
    pub async fn probe(
        &self,
        config: &ProviderConfig,
        abort: AbortHandle,
    ) -> Result<ProviderProbe, TransportError> {
        let endpoint = metadata_endpoint(config)?;
        let client = build_client(&endpoint, config.timeouts.connect)?;
        let authentication = authentication_headers(self.vault.as_ref(), config)?;
        let started = Instant::now();
        let deadline = started + config.timeouts.total;
        let request = client
            .get(endpoint)
            .headers(authentication)
            .header(ACCEPT, "application/json");
        let response = tokio::select! {
            _ = abort.0.cancelled() => return Err(TransportError::Aborted),
            result = timeout_at(deadline, request.send()) => {
                match result {
                    Ok(Ok(response)) => response,
                    Ok(Err(error)) if error.is_timeout() => return Err(TransportError::Timeout),
                    Ok(Err(_)) => return Err(TransportError::Connect),
                    Err(_) => return Err(TransportError::Timeout),
                }
            }
        };

        let status = response.status();
        if !status.is_success() {
            return Err(map_status(status));
        }
        if !is_json(response.headers().get(CONTENT_TYPE)) {
            return Err(TransportError::UnexpectedResponse);
        }
        if response
            .content_length()
            .is_some_and(|length| length > MAX_PROBE_RESPONSE_BYTES as u64)
        {
            return Err(TransportError::ResponseTooLarge);
        }

        let mut received = 0_usize;
        let mut chunks = response.bytes_stream();
        loop {
            let idle_deadline = cmp::min(deadline, Instant::now() + config.timeouts.idle);
            let next = tokio::select! {
                _ = abort.0.cancelled() => return Err(TransportError::Aborted),
                result = timeout_at(idle_deadline, chunks.next()) => {
                    result.map_err(|_| TransportError::Timeout)?
                }
            };
            match next {
                Some(Ok(chunk)) => {
                    received = received
                        .checked_add(chunk.len())
                        .ok_or(TransportError::ResponseTooLarge)?;
                    if received > MAX_PROBE_RESPONSE_BYTES {
                        return Err(TransportError::ResponseTooLarge);
                    }
                }
                Some(Err(error)) if error.is_timeout() => {
                    return Err(TransportError::Timeout);
                }
                Some(Err(_)) => return Err(TransportError::Connect),
                None => {
                    return Ok(ProviderProbe {
                        latency: started.elapsed(),
                    });
                }
            }
        }
    }
}

fn configured_endpoint(config: &ProviderConfig) -> Result<Url, ProviderConfigError> {
    match config.protocol {
        Protocol::OpenAiChat | Protocol::OpenAiResponses => config.endpoint(),
        Protocol::AnthropicMessages => config.anthropic_endpoint(),
    }
}

fn metadata_endpoint(config: &ProviderConfig) -> Result<Url, ProviderConfigError> {
    configured_endpoint(config)?;
    let mut base = Url::parse(&config.base_url).map_err(|_| ProviderConfigError::InvalidUrl)?;
    base.set_path(&format!("{}/", base.path().trim_end_matches('/')));
    base.join("models")
        .map_err(|_| ProviderConfigError::InvalidUrl)
}

fn build_client(endpoint: &Url, connect_timeout: Duration) -> Result<Client, TransportError> {
    Client::builder()
        .redirect(Policy::none())
        .connect_timeout(connect_timeout)
        .https_only(endpoint.scheme() == "https")
        .build()
        .map_err(|_| TransportError::Configuration)
}

fn authentication_headers<V: CredentialVault>(
    vault: &V,
    config: &ProviderConfig,
) -> Result<HeaderMap, TransportError> {
    let secret = vault.load(&config.id)?;
    let mut headers = HeaderMap::new();
    match config.protocol {
        Protocol::OpenAiChat | Protocol::OpenAiResponses => {
            let mut bearer = Zeroizing::new(String::with_capacity(secret.expose().len() + 7));
            bearer.push_str("Bearer ");
            bearer.push_str(secret.expose());
            let mut authorization = HeaderValue::from_bytes(bearer.as_bytes())
                .map_err(|_| TransportError::Credential)?;
            authorization.set_sensitive(true);
            headers.insert(AUTHORIZATION, authorization);
        }
        Protocol::AnthropicMessages => {
            let mut api_key = HeaderValue::from_bytes(secret.expose().as_bytes())
                .map_err(|_| TransportError::Credential)?;
            api_key.set_sensitive(true);
            headers.insert(HeaderName::from_static("x-api-key"), api_key);
            headers.insert(
                HeaderName::from_static("anthropic-version"),
                HeaderValue::from_static(ANTHROPIC_VERSION),
            );
        }
    }
    drop(secret);
    Ok(headers)
}

fn validate_body(config: &ProviderConfig, body: &Value) -> Result<(), TransportError> {
    let object = body.as_object().ok_or(TransportError::Configuration)?;
    if object.get("model").and_then(Value::as_str) != Some(config.model.as_str())
        || object.get("stream").and_then(Value::as_bool) != Some(true)
    {
        return Err(TransportError::Configuration);
    }
    match config.protocol {
        Protocol::OpenAiChat | Protocol::OpenAiResponses
            if object.get("store").and_then(Value::as_bool) == Some(false) =>
        {
            Ok(())
        }
        Protocol::AnthropicMessages if !object.contains_key("store") => Ok(()),
        _ => Err(TransportError::Configuration),
    }
}

fn is_event_stream(value: Option<&HeaderValue>) -> bool {
    value
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .is_some_and(|mime| mime.trim().eq_ignore_ascii_case("text/event-stream"))
}

fn is_json(value: Option<&HeaderValue>) -> bool {
    value
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .is_some_and(|mime| {
            let mime = mime.trim();
            mime.eq_ignore_ascii_case("application/json")
                || mime.to_ascii_lowercase().ends_with("+json")
        })
}

fn parse_retry_after(value: &str) -> Option<Duration> {
    value.trim().parse::<u64>().ok().map(Duration::from_secs)
}

fn retry_delay(config: &ProviderConfig, attempt: u8, retry_after: Option<Duration>) -> Duration {
    if let Some(delay) = retry_after {
        return cmp::min(delay, config.retry.max_delay);
    }
    let multiplier = 1_u32.checked_shl(attempt.into()).unwrap_or(u32::MAX);
    cmp::min(
        config.retry.base_delay.saturating_mul(multiplier),
        config.retry.max_delay,
    )
}

async fn wait(
    delay: Duration,
    deadline: Instant,
    abort: &AbortHandle,
) -> Result<(), TransportError> {
    tokio::select! {
        _ = abort.0.cancelled() => Err(TransportError::Aborted),
        result = timeout_at(deadline, sleep(delay)) => {
            result.map_err(|_| TransportError::Timeout)
        }
    }
}

fn map_status(status: StatusCode) -> TransportError {
    match classify_http(status.as_u16()) {
        HttpFailure::Authentication => TransportError::Authentication,
        HttpFailure::RateLimited => TransportError::RateLimited,
        HttpFailure::InvalidRequest => TransportError::InvalidRequest,
        HttpFailure::NotFound => TransportError::NotFound,
        HttpFailure::Server => TransportError::Server,
        HttpFailure::Other => TransportError::UnexpectedResponse,
    }
}

pub struct ProviderStream {
    chunks: BoxStream<'static, reqwest::Result<Bytes>>,
    abort: AbortHandle,
    idle: Duration,
    deadline: Instant,
    received: usize,
    closed: bool,
}

impl fmt::Debug for ProviderStream {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderStream")
            .field("received", &self.received)
            .field("closed", &self.closed)
            .finish()
    }
}

impl ProviderStream {
    /// Returns bounded raw SSE bytes. Protocol decoders remain fail-closed and
    /// must confirm their terminal event; EOF alone is not success.
    pub async fn next_chunk(&mut self) -> Result<Option<Bytes>, TransportError> {
        if self.closed {
            return Ok(None);
        }
        let idle_deadline = cmp::min(self.deadline, Instant::now() + self.idle);
        let next = tokio::select! {
            _ = self.abort.0.cancelled() => {
                self.closed = true;
                return Err(TransportError::Aborted);
            }
            result = timeout_at(idle_deadline, self.chunks.next()) => {
                result.map_err(|_| {
                    self.closed = true;
                    TransportError::Timeout
                })?
            }
        };
        match next {
            Some(Ok(chunk)) => {
                self.received = self
                    .received
                    .checked_add(chunk.len())
                    .ok_or(TransportError::ResponseTooLarge)?;
                if chunk.len() > SseDecoder::MAX_CHUNK || self.received > MAX_RESPONSE_BYTES {
                    self.closed = true;
                    return Err(TransportError::ResponseTooLarge);
                }
                Ok(Some(chunk))
            }
            Some(Err(error)) if error.is_timeout() => {
                self.closed = true;
                Err(TransportError::Timeout)
            }
            Some(Err(_)) => {
                self.closed = true;
                Err(TransportError::Connect)
            }
            None => {
                self.closed = true;
                Ok(None)
            }
        }
    }

    pub fn abort(&self) {
        self.abort.abort();
    }
}
