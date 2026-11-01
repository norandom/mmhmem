//! Shunt engine tests (tasks 5.1/5.2; Req 3.1-3.9, 9.2, 10.1, 10.3).
//! 5.1's evaluation/confinement tests were moved here from src/services/shunt.rs
//! to keep the src file under the Req 13.3 500-LOC cap. The stub is a raw
//! tokio TCP listener (no extra dev-deps).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use mmhmem::config::{
    Config, ENV_ALLOWED_ROOTS, ENV_SHUNT_MIN_LINES, ENV_TOKEN, ENV_WORKER_API_URL, ENV_WORKER_MODEL,
};
use mmhmem::services::shunt::{BypassReason, ReadOutcome, ShuntEngine, ShuntError};
use mmhmem::services::shunt_types::ReadRequest;
use mmhmem::services::worker::WorkerClient;
use mmhmem::storage::audit::AuditWriter;
use mmhmem::storage::db::Db;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

fn test_config(root: &Path, shunt_min_lines: u32, worker: Option<(&str, &str)>) -> Config {
    let mut env: HashMap<String, String> = HashMap::new();
    env.insert(ENV_TOKEN.to_string(), "tok".to_string());
    env.insert(ENV_ALLOWED_ROOTS.to_string(), root.display().to_string());
    env.insert(ENV_SHUNT_MIN_LINES.to_string(), shunt_min_lines.to_string());
    if let Some((url, model)) = worker {
        env.insert(ENV_WORKER_API_URL.to_string(), url.to_string());
        env.insert(ENV_WORKER_MODEL.to_string(), model.to_string());
    }
    Config::from_env(&|k| env.get(k).cloned()).unwrap()
}

fn engine(
    root: &Path,
    worker: Option<WorkerClient>,
) -> (tempfile::TempDir, tempfile::TempDir, ShuntEngine) {
    let data = tempfile::tempdir().unwrap();
    let logs = tempfile::tempdir().unwrap();
    let db = Db::open(data.path()).unwrap();
    let audit = AuditWriter::new(logs.path().join("audit.jsonl"));
    let cfg = test_config(root, 150, None);
    let eng = ShuntEngine::new(db, audit, cfg, None, worker);
    (data, logs, eng)
}

fn engine_with_worker(
    root: &Path,
    url: &str,
) -> (tempfile::TempDir, tempfile::TempDir, ShuntEngine) {
    let cfg = test_config(root, 150, Some((url, "stub-worker")));
    let worker = WorkerClient::from_config(&cfg).ok();
    let data = tempfile::tempdir().unwrap();
    let logs = tempfile::tempdir().unwrap();
    let db = Db::open(data.path()).unwrap();
    let audit = AuditWriter::new(logs.path().join("audit.jsonl"));
    let eng = ShuntEngine::new(db, audit, cfg, None, worker);
    (data, logs, eng)
}

/// Builds a smart_read request with defaults (project "p", harness "h").
fn req<'a>(
    path: &'a str,
    question: &'a str,
    offset: Option<u32>,
    limit: Option<u32>,
) -> ReadRequest<'a> {
    ReadRequest {
        project: "p",
        harness: "h",
        task: None,
        path,
        question,
        offset,
        limit,
    }
}

/// Builds a smart_read request with a task ID.
fn req_task<'a>(
    task: &'a str,
    harness: &'a str,
    path: &'a str,
    question: &'a str,
) -> ReadRequest<'a> {
    ReadRequest {
        project: "p",
        harness,
        task: Some(task),
        path,
        question,
        offset: None,
        limit: None,
    }
}

fn write_file(root: &Path, name: &str, lines: usize) -> PathBuf {
    let p = root.join(name);
    let content = (1..=lines)
        .map(|i| format!("line {i}\n"))
        .collect::<String>();
    std::fs::write(&p, content).unwrap();
    p
}

