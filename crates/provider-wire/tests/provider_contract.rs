use alicent_provider_wire::*;
use serde_json::json;
use std::{
    collections::BTreeSet,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

const SYNTHETIC_SECRET: &str = "synthetic-fixture-not-a-real-key";

#[derive(Default)]
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
        SecretString::new(SYNTHETIC_SECRET.into())
    }

    fn delete(&self, _provider_id: &str) -> std::result::Result<(), VaultError> {
        Ok(())
    }
}

fn custom_config(base_url: String, protocol: Protocol) -> ProviderConfig {
    ProviderConfig {
        id: "fixture".into(),
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
            idle: Duration::from_millis(250),
            total: Duration::from_secs(3),
        },
        retry: RetryPolicy {
            max_attempts: 3,
            base_delay: Duration::ZERO,
            max_delay: Duration::ZERO,
        },
    }
}

async fn fixture_server(
    responses: Vec<String>,
) -> (String, Arc<Mutex<Vec<String>>>, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let captured = Arc::clone(&requests);
    let task = tokio::spawn(async move {
        for response in responses {
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
                            line.strip_prefix("content-length: ")
                                .or_else(|| line.strip_prefix("Content-Length: "))
                        })
                        .and_then(|value| value.trim().parse::<usize>().ok())
                        .unwrap_or(0);
                    if request.len() >= headers_end + length {
                        break;
                    }
                }
            }
            captured
                .lock()
                .unwrap()
                .push(String::from_utf8_lossy(&request).into_owned());
            socket.write_all(response.as_bytes()).await.unwrap();
            socket.shutdown().await.unwrap();
        }
    });
    (format!("http://{address}/v1/"), requests, task)
}

fn success_response(data: &str) -> String {
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{data}",
        data.len()
    )
}

#[test]
fn endpoint_policy_supports_chat_and_responses_without_url_credential_leaks() {
    let chat = custom_config("http://127.0.0.1:1234/v1".into(), Protocol::OpenAiChat);
    assert_eq!(
        chat.endpoint().unwrap().as_str(),
        "http://127.0.0.1:1234/v1/chat/completions"
    );
    let responses = ProviderConfig {
        protocol: Protocol::OpenAiResponses,
        ..chat.clone()
    };
    assert_eq!(
        responses.endpoint().unwrap().as_str(),
        "http://127.0.0.1:1234/v1/responses"
    );

    let mut denied = chat.clone();
    denied.privacy.allow_model_requests = false;
    assert_eq!(denied.endpoint(), Err(ProviderConfigError::NetworkDisabled));
    let mut credentials = chat.clone();
    credentials.base_url = "https://user:password@example.test/v1".into();
    assert_eq!(credentials.endpoint(), Err(ProviderConfigError::InvalidUrl));
    let mut insecure_remote = chat;
    insecure_remote.base_url = "http://example.test/v1".into();
    assert_eq!(
        insecure_remote.endpoint(),
        Err(ProviderConfigError::InvalidUrl)
    );
}

#[tokio::test]
async fn transport_retries_allowlisted_status_before_stream_and_never_retries_output() {
    let sse = "data: fixture\n\n";
    let unavailable =
        "HTTP/1.1 503 Service Unavailable\r\nRetry-After: 0\r\nContent-Length: 0\r\n\r\n";
    let (base_url, requests, task) =
        fixture_server(vec![unavailable.into(), success_response(sse)]).await;
    let config = custom_config(base_url, Protocol::OpenAiResponses);
    let transport = ProviderTransport::new(Arc::new(FixtureVault));
    let mut stream = transport
        .stream(
            &config,
            json!({"model":"synthetic-model","stream":true,"store":false}),
            AbortHandle::default(),
        )
        .await
        .unwrap();
    assert_eq!(stream.next_chunk().await.unwrap().unwrap(), sse);
    assert!(stream.next_chunk().await.unwrap().is_none());
    task.await.unwrap();

    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    for request in requests.iter() {
        assert!(request.starts_with("POST /v1/responses HTTP/1.1\r\n"));
        assert!(request
            .to_ascii_lowercase()
            .contains("accept: text/event-stream"));
        assert!(request.contains("\"store\":false"));
        assert!(request.contains(&format!("Bearer {SYNTHETIC_SECRET}")));
    }
}

