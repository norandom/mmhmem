//! Cache-hit and delegation tests for `shunt_content` (shunt-content-tool
//! task 2.2; requirements.md Req 1.6, 2.2, 4.2, 4.3, 5.1-5.5). Split from
//! `tests/shunt_content.rs` / `tests/shunt_content_bypass.rs` at the
//! 800-line test-file cap of `tests/loc_caps.rs`; the stub-worker helper
//! shape is the same as `tests/shunt.rs`. Pinned here:
//!   - the first threshold-passing call delegates through the EXISTING
//!     summarize operation with an `L<n>:`-prefixed payload (one worker
//!     request) and returns `summarized` with the worker usage and model
//!     (Req 5.1);
//!   - the second identical call is `cached` with a byte-identical summary
//!     and ZERO worker requests, also across callers (Req 2.2, 4.2, 1.6);
//!   - the delegated and cache-hit statistics rows are committed BEFORE
//!     the call returns — visible from a separate connection with no sleep
//!     (Req 5.1, 5.2);
//!   - every outcome audits hash-only details carrying kind, line count
//!     and latency (Req 5.3) and appends one breadcrumb naming the tool,
//!     the outcome and the hash prefix (Req 5.4);
//!   - worker timeout / not-configured / HTTP-error stubs return the
//!     documented actionable errors, audit WORKER_ERROR with the hash and
//!     write no cache row and no statistics row (Req 5.5);
//!   - delegation stores through the content cache with the CONFIGURED
//!     cap and audits the eviction as one count event (Req 4.3).

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use mmhmem::config::{
    Config, ENV_ALLOWED_ROOTS, ENV_SHUNT_CONTENT_CACHE_MAX_ROWS, ENV_SHUNT_MIN_LINES, ENV_TOKEN,
    ENV_WORKER_API_URL, ENV_WORKER_MODEL,
};
use mmhmem::services::session_bus::SessionBus;
use mmhmem::services::shunt::ShuntEngine;
use mmhmem::services::shunt_content::{ContentKind, ContentOutcome, ContentRequest};
use mmhmem::services::shunt_types::ShuntError;
use mmhmem::services::worker::{WorkerClient, WorkerUsage};
use mmhmem::storage::audit::{AuditWriter, sha256_hex};
use mmhmem::storage::db::Db;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

// ---------- shared helpers (same stub shape as tests/shunt.rs) ----------

/// The smart_read default question (design.md shunt_content API contract);
/// identical content from different callers must share one cache identity
/// through it (Req 1.6).
const DEFAULT_Q: &str =
    "Summarize exported symbols, signatures, and control flow with exact line ranges.";

fn test_config(root: &Path, worker: Option<(&str, &str)>, cap: Option<u32>) -> Config {
    let mut env: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    env.insert(ENV_TOKEN.to_string(), "tok".to_string());
    env.insert(ENV_ALLOWED_ROOTS.to_string(), root.display().to_string());
    env.insert(ENV_SHUNT_MIN_LINES.to_string(), "150".to_string());
    if let Some((url, model)) = worker {
        env.insert(ENV_WORKER_API_URL.to_string(), url.to_string());
        env.insert(ENV_WORKER_MODEL.to_string(), model.to_string());
    }
    if let Some(n) = cap {
        env.insert(ENV_SHUNT_CONTENT_CACHE_MAX_ROWS.to_string(), n.to_string());
    }
    Config::from_env(&|k| env.get(k).cloned()).unwrap()
}

/// Engine harness: keeps the tempdirs alive, exposes the engine's own `Db`
/// handle and the audit log dir for side-effect assertions.
struct Harness {
    data: tempfile::TempDir,
    logs: tempfile::TempDir,
    db: Db,
    eng: ShuntEngine,
}

fn harness_with(cfg: Config, worker: Option<WorkerClient>, bus: bool) -> Harness {
    let worker = worker.map_or_else(|| WorkerClient::from_config(&cfg).ok(), Some);
    let data = tempfile::tempdir().unwrap();
    let logs = tempfile::tempdir().unwrap();
    let db = Db::open(data.path()).unwrap();
    let audit = AuditWriter::new(logs.path().join("audit.jsonl"));
    let bus = if bus {
        Some(SessionBus::new(db.clone(), audit.clone()))
    } else {
        None
    };
    let eng = ShuntEngine::new(db.clone(), audit, cfg, bus, worker);
    Harness {
        data,
        logs,
        db,
        eng,
    }
}

