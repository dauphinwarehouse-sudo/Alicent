use alicent_provider_wire::*;
use serde_json::{json, Value};
use std::{
    collections::BTreeSet,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::oneshot,
};
use zeroize::Zeroizing;

const FIXTURE_CREDENTIAL: &str = "synthetic-anthropic-fixture";
const PRIVATE_FIXTURE_TEXT: &str = "private manuscript fixture must stay out of diagnostics";

struct FixtureVault {
    loads: Arc<AtomicUsize>,
}

impl FixtureVault {
    fn new() -> (Arc<Self>, Arc<AtomicUsize>) {
        let loads = Arc::new(AtomicUsize::new(0));
        (
            Arc::new(Self {
                loads: Arc::clone(&loads),
            }),
            loads,
        )
    }
}

impl CredentialVault for FixtureVault {
    fn store(
        &self,
        _provider_id: &str,
        _secret: SecretString,
    ) -> std::result::Result<(), VaultError> {
        Ok(())
    }

    fn load(&self, _provider_id: &str) -> std::result::Result<SecretString, VaultError> {
        self.loads.fetch_add(1, Ordering::Relaxed);
        SecretString::new(FIXTURE_CREDENTIAL.into())
    }

    fn delete(&self, _provider_id: &str) -> std::result::Result<(), VaultError> {
        Ok(())
    }
}

fn anthropic_config(base_url: String) -> ProviderConfig {
    ProviderConfig {
        id: "anthropic-fixture".into(),
        protocol: Protocol::AnthropicMessages,
        base_url,
        model: "claude-fixture".into(),
        privacy: PrivacyControls {
            allow_model_requests: true,
            allow_custom_endpoints: true,
            allow_http_loopback: true,
        },
        timeouts: TimeoutPolicy {
            connect: Duration::from_secs(1),
            idle: Duration::from_millis(250),
            total: Duration::from_secs(3),
        },
        retry: RetryPolicy {
            max_attempts: 1,
            ..RetryPolicy::default()
        },
    }
}

fn body() -> Value {
    json!({
        "model": "claude-fixture",
        "stream": true,
        "max_tokens": 32,
        "messages": [{"role": "user", "content": PRIVATE_FIXTURE_TEXT}],
    })
}

fn complete_request_len(request: &[u8]) -> Option<usize> {
    let header_end = request.windows(4).position(|part| part == b"\r\n\r\n")?;
    let headers = String::from_utf8_lossy(&request[..header_end]);
    let content_length = headers
        .lines()
        .filter_map(|line| line.split_once(':'))
        .find_map(|(name, value)| {
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())
                .flatten()
        })
        .unwrap_or(0);
    Some(header_end + 4 + content_length)
}

async fn scripted_server(
    response: Vec<u8>,
    write_chunk_size: usize,
    write_delay: Duration,
) -> (
    String,
    oneshot::Receiver<String>,
    tokio::task::JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (send_request, receive_request) = oneshot::channel();
    let task = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        let mut buffer = [0_u8; 4096];
        loop {
            let read = socket.read(&mut buffer).await.unwrap();
            if read == 0 {
                break;
            }
            request.extend_from_slice(&buffer[..read]);
            if complete_request_len(&request).is_some_and(|length| request.len() >= length) {
                break;
            }
        }
        let _ = send_request.send(String::from_utf8_lossy(&request).into_owned());
        for chunk in response.chunks(write_chunk_size.max(1)) {
            if socket.write_all(chunk).await.is_err() {
                break;
            }
            if !write_delay.is_zero() {
                tokio::time::sleep(write_delay).await;
            }
        }
        let _ = socket.shutdown().await;
    });
    (
        ["http://", &address.to_string(), "/v1/"].concat(),
        receive_request,
        task,
    )
}