fn find_subslice(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// Multi-shot HTTP stub: serves `response` up to `max` connections, counts
/// requests, captures bodies. Returns (url, request_count, bodies).
async fn spawn_stub_multi(
    status: u16,
    response: &'static str,
    max: usize,
) -> (String, Arc<AtomicUsize>, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let count = Arc::new(AtomicUsize::new(0));
    let bodies: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let (c_count, c_bodies) = (Arc::clone(&count), Arc::clone(&bodies));
    tokio::spawn(async move {
        for _ in 0..max {
            let (mut sock, _) = match listener.accept().await {
                Ok(x) => x,
                Err(_) => break,
            };
            c_count.fetch_add(1, Ordering::SeqCst);
            let mut buf: Vec<u8> = Vec::new();
            let mut tmp = [0u8; 8192];
            let header_end = loop {
                let r = sock.read(&mut tmp).await.unwrap();
                if r == 0 {
                    break buf.len();
                }
                buf.extend_from_slice(&tmp[..r]);
                if let Some(pos) = find_subslice(&buf, b"\r\n\r\n") {
                    break pos + 4;
                }
            };
            let head = String::from_utf8_lossy(&buf[..header_end]).to_lowercase();
            let need: usize = head
                .lines()
                .find_map(|l| l.strip_prefix("content-length:"))
                .and_then(|v| v.trim().parse().ok())
                .unwrap_or(0);
            let mut body = buf[header_end..].to_vec();
            while body.len() < need {
                let r = sock.read(&mut tmp).await.unwrap();
                if r == 0 {
                    break;
                }
                body.extend_from_slice(&tmp[..r]);
            }
            c_bodies
                .lock()
                .unwrap()
                .push(String::from_utf8_lossy(&body).into_owned());
            let reason = if status == 200 { "OK" } else { "ERR" };
            let resp = format!(
                "HTTP/1.1 {status} {reason}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{response}",
                response.len()
            );
            let _ = sock.write_all(resp.as_bytes()).await;
        }
    });
    (format!("http://{addr}/v1/chat/completions"), count, bodies)
}

const SUMMARY_CHAT: &str = r#"{"choices":[{"message":{"content":"- foo [L1-L9]\n- bar [L10-L20]"}}],"usage":{"prompt_tokens":500,"completion_tokens":30}}"#;

// ---------- 5.1: evaluation + confinement ----------

#[tokio::test]
async fn targeted_read_returns_exact_line_numbered_slice() {
    let root = tempfile::tempdir().unwrap();
    let (_d, _l, eng) = engine(root.path(), None);
    let f = write_file(root.path(), "big.rs", 200);
    let out = eng
        .smart_read(&req(f.to_str().unwrap(), "q", Some(10), Some(5)))
        .await
        .unwrap();
    match out {
        ReadOutcome::Targeted {
            text,
            start,
            end,
            total_lines,
        } => {
            assert_eq!((start, end, total_lines), (10, 14, 200));
            assert!(text.starts_with("L10: line 10"));
            assert!(text.contains("L14: line 14"));
            assert!(!text.contains("L15:"));
        }
        other => panic!("expected Targeted, got {other:?}"),
    }
}

#[tokio::test]
async fn small_file_direct_below_threshold() {
    let root = tempfile::tempdir().unwrap();
    let (_d, _l, eng) = engine(root.path(), None);
    let f = write_file(root.path(), "small.rs", 10);
    let out = eng
        .smart_read(&req(f.to_str().unwrap(), "q", None, None))
        .await
        .unwrap();
    match out {
        ReadOutcome::Direct {
            text,
            reason,
            total_lines,
        } => {
            assert_eq!(reason, BypassReason::BelowThreshold);
            assert_eq!(total_lines, 10);
            assert!(text.starts_with("L1: line 1"));
        }
        other => panic!("expected Direct, got {other:?}"),
    }
}

#[tokio::test]
async fn security_critical_path_bypasses_even_when_large() {
    let root = tempfile::tempdir().unwrap();
    let (_d, logs, eng) = engine(root.path(), None);
    let f = write_file(root.path(), "auth_handler.rs", 300);
    assert!(eng.is_security_critical(&f));
    let out = eng
        .smart_read(&req(f.to_str().unwrap(), "q", None, None))
        .await
        .unwrap();
    match out {
        ReadOutcome::Direct {
            reason,
            total_lines,
            ..
        } => {
            assert_eq!(reason, BypassReason::SecurityCritical);
            assert_eq!(total_lines, 300);
        }
        other => panic!("expected Direct, got {other:?}"),
    }
    let log = std::fs::read_to_string(logs.path().join("audit.jsonl")).unwrap();
    assert!(log.contains("IO_DIRECT_READ"));
    assert!(log.contains("security_critical_bypass"));
}

#[tokio::test]
async fn missing_file_is_a_clear_error() {
    let root = tempfile::tempdir().unwrap();
    let (_d, _l, eng) = engine(root.path(), None);
    let missing = root.path().join("nope.rs");
    let err = eng
        .smart_read(&req(missing.to_str().unwrap(), "q", None, None))
        .await
        .unwrap_err();
    assert!(matches!(err, ShuntError::FileNotFound(_)), "{err:?}");
    assert!(err.to_string().contains("file not found"));
}

#[tokio::test]
async fn path_outside_allowed_roots_is_rejected_and_audited() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let (_d, logs, eng) = engine(root.path(), None);
    let f = write_file(outside.path(), "secret.txt", 5);
    let err = eng
        .smart_read(&req(f.to_str().unwrap(), "q", None, None))
        .await
        .unwrap_err();
    assert!(matches!(err, ShuntError::OutsideAllowedRoots(_)), "{err:?}");
    let log = std::fs::read_to_string(logs.path().join("audit.jsonl")).unwrap();
    assert!(log.contains("path_outside_allowed_roots"), "log={log}");
}