fn harness(root: &Path, url: &str) -> Harness {
    harness_with(
        test_config(root, Some((url, "stub-worker")), None),
        None,
        false,
    )
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
    spawn_stub_delayed(status, response, max, Duration::ZERO).await
}

/// `spawn_stub_multi` with a response delay (timeout tests).
async fn spawn_stub_delayed(
    status: u16,
    response: &'static str,
    max: usize,
    delay: Duration,
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
            if !delay.is_zero() {
                tokio::time::sleep(delay).await;
            }
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
const STUB_SUMMARY: &str = "- foo [L1-L9]\n- bar [L10-L20]";

/// A FileRead content request for `harness_name`.
fn content_req_for<'a>(
    harness_name: &'a str,
    content: &'a str,
    question: &'a str,
) -> ContentRequest<'a> {
    ContentRequest {
        project: "p",
        harness: harness_name,
        task: None,
        content,
        kind: ContentKind::FileRead,
        source_hint: None,
        question,
    }
}

fn content_req<'a>(content: &'a str, question: &'a str) -> ContentRequest<'a> {
    content_req_for("mvibe-proxy", content, question)
}

/// `total` benign filler lines above the 150-line threshold, tagged so
/// distinct tags give distinct content hashes.
fn big_content(total: usize, tag: &str) -> String {
    (1..=total)
        .map(|i| format!("{tag} filler line {i}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Row count of one side-effect table.
async fn row_count(db: &Db, table: &'static str) -> i64 {
    let sql = format!("SELECT COUNT(*) FROM {table}");
    db.with_conn(move |c| c.query_row(&sql, [], |r| r.get::<_, i64>(0)))
        .await
        .unwrap()
}

/// (kind, worker_prompt_tokens, worker_completion_tokens, details) rows.
async fn stat_rows(db: &Db) -> Vec<(String, Option<i64>, Option<i64>, String)> {
    db.with_conn(|c| {
        let mut s = c
            .prepare(
                "SELECT kind, worker_prompt_tokens, worker_completion_tokens, details \
                      FROM token_stats ORDER BY id",
            )
            .unwrap();
        let rows = s
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, Option<i64>>(1)?,
                    r.get::<_, Option<i64>>(2)?,
                    r.get::<_, String>(3)?,
                ))
            })
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        Ok(rows)
    })
    .await
    .unwrap()
}

/// (tool_name, action_summary) breadcrumb rows.
async fn crumbs(db: &Db) -> Vec<(String, String)> {
    db.with_conn(|c| {
        let mut s = c
            .prepare("SELECT tool_name, action_summary FROM session_breadcrumbs ORDER BY id")
            .unwrap();
        let rows = s
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        Ok(rows)
    })
    .await
    .unwrap()
}

fn audit_log(h: &Harness) -> String {
    std::fs::read_to_string(h.logs.path().join("audit.jsonl")).unwrap()
}

/// Details objects of every audit record with this event type.
fn audit_details(log: &str, event: &str) -> Vec<serde_json::Value> {
    log.lines()
        .filter_map(|line| {
            let rec: serde_json::Value = serde_json::from_str(line).ok()?;
            (rec["event_type"].as_str() == Some(event)).then(|| rec["details"].clone())
        })
        .collect()
}

// ---------- 2.2: delegation (first call) ----------