fn http_response(status: u16, content_type: &str, body: &[u8]) -> Vec<u8> {
    format!(
        "HTTP/1.1 {status} Fixture\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .into_bytes()
    .into_iter()
    .chain(body.iter().copied())
    .collect()
}

const COMPLETE_STREAM: &str = concat!(
    "event: message_start\n",
    "data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_fixture\",\"type\":\"message\",\"role\":\"assistant\",\"model\":\"claude-fixture\",\"content\":[],\"usage\":{\"input_tokens\":4}}}\n\n",
    "event: content_block_start\n",
    "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
    "event: content_block_delta\n",
    "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Привет ❄\"}}\n\n",
    "event: content_block_stop\n",
    "data: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
    "event: message_delta\n",
    "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":3}}\n\n",
    "event: message_stop\n",
    "data: {\"type\":\"message_stop\"}\n\n",
);

#[tokio::test]
async fn native_anthropic_stream_survives_split_network_chunks() {
    let response = http_response(
        200,
        "text/event-stream; charset=utf-8",
        COMPLETE_STREAM.as_bytes(),
    );
    let (base_url, captured_request, task) =
        scripted_server(response, 3, Duration::from_millis(1)).await;
    let (vault, _) = FixtureVault::new();
    let config = anthropic_config(base_url);
    let mut stream = ProviderTransport::new(vault)
        .stream(&config, body(), AbortHandle::default())
        .await
        .unwrap();
    let mut decoder = AnthropicDecoder::new(BTreeSet::new()).unwrap();
    let mut text = String::new();
    while let Some(chunk) = stream.next_chunk().await.unwrap() {
        for event in decoder.push(&chunk).unwrap() {
            let ChatEvent::TextDelta(delta) = event;
            text.push_str(&delta);
        }
    }
    let completion = decoder.finish().unwrap();
    assert_eq!(text, "Привет ❄");
    assert_eq!(completion.reason, ChatStopReason::Stop);
    assert_eq!(
        completion.usage,
        Some(TokenUsage {
            input_tokens: 4,
            output_tokens: 3,
            total_tokens: 7,
        })
    );

    let request = captured_request.await.unwrap();
    let normalized = request.to_ascii_lowercase();
    assert!(request.starts_with("POST /v1/messages HTTP/1.1\r\n"));
    assert!(normalized.contains("accept: text/event-stream"));
    assert!(normalized.contains("content-type: application/json"));
    assert!(normalized.contains("anthropic-version: 2023-06-01"));
    assert!(request.contains(&format!("x-api-key: {FIXTURE_CREDENTIAL}")));
    assert!(!normalized.contains("authorization:"));
    let diagnostic = format!("{:?}", ChatEvent::TextDelta(PRIVATE_FIXTURE_TEXT.into()));
    assert!(!diagnostic.contains(PRIVATE_FIXTURE_TEXT));
    task.await.unwrap();
}

#[tokio::test]
async fn anthropic_http_failures_are_allowlisted_and_redacted() {
    for (status, expected) in [
        (400, TransportError::InvalidRequest),
        (401, TransportError::Authentication),
        (403, TransportError::Authentication),
        (404, TransportError::NotFound),
        (429, TransportError::RateLimited),
        (500, TransportError::Server),
        (503, TransportError::Server),
    ] {
        let response = http_response(
            status,
            "application/json",
            PRIVATE_FIXTURE_TEXT.as_bytes(),
        );
        let (base_url, _request, task) =
            scripted_server(response, usize::MAX, Duration::ZERO).await;
        let (vault, _) = FixtureVault::new();
        let error = ProviderTransport::new(vault)
            .stream(
                &anthropic_config(base_url),
                body(),
                AbortHandle::default(),
            )
            .await
            .unwrap_err();
        assert_eq!(error, expected, "status {status}");
        let diagnostic = format!("{error:?}: {error}");
        assert!(!diagnostic.contains(FIXTURE_CREDENTIAL));
        assert!(!diagnostic.contains(PRIVATE_FIXTURE_TEXT));
        task.await.unwrap();
    }
}