#[tokio::test]
async fn traversal_attempt_is_rejected() {
    let root = tempfile::tempdir().unwrap();
    let (_d, _l, eng) = engine(root.path(), None);
    let inner = root.path().join("inner");
    std::fs::create_dir(&inner).unwrap();
    write_file(root.path(), "ok.rs", 5);
    let trav = inner.join("..").join("..").join("etc").join("passwd");
    let err = eng
        .smart_read(&req(trav.to_str().unwrap(), "q", None, None))
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        ShuntError::OutsideAllowedRoots(_) | ShuntError::FileNotFound(_)
    ));
}

#[test]
fn security_regex_covers_all_pattern_classes() {
    let root = tempfile::tempdir().unwrap();
    let (_d, _l, eng) = engine(root.path(), None);
    for name in [
        "auth.rs",
        "crypto_util.rs",
        "secrets.yaml",
        "token_store.rs",
        "rbac.rs",
        "jwt.rs",
        "tls_config.rs",
        "cert.pem",
        "mutex.rs",
        "lock.rs",
        "concurrency.rs",
        "permissions.rs",
        "sudoers",
        "pam.d",
        "AUTH_UPPER.rs",
    ] {
        assert!(
            eng.is_security_critical(Path::new(name)),
            "must match: {name}"
        );
    }
    for name in ["main.rs", "parser.rs", "readme.md"] {
        assert!(
            !eng.is_security_critical(Path::new(name)),
            "must NOT match: {name}"
        );
    }
}

// ---------- 5.2: cache + worker delegation ----------

#[tokio::test]
async fn cache_miss_delegates_then_hit_serves_cached_with_zero_worker_calls() {
    let root = tempfile::tempdir().unwrap();
    let (url, calls, _bodies) = spawn_stub_multi(200, SUMMARY_CHAT, 4).await;
    let (_d, logs, eng) = engine_with_worker(root.path(), &url);
    let f = write_file(root.path(), "big.rs", 200);
    let path = f.to_str().unwrap().to_string();

    let out1 = eng
        .smart_read(&req_task("t1", "h", &path, "what?"))
        .await
        .unwrap();
    match &out1 {
        ReadOutcome::Shunted {
            summary,
            latency_ms,
            avoided_input_tokens_est,
        } => {
            assert!(summary.contains("- foo [L1-L9]"));
            assert!(*latency_ms < 5_000);
            assert!(*avoided_input_tokens_est > 0);
        }
        other => panic!("expected Shunted, got {other:?}"),
    }
    let out2 = eng
        .smart_read(&req_task("t1", "h2", &path, "what?"))
        .await
        .unwrap();
    match &out2 {
        ReadOutcome::Cached {
            summary,
            avoided_input_tokens_est,
        } => {
            assert_eq!(summary, "- foo [L1-L9]\n- bar [L10-L20]");
            assert!(*avoided_input_tokens_est > 0);
        }
        other => panic!("expected Cached, got {other:?}"),
    }
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "cache hit must not call the worker"
    );

    let log = std::fs::read_to_string(logs.path().join("audit.jsonl")).unwrap();
    assert!(log.contains("IO_SHUNT_DELEGATED"));
    assert!(log.contains("IO_SHUNT_CACHE_HIT"));
    assert!(log.contains("stub-worker"));
    assert!(log.contains("avoided_input_tokens_est"));
}