#[tokio::test]
async fn redirects_are_not_followed_and_public_errors_are_redacted() {
    let redirect = "HTTP/1.1 307 Temporary Redirect\r\nLocation: https://example.test/steal\r\nContent-Length: 0\r\n\r\n";
    let (base_url, requests, task) = fixture_server(vec![redirect.into()]).await;
    let config = custom_config(base_url, Protocol::OpenAiChat);
    let transport = ProviderTransport::new(Arc::new(FixtureVault));
    let error = transport
        .stream(&config, json!({}), AbortHandle::default())
        .await
        .unwrap_err();
    task.await.unwrap();
    assert_eq!(error, TransportError::UnexpectedResponse);
    assert_eq!(requests.lock().unwrap().len(), 1);
    let formatted = format!("{error:?} {error}");
    assert!(!formatted.contains(SYNTHETIC_SECRET));
}

#[test]
fn responses_decoder_normalizes_text_tools_usage_and_requires_completion() {
    let mut decoder = ResponsesDecoder::new(BTreeSet::from(["read_scene".to_owned()])).unwrap();
    let fixture = concat!(
        "event: response.output_item.added\n",
        "data: {\"type\":\"response.output_item.added\",\"output_index\":0,\"item\":{\"type\":\"function_call\",\"call_id\":\"call_1\",\"name\":\"read_scene\",\"arguments\":\"\"}}\n\n",
        "event: response.output_text.delta\n",
        "data: {\"type\":\"response.output_text.delta\",\"delta\":\"Привет\"}\n\n",
        "event: response.function_call_arguments.delta\n",
        "data: {\"type\":\"response.function_call_arguments.delta\",\"output_index\":0,\"delta\":\"{\\\"id\\\":\\\"scene-1\\\"}\"}\n\n",
        "event: response.function_call_arguments.done\n",
        "data: {\"type\":\"response.function_call_arguments.done\",\"output_index\":0,\"call_id\":\"call_1\",\"name\":\"read_scene\",\"arguments\":\"{\\\"id\\\":\\\"scene-1\\\"}\"}\n\n",
        "event: response.completed\n",
        "data: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\",\"usage\":{\"input_tokens\":4,\"output_tokens\":3,\"total_tokens\":7}}}\n\n"
    );
    let mut events = Vec::new();
    for byte in fixture.as_bytes().chunks(7) {
        events.extend(decoder.push(byte).unwrap());
    }
    assert_eq!(events, vec![ChatEvent::TextDelta("Привет".into())]);
    let completion = decoder.finish().unwrap();
    assert_eq!(completion.reason, ChatStopReason::ToolCalls);
    assert_eq!(completion.tools[0].name, "read_scene");
    assert_eq!(completion.tools[0].arguments, json!({"id":"scene-1"}));
    assert_eq!(
        completion.usage,
        Some(TokenUsage {
            input_tokens: 4,
            output_tokens: 3,
            total_tokens: 7,
        })
    );

    let mut truncated = ResponsesDecoder::new(BTreeSet::new()).unwrap();
    truncated
        .push(b"event: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"x\"}\n\n")
        .unwrap();
    assert_eq!(truncated.finish(), Err(WireError::TruncatedStream));
}

#[test]
fn secret_and_config_debug_output_never_contain_api_key_material() {
    let secret = SecretString::new(SYNTHETIC_SECRET.into()).unwrap();
    assert_eq!(format!("{secret:?}"), "SecretString([REDACTED])");
    assert!(!format!("{secret:?}").contains(SYNTHETIC_SECRET));
    let config = ProviderConfig::openai("openai", "gpt-5");
    assert!(!format!("{config:?}").contains(SYNTHETIC_SECRET));
}
