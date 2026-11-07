//! Content-shunt type + shared source seam tests (shunt-content-tool task
//! 1.1; requirements.md Req 6.2 — the engine is extended through a shared
//! evaluation path, not duplicated). The task 2.1 evaluation tests (secret
//! patterns, hint path, threshold, hash-only bypass audit) live in
//! `tests/shunt_content_bypass.rs` (the 800-line test-file cap of
//! `tests/loc_caps.rs`); the cache-hit and delegation tests land with
//! task 2.2. This file pins:
//!   - the content request, kind and outcome types exist with design.md's
//!     exact field names and render the wire result contract (one flat
//!     JSON object, absent fields omitted, never null);
//!   - `ShuntSource` labels its audit/stat/breadcrumb subjects for both
//!     variants;
//!   - audit details for FILE reads now carry an explicit `source` label
//!     (delegated and cache-hit events) with file behaviour otherwise
//!     unchanged.

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use mmhmem::config::{
    Config, ENV_ALLOWED_ROOTS, ENV_SHUNT_MIN_LINES, ENV_TOKEN, ENV_WORKER_API_URL, ENV_WORKER_MODEL,
};
use mmhmem::services::shunt::ShuntEngine;
use mmhmem::services::shunt_content::{ContentKind, ContentOutcome, ContentRequest};
use mmhmem::services::shunt_types::{BypassReason, ReadRequest, ShuntSource};
use mmhmem::services::worker::WorkerClient;
use mmhmem::storage::audit::AuditWriter;
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

fn read_req<'a>(path: &'a str, question: &'a str) -> ReadRequest<'a> {
    ReadRequest {
        project: "p",
        harness: "h",
        task: None,
        path,
        question,
        offset: None,
        limit: None,
    }
}