#[tokio::test]
async fn malformed_json_sse_and_truncated_streams_fail_closed() {
    let malformed_cases = [
        b"event: message_start\ndata: {\"type\":\n\n".as_slice(),
        b"event: message_start\ndata: \xff\n\n".as_slice(),
    ];
    for malformed in malformed_cases {
        let response = http_response(200, "text/event-stream", malformed);
        let (base_url, _request, task) =
            scripted_server(response, 5, Duration::ZERO).await;
        let (vault, _) = FixtureVault::new();
        let mut stream = ProviderTransport::new(vault)
            .stream(
                &anthropic_config(base_url),
                body(),
                AbortHandle::default(),
            )
            .await
            .unwrap();
        let mut decoder = AnthropicDecoder::new(BTreeSet::new()).unwrap();
        let mut failed = false;
        while let Some(chunk) = stream.next_chunk().await.unwrap() {
            if decoder.push(&chunk).is_err() {
                failed = true;
                break;
            }
        }
        assert!(failed);
        assert_eq!(decoder.finish(), Err(WireError::Closed));
        task.await.unwrap();
    }

    let truncated = concat!(
        "event: message_start\n",
        "data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_truncated\",\"type\":\"message\",\"role\":\"assistant\",\"model\":\"claude-fixture\",\"content\":[],\"usage\":{\"input_tokens\":1}}}\n\n",
    );
    let response = http_response(200, "text/event-stream", truncated.as_bytes());
    let (base_url, _request, task) =
        scripted_server(response, 7, Duration::ZERO).await;
    let (vault, _) = FixtureVault::new();
    let mut stream = ProviderTransport::new(vault)
        .stream(
            &anthropic_config(base_url),
            body(),
            AbortHandle::default(),
        )
        .await
        .unwrap();
    let mut decoder = AnthropicDecoder::new(BTreeSet::new()).unwrap();
    while let Some(chunk) = stream.next_chunk().await.unwrap() {
        decoder.push(&chunk).unwrap();
    }
    assert_eq!(decoder.finish(), Err(WireError::TruncatedStream));
    task.await.unwrap();
}

async fn stalled_stream_server() -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = [0_u8; 4096];
        let _ = socket.read(&mut request).await;
        socket
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n",
            )
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_secs(2)).await;
    });
    (["http://", &address.to_string(), "/v1/"].concat(), task)
}

#[tokio::test]
async fn anthropic_stream_cancellation_and_idle_timeout_close_the_stream() {
    let (base_url, task) = stalled_stream_server().await;
    let (vault, _) = FixtureVault::new();
    let abort = AbortHandle::default();
    let mut stream = ProviderTransport::new(vault)
        .stream(&anthropic_config(base_url), body(), abort.clone())
        .await
        .unwrap();
    abort.abort();
    assert_eq!(stream.next_chunk().await, Err(TransportError::Aborted));
    task.abort();

    let (base_url, task) = stalled_stream_server().await;
    let (vault, _) = FixtureVault::new();
    let mut config = anthropic_config(base_url);
    config.timeouts.idle = Duration::from_millis(100);
    let mut stream = ProviderTransport::new(vault)
        .stream(&config, body(), AbortHandle::default())
        .await
        .unwrap();
    assert_eq!(stream.next_chunk().await, Err(TransportError::Timeout));
    task.abort();
}

#[tokio::test]
async fn request_and_declared_response_limits_fail_before_sensitive_work() {
    let (vault, loads) = FixtureVault::new();
    let config = ProviderConfig::anthropic("anthropic-fixture", "claude-fixture");
    let oversized = json!({
        "model": "claude-fixture",
        "stream": true,
        "max_tokens": 1,
        "messages": [{
            "role": "user",
            "content": "x".repeat(MAX_TRANSPORT_REQUEST_BYTES),
        }],
    });
    assert!(matches!(
        ProviderTransport::new(vault)
            .stream(&config, oversized, AbortHandle::default())
            .await,
        Err(TransportError::Configuration)
    ));
    assert_eq!(loads.load(Ordering::Relaxed), 0);

    let declared = MAX_TRANSPORT_RESPONSE_BYTES + 1;
    let headers = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {declared}\r\nConnection: close\r\n\r\n"
    )
    .into_bytes();
    let (base_url, _request, task) =
        scripted_server(headers, usize::MAX, Duration::ZERO).await;
    let (vault, _) = FixtureVault::new();
    assert!(matches!(
        ProviderTransport::new(vault)
            .stream(
                &anthropic_config(base_url),
                body(),
                AbortHandle::default(),
            )
            .await,
        Err(TransportError::ResponseTooLarge)
    ));
    task.await.unwrap();
}

