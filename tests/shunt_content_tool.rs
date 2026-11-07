//! `shunt_content` MCP tool-surface integration tests (shunt-content-tool
//! task 3.1; design.md "mcp / shunt_content tool"): enumeration with the
//! conservative annotations over Streamable HTTP, full round trips over
//! HTTP (serve mode) AND stdio (local mode) that parse the one-text-block
//! JSON contract for summarized, cached and bypassed outcomes, and the
//! documented argument-error texts leaving no rows behind
//! (Req 1.1, 1.3, 1.4, 2.1, 2.3, 2.4, 6.4).

mod common;
mod routing_metadata;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use mmhmem::config::{
    Config, ENV_ALLOWED_ROOTS, ENV_DATA_DIR, ENV_LOG_FILE, ENV_TOKEN, ENV_WORKER_API_URL,
    ENV_WORKER_MODEL,
};
use mmhmem::services::shunt_content::MAX_CONTENT_BYTES;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::time::timeout;

use common::{mcp_call, mcp_init, mcp_tool, spawn_server, sse_json};

const STUB_MODEL: &str = "stub-model";
const STUB_SUMMARY: &str = "- exports [L1-L160]";

/// Benign multi-line file-read tool-result fixture (no secret pattern
/// matches; every line avoids `:`/`=` assignments).
fn benign_content(lines: usize) -> String {
    (1..=lines)
        .map(|i| format!("line {i} of the historical file-read result\n"))
        .collect()
}

/// One counting OpenAI-shaped stub worker on an ephemeral port.
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

// ---------- Streamable HTTP (serve mode) ----------

#[tokio::test]
async fn http_enumerates_shunt_content_with_conservative_annotations() {
    let (_roots, _data, _logs, state) = common::test_state("tok");
    let (port, server) = spawn_server(state).await;
    let base = format!("http://127.0.0.1:{port}");
    let client = reqwest::Client::new();
    let mut session = mcp_init(&client, &base, "tok").await;

    let (status, body) = mcp_call(
        &client,
        &base,
        "tok",
        &mut session,
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}"#,
    )
    .await;
    assert_eq!(status, 200, "body={body}");
    let tools = sse_json(&body)[0]["result"]["tools"].clone();
    routing_metadata::assert_metadata(&tools);

    let sc = tools
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "shunt_content")
        .expect("shunt_content listed (Req 1.1)");
    // Conservative annotation matrix (Req 2.4 / design.md API contract).
    assert_eq!(sc["annotations"]["readOnlyHint"], false);
    assert_eq!(sc["annotations"]["destructiveHint"], true);
    assert_eq!(sc["annotations"]["idempotentHint"], false);
    assert_eq!(sc["annotations"]["openWorldHint"], true);
    // Description phrases mandated by Req 2.4.
    let desc = sc["description"].as_str().unwrap();
    for phrase in [
        "programmatic consumers",
        "use smart_read",
        "not exact source evidence",
        "Cache, audit and statistics are written",
    ] {
        assert!(
            desc.contains(phrase),
            "description missing {phrase}: {desc}"
        );
    }
    // Documented parameter schema (design.md API contract).
    let props = &sc["inputSchema"]["properties"];
    for key in [
        "project",
        "agent_harness",
        "content",
        "kind",
        "source_hint",
        "question",
    ] {
        assert!(props[key].is_object(), "schema property {key}");
    }
    let required = sc["inputSchema"]["required"].as_array().unwrap();
    for key in ["content", "kind"] {
        assert!(required.iter().any(|r| r == key), "{key} required");
    }
    for key in ["source_hint", "question"] {
        assert!(!required.iter().any(|r| r == key), "{key} optional");
    }
    server.abort();
}

/// Serve-mode HTTP rig: real server + counting stub worker, no network.
struct HttpRig {
    _temp: tempfile::TempDir,
    data: PathBuf,
    base: String,
    client: reqwest::Client,
    session: Option<String>,
    worker_calls: Arc<AtomicUsize>,
    next_id: i64,
    _worker: tokio::task::JoinHandle<()>,
    _server: tokio::task::JoinHandle<()>,
}