#[tokio::test]
async fn first_call_delegates_with_line_numbered_payload_and_one_worker_request() {
    let root = tempfile::tempdir().unwrap();
    let (url, calls, bodies) = spawn_stub_multi(200, SUMMARY_CHAT, 2).await;
    let h = harness(root.path(), &url);
    let content = big_content(160, "benign");
    let sha = sha256_hex(content.as_bytes());

    let out = h.eng.shunt_content(&content_req(&content, DEFAULT_Q)).await;
    let out = out.expect("first threshold-passing call delegates");
    let ContentOutcome::Summarized {
        summary,
        content_sha256,
        line_count,
        avoided_input_tokens_est,
        usage,
        worker_model,
        ..
    } = out
    else {
        panic!("expected Summarized, got {out:?}");
    };
    assert_eq!(summary, STUB_SUMMARY);
    assert_eq!(content_sha256, sha);
    assert_eq!(line_count, 160);
    assert!(avoided_input_tokens_est > 0, "labelled estimate recorded");
    assert_eq!(
        usage,
        WorkerUsage {
            prompt_tokens: 500,
            completion_tokens: 30
        }
    );
    assert_eq!(worker_model, "stub-worker");
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "exactly one worker request"
    );

    // Payload: `L<n>:`-prefixed lines inside a content wrapper, so worker
    // ranges refer to the original line positions (design.md Payload). The
    // needles keep the JSON escaping of the captured HTTP body (`\"`, `\n`).
    let body = bodies.lock().unwrap()[0].clone();
    assert!(body.contains("<content lines=\\\"160\\\">"), "body={body}");
    assert!(body.contains("L1: benign filler line 1\\n"), "body={body}");
    assert!(
        body.contains("L160: benign filler line 160\\n"),
        "body={body}"
    );
    assert!(!body.contains("L161:"), "no phantom lines: {body}");
    assert!(body.contains(DEFAULT_Q), "question reaches the worker");

    // Exactly one delegated audit event, hash-only details with kind,
    // line count and latency (Req 5.3).
    let log = audit_log(&h);
    let events = audit_details(&log, "IO_SHUNT_DELEGATED");
    assert_eq!(events.len(), 1, "log={log}");
    let d = &events[0];
    assert_eq!(d["source"], "content");
    assert_eq!(d["sha256"], sha);
    assert_eq!(d["kind"], "file_read");
    assert_eq!(d["total_lines"], 160);
    assert!(d["avoided_input_tokens_est"].as_u64().unwrap() > 0);
    assert!(d.get("latency_ms").is_some(), "latency carried: {d}");
    assert_eq!(d["worker_model"], "stub-worker");
    assert_eq!(d["worker_prompt_tokens"], 500);
    assert_eq!(d["worker_completion_tokens"], 30);
    assert!(
        !log.contains("benign filler line"),
        "content never reaches the audit log"
    );

    // One delegated stats row, committed before return, with worker usage
    // (Req 5.1) and the content stat subject.
    let rows = stat_rows(&h.db).await;
    assert_eq!(rows.len(), 1, "rows={rows:?}");
    assert_eq!(rows[0].0, "shunt_delegated");
    assert_eq!(rows[0].1, Some(500));
    assert_eq!(rows[0].2, Some(30));
    let details: serde_json::Value = serde_json::from_str(&rows[0].3).unwrap();
    assert_eq!(details["file"], format!("content:{}", &sha[..12]));
    assert_eq!(details["sha256"], sha);
}

// ---------- 2.2: cache hit (second identical call) ----------

#[tokio::test]
async fn second_identical_call_is_cached_byte_identical_with_zero_worker_requests() {
    let root = tempfile::tempdir().unwrap();
    let (url, calls, _bodies) = spawn_stub_multi(200, SUMMARY_CHAT, 2).await;
    let h = harness(root.path(), &url);
    let content = big_content(160, "benign");
    let sha = sha256_hex(content.as_bytes());

    let first = h.eng.shunt_content(&content_req(&content, DEFAULT_Q)).await;
    let ContentOutcome::Summarized {
        summary: first_summary,
        ..
    } = first.unwrap()
    else {
        panic!("first call must delegate");
    };
    let second = h
        .eng
        .shunt_content(&content_req(&content, DEFAULT_Q))
        .await
        .unwrap();
    let ContentOutcome::Cached {
        summary,
        content_sha256,
        line_count,
        avoided_input_tokens_est,
        latency_ms,
    } = second
    else {
        panic!("second identical call must be cached, got {second:?}")
    };
    assert_eq!(summary, first_summary, "byte-identical summary (Req 2.2)");
    assert_eq!(summary, STUB_SUMMARY);
    assert_eq!(content_sha256, sha);
    assert_eq!(line_count, 160);
    assert!(avoided_input_tokens_est > 0);
    assert!(latency_ms < 5_000, "lookup latency, not a worker call");
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "zero worker requests on hit"
    );

    // One cache-hit audit event with kind, line count and latency (5.3).
    let log = audit_log(&h);
    let events = audit_details(&log, "IO_SHUNT_CACHE_HIT");
    assert_eq!(events.len(), 1, "log={log}");
    let d = &events[0];
    assert_eq!(d["source"], "content");
    assert_eq!(d["sha256"], sha);
    assert_eq!(d["kind"], "file_read");
    assert_eq!(d["total_lines"], 160);
    assert!(d.get("latency_ms").is_some(), "latency carried: {d}");
    assert!(d.get("worker_model").is_none(), "no worker on a hit: {d}");

    // Delegated then cache-hit stats rows; the hit has no worker usage (5.2).
    let rows = stat_rows(&h.db).await;
    assert_eq!(rows.len(), 2, "rows={rows:?}");
    assert_eq!(rows[0].0, "shunt_delegated");
    assert_eq!(rows[1].0, "shunt_cache_hit");
    assert_eq!(rows[1].1, None, "cache hit records no worker usage");
}

