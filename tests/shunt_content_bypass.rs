//! Content-secret patterns and bypass evaluation tests (shunt-content-tool
//! task 2.1; requirements.md Req 1.2 evaluation order, 3.1-3.5 bypass
//! rules). Split from `tests/shunt_content.rs` at the 800-line test-file
//! cap of `tests/loc_caps.rs`; the stub-worker helper shape is the same as
//! `tests/shunt.rs` / `tests/shunt_content.rs`. Pinned here:
//!   - the documented secret pattern set (PEM private-key headers, cloud
//!     access-key ids, api-key assignments, bearer tokens, secret-bearing
//!     environment lines) each yield a `security_critical` bypass (Req 3.2),
//!   - a sensitive source-hint path yields `security_critical` through the
//!     existing path rule, hint as a label only (Req 3.1, 1.5),
//!   - both security checks run BEFORE the threshold check (Req 1.2), and
//!     a 149-line benign content is `below_threshold` while 150 lines pass
//!     the stage (Req 3.3),
//!   - bypass audits carry the reason and the content hash, never content
//!     (Req 3.4), and a bypass writes no cache row and no statistics row
//!     (Req 3.5).

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use mmhmem::config::{
    Config, ENV_ALLOWED_ROOTS, ENV_SHUNT_MIN_LINES, ENV_TOKEN, ENV_WORKER_API_URL, ENV_WORKER_MODEL,
};
use mmhmem::services::shunt::ShuntEngine;
use mmhmem::services::shunt_content::{
    ContentKind, ContentOutcome, ContentRequest, MAX_CONTENT_BYTES, is_secret_content,
};
use mmhmem::services::shunt_types::{BypassReason, ShuntError};
use mmhmem::services::worker::WorkerClient;
use mmhmem::storage::audit::{AuditWriter, sha256_hex};
use mmhmem::storage::db::Db;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

// ---------- shared helpers (same stub shape as tests/shunt.rs) ----------