impl HttpRig {
    async fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let (url, calls, worker) = spawn_stub_worker().await;
        let store = temp.path().join("store").display().to_string();
        let env: HashMap<String, String> = [
            (ENV_TOKEN.to_string(), "tok".to_string()),
            (
                ENV_ALLOWED_ROOTS.to_string(),
                temp.path().join("roots").display().to_string(),
            ),
            (ENV_DATA_DIR.to_string(), store.clone()),
            (
                ENV_LOG_FILE.to_string(),
                temp.path().join("audit.jsonl").display().to_string(),
            ),
            (ENV_WORKER_API_URL.to_string(), url),
            (ENV_WORKER_MODEL.to_string(), STUB_MODEL.to_string()),
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
            data: PathBuf::from(store),
            base,
            client,
            session,
            worker_calls: calls,
            next_id: 2,
            _worker: worker,
            _server: server,
        }
    }

    /// One `tools/call shunt_content`; returns the full response message.
    async fn call(&mut self, args: Value) -> Value {
        self.next_id += 1;
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
        sse_json(&body).into_iter().next().unwrap()
    }
}

/// The single text block of a successful call, parsed as the JSON contract.
fn parse_contract(msg: &Value) -> Value {
    assert!(msg.get("error").is_none(), "unexpected error: {msg}");
    let result = &msg["result"];
    assert_ne!(result["isError"], json!(true), "unexpected isError: {msg}");
    let blocks = result["content"].as_array().expect("content array");
    assert_eq!(blocks.len(), 1, "exactly one text block (Req 2.1)");
    assert_eq!(blocks[0]["type"], "text");
    serde_json::from_str(blocks[0]["text"].as_str().unwrap())
        .expect("text block holds one JSON object (Req 2.1)")
}

