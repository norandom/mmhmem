//! Integration tests for the worker-model client (task 3.1; Req 6.1-6.4,
//! 3.5, 4.1, 9.4, 10.2). The stub is a raw tokio TCP listener speaking minimal
//! HTTP/1.1 — no extra dev-dependency, keeping the minimal dependency set
//! (research.md "Minimal dependency set").

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use mmhmem::config::{
    Config, ENV_ALLOWED_ROOTS, ENV_TOKEN, ENV_WORKER_API_KEY, ENV_WORKER_API_URL, ENV_WORKER_MODEL,
};
use mmhmem::services::worker::{WorkerClient, WorkerError};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

struct Captured {
    head: Vec<u8>,
    body: Vec<u8>,
}

fn find_subslice(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

fn content_length(head: &[u8]) -> usize {
    let text = String::from_utf8_lossy(head).to_lowercase();
    text.lines()
        .find_map(|l| l.strip_prefix("content-length:"))
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(0)
}

/// Spawns a one-shot HTTP stub; returns (url, captured request).
async fn spawn_stub(
    status: u16,
    response: &'static str,
    delay: Duration,
) -> (String, Arc<Mutex<Option<Captured>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let captured: Arc<Mutex<Option<Captured>>> = Arc::new(Mutex::new(None));
    let cap = Arc::clone(&captured);
    tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        if !delay.is_zero() {
            tokio::time::sleep(delay).await;
        }
        let mut buf: Vec<u8> = Vec::new();
        let mut tmp = [0u8; 4096];
        let header_end = loop {
            let n = sock.read(&mut tmp).await.unwrap();
            assert!(n > 0, "stub: eof before headers complete");
            buf.extend_from_slice(&tmp[..n]);
            if let Some(pos) = find_subslice(&buf, b"\r\n\r\n") {
                break pos + 4;
            }
        };
        let head = buf[..header_end].to_vec();
        let need = content_length(&head);
        let mut body = buf[header_end..].to_vec();
        while body.len() < need {
            let n = sock.read(&mut tmp).await.unwrap();
            if n == 0 {
                break;
            }
            body.extend_from_slice(&tmp[..n]);
        }
        *cap.lock().unwrap() = Some(Captured { head, body });
        let reason = if status == 200 { "OK" } else { "ERR" };
        let resp = format!(
            "HTTP/1.1 {status} {reason}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{response}",
            response.len()
        );
        sock.write_all(resp.as_bytes()).await.unwrap();
    });
    (format!("http://{addr}/v1/chat/completions"), captured)
}

fn config_with(worker_url: &str, model: Option<&str>, key: Option<&str>) -> Config {
    let mut env: HashMap<String, String> = HashMap::new();
    env.insert(ENV_TOKEN.to_string(), "tok".to_string());
    env.insert(ENV_ALLOWED_ROOTS.to_string(), "/ws".to_string());
    env.insert(ENV_WORKER_API_URL.to_string(), worker_url.to_string());
    if let Some(m) = model {
        env.insert(ENV_WORKER_MODEL.to_string(), m.to_string());
    }
    if let Some(k) = key {
        env.insert(ENV_WORKER_API_KEY.to_string(), k.to_string());
    }
    Config::from_env(&|k| env.get(k).cloned()).unwrap()
}

const OK_CHAT: &str = r#"{"choices":[{"message":{"content":"- foo [L1-L9]"}}],"usage":{"prompt_tokens":111,"completion_tokens":22}}"#;

#[tokio::test]
async fn summarize_success_sends_temp_zero_model_bearer_and_returns_content_plus_usage() {
    let (url, captured) = spawn_stub(200, OK_CHAT, Duration::ZERO).await;
    let cfg = config_with(&url, Some("test-worker"), Some("wk-secret"));
    let client = WorkerClient::from_config(&cfg).unwrap();
    assert_eq!(client.model(), "test-worker");

    let (content, usage) = client
        .summarize_file("<file path=\"a.rs\">x</file>", "what is exported?")
        .await
        .unwrap();
    assert_eq!(content, "- foo [L1-L9]");
    assert_eq!(usage.prompt_tokens, 111);
    assert_eq!(usage.completion_tokens, 22);

    let guard = captured.lock().unwrap();
    let cap = guard.as_ref().unwrap();
    let head = String::from_utf8_lossy(&cap.head).to_lowercase();
    let body = String::from_utf8_lossy(&cap.body);
    assert!(
        head.contains("authorization: bearer wk-secret"),
        "head={head}"
    );
    assert!(body.contains("\"temperature\":0.0"), "body={body}");
    assert!(body.contains("\"model\":\"test-worker\""), "body={body}");
    assert!(body.contains("what is exported?"), "body={body}");
}

#[tokio::test]
async fn generate_code_success_returns_raw_content() {
    let (url, _cap) = spawn_stub(200, OK_CHAT, Duration::ZERO).await;
    let cfg = config_with(&url, Some("w"), None);
    let client = WorkerClient::from_config(&cfg).unwrap();
    let (content, usage) = client
        .generate_code("<file>ref</file>", "a test stub")
        .await
        .unwrap();
    assert_eq!(content, "- foo [L1-L9]");
    assert_eq!(usage.prompt_tokens, 111);
}

#[tokio::test]
async fn http_error_status_is_an_explicit_error() {
    let (url, _cap) = spawn_stub(500, "{\"error\":\"boom\"}", Duration::ZERO).await;
    let cfg = config_with(&url, Some("w"), None);
    let client = WorkerClient::from_config(&cfg).unwrap();
    let err = client.summarize_file("x", "q").await.unwrap_err();
    assert!(
        matches!(err, WorkerError::HttpStatus(500, _)),
        "err={err:?}"
    );
}

#[tokio::test]
async fn timeout_is_an_explicit_error() {
    let (url, _cap) = spawn_stub(200, OK_CHAT, Duration::from_secs(2)).await;
    let cfg = config_with(&url, Some("w"), None);
    let client = WorkerClient::from_config(&cfg)
        .unwrap()
        .with_timeouts(Duration::from_millis(50), Duration::from_millis(50));
    let err = client.summarize_file("x", "q").await.unwrap_err();
    assert!(matches!(err, WorkerError::Timeout), "err={err:?}");
}

#[tokio::test]
async fn connection_refused_is_unreachable_not_empty_success() {
    // Bind then drop to get a definitely-closed port.
    let addr = {
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        l.local_addr().unwrap()
    };
    let cfg = config_with(
        &format!("http://{addr}/v1/chat/completions"),
        Some("w"),
        None,
    );
    let client = WorkerClient::from_config(&cfg).unwrap();
    let err = client.generate_code("x", "q").await.unwrap_err();
    assert!(matches!(err, WorkerError::Unreachable(_)), "err={err:?}");
}

#[tokio::test]
async fn malformed_response_is_an_explicit_error() {
    let (url, _cap) = spawn_stub(200, "this is not json", Duration::ZERO).await;
    let cfg = config_with(&url, Some("w"), None);
    let client = WorkerClient::from_config(&cfg).unwrap();
    let err = client.summarize_file("x", "q").await.unwrap_err();
    assert!(matches!(err, WorkerError::Malformed(_)), "err={err:?}");
}

#[test]
fn not_configured_without_worker_model() {
    let cfg = config_with("http://127.0.0.1:1/v1/chat/completions", None, None);
    // No unwrap_err(): WorkerClient deliberately has no Debug impl (it holds
    // the API key), so assert with matches! instead.
    assert!(
        matches!(
            WorkerClient::from_config(&cfg),
            Err(WorkerError::NotConfigured)
        ),
        "expected NotConfigured"
    );
}
