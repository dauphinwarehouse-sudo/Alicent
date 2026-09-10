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

async fn fixture_server(response: String) -> (String, oneshot::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (send_request, receive_request) = oneshot::channel();
    tokio::spawn(async move {
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
        socket.write_all(response.as_bytes()).await.unwrap();
    });
    let base_url = ["http://", &address.to_string(), "/v1/"].concat();
    (base_url, receive_request)
}

fn response(content_type: &str, body: &str) -> String {
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

#[tokio::test]
async fn anthropic_stream_uses_native_auth_and_messages_endpoint() {
    let sse = "event: ping\ndata: {\"type\":\"ping\"}\n\n";
    let (base_url, request) = fixture_server(response("text/event-stream", sse)).await;
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
    assert_eq!(
        stream.next_chunk().await.unwrap().unwrap().as_ref(),
        sse.as_bytes()
    );

    let request = request.await.unwrap();
    let normalized = request.to_ascii_lowercase();
    assert!(request.starts_with("POST /v1/messages HTTP/1.1\r\n"));
    assert!(normalized.contains(&format!("x-api-key: {FIXTURE_CREDENTIAL}")));
    assert!(normalized.contains("anthropic-version: 2023-06-01"));
    assert!(!normalized.contains("authorization:"));
    let body = request.split_once("\r\n\r\n").unwrap().1;
    assert!(!body.contains("\"store\""));
}

#[tokio::test]
async fn openai_probe_is_non_generation_and_bounded() {
    let (base_url, request) =
        fixture_server(response("application/json", "{\"data\":[]}")).await;
    let transport = ProviderTransport::new(Arc::new(FixtureVault));
    let config = config(base_url, Protocol::OpenAiResponses);
    let probe = transport
        .probe(&config, AbortHandle::default())
        .await
        .unwrap();
    assert!(probe.latency <= config.timeouts.total);

    let request = request.await.unwrap();
    let normalized = request.to_ascii_lowercase();
    assert!(request.starts_with("GET /v1/models HTTP/1.1\r\n"));
    assert!(normalized.contains(&format!("authorization: bearer {FIXTURE_CREDENTIAL}")));
    assert!(!normalized.contains("content-length:"));
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