/// The text of an isError tool result (design.md "Error Handling": the
/// documented messages surface as `isError`, the form the mvibe consumer
/// reads).
fn error_text(msg: &Value) -> String {
    assert_eq!(
        msg["result"]["isError"],
        json!(true),
        "expected isError: {msg}"
    );
    msg["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .to_string()
}

/// Default consumer-shaped arguments, optionally with a source hint.
fn tool_args(content: &str, source_hint: Option<&str>) -> Value {
    let mut a = json!({
        "project": "proxy-econ", "agent_harness": "mvibe-proxy",
        "content": content, "kind": "file_read"
    });
    if let Some(hint) = source_hint {
        a["source_hint"] = json!(hint);
    }
    a
}

#[tokio::test]
async fn http_round_trips_json_contract_for_summarized_cached_bypassed() {
    let mut rig = HttpRig::new().await;
    let content = benign_content(160);

    // Delegation (Req 2.1 summarized).
    let first = parse_contract(&rig.call(tool_args(&content, None)).await);
    assert_eq!(first["outcome"], "summarized");
    assert_eq!(first["summary"], STUB_SUMMARY);
    assert_eq!(first["worker_model"], STUB_MODEL);
    assert_eq!(first["line_count"], 160);
    let sha = first["content_sha256"].as_str().unwrap().to_string();
    assert_eq!(sha.len(), 64, "sha256 hex: {sha}");
    assert!(first["avoided_input_tokens_est"].is_u64());
    assert!(first["latency_ms"].is_u64());
    assert!(
        first.get("bypass_reason").is_none(),
        "absent fields omitted"
    );

    // Cache hit (Req 2.1/2.2): byte-identical summary, zero worker calls.
    let second = parse_contract(&rig.call(tool_args(&content, None)).await);
    assert_eq!(second["outcome"], "cached");
    assert_eq!(
        second["summary"], first["summary"],
        "byte-identical (Req 2.2)"
    );
    assert_eq!(second["content_sha256"], sha);
    assert!(
        second.get("worker_model").is_none(),
        "worker_model only when the worker was called"
    );
    assert_eq!(rig.worker_calls.load(Ordering::SeqCst), 1);

    // Below-threshold bypass (Req 3.3 via the tool).
    let short = parse_contract(
        &rig.call(tool_args("one short line of output\n", None))
            .await,
    );
    assert_eq!(short["outcome"], "bypassed");
    assert_eq!(short["bypass_reason"], "below_threshold");
    assert!(short.get("summary").is_none());
    assert!(short.get("avoided_input_tokens_est").is_none());

    // Security-critical bypass through the source-hint label (Req 3.1):
    // long benign content, sensitive hint path.
    let hinted = parse_contract(&rig.call(tool_args(&content, Some("auth/token.rs"))).await);
    assert_eq!(hinted["outcome"], "bypassed");
    assert_eq!(hinted["bypass_reason"], "security_critical");

    // Secret content bypass (Req 3.2) — never summarized, no worker call.
    let secret = "-----BEGIN RSA PRIVATE KEY-----\nMIIfake\n-----END RSA PRIVATE KEY-----\n";
    let leaked = parse_contract(&rig.call(tool_args(secret, None)).await);
    assert_eq!(leaked["outcome"], "bypassed");
    assert_eq!(leaked["bypass_reason"], "security_critical");
    assert_eq!(
        rig.worker_calls.load(Ordering::SeqCst),
        1,
        "one worker call total"
    );
}

fn store_counts(db: &Path) -> Vec<(&'static str, i64)> {
    let conn = rusqlite::Connection::open(db).unwrap();
    ["shunt_content_cache", "token_stats", "session_breadcrumbs"]
        .iter()
        .map(|t| {
            let n: i64 = conn
                .query_row(&format!("SELECT count(*) FROM {t}"), [], |r| r.get(0))
                .unwrap();
            (*t, n)
        })
        .collect()
}

#[tokio::test]
async fn http_invalid_kind_empty_oversized_return_documented_errors_no_rows() {
    let mut rig = HttpRig::new().await;
    let long = benign_content(160);

    // Invalid kind (Req 1.3): supported values named, nothing recorded.
    let bad_kind = rig
        .call(json!({
            "project": "p", "agent_harness": "mvibe-proxy",
            "content": &long, "kind": "note"
        }))
        .await;
    assert_eq!(
        error_text(&bad_kind),
        "unsupported kind 'note'; supported: file_read"
    );

    // Empty content (Req 1.4).
    let empty = rig
        .call(json!({
            "project": "p", "agent_harness": "mvibe-proxy",
            "content": "", "kind": "file_read"
        }))
        .await;
    assert_eq!(error_text(&empty), "content is empty");

    // Oversized content (Req 1.4): one byte past the 1 MiB cap.
    let oversized = "a".repeat(MAX_CONTENT_BYTES + 1);
    let big = rig
        .call(json!({
            "project": "p", "agent_harness": "mvibe-proxy",
            "content": oversized, "kind": "file_read"
        }))
        .await;
    assert_eq!(error_text(&big), "content exceeds 1048576 bytes");

    // No rows behind (Req 1.3/1.4): no cache, stats or breadcrumb rows,
    // and the worker was never called.
    assert_eq!(rig.worker_calls.load(Ordering::SeqCst), 0);
    let db = rig.data.join("memory.db");
    assert!(db.is_file(), "store auto-created");
    for (table, n) in store_counts(&db) {
        assert_eq!(n, 0, "{table} rows after rejected calls");
    }
}

// ---------- stdio (local mode) ----------

const INIT: &str = "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2025-11-25\",\"capabilities\":{},\"clientInfo\":{\"name\":\"t\",\"version\":\"0\"}}}";
const READ_TIMEOUT: Duration = Duration::from_secs(20);

/// A spawned local-mode `mmhmem stdio` child with a stub worker armed.
struct StdioMcp {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<tokio::process::ChildStdout>,
    next_id: u64,
}

impl StdioMcp {
    async fn new(data_dir: &Path, worker_url: &str) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_mmhmem"))
            .arg("stdio")
            .env("AGENT_MEM_DATA_DIR", data_dir)
            .env("MMHMEM_NO_UPDATE_CHECK", "1")
            .env("WORKER_API_URL", worker_url)
            .env("WORKER_MODEL", STUB_MODEL)
            .env("WORKER_API_KEY", "")
            .env_remove("AGENT_MEM_TOKEN")
            .env_remove("AGENT_MEM_ALLOWED_ROOTS")
            .env_remove("AGENT_MEM_LOG_FILE")
            .env_remove("OTEL_EXPORTER_OTLP_ENDPOINT")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .expect("spawn mmhmem stdio");
        let stdin = child.stdin.take().expect("stdin pipe");
        let stdout = BufReader::new(child.stdout.take().expect("stdout pipe"));
        let mut me = Self {
            child,
            stdin,
            stdout,
            next_id: 0,
        };
        me.send(INIT).await;
        let init = me.wait_for(1).await;
        assert!(init["result"]["capabilities"]["tools"].is_object());
        me.send("{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}")
            .await;
        me
    }

    async fn send(&mut self, line: &str) {
        self.stdin.write_all(line.as_bytes()).await.unwrap();
        self.stdin.write_all(b"\n").await.unwrap();
        self.stdin.flush().await.unwrap();
    }

    async fn wait_for(&mut self, id: u64) -> Value {
        loop {
            let mut line = String::new();
            let n = timeout(READ_TIMEOUT, self.stdout.read_line(&mut line))
                .await
                .expect("timed out waiting for a response line")
                .expect("stdout ended before the response");
            assert!(n > 0);
            let msg: Value = serde_json::from_str(line.trim()).unwrap();
            if msg.get("id").and_then(Value::as_u64) == Some(id) {
                return msg;
            }
        }
    }

    async fn request(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let id = self.next_id;
        let req = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        self.send(&req.to_string()).await;
        self.wait_for(id).await
    }

    async fn shunt(&mut self, args: Value) -> Value {
        self.request(
            "tools/call",
            json!({"name": "shunt_content", "arguments": args}),
        )
        .await
    }
}

