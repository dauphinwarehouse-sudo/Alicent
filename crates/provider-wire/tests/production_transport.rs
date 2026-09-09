use alicent_provider_wire::*;
use serde_json::json;
use std::{sync::Arc, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::oneshot,
};

const FIXTURE_CREDENTIAL: &str = "synthetic-provider-fixture";

struct FixtureVault;

impl CredentialVault for FixtureVault {
    fn store(
        &self,
        _provider_id: &str,
        _secret: SecretString,
    ) -> std::result::Result<(), VaultError> {
        Ok(())
    }

    fn load(&self, _provider_id: &str) -> std::result::Result<SecretString, VaultError> {
        SecretString::new(FIXTURE_CREDENTIAL.into())
    }

    fn delete(&self, _provider_id: &str) -> std::result::Result<(), VaultError> {
        Ok(())
    }
}

fn config(base_url: String, protocol: Protocol) -> ProviderConfig {
    ProviderConfig {
        id: "fixture-provider".into(),
        protocol,
        base_url,
        model: "synthetic-model".into(),
        privacy: PrivacyControls {
            allow_model_requests: true,
            allow_custom_endpoints: true,
            allow_http_loopback: true,
        },
        timeouts: TimeoutPolicy {
            connect: Duration::from_secs(1),
            idle: Duration::from_secs(1),
            total: Duration::from_secs(3),
        },
        retry: RetryPolicy {
            max_attempts: 1,
            ..RetryPolicy::default()
        },
    }
}

async fn fixture_server(
    response: String,
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
            if let Some(headers_end) = request.windows(4).position(|part| part == b"\r\n\r\n") {
                let headers_end = headers_end + 4;
                let headers = String::from_utf8_lossy(&request[..headers_end]);
                let length = headers
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length: ")
                            .and_then(|value| value.trim().parse::<usize>().ok())
                    })
                    .unwrap_or(0);
                if request.len() >= headers_end + length {
                    break;
                }
            }
        }
        let _ = send_request.send(String::from_utf8_lossy(&request).into_owned());
        socket.write_all(response.as_bytes()).await.unwrap();
        socket.shutdown().await.unwrap();
    });
    (format!("{{http://{address}}}/v1/"), receive_request, task)
}

fn response(content_type: &str, body: &str) -> String {
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

#[tokio::test]
async fn anthropic_stream_uses_native_auth_and_bounded_messages_endpoint() {
    let sse = "event: ping\ndata: {\"type\":\"ping\"}\n\n";
    let (base_url, request, task) = fixture_server(response("text/event-stream", sse)).await;
    let transport = ProviderTransport::new(Arc::new(FixtureVault));
    let config = config(base_url, Protocol::AnthropicMessages);
    let mut stream = transport
        .stream(
            &config,
            json!({"model":"synthetic-model","stream":true,"max_tokens":1}),
            AbortHandle::default(),
        )
        .await
        .unwrap();
    assert_eq!(stream.next_chunk().await.unwrap().unwrap(), sse);
    assert!(stream.next_chunk().await.unwrap().is_none());
    task.await.unwrap();

    let request = request.await.unwrap();
    let normalized = request.to_ascii_lowercase();
    assert!(request.starts_with("POST /v1/messages HTTP/1.1\r\n"));
    assert!(normalized.contains(&format!("x-api-key: {FIXTURE_CREDENTIAL}")));
    assert!(normalized.contains("anthropic-version: 2023-06-01"));
    assert!(!normalized.contains("authorization:"));
    assert!(!request.contains("\"store\""));
}

#[tokio::test]
async fn openai_probe_is_non_generation_and_never_returns_the_body() {
    let body = "{\"data\":[]}";
    let (base_url, request, task) = fixture_server(response("application/json", body)).await;
    let transport = ProviderTransport::new(Arc::new(FixtureVault));
    let config = config(base_url, Protocol::OpenAiResponses);
    let probe = transport
        .probe(&config, AbortHandle::default())
        .await
        .unwrap();
    assert!(probe.latency <= config.timeouts.total);
    task.await.unwrap();

    let request = request.await.unwrap();
    let normalized = request.to_ascii_lowercase();
    assert!(request.starts_with("GET /v1/models HTTP/1.1\r\n"));
    assert!(normalized.contains(&format!("authorization: bearer {FIXTURE_CREDENTIAL}")));
    assert!(!normalized.contains("content-length:"));
    assert!(!request.contains(body));
}

#[tokio::test]
async fn probe_rejects_an_oversized_fixture_before_reading_it() {
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        1024 * 1024 + 1
    );
    let (base_url, request, task) = fixture_server(response).await;
    let transport = ProviderTransport::new(Arc::new(FixtureVault));
    let config = config(base_url, Protocol::AnthropicMessages);
    assert_eq!(
        transport.probe(&config, AbortHandle::default()).await,
        Err(TransportError::ResponseTooLarge)
    );
    task.await.unwrap();
    assert!(request.await.unwrap().starts_with("GET /v1/models "));
}

#[tokio::test]
async fn anthropic_body_and_loopback_policy_fail_closed() {
    let transport = ProviderTransport::new(Arc::new(FixtureVault));
    let config = ProviderConfig::anthropic("anthropic", "synthetic-model");
    assert!(matches!(
        transport
            .stream(
                &config,
                json!({"model":"synthetic-model","stream":true,"store":false}),
                AbortHandle::default(),
            )
            .await,
        Err(TransportError::Configuration)
    ));

    let mut loopback = config;
    loopback.base_url = "http://127.0.0.1:3210/v1/".into();
    loopback.privacy.allow_custom_endpoints = true;
    assert_eq!(
        loopback.anthropic_endpoint(),
        Err(ProviderConfigError::InvalidUrl)
    );
    loopback.privacy.allow_http_loopback = true;
    assert!(loopback.anthropic_endpoint().is_ok());
}