fn write_file(root: &Path, name: &str, lines: usize) -> std::path::PathBuf {
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

/// 64 hex chars (content_sha256 shape).
fn sha64(c: char) -> String {
    std::iter::repeat_n(c, 64).collect()
}

/// Exact key set of a rendered contract object.
fn key_set(v: &serde_json::Value) -> BTreeSet<&str> {
    v.as_object()
        .expect("contract json is an object")
        .keys()
        .map(String::as_str)
        .collect()
}

// ---------- 1.1: content types + wire contract ----------

#[test]
fn content_kind_and_request_types_match_the_design_interface() {
    assert_eq!(ContentKind::FileRead.as_str(), "file_read");
    let req = ContentRequest {
        project: "p",
        harness: "mvibe-proxy",
        task: Some("t1"),
        content: "L1: alpha",
        kind: ContentKind::FileRead,
        source_hint: Some("src/proxy/econ.rs"),
        question: "Summarize exported symbols, signatures, and control flow with exact line ranges.",
    };
    let ctx = req.ctx();
    assert_eq!(
        (ctx.project, ctx.harness, ctx.task),
        ("p", "mvibe-proxy", Some("t1"))
    );
}

#[test]
fn bypassed_outcome_renders_only_the_contract_fields() {
    let sha = sha64('a');
    let out = ContentOutcome::Bypassed {
        reason: BypassReason::SecurityCritical,
        content_sha256: sha.clone(),
        line_count: 149,
    };
    let v = out.to_contract_json();
    assert_eq!(v["outcome"], "bypassed");
    assert_eq!(v["bypass_reason"], "security_critical");
    assert_eq!(v["content_sha256"], sha);
    assert_eq!(v["line_count"], 149);
    assert!(
        sha.chars().all(|c| c.is_ascii_hexdigit()) && sha.len() == 64,
        "content_sha256 is 64 hex chars"
    );
    assert_eq!(
        key_set(&v),
        ["bypass_reason", "content_sha256", "line_count", "outcome"]
            .into_iter()
            .collect::<BTreeSet<_>>(),
        "absent fields are omitted, never null: {v}"
    );

    let below = ContentOutcome::Bypassed {
        reason: BypassReason::BelowThreshold,
        content_sha256: sha,
        line_count: 10,
    }
    .to_contract_json();
    assert_eq!(below["bypass_reason"], "below_threshold");
}

#[test]
fn cached_outcome_renders_only_the_contract_fields() {
    let sha = sha64('b');
    let out = ContentOutcome::Cached {
        summary: "- foo [L1-L9]\n- bar [L10-L20]".to_string(),
        content_sha256: sha.clone(),
        line_count: 171,
        avoided_input_tokens_est: 1234,
        latency_ms: 812,
    };
    let v = out.to_contract_json();
    assert_eq!(v["outcome"], "cached");
    assert_eq!(v["summary"], "- foo [L1-L9]\n- bar [L10-L20]");
    assert_eq!(v["content_sha256"], sha);
    assert_eq!(v["line_count"], 171);
    assert_eq!(v["avoided_input_tokens_est"], 1234);
    assert_eq!(v["latency_ms"], 812);
    assert_eq!(
        key_set(&v),
        [
            "avoided_input_tokens_est",
            "content_sha256",
            "latency_ms",
            "line_count",
            "outcome",
            "summary",
        ]
        .into_iter()
        .collect::<BTreeSet<_>>(),
        "no bypass_reason, no worker_model on a cache hit: {v}"
    );
}

#[test]
fn summarized_outcome_renders_the_worker_model_too() {
    let sha = sha64('c');
    let out = ContentOutcome::Summarized {
        summary: "- foo [L1-L9]".to_string(),
        content_sha256: sha,
        line_count: 200,
        avoided_input_tokens_est: 4321,
        latency_ms: 9,
        usage: mmhmem::services::worker::WorkerUsage {
            prompt_tokens: 500,
            completion_tokens: 30,
        },
        worker_model: "qwen2.5-coder:7b".to_string(),
    };
    let v = out.to_contract_json();
    assert_eq!(v["outcome"], "summarized");
    assert_eq!(v["worker_model"], "qwen2.5-coder:7b");
    assert_eq!(v["latency_ms"], 9);
    assert_eq!(
        key_set(&v),
        [
            "avoided_input_tokens_est",
            "content_sha256",
            "latency_ms",
            "line_count",
            "outcome",
            "summary",
            "worker_model",
        ]
        .into_iter()
        .collect::<BTreeSet<_>>(),
        "worker_model appears exactly when the worker was called: {v}"
    );
}

// ---------- 1.1: ShuntSource seam labels ----------

#[test]
fn shunt_source_labels_audit_stat_and_crumb_subjects() {
    let file = ShuntSource::File(std::path::PathBuf::from("/repo/big.rs"));
    assert_eq!(file.label(), "file");
    let d = file.audit_details();
    assert_eq!(d["source"], "file");
    assert_eq!(d["file"], "/repo/big.rs");
    assert_eq!(
        key_set(&d),
        ["file", "source"].into_iter().collect::<BTreeSet<_>>()
    );
    assert_eq!(file.stat_subject(&sha64('d')), "/repo/big.rs");
    assert_eq!(file.crumb_resource(), "/repo/big.rs");

    let long_hint: String = std::iter::repeat_n('x', 300).collect();
    let content = ShuntSource::content(Some(&long_hint));
    assert_eq!(content.label(), "content");
    let d = content.audit_details();
    assert_eq!(d["source"], "content");
    assert_eq!(
        d["hint"].as_str().unwrap().len(),
        256,
        "hint trimmed to 256"
    );
    let sha = sha64('e');
    assert_eq!(
        content.stat_subject(&sha),
        format!("content:{}", &sha[..12])
    );
    assert_eq!(content.crumb_resource().len(), 256);

    let bare = ShuntSource::content(None);
    let d = bare.audit_details();
    assert_eq!(d["source"], "content");
    assert_eq!(key_set(&d), ["source"].into_iter().collect::<BTreeSet<_>>());
    assert_eq!(bare.crumb_resource(), "content");
}

// ---------- 1.1: audit source label for file reads (behavioural seam) ----------

#[tokio::test]
async fn audit_details_carry_explicit_source_label_for_file_reads() {
    let root = tempfile::tempdir().unwrap();
    let (url, calls, _bodies) = spawn_stub_multi(200, SUMMARY_CHAT, 4).await;
    let (_d, logs, eng) = engine_with_worker(root.path(), &url);
    let f = write_file(root.path(), "big.rs", 200);
    let path = f.to_str().unwrap().to_string();

    eng.smart_read(&read_req(&path, "q")).await.unwrap(); // delegated
    eng.smart_read(&read_req(&path, "q")).await.unwrap(); // cache hit
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "second read is a cache hit"
    );

    let canon = f.canonicalize().unwrap().display().to_string();
    let log = std::fs::read_to_string(logs.path().join("audit.jsonl")).unwrap();
    let mut saw_delegated = false;
    let mut saw_hit = false;
    for line in log.lines() {
        let rec: serde_json::Value = serde_json::from_str(line).unwrap();
        match rec["event_type"].as_str() {
            Some("IO_SHUNT_DELEGATED") if !saw_delegated => {
                saw_delegated = true;
                assert_eq!(
                    rec["details"]["source"].as_str(),
                    Some("file"),
                    "delegated audit carries the source label: {line}"
                );
                assert_eq!(rec["details"]["file"].as_str(), Some(canon.as_str()));
                let sha = rec["details"]["sha256"].as_str().unwrap();
                assert_eq!(sha.len(), 64);
                assert!(sha.chars().all(|c| c.is_ascii_hexdigit()));
            }
            Some("IO_SHUNT_CACHE_HIT") if !saw_hit => {
                saw_hit = true;
                assert_eq!(
                    rec["details"]["source"].as_str(),
                    Some("file"),
                    "cache-hit audit carries the source label: {line}"
                );
                assert_eq!(rec["details"]["file"].as_str(), Some(canon.as_str()));
            }
            _ => {}
        }
    }
    assert!(saw_delegated, "no IO_SHUNT_DELEGATED event in {log}");
    assert!(saw_hit, "no IO_SHUNT_CACHE_HIT event in {log}");
}