#[test]
fn strict_anthropic_endpoint_policy_rejects_confused_origins_and_url_secrets() {
    let official = ProviderConfig::anthropic("anthropic", "claude-fixture");
    assert_eq!(
        official.anthropic_endpoint().unwrap().as_str(),
        "https://api.anthropic.com/v1/messages"
    );

    for base_url in [
        "https://api.anthropic.com.evil.example/v1/",
        "https://api.anthropic.com:444/v1/",
        "https://api.anthropic.com/v1/?redirect=evil",
        "https://user:password@api.anthropic.com/v1/",
    ] {
        let mut denied = official.clone();
        denied.base_url = base_url.into();
        assert!(denied.anthropic_endpoint().is_err(), "{base_url}");
    }
}

struct SmokeVault(Zeroizing<String>);

impl CredentialVault for SmokeVault {
    fn store(&self, _: &str, _: SecretString) -> std::result::Result<(), VaultError> {
        Err(VaultError::OperationFailed)
    }

    fn load(&self, _: &str) -> std::result::Result<SecretString, VaultError> {
        SecretString::new(self.0.as_str().to_owned())
    }

    fn delete(&self, _: &str) -> std::result::Result<(), VaultError> {
        Err(VaultError::OperationFailed)
    }
}

/// Optional real-endpoint smoke. It is inert unless explicitly enabled and it
/// never embeds or prints the credential. CI does not need provider secrets.
#[tokio::test]
async fn real_anthropic_endpoint_smoke_is_explicitly_env_gated() {
    if std::env::var("ALICENT_ANTHROPIC_SMOKE")
        .ok()
        .as_deref()
        != Some("1")
    {
        return;
    }
    let api_key = std::env::var("ANTHROPIC_API_KEY")
        .expect("ANTHROPIC_API_KEY is required when ALICENT_ANTHROPIC_SMOKE=1");
    let model = std::env::var("ALICENT_ANTHROPIC_SMOKE_MODEL")
        .expect("ALICENT_ANTHROPIC_SMOKE_MODEL is required when smoke is enabled");
    let mut config = ProviderConfig::anthropic("anthropic-smoke", model.clone());
    config.retry.max_attempts = 1;
    config.timeouts = TimeoutPolicy {
        connect: Duration::from_secs(5),
        idle: Duration::from_secs(15),
        total: Duration::from_secs(30),
    };
    let request = Request {
        model,
        max_output_tokens: 8,
        messages: vec![Message::User("Reply with OK.".into())],
        tools: Vec::new(),
    };
    let body = build_anthropic_request(&request, true).unwrap();
    let mut stream = ProviderTransport::new(Arc::new(SmokeVault(Zeroizing::new(api_key))))
        .stream(&config, body, AbortHandle::default())
        .await
        .unwrap();
    let mut decoder = AnthropicDecoder::new(BTreeSet::new()).unwrap();
    let mut text = String::new();
    while let Some(chunk) = stream.next_chunk().await.unwrap() {
        for event in decoder.push(&chunk).unwrap() {
            let ChatEvent::TextDelta(delta) = event;
            text.push_str(&delta);
        }
    }
    let completion = decoder.finish().unwrap();
    assert_eq!(completion.reason, ChatStopReason::Stop);
    assert!(!text.trim().is_empty());
}