fn test_config(root: &Path, shunt_min_lines: u32, worker: Option<(&str, &str)>) -> Config {
    let mut env: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    env.insert(ENV_TOKEN.to_string(), "tok".to_string());
    env.insert(ENV_ALLOWED_ROOTS.to_string(), root.display().to_string());
    env.insert(ENV_SHUNT_MIN_LINES.to_string(), shunt_min_lines.to_string());
    if let Some((url, model)) = worker {
        env.insert(ENV_WORKER_API_URL.to_string(), url.to_string());
        env.insert(ENV_WORKER_MODEL.to_string(), model.to_string());
    }
    Config::from_env(&|k| env.get(k).cloned()).unwrap()
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

/// Engine plus a live `Db` handle for side-effect assertions.
fn engine_with_db(
    root: &Path,
    url: &str,
) -> (tempfile::TempDir, tempfile::TempDir, Db, ShuntEngine) {
    let cfg = test_config(root, 150, Some((url, "stub-worker")));
    let worker = WorkerClient::from_config(&cfg).ok();
    let data = tempfile::tempdir().unwrap();
    let logs = tempfile::tempdir().unwrap();
    let db = Db::open(data.path()).unwrap();
    let audit = AuditWriter::new(logs.path().join("audit.jsonl"));
    let eng = ShuntEngine::new(db.clone(), audit, cfg, None, worker);
    (data, logs, db, eng)
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

// ---------- 2.1 helpers ----------

/// A FileRead content request with the default question shape.
fn content_req<'a>(content: &'a str, hint: Option<&'a str>) -> ContentRequest<'a> {
    ContentRequest {
        project: "p",
        harness: "mvibe-proxy",
        task: None,
        content,
        kind: ContentKind::FileRead,
        source_hint: hint,
        question: "Summarize exported symbols, signatures, and control flow with exact line ranges.",
    }
}

/// `total` benign filler lines with `inject` replacing the middle line, so
/// a corpus sits ABOVE the 150-line threshold unless a secret says otherwise.
fn benign_lines(total: usize, inject: Option<&str>) -> String {
    let mut lines: Vec<String> = (1..=total)
        .map(|i| format!("benign filler line {i}"))
        .collect();
    if let Some(secret) = inject {
        lines[total / 2] = secret.to_string();
    }
    lines.join("\n")
}

/// Row count of one side-effect table.
async fn row_count(db: &Db, table: &'static str) -> i64 {
    let sql = format!("SELECT COUNT(*) FROM {table}");
    db.with_conn(move |c| c.query_row(&sql, [], |r| r.get::<_, i64>(0)))
        .await
        .unwrap()
}

// ---------- 2.1: the documented secret pattern set ----------

#[test]
fn is_secret_content_matches_each_documented_pattern() {
    let cases = [
        (
            "-----BEGIN RSA PRIVATE KEY-----\nMIIabc...\n",
            "PEM RSA header",
        ),
        ("-----BEGIN PRIVATE KEY-----\n", "PEM generic header"),
        (
            "-----BEGIN OPENSSH PRIVATE KEY-----\n",
            "PEM openssh header",
        ),
        (
            "-----BEGIN ENCRYPTED PRIVATE KEY-----\n",
            "PEM encrypted header",
        ),
        ("-----BEGIN EC PRIVATE KEY-----\n", "PEM EC header"),
        (
            "the key id is AKIAIOSFODNN7EXAMPLE here\n",
            "cloud access-key id",
        ),
        (
            "api_key = \"sk-live-abcdefghijklmnop\"\n",
            "api-key assignment",
        ),
        ("X-Api-Key: 0123456789abcdef\n", "api-key header assignment"),
        (
            "Authorization: Bearer eyJhbGciOiJIUzI1NiJ9.sig.part\n",
            "bearer token",
        ),
        (
            "GITHUB_TOKEN=ghp_16C7e42F292c6912E7710c838347Ae178B4a\n",
            "secret-bearing env line",
        ),
        ("MY_APP_PASSWORD=hunter2hunter2\n", "password env line"),
        ("db_secret=topsecretvalue1\n", "secret env line"),
    ];
    for (content, label) in cases {
        assert!(is_secret_content(content), "{label} must match: {content}");
    }
}

#[test]
fn is_secret_content_rejects_benign_and_borderline_text() {
    let cases = [
        "fn main() { println!(\"hello\"); }\n",
        "benign filler line 1\nbenign filler line 2\n",
        "token: short\n",                // assignment value under 8 chars
        "AKIA123\n",                     // access-key id under length
        "-----BEGIN CERTIFICATE-----\n", // public certificate, not a private key
        "bearer abc123\n",               // bearer value under 16 chars
        "export PATH=/usr/local/bin\n",  // benign environment line
    ];
    for content in cases {
        assert!(
            !is_secret_content(content),
            "benign text must not match: {content}"
        );
    }
}

// ---------- 2.1: bypass evaluation (engine) ----------

#[tokio::test]
async fn secret_fixture_corpus_yields_security_critical() {
    let root = tempfile::tempdir().unwrap();
    let (url, calls, _bodies) = spawn_stub_multi(200, SUMMARY_CHAT, 4).await;
    let (_d, _l, eng) = engine_with_worker(root.path(), &url);
    // Each corpus entry sits inside 160 benign lines, ABOVE the threshold,
    // so only the secret pattern can explain a security_critical bypass.
    let corpus: [(&str, &str); 6] = [
        (
            "-----BEGIN OPENSSH PRIVATE KEY-----",
            "PEM private-key header",
        ),
        ("-----BEGIN RSA PRIVATE KEY-----", "PEM RSA header"),
        ("AKIAIOSFODNN7EXAMPLE", "cloud access-key id"),
        (
            "api_key = \"sk-live-abcdefghijklmnop\"",
            "api-key assignment",
        ),
        (
            "Authorization: Bearer eyJhbGciOiJIUzI1NiJ9.sig.part",
            "bearer token",
        ),
        (
            "GITHUB_TOKEN=ghp_16C7e42F292c6912E7710c838347Ae178B4a",
            "secret-bearing env line",
        ),
    ];
    for (secret, label) in corpus {
        let content = benign_lines(160, Some(secret));
        let expected_sha = sha256_hex(content.as_bytes());
        let out = eng
            .shunt_content(&content_req(&content, None))
            .await
            .unwrap_or_else(|e| panic!("{label}: {e}"));
        match out {
            ContentOutcome::Bypassed {
                reason,
                content_sha256,
                line_count,
            } => {
                assert_eq!(reason, BypassReason::SecurityCritical, "{label}");
                assert_eq!(line_count, 160, "{label}");
                assert_eq!(content_sha256, expected_sha, "{label}");
            }
            other => panic!("{label}: expected a bypass, got {other:?}"),
        }
    }
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "a secret bypass never calls the worker"
    );
}

#[tokio::test]
async fn sensitive_hint_path_yields_security_critical() {
    let root = tempfile::tempdir().unwrap();
    let (url, calls, _bodies) = spawn_stub_multi(200, SUMMARY_CHAT, 4).await;
    let (_d, _l, eng) = engine_with_worker(root.path(), &url);
    // 160 benign lines, no secret anywhere: only the hint can bypass.
    let content = benign_lines(160, None);
    for hint in ["src/auth/token.rs", "config/tls/server-bundle.pem"] {
        let out = eng
            .shunt_content(&content_req(&content, Some(hint)))
            .await
            .unwrap_or_else(|e| panic!("hint {hint}: {e}"));
        assert!(
            matches!(
                out,
                ContentOutcome::Bypassed {
                    reason: BypassReason::SecurityCritical,
                    ..
                }
            ),
            "hint {hint} must bypass as security_critical: {out:?}"
        );
    }
    // A benign hint must not trigger the path rule: short content with it
    // bypasses for the threshold reason only.
    let short = benign_lines(149, None);
    let out = eng
        .shunt_content(&content_req(&short, Some("src/main.rs")))
        .await
        .unwrap();
    assert!(matches!(
        out,
        ContentOutcome::Bypassed {
            reason: BypassReason::BelowThreshold,
            ..
        }
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn secret_check_precedes_the_threshold_check() {
    let root = tempfile::tempdir().unwrap();
    let (url, _calls, _bodies) = spawn_stub_multi(200, SUMMARY_CHAT, 4).await;
    let (_d, _l, eng) = engine_with_worker(root.path(), &url);
    // Both contents are far below the threshold; the secret one must still
    // report security_critical, proving the secret check runs FIRST (Req 1.2).
    let with_secret = benign_lines(3, Some("-----BEGIN EC PRIVATE KEY-----"));
    let out = eng
        .shunt_content(&content_req(&with_secret, None))
        .await
        .unwrap();
    assert!(matches!(
        out,
        ContentOutcome::Bypassed {
            reason: BypassReason::SecurityCritical,
            ..
        }
    ));
    let benign = benign_lines(3, None);
    let out = eng
        .shunt_content(&content_req(&benign, None))
        .await
        .unwrap();
    assert!(matches!(
        out,
        ContentOutcome::Bypassed {
            reason: BypassReason::BelowThreshold,
            ..
        }
    ));
}

#[tokio::test]
async fn benign_149_line_content_is_below_threshold() {
    let root = tempfile::tempdir().unwrap();
    let (url, calls, _bodies) = spawn_stub_multi(200, SUMMARY_CHAT, 4).await;
    let (_d, logs, eng) = engine_with_worker(root.path(), &url);
    let content = benign_lines(149, None);
    let expected_sha = sha256_hex(content.as_bytes());
    let out = eng
        .shunt_content(&content_req(&content, None))
        .await
        .unwrap();
    match &out {
        ContentOutcome::Bypassed {
            reason,
            content_sha256,
            line_count,
        } => {
            assert_eq!(*reason, BypassReason::BelowThreshold);
            assert_eq!(*line_count, 149);
            assert_eq!(content_sha256, &expected_sha);
        }
        other => panic!("149 benign lines must bypass: {other:?}"),
    }
    // Wire vocabulary is `below_threshold` (Req 2.1 contract).
    assert_eq!(out.to_contract_json()["bypass_reason"], "below_threshold");
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let log = std::fs::read_to_string(logs.path().join("audit.jsonl")).unwrap();
    assert!(
        log.contains("below_line_threshold"),
        "audit carries the audit-vocabulary reason: {log}"
    );
    assert!(
        log.contains(&expected_sha),
        "audit carries the content hash"
    );
}

#[tokio::test]
async fn benign_150_line_content_passes_the_threshold_stage() {
    let root = tempfile::tempdir().unwrap();
    let (url, calls, _bodies) = spawn_stub_multi(200, SUMMARY_CHAT, 4).await;
    let (_d, logs, eng) = engine_with_worker(root.path(), &url);
    let content = benign_lines(150, None);
    let sha = sha256_hex(content.as_bytes());
    let res = eng.shunt_content(&content_req(&content, None)).await;
    // Task 2.2 replaces this interim guard with content-cache lookup and
    // delegation; the Invalid error proves the threshold stage let the
    // 150-line content pass (not below_threshold, never a worker call).
    match res {
        Err(ShuntError::Invalid(m))
            if m.as_str() == "content delegation requires the content cache" => {}
        other => panic!("150 benign lines must pass the threshold stage: {other:?}"),
    }
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    // No audit event is written for threshold-passing content (the interim
    // guard errors before any stage records anything), so the log may not
    // even exist; if it does, it must not mention this content's hash.
    let log = std::fs::read_to_string(logs.path().join("audit.jsonl")).unwrap_or_default();
    assert!(
        !log.contains(&sha),
        "no bypass audit for threshold-passing content: {log}"
    );
}

#[tokio::test]
async fn bypass_audit_lines_carry_reason_and_hash_but_no_content() {
    let root = tempfile::tempdir().unwrap();
    let (url, _calls, _bodies) = spawn_stub_multi(200, SUMMARY_CHAT, 4).await;
    let (_d, logs, eng) = engine_with_worker(root.path(), &url);
    let secret = "AKIAIOSFODNN7EXAMPLE";
    let secret_content = benign_lines(160, Some(secret));
    let secret_sha = sha256_hex(secret_content.as_bytes());
    let short_content = benign_lines(149, None);
    let hint_content = benign_lines(160, None);
    let hint_sha = sha256_hex(hint_content.as_bytes());

    eng.shunt_content(&content_req(&secret_content, None))
        .await
        .unwrap(); // security_critical (content pattern)
    eng.shunt_content(&content_req(&short_content, None))
        .await
        .unwrap(); // below_threshold
    eng.shunt_content(&content_req(&hint_content, Some("src/auth/token.rs")))
        .await
        .unwrap(); // security_critical (hint path)

    let log = std::fs::read_to_string(logs.path().join("audit.jsonl")).unwrap();
    let allowed_keys: BTreeSet<&str> = [
        "bypass_reason",
        "hint",
        "kind",
        "sha256",
        "source",
        "total_lines",
    ]
    .into_iter()
    .collect();
    let mut reasons: Vec<String> = Vec::new();
    let mut saw_hint_label = false;
    let mut saw_secret_sha = false;
    let mut saw_hint_sha = false;
    let mut events = 0;
    for line in log.lines() {
        let rec: serde_json::Value = serde_json::from_str(line).unwrap();
        if rec["event_type"].as_str() != Some("IO_DIRECT_READ") {
            continue;
        }
        events += 1;
        let d = &rec["details"];
        assert_eq!(d["source"], "content", "content bypass label: {line}");
        let sha = d["sha256"].as_str().unwrap();
        assert_eq!(sha.len(), 64);
        assert!(sha.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(d["kind"], "file_read");
        assert!(
            [149u64, 160].contains(&d["total_lines"].as_u64().unwrap()),
            "line count is carried, not content: {line}"
        );
        let reason = d["bypass_reason"].as_str().unwrap();
        assert!(
            ["security_critical_bypass", "below_line_threshold"].contains(&reason),
            "audit-vocabulary reason: {line}"
        );
        let keys: BTreeSet<&str> = d.as_object().unwrap().keys().map(String::as_str).collect();
        assert!(
            keys.is_subset(&allowed_keys),
            "audit details carry reason + hash (+labels) only: {line}"
        );
        // The raw line must never carry content (Req 3.4).
        assert!(!line.contains(secret), "secret leaked into audit: {line}");
        assert!(
            !line.contains("benign filler line"),
            "content leaked: {line}"
        );
        assert!(!line.contains("sk-"), "fixture material leaked: {line}");
        reasons.push(reason.to_string());
        if sha == secret_sha {
            saw_secret_sha = true;
        }
        if sha == hint_sha {
            saw_hint_sha = true;
            saw_hint_label = d["hint"].as_str() == Some("src/auth/token.rs");
        }
    }
    assert_eq!(events, 3, "exactly one audit event per bypass: {log}");
    assert_eq!(
        reasons
            .iter()
            .filter(|r| r.as_str() == "security_critical_bypass")
            .count(),
        2
    );
    assert!(reasons.iter().any(|r| r.as_str() == "below_line_threshold"));
    assert!(
        saw_secret_sha && saw_hint_sha,
        "hashes identify each content"
    );
    assert!(saw_hint_label, "the hint event carries its label");
}

#[tokio::test]
async fn bypass_writes_no_cache_row_and_no_statistics() {
    let root = tempfile::tempdir().unwrap();
    let (url, _calls, _bodies) = spawn_stub_multi(200, SUMMARY_CHAT, 4).await;
    let (_d, _l, db, eng) = engine_with_db(root.path(), &url);

    let secret_content = benign_lines(
        160,
        Some("GITHUB_TOKEN=ghp_16C7e42F292c6912E7710c838347Ae178B4a"),
    );
    let hint_content = benign_lines(160, None);
    let short_content = benign_lines(149, None);
    eng.shunt_content(&content_req(&secret_content, None))
        .await
        .unwrap();
    eng.shunt_content(&content_req(&hint_content, Some("src/auth/token.rs")))
        .await
        .unwrap();
    eng.shunt_content(&content_req(&short_content, None))
        .await
        .unwrap();

    assert_eq!(
        row_count(&db, "token_stats").await,
        0,
        "no savings statistics on bypass"
    );
    assert_eq!(
        row_count(&db, "shunt_content_cache").await,
        0,
        "no content cache row on bypass"
    );
    assert_eq!(
        row_count(&db, "shunt_file_cache").await,
        0,
        "no file cache row on bypass"
    );
}

#[tokio::test]
async fn service_rechecks_the_content_size_guard() {
    let root = tempfile::tempdir().unwrap();
    let (url, calls, _bodies) = spawn_stub_multi(200, SUMMARY_CHAT, 4).await;
    let (_d, _l, db, eng) = engine_with_db(root.path(), &url);

    let empty = "";
    match eng.shunt_content(&content_req(empty, None)).await {
        Err(ShuntError::Invalid(m)) if m.as_str() == "content is empty" => {}
        other => panic!("empty content must be rejected: {other:?}"),
    }
    let oversized = format!("{}\n", "x".repeat(MAX_CONTENT_BYTES));
    match eng.shunt_content(&content_req(&oversized, None)).await {
        Err(ShuntError::Invalid(m)) if m.as_str() == "content exceeds 1048576 bytes" => {}
        other => panic!("oversized content must be rejected: {other:?}"),
    }
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "the guard runs before any worker call"
    );
    assert_eq!(row_count(&db, "token_stats").await, 0);
    assert_eq!(row_count(&db, "shunt_content_cache").await, 0);
}
