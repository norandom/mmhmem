//! boilerplate_write tests (task 6.1; Req 4.1-4.4, 10.1). Raw tokio TCP stub,
//! no extra dev-deps.

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use mmhmem::config::{Config, ENV_ALLOWED_ROOTS, ENV_TOKEN, ENV_WORKER_API_URL, ENV_WORKER_MODEL};
use mmhmem::services::boilerplate::{BoilerplateError, BoilerplateWriter, WriteRequest};
use mmhmem::services::worker::WorkerClient;
use mmhmem::storage::audit::AuditWriter;
use mmhmem::storage::db::Db;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

fn test_config(root: &Path, worker: Option<(&str, &str)>) -> Config {
    let mut env: HashMap<String, String> = HashMap::new();
    env.insert(ENV_TOKEN.to_string(), "tok".to_string());
    env.insert(ENV_ALLOWED_ROOTS.to_string(), root.display().to_string());
    if let Some((url, model)) = worker {
        env.insert(ENV_WORKER_API_URL.to_string(), url.to_string());
        env.insert(ENV_WORKER_MODEL.to_string(), model.to_string());
    }
    Config::from_env(&|k| env.get(k).cloned()).unwrap()
}

async fn spawn_stub(status: u16, response: &'static str) -> (String, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let count = Arc::new(AtomicUsize::new(0));
    let c = Arc::clone(&count);
    tokio::spawn(async move {
        while let Ok((mut sock, _)) = listener.accept().await {
            c.fetch_add(1, Ordering::SeqCst);
            let mut buf = Vec::new();
            let mut tmp = [0u8; 8192];
            loop {
                let n = sock.read(&mut tmp).await.unwrap();
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&tmp[..n]);
                if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&buf).to_lowercase();
                    let need: usize = head
                        .lines()
                        .find_map(|l| l.strip_prefix("content-length:"))
                        .and_then(|v| v.trim().parse().ok())
                        .unwrap_or(0);
                    let header_end = buf.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
                    if buf.len() - header_end >= need {
                        break;
                    }
                }
            }
            let reason = if status == 200 { "OK" } else { "ERR" };
            let resp = format!(
                "HTTP/1.1 {status} {reason}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{response}",
                response.len()
            );
            let _ = sock.write_all(resp.as_bytes()).await;
        }
    });
    (format!("http://{addr}/v1/chat/completions"), count)
}

const CODE_CHAT: &str = r#"{"choices":[{"message":{"content":"```rust\n#[test]\nfn it_works() { assert_eq!(2 + 2, 4); }\n```\n"}}],"usage":{"prompt_tokens":700,"completion_tokens":25}}"#;

fn writer(
    root: &Path,
    url: Option<&str>,
) -> (tempfile::TempDir, tempfile::TempDir, BoilerplateWriter) {
    let data = tempfile::tempdir().unwrap();
    let logs = tempfile::tempdir().unwrap();
    let cfg = test_config(root, url.map(|u| (u, "stub-w")));
    let db = Db::open(data.path()).unwrap();
    let audit = AuditWriter::new(logs.path().join("audit.jsonl"));
    let worker = WorkerClient::from_config(&cfg).ok();
    let w = BoilerplateWriter::new(db, audit, &cfg, None, worker);
    (data, logs, w)
}

fn req<'a>(target: &'a str, reference: &'a str, spec: &'a str) -> WriteRequest<'a> {
    WriteRequest {
        project: "p",
        harness: "h",
        task: Some("t"),
        target,
        reference,
        spec,
    }
}

