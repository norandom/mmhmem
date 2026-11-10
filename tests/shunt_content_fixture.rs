//! Consumer-contract pin for `shunt_content` (shunt-content-tool task 4.1;
//! design.md "mcp / shunt_content tool" API contract, Req 2.1/2.2): the
//! wire result a real serve-mode mmhmem returns must match the saved
//! fixture byte for byte — summarized, cached and both bypass reasons — so
//! the mvibe consumer (`../mvibe` proxy-econ-client) can point its fake
//! endpoint (`tests/econ_helpers.py` `FakeMmhmem`) at real wire bytes
//! instead of a hand-written approximation. `latency_ms` is the contract's
//! one wall-clock field; its digits are masked on both sides, everything
//! else must be identical bytes. This is a pinning test, not TDD: it was
//! green the moment the captured fixture was saved (task 4.1 report).

mod common;

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use common::{mcp_init, mcp_tool, spawn_server, sse_json};
use mmhmem::config::{
    Config, ENV_ALLOWED_ROOTS, ENV_DATA_DIR, ENV_LOG_FILE, ENV_SHUNT_MIN_LINES, ENV_TOKEN,
    ENV_WORKER_API_URL, ENV_WORKER_MODEL,
};
use mmhmem::storage::audit::sha256_hex;
use serde_json::{Value, json};

const STUB_MODEL: &str = "stub-model";
const STUB_SUMMARY: &str = "- exports [L1-L160]";
const FIXTURE_REL: &str = "tests/fixtures/shunt_content_wire_results.json";
/// Lines of the benign corpus the fixture was recorded with (no secret
/// pattern matches). The fixture pins its bytes, so the rig runs with the
/// threshold at exactly this size, independent of the shipped default.
const CORPUS_LINES: usize = 160;
const SHORT_LINE: &str = "one short line of output\n";
const SECRET_PEM: &str =
    "-----BEGIN RSA PRIVATE KEY-----\nMIIfake\n-----END RSA PRIVATE KEY-----\n";

/// Benign multi-line file-read tool-result fixture (no secret pattern
/// matches; every line avoids `:`/`=` assignments).
fn benign_content(lines: usize) -> String {
    (1..=lines)
        .map(|i| format!("line {i} of the historical file-read result\n"))
        .collect()
}

/// One counting OpenAI-shaped stub worker on an ephemeral port (loopback
/// only, no network egress).
async fn spawn_stub_worker() -> (String, Arc<AtomicUsize>, tokio::task::JoinHandle<()>) {
    let calls = Arc::new(AtomicUsize::new(0));
    let c = Arc::clone(&calls);
    let worker = axum::Router::new().route(
        "/v1/chat/completions",
        axum::routing::post(move || {
            let c = c.clone();
            async move {
                c.fetch_add(1, Ordering::SeqCst);
                (
                    axum::http::StatusCode::OK,
                    axum::Json(json!({"choices":[{"message":{"content": STUB_SUMMARY}}],
                                      "usage":{"prompt_tokens":500,"completion_tokens":30}})),
                )
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        axum::serve(listener, worker).await.unwrap();
    });
    (format!("http://{addr}/v1/chat/completions"), calls, task)
}

/// Serve-mode HTTP rig: real server + counting stub worker, no network.
struct HttpRig {
    _temp: tempfile::TempDir,
    client: reqwest::Client,
    base: String,
    session: Option<String>,
    next_id: i64,
    _worker: tokio::task::JoinHandle<()>,
    _server: tokio::task::JoinHandle<()>,
}

impl HttpRig {
    async fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let (url, _calls, worker) = spawn_stub_worker().await;
        let env: HashMap<String, String> = [
            (ENV_TOKEN.to_string(), "tok".to_string()),
            (
                ENV_ALLOWED_ROOTS.to_string(),
                temp.path().join("roots").display().to_string(),
            ),
            (
                ENV_DATA_DIR.to_string(),
                temp.path().join("store").display().to_string(),
            ),
            (
                ENV_LOG_FILE.to_string(),
                temp.path().join("audit.jsonl").display().to_string(),
            ),
            (ENV_WORKER_API_URL.to_string(), url),
            (ENV_WORKER_MODEL.to_string(), STUB_MODEL.to_string()),
            (ENV_SHUNT_MIN_LINES.to_string(), CORPUS_LINES.to_string()),
        ]
        .into_iter()
        .collect();
        let config = Config::from_env(&|k| env.get(k).cloned()).unwrap();
        let state = Arc::new(mmhmem::mcp::server::AppState::build(config).unwrap());
        let (port, server) = spawn_server(state).await;
        let base = format!("http://127.0.0.1:{port}");
        let client = reqwest::Client::new();
        let session = mcp_init(&client, &base, "tok").await;
        Self {
            _temp: temp,
            client,
            base,
            session,
            next_id: 2,
            _worker: worker,
            _server: server,
        }
    }

    /// One `tools/call shunt_content`; returns the RAW text-block bytes
    /// (not parsed) — the bytes the consumer's fake endpoint must serve.
    async fn wire_text(&mut self, content: &str) -> String {
        self.next_id += 1;
        let args = json!({
            "project": "proxy-econ", "agent_harness": "mvibe-proxy",
            "content": content, "kind": "file_read"
        });
        let (status, body) = mcp_tool(
            &self.client,
            &self.base,
            "tok",
            &mut self.session,
            self.next_id,
            "shunt_content",
            &args.to_string(),
        )
        .await;
        assert_eq!(status, 200, "body={body}");
        let msg = sse_json(&body).into_iter().next().unwrap();
        assert!(msg.get("error").is_none(), "unexpected error: {msg}");
        let result = &msg["result"];
        assert_ne!(result["isError"], json!(true), "unexpected isError: {msg}");
        let blocks = result["content"].as_array().expect("content array");
        assert_eq!(blocks.len(), 1, "exactly one text block (Req 2.1)");
        blocks[0]["text"].as_str().expect("text block").to_string()
    }
}

/// Replaces the digits of the `latency_ms` value with `N` — the contract's
/// only wall-clock field; every other byte must match exactly.
fn mask_latency(text: &str) -> String {
    const KEY: &str = "\"latency_ms\":";
    let Some(at) = text.find(KEY) else {
        return text.to_string();
    };
    let start = at + KEY.len();
    let end = text[start..]
        .find(|c: char| !c.is_ascii_digit())
        .map_or(text.len(), |n| start + n);
    format!("{}N{}", &text[..start], &text[end..])
}

/// Asserts the parsed wire object carries exactly the design.md key set
/// (absent fields omitted, never null — Req 2.1).
fn assert_key_set(text: &str, keys: &[&str]) {
    let v: Value = serde_json::from_str(text).expect("one JSON object (Req 2.1)");
    let mut got: Vec<&str> = v
        .as_object()
        .expect("object")
        .keys()
        .map(String::as_str)
        .collect();
    got.sort_unstable();
    let mut want = keys.to_vec();
    want.sort_unstable();
    assert_eq!(got, want, "wire key set for: {text}");
}

/// The saved fixture (byte-exact wire texts the consumer's fake serves).
fn fixture() -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(FIXTURE_REL);
    let text = fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
        .replace("\r\n", "\n");
    serde_json::from_str(&text).expect("fixture JSON")
}