// ---------- 2.2: one cache identity across callers (Req 1.6) ----------

#[tokio::test]
async fn identical_content_from_different_callers_shares_one_cache_identity() {
    let root = tempfile::tempdir().unwrap();
    let (url, calls, _bodies) = spawn_stub_multi(200, SUMMARY_CHAT, 4).await;
    let h = harness(root.path(), &url);
    let content = big_content(160, "benign");

    h.eng
        .shunt_content(&content_req_for("mvibe-proxy", &content, DEFAULT_Q))
        .await
        .unwrap();
    // A different caller with the same content and the same (default)
    // question hits the same cache row — the key has no harness component.
    let out = h
        .eng
        .shunt_content(&content_req_for("other-harness", &content, DEFAULT_Q))
        .await
        .unwrap();
    assert!(
        matches!(out, ContentOutcome::Cached { .. }),
        "cross-caller hit: {out:?}"
    );
    // A different question is a different key: it delegates again.
    let out = h
        .eng
        .shunt_content(&content_req_for("mvibe-proxy", &content, "List the tests."))
        .await
        .unwrap();
    assert!(
        matches!(out, ContentOutcome::Summarized { .. }),
        "different question delegates: {out:?}"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    // The first key still hits afterwards.
    let out = h
        .eng
        .shunt_content(&content_req_for("third", &content, DEFAULT_Q))
        .await
        .unwrap();
    assert!(matches!(out, ContentOutcome::Cached { .. }));
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert_eq!(row_count(&h.db, "shunt_content_cache").await, 2);
}

// ---------- 2.2: stats visible from a separate connection (Req 5.1/5.2) ----------

#[tokio::test]
async fn stats_rows_are_committed_before_return_visible_from_a_separate_connection() {
    let root = tempfile::tempdir().unwrap();
    let (url, _calls, _bodies) = spawn_stub_multi(200, SUMMARY_CHAT, 2).await;
    let h = harness(root.path(), &url);
    let sep = Db::open(h.data.path()).unwrap();
    let content = big_content(160, "benign");

    h.eng
        .shunt_content(&content_req(&content, DEFAULT_Q))
        .await
        .unwrap();
    // NO sleep: the row must already be committed when the call returned.
    let (kind, wp): (String, Option<i64>) = sep
        .with_conn(|c| {
            c.query_row(
                "SELECT kind, worker_prompt_tokens FROM token_stats ORDER BY id DESC LIMIT 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
        })
        .await
        .unwrap();
    assert_eq!(
        kind, "shunt_delegated",
        "delegated row committed pre-return"
    );
    assert_eq!(wp, Some(500));

    h.eng
        .shunt_content(&content_req(&content, DEFAULT_Q))
        .await
        .unwrap();
    let (kind, wp): (String, Option<i64>) = sep
        .with_conn(|c| {
            c.query_row(
                "SELECT kind, worker_prompt_tokens FROM token_stats ORDER BY id DESC LIMIT 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
        })
        .await
        .unwrap();
    assert_eq!(
        kind, "shunt_cache_hit",
        "cache-hit row committed pre-return"
    );
    assert_eq!(wp, None);
}

// ---------- 2.2: worker failures (Req 5.5) ----------

#[tokio::test]
async fn worker_timeout_returns_documented_error_with_no_rows() {
    let root = tempfile::tempdir().unwrap();
    let (url, _calls, _bodies) =
        spawn_stub_delayed(200, SUMMARY_CHAT, 1, Duration::from_secs(2)).await;
    let cfg = test_config(root.path(), Some((&url, "stub-worker")), None);
    let worker = WorkerClient::from_config(&cfg)
        .unwrap()
        .with_timeouts(Duration::from_millis(80), Duration::from_millis(80));
    let h = harness_with(cfg, Some(worker), false);
    let content = big_content(160, "benign");
    let sha = sha256_hex(content.as_bytes());

    let err = h
        .eng
        .shunt_content(&content_req(&content, DEFAULT_Q))
        .await
        .unwrap_err();
    assert!(matches!(err, ShuntError::Worker(_)), "{err:?}");
    assert!(
        err.to_string().contains("worker call timed out"),
        "documented timeout text: {err}"
    );
    // (No request-count assertion here: the stub task may not have been
    // polled past accept() before the 80 ms client timeout fires.)
    assert_eq!(
        row_count(&h.db, "shunt_content_cache").await,
        0,
        "no cache row"
    );
    assert_eq!(row_count(&h.db, "token_stats").await, 0, "no savings stat");
    let log = audit_log(&h);
    let events = audit_details(&log, "WORKER_ERROR");
    assert_eq!(events.len(), 1, "log={log}");
    assert_eq!(events[0]["sha256"], sha, "hash-only worker audit");
    assert_eq!(events[0]["source"], "content");
    assert!(
        !log.contains("benign filler line"),
        "content never reaches the audit log"
    );
}

#[tokio::test]
async fn worker_http_error_returns_documented_error_with_no_rows() {
    let root = tempfile::tempdir().unwrap();
    let (url, calls, _bodies) = spawn_stub_multi(500, "{\"error\":\"boom\"}", 2).await;
    let h = harness(root.path(), &url);
    let content = big_content(160, "benign");
    let sha = sha256_hex(content.as_bytes());

    let err = h
        .eng
        .shunt_content(&content_req(&content, DEFAULT_Q))
        .await
        .unwrap_err();
    assert!(matches!(err, ShuntError::Worker(_)), "{err:?}");
    assert!(
        err.to_string()
            .contains("worker endpoint returned HTTP 500"),
        "documented HTTP text: {err}"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(row_count(&h.db, "shunt_content_cache").await, 0);
    assert_eq!(row_count(&h.db, "token_stats").await, 0);
    let log = audit_log(&h);
    let events = audit_details(&log, "WORKER_ERROR");
    assert_eq!(events.len(), 1, "log={log}");
    assert_eq!(events[0]["sha256"], sha);
}

#[tokio::test]
async fn unconfigured_worker_returns_documented_error_with_no_rows() {
    let root = tempfile::tempdir().unwrap();
    let cfg = test_config(root.path(), None, None);
    let h = harness_with(cfg, None, false);
    let content = big_content(160, "benign");
    let sha = sha256_hex(content.as_bytes());

    let err = h
        .eng
        .shunt_content(&content_req(&content, DEFAULT_Q))
        .await
        .unwrap_err();
    assert!(matches!(err, ShuntError::WorkerNotConfigured), "{err:?}");
    assert!(
        err.to_string().contains("no worker model is configured"),
        "documented not-configured text: {err}"
    );
    assert_eq!(row_count(&h.db, "shunt_content_cache").await, 0);
    assert_eq!(row_count(&h.db, "token_stats").await, 0);
    let log = audit_log(&h);
    let events = audit_details(&log, "WORKER_ERROR");
    assert_eq!(events.len(), 1, "log={log}");
    assert_eq!(events[0]["sha256"], sha);
}

// ---------- 2.2: breadcrumbs (Req 5.4) ----------

#[tokio::test]
async fn breadcrumbs_name_the_tool_outcome_and_hash_prefix() {
    let root = tempfile::tempdir().unwrap();
    let (url, _calls, _bodies) = spawn_stub_multi(200, SUMMARY_CHAT, 2).await;
    let cfg = test_config(root.path(), Some((&url, "stub-worker")), None);
    let h = harness_with(cfg, None, true);
    let content = big_content(160, "benign");
    let sha12 = sha256_hex(content.as_bytes())[..12].to_string();
    // A below-threshold content: bypassed, and its own hash prefix.
    let short = big_content(149, "brief");
    let short_sha12 = sha256_hex(short.as_bytes())[..12].to_string();

    h.eng
        .shunt_content(&content_req(&content, DEFAULT_Q))
        .await
        .unwrap();
    h.eng
        .shunt_content(&content_req(&content, DEFAULT_Q))
        .await
        .unwrap();
    let out = h
        .eng
        .shunt_content(&content_req(&short, DEFAULT_Q))
        .await
        .unwrap();
    assert!(
        matches!(out, ContentOutcome::Bypassed { .. }),
        "short content bypasses: {out:?}"
    );
    let rows = crumbs(&h.db).await;
    assert_eq!(
        rows.len(),
        3,
        "one crumb per non-error outcome incl. bypassed: {rows:?}"
    );
    let tools: BTreeSet<&str> = rows.iter().map(|(t, _)| t.as_str()).collect();
    assert_eq!(
        tools,
        [
            "shunt_content:bypassed",
            "shunt_content:cache_hit",
            "shunt_content:delegated",
        ]
        .into_iter()
        .collect::<BTreeSet<_>>(),
        "tool+outcome names: {rows:?}"
    );
    for (tool, summary) in &rows {
        let prefix = if tool == "shunt_content:bypassed" {
            &short_sha12
        } else {
            &sha12
        };
        assert!(summary.contains(prefix.as_str()), "{tool}: {summary:?}");
    }
    let bypass_summary = rows
        .iter()
        .find(|(t, _)| t == "shunt_content:bypassed")
        .map(|(_, s)| s.clone())
        .unwrap();
    assert!(
        bypass_summary.contains("below_line_threshold"),
        "bypass crumb names the reason: {bypass_summary}"
    );
}

// ---------- 2.2: configured cap and eviction audit (Req 4.3) ----------

#[tokio::test]
async fn delegation_stores_within_the_configured_cap_and_audits_the_eviction_count() {
    let root = tempfile::tempdir().unwrap();
    let (url, calls, _bodies) = spawn_stub_multi(200, SUMMARY_CHAT, 8).await;
    let cfg = test_config(root.path(), Some((&url, "stub-worker")), Some(2));
    let h = harness_with(cfg, None, false);
    let a = big_content(160, "alpha");
    let b = big_content(160, "beta");
    let c = big_content(160, "gamma");

    for content in [&a, &b, &c] {
        let out = h.eng.shunt_content(&content_req(content, DEFAULT_Q)).await;
        assert!(
            matches!(out.unwrap(), ContentOutcome::Summarized { .. }),
            "fresh content delegates"
        );
    }
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    assert_eq!(
        row_count(&h.db, "shunt_content_cache").await,
        2,
        "cap holds after three inserts"
    );
    let log = audit_log(&h);
    let events = audit_details(&log, "CONTENT_CACHE_EVICT");
    assert_eq!(events.len(), 1, "one eviction event, not per row: {log}");
    assert_eq!(events[0]["evicted"], 1, "evicted count: {log}");
    assert_eq!(events[0]["source"], "content");

    // The newest row (C) is always retained by the LRU order, and the
    // evicted row (A) is really gone — re-calling it delegates again, while
    // C still hits. (Ties in `last_hit_epoch` break by recency of insert,
    // so only the oldest rows are ever deleted.)
    let out = h.eng.shunt_content(&content_req(&c, DEFAULT_Q)).await;
    assert!(
        matches!(out.unwrap(), ContentOutcome::Cached { .. }),
        "newest row survives eviction"
    );
    let out = h.eng.shunt_content(&content_req(&a, DEFAULT_Q)).await;
    assert!(
        matches!(out.unwrap(), ContentOutcome::Summarized { .. }),
        "evicted row re-delegates"
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        4,
        "one re-delegation after eviction"
    );
    assert_eq!(row_count(&h.db, "shunt_content_cache").await, 2);
}