#[tokio::test]
async fn round_trip_strips_fences_writes_file_and_returns_one_line_ack() {
    let root = tempfile::tempdir().unwrap();
    let (url, _calls) = spawn_stub(200, CODE_CHAT).await;
    let (_d, logs, w) = writer(root.path(), Some(&url));
    let reference = root.path().join("reference.rs");
    std::fs::write(&reference, "pub fn existing() {}\n").unwrap();
    let target = root.path().join("gen").join("test_gen.rs");

    let ack = w
        .write(&req(
            target.to_str().unwrap(),
            reference.to_str().unwrap(),
            "a test for existing()",
        ))
        .await
        .unwrap();
    let on_disk = std::fs::read_to_string(&target).unwrap();
    assert_eq!(
        on_disk, "#[test]\nfn it_works() { assert_eq!(2 + 2, 4); }\n",
        "fences must be stripped"
    );
    assert_eq!(ack.lines, 2);
    assert_eq!(ack.model, "stub-w");
    assert_eq!(ack.sha256.len(), 64);
    assert!(ack.avoided_input_tokens_est > 0 && ack.avoided_output_tokens_est > 0);

    let log = std::fs::read_to_string(logs.path().join("audit.jsonl")).unwrap();
    assert!(log.contains("IO_SHUNT_CODE_WRITE"), "log={log}");
    assert!(log.contains("avoided_input_tokens_est"));
}

#[tokio::test]
async fn missing_reference_errors_and_writes_nothing() {
    let root = tempfile::tempdir().unwrap();
    let (url, calls) = spawn_stub(200, CODE_CHAT).await;
    let (_d, _l, w) = writer(root.path(), Some(&url));
    let target = root.path().join("out.rs");
    let missing = root.path().join("nope.rs");
    let err = w
        .write(&req(
            target.to_str().unwrap(),
            missing.to_str().unwrap(),
            "spec",
        ))
        .await
        .unwrap_err();
    assert!(
        matches!(err, BoilerplateError::ReferenceNotFound(_)),
        "{err:?}"
    );
    assert!(!target.exists(), "nothing may be written (Req 4.3)");
    assert_eq!(calls.load(Ordering::SeqCst), 0, "worker must not be called");
}

#[tokio::test]
async fn reference_outside_allowed_roots_sends_zero_bytes_to_worker() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let (url, calls) = spawn_stub(200, CODE_CHAT).await;
    let (_d, logs, w) = writer(root.path(), Some(&url));
    let reference = outside.path().join("secret.rs");
    std::fs::write(&reference, "top secret\n").unwrap();
    let target = root.path().join("out.rs");
    let err = w
        .write(&req(
            target.to_str().unwrap(),
            reference.to_str().unwrap(),
            "spec",
        ))
        .await
        .unwrap_err();
    assert!(
        matches!(err, BoilerplateError::ReferenceOutsideRoots(_)),
        "{err:?}"
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "zero bytes may reach the worker"
    );
    assert!(!target.exists());
    let log = std::fs::read_to_string(logs.path().join("audit.jsonl")).unwrap();
    assert!(log.contains("reference_outside_allowed_roots"), "log={log}");
    assert!(!log.contains("top secret"));
}

#[tokio::test]
async fn worker_error_is_explicit_and_writes_nothing() {
    let root = tempfile::tempdir().unwrap();
    let (url, _c) = spawn_stub(500, "{\"error\":\"boom\"}").await;
    let (_d, logs, w) = writer(root.path(), Some(&url));
    let reference = root.path().join("reference.rs");
    std::fs::write(&reference, "pub fn x() {}\n").unwrap();
    let target = root.path().join("out.rs");
    let err = w
        .write(&req(
            target.to_str().unwrap(),
            reference.to_str().unwrap(),
            "spec",
        ))
        .await
        .unwrap_err();
    assert!(matches!(err, BoilerplateError::Worker(_)), "{err:?}");
    assert!(!target.exists());
    let log = std::fs::read_to_string(logs.path().join("audit.jsonl")).unwrap();
    assert!(log.contains("WORKER_ERROR"), "log={log}");
}

#[tokio::test]
async fn no_worker_configured_is_a_clear_error() {
    let root = tempfile::tempdir().unwrap();
    let (_d, _l, w) = writer(root.path(), None);
    let reference = root.path().join("reference.rs");
    std::fs::write(&reference, "pub fn x() {}\n").unwrap();
    let target = root.path().join("out.rs");
    let err = w
        .write(&req(
            target.to_str().unwrap(),
            reference.to_str().unwrap(),
            "spec",
        ))
        .await
        .unwrap_err();
    assert!(
        matches!(err, BoilerplateError::WorkerNotConfigured),
        "{err:?}"
    );
}