#[tokio::test]
async fn wire_results_match_the_saved_consumer_contract_fixture() {
    let fx = fixture();
    let mut rig = HttpRig::new().await;
    let content = benign_content(CORPUS_LINES);
    assert_eq!(
        fx["request"]["content_sha256"].as_str().unwrap(),
        sha256_hex(content.as_bytes()),
        "fixture content recipe must match this test's content"
    );

    // Delegation then cache hit on identical content (Req 2.1/2.2).
    let summarized = rig.wire_text(&content).await;
    let cached = rig.wire_text(&content).await;

    // Bypasses: below threshold and content secret (Req 3.2/3.3) — these
    // carry no latency field, so the fixture must match with NO masking.
    let below = rig.wire_text(SHORT_LINE).await;
    let secret = rig.wire_text(SECRET_PEM).await;

    // Byte-for-byte against the fixture (latency masked where present).
    let want = |key: &str| {
        fx["wire"][key]
            .as_str()
            .unwrap_or_else(|| panic!("wire.{key}"))
    };
    assert_eq!(
        mask_latency(&summarized),
        mask_latency(want("summarized")),
        "summarized wire bytes must match the fixture byte for byte"
    );
    assert_eq!(
        mask_latency(&cached),
        mask_latency(want("cached")),
        "cached wire bytes must match the fixture byte for byte"
    );
    assert_eq!(below, want("bypassed_below_threshold"), "no volatile field");
    assert_eq!(
        secret,
        want("bypassed_security_critical"),
        "no volatile field"
    );

    // The design.md contract's key sets, machine-checked (Req 2.1).
    assert_key_set(
        &summarized,
        &[
            "outcome",
            "summary",
            "content_sha256",
            "line_count",
            "avoided_input_tokens_est",
            "latency_ms",
            "worker_model",
        ],
    );
    assert_key_set(
        &cached,
        &[
            "outcome",
            "summary",
            "content_sha256",
            "line_count",
            "avoided_input_tokens_est",
            "latency_ms",
        ],
    );
    for (text, reason, source) in [
        (&below, "below_threshold", SHORT_LINE),
        (&secret, "security_critical", SECRET_PEM),
    ] {
        assert_key_set(
            text,
            &["outcome", "bypass_reason", "content_sha256", "line_count"],
        );
        let v: Value = serde_json::from_str(text).unwrap();
        assert_eq!(v["bypass_reason"], json!(reason));
        assert_eq!(
            v["content_sha256"],
            json!(sha256_hex(source.as_bytes())),
            "hash of the fixture's bypass content"
        );
    }

    // Cached summary bytes are the delegation bytes (Req 2.2).
    let s: Value = serde_json::from_str(&summarized).unwrap();
    let c: Value = serde_json::from_str(&cached).unwrap();
    assert_eq!(s["summary"], c["summary"], "byte-identical (Req 2.2)");
    assert_eq!(s["summary"], json!(STUB_SUMMARY));
    assert_eq!(s["worker_model"], json!(STUB_MODEL));
}