#[tokio::test]
async fn stdio_local_mode_round_trips_json_contract_and_documented_errors() {
    let data = tempfile::tempdir().unwrap();
    let (url, worker_calls, _worker) = spawn_stub_worker().await;
    let mut mcp = StdioMcp::new(data.path(), &url).await;

    // Local-mode enumeration: the tool is listed with the conservative
    // annotations (Req 1.1, 2.4, 6.4).
    let list = mcp.request("tools/list", json!({})).await;
    routing_metadata::assert_metadata(&list["result"]["tools"]);

    let content = benign_content(160);
    let args = json!({
        "project": "proxy-econ", "agent_harness": "mvibe-proxy",
        "content": &content, "kind": "file_read"
    });
    let first = parse_contract(&mcp.shunt(args.clone()).await);
    assert_eq!(first["outcome"], "summarized");
    assert_eq!(first["summary"], STUB_SUMMARY);
    assert_eq!(first["worker_model"], STUB_MODEL);

    let second = parse_contract(&mcp.shunt(args).await);
    assert_eq!(second["outcome"], "cached");
    assert_eq!(second["summary"], STUB_SUMMARY);
    assert_eq!(worker_calls.load(Ordering::SeqCst), 1);

    let short = parse_contract(
        &mcp.shunt(json!({
            "project": "p", "agent_harness": "mvibe-proxy",
            "content": "two lines\nof short output\n", "kind": "file_read"
        }))
        .await,
    );
    assert_eq!(short["outcome"], "bypassed");
    assert_eq!(short["bypass_reason"], "below_threshold");

    // Rejected calls (Req 1.3/1.4): documented texts over stdio too, and
    // no new rows behind.
    let db = data.path().join("memory.db");
    let before = store_counts(&db);
    let workers_before = worker_calls.load(Ordering::SeqCst);
    let bad_kind = mcp
        .shunt(json!({
            "project": "p", "agent_harness": "mvibe-proxy",
            "content": &content, "kind": "transcript"
        }))
        .await;
    assert_eq!(
        error_text(&bad_kind),
        "unsupported kind 'transcript'; supported: file_read"
    );
    let empty = mcp
        .shunt(json!({
            "project": "p", "agent_harness": "mvibe-proxy",
            "content": "", "kind": "file_read"
        }))
        .await;
    assert_eq!(error_text(&empty), "content is empty");
    for (table, n) in store_counts(&db) {
        let (_, was) = before.iter().find(|(t, _)| *t == table).unwrap();
        assert_eq!(n, *was, "{table} rows unchanged by rejected calls");
    }
    assert_eq!(worker_calls.load(Ordering::SeqCst), workers_before);

    // Clean exit on stdin EOF (local-mode Req 2.3 spirit).
    drop(mcp.stdin);
    let ok = timeout(Duration::from_secs(10), mcp.child.wait())
        .await
        .expect("child must exit after EOF")
        .expect("wait child")
        .success();
    assert!(ok);
}
