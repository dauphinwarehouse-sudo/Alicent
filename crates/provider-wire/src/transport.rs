use crate::{
    classify_http, may_retry, CredentialVault, HttpFailure, ProviderConfig, ProviderConfigError,
    SseDecoder, VaultError,
};
use bytes::Bytes;
use futures_util::{stream::BoxStream, StreamExt};
use reqwest::{
    header::{HeaderValue, ACCEPT, AUTHORIZATION, CONTENT_TYPE, RETRY_AFTER},
    redirect::Policy,
    Body, Client, StatusCode,
};
use serde_json::Value;
use std::{cmp, fmt, sync::Arc, time::Duration};
use tokio::time::{sleep, timeout_at, Instant};
use tokio_util::sync::CancellationToken;
use zeroize::Zeroizing;

const MAX_RESPONSE_BYTES: usize = SseDecoder::MAX_STREAM;

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

    /// Starts one OpenAI-compatible streaming request. Redirects are disabled so
    /// Authorization can never be forwarded to a different origin.
    pub async fn stream(
        &self,
        config: &ProviderConfig,
        body: Value,
        abort: AbortHandle,
    ) -> Result<ProviderStream, TransportError> {
        let endpoint = config.endpoint()?;
        validate_body(config, &body)?;
        let client = Client::builder()
            .redirect(Policy::none())
            .connect_timeout(config.timeouts.connect)
            .https_only(endpoint.scheme() == "https")
            .build()
            .map_err(|_| TransportError::Configuration)?;
        let secret = self.vault.load(&config.id)?;
        let mut bearer = Zeroizing::new(String::with_capacity(secret.expose().len() + 7));
        bearer.push_str("Bearer ");
        bearer.push_str(secret.expose());
        let mut authorization =
            HeaderValue::from_bytes(bearer.as_bytes()).map_err(|_| TransportError::Credential)?;
        authorization.set_sensitive(true);
        drop(bearer);
        drop(secret);

        let deadline = Instant::now() + config.timeouts.total;
        let mut attempt = 0_u8;
        loop {
            if abort.is_aborted() {
                return Err(TransportError::Aborted);
            }
            let request = client
                .post(endpoint.clone())
                .header(AUTHORIZATION, authorization.clone())
                .header(ACCEPT, "text/event-stream")
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::to_vec(&body).map_err(|_| TransportError::Configuration)?,
                ));
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
}

fn validate_body(config: &ProviderConfig, body: &Value) -> Result<(), TransportError> {
    let object = body.as_object().ok_or(TransportError::Configuration)?;
    if object.get("model").and_then(Value::as_str) != Some(config.model.as_str())
        || object.get("stream").and_then(Value::as_bool) != Some(true)
        || object.get("store").and_then(Value::as_bool) != Some(false)
    {
        return Err(TransportError::Configuration);
    }
    Ok(())
}

fn is_event_stream(value: Option<&HeaderValue>) -> bool {
    value
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .is_some_and(|mime| mime.trim().eq_ignore_ascii_case("text/event-stream"))
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
