use alicent_provider_wire::*;
use serde_json::json;
use std::{sync::Arc, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

struct FixtureVault;

impl CredentialVault for FixtureVault {
    fn store(&self, _: &str, _: SecretString) -> std::result::Result<(), VaultError> {
        Ok(())
    }

    fn load(&self, _: &str) -> std::result::Result<SecretString, VaultError> {
        SecretString::new("synthetic-security-fixture".into())
    }

    fn delete(&self, _: &str) -> std::result::Result<(), VaultError> {
        Ok(())
    }
}

async fn hanging_server() -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = vec![0_u8; 4096];
        let _ = socket.read(&mut request).await.unwrap();
        socket
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n",
            )
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_secs(2)).await;
    });
    (format!("http://{address}/v1/"), task)
}

fn config(base_url: String) -> ProviderConfig {
    ProviderConfig {
        id: "security-fixture".into(),
        protocol: Protocol::OpenAiChat,
        base_url,
        model: "fixture".into(),
        privacy: PrivacyControls {
            allow_model_requests: true,
            allow_custom_endpoints: true,
            allow_http_loopback: true,
        },
        timeouts: TimeoutPolicy {
            connect: Duration::from_millis(250),
            idle: Duration::from_millis(100),
            total: Duration::from_secs(1),
        },
        retry: RetryPolicy {
            max_attempts: 1,
            ..RetryPolicy::default()
        },
    }
}

#[tokio::test]
async fn idle_timeout_closes_a_stalled_stream() {
    let (base_url, task) = hanging_server().await;
    let transport = ProviderTransport::new(Arc::new(FixtureVault));
    let mut stream = transport
        .stream(
            &config(base_url),
            json!({"model":"fixture","stream":true,"store":false}),
            AbortHandle::default(),
        )
        .await
        .unwrap();
    assert_eq!(stream.next_chunk().await, Err(TransportError::Timeout));
    task.abort();
}

#[tokio::test]
async fn abort_interrupts_waiting_for_stream_bytes() {
    let (base_url, task) = hanging_server().await;
    let transport = ProviderTransport::new(Arc::new(FixtureVault));
    let abort = AbortHandle::default();
    let mut stream = transport
        .stream(
            &config(base_url),
            json!({"model":"fixture","stream":true,"store":false}),
            abort.clone(),
        )
        .await
        .unwrap();
    abort.abort();
    assert_eq!(stream.next_chunk().await, Err(TransportError::Aborted));
    task.abort();
}

#[test]
fn system_vault_fails_closed_off_windows_and_rejects_invalid_ids() {
    let vault = WindowsCredentialManager;
    assert!(matches!(
        vault.load("../credential"),
        Err(VaultError::InvalidId)
    ));
    #[cfg(not(windows))]
    assert!(matches!(
        vault.load("valid-id"),
        Err(VaultError::Unavailable)
    ));
}