#[tokio::test]
async fn token_stats_rows_written_for_delegated_and_cache_hit() {
    let root = tempfile::tempdir().unwrap();
    let (url, _calls, _b) = spawn_stub_multi(200, SUMMARY_CHAT, 4).await;
    let data = tempfile::tempdir().unwrap();
    let logs = tempfile::tempdir().unwrap();
    let cfg = test_config(root.path(), 150, Some((&url, "stub-worker")));
    let db = Db::open(data.path()).unwrap();
    let audit = AuditWriter::new(logs.path().join("audit.jsonl"));
    let worker = WorkerClient::from_config(&cfg).ok();
    let eng = ShuntEngine::new(db.clone(), audit, cfg, None, worker);
    let f = write_file(root.path(), "big.rs", 200);
    let path = f.to_str().unwrap().to_string();
    eng.smart_read(&req_task("t", "h", &path, "q"))
        .await
        .unwrap();
    eng.smart_read(&req_task("t", "h", &path, "q"))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await; // let the spawned inserts land
    let rows: Vec<(String, i64, i64)> = db
        .with_conn(|c| {
            let mut s = c.prepare(
                "SELECT kind, avoided_input_tokens_est, worker_prompt_tokens FROM token_stats ORDER BY id",
            )?;
            let rows = s
                .query_map([], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, i64>(1)?,
                        r.get::<_, Option<i64>>(2)?.unwrap_or(-1),
                    ))
                })?;
            rows.collect()
        })
        .await
        .unwrap();
    assert_eq!(rows.len(), 2, "rows={rows:?}");
    assert_eq!(rows[0].0, "shunt_delegated");
    assert_eq!(rows[1].0, "shunt_cache_hit");
    assert!(rows[0].1 > 0 && rows[1].1 > 0);
    assert_eq!(rows[0].2, 500, "worker prompt tokens recorded (Req 10.2)");
    assert_eq!(rows[1].2, -1, "cache hit has no worker usage");
}

#[tokio::test]
async fn content_change_invalidates_cache() {
    let root = tempfile::tempdir().unwrap();
    let (url, calls, _b) = spawn_stub_multi(200, SUMMARY_CHAT, 4).await;
    let (_d, _l, eng) = engine_with_worker(root.path(), &url);
    let f = write_file(root.path(), "big.rs", 200);
    let path = f.to_str().unwrap().to_string();
    let out1 = eng.smart_read(&req(&path, "q", None, None)).await.unwrap();
    assert!(matches!(out1, ReadOutcome::Shunted { .. }));
    // Change one line but keep the file above the threshold -> different
    // SHA-256 -> the stale cached summary must NOT be served (Req 3.7).
    let mut changed: String = (1..=200).map(|i| format!("line {i}\n")).collect();
    changed.push_str("CHANGED TAIL\n");
    std::fs::write(&f, changed).unwrap();
    let out2 = eng.smart_read(&req(&path, "q", None, None)).await.unwrap();
    assert!(
        matches!(out2, ReadOutcome::Shunted { .. }),
        "stale cache served: {out2:?}"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn worker_error_is_explicit_and_not_cached() {
    let root = tempfile::tempdir().unwrap();
    let (url, calls, _b) = spawn_stub_multi(500, "{\"error\":\"boom\"}", 4).await;
    let (_d, logs, eng) = engine_with_worker(root.path(), &url);
    let f = write_file(root.path(), "big.rs", 200);
    let err = eng
        .smart_read(&req(f.to_str().unwrap(), "q", None, None))
        .await
        .unwrap_err();
    assert!(matches!(err, ShuntError::Worker(_)), "{err:?}");
    assert!(err.to_string().contains("HTTP 500"));
    let log = std::fs::read_to_string(logs.path().join("audit.jsonl")).unwrap();
    assert!(log.contains("WORKER_ERROR"), "log={log}");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    // Nothing was cached: a healthy worker is called again on retry.
    let (url2, calls2, _b2) = spawn_stub_multi(200, SUMMARY_CHAT, 4).await;
    let (_d2, _l2, eng2) = engine_with_worker(root.path(), &url2);
    let out = eng2
        .smart_read(&req(f.to_str().unwrap(), "q", None, None))
        .await
        .unwrap();
    assert!(matches!(out, ReadOutcome::Shunted { .. }));
    assert_eq!(calls2.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn shuntable_file_without_worker_is_a_clear_error() {
    let root = tempfile::tempdir().unwrap();
    let (_d, _l, eng) = engine(root.path(), None);
    let f = write_file(root.path(), "big.rs", 200);
    let err = eng
        .smart_read(&req(f.to_str().unwrap(), "q", None, None))
        .await
        .unwrap_err();
    assert!(matches!(err, ShuntError::WorkerNotConfigured), "{err:?}");
}
