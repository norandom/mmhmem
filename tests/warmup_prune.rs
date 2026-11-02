//! Warmup + prune tool integration through a real stdio child
//! (corpus-warmup-prune tasks 6.1/6.2; Req 1.2-1.8, 3.1-3.6, 4.1-4.3).
//! The worker is an in-test HTTP stub (the tests/shunt.rs pattern): a real
//! delegation must produce real cache rows, real stats rows, and honest
//! worker token totals.

use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio::process::{Child, ChildStdin, Command};

use mmhmem::services::shunt_cache::{cache_lookup, cache_store, query_fingerprint};
use mmhmem::storage::audit::sha256_hex;
use mmhmem::storage::db::Db;

const READ_TIMEOUT: Duration = Duration::from_secs(10);

/// A canned chat-completions response with real usage numbers.
const SUMMARY_CHAT: &str = r#"{"choices":[{"message":{"content":"- foo [L1-L9]\n- bar [L10-L20]"}}],"usage":{"prompt_tokens":500,"completion_tokens":30}}"#;

/// The question every warmup call in this file passes explicitly, so the
/// seeded cache rows can use the same fingerprint.
const QUESTION: &str = "summarize the warmup corpus";

// ---------- stdio child plumbing (tests/local_stdio.rs pattern) ----------

const INIT: &str = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test","version":"0"}}}"#;
const INITIALIZED: &str = r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;

/// Spawns `mmhmem stdio` with the data dir, optional confinement roots,
/// and an optional worker.
fn spawn_cmd(data_dir: &Path, roots: Option<&str>, worker: Option<(&str, &str)>) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_mmhmem"));
    cmd.arg("stdio")
        .env("AGENT_MEM_DATA_DIR", data_dir)
        .env_remove("AGENT_MEM_LOG_FILE")
        .env_remove("OTEL_EXPORTER_OTLP_ENDPOINT");
    match roots {
        Some(roots) => {
            cmd.env("AGENT_MEM_ALLOWED_ROOTS", roots);
        }
        None => {
            cmd.env_remove("AGENT_MEM_ALLOWED_ROOTS");
        }
    }
    match worker {
        Some((url, model)) => {
            cmd.env("WORKER_API_URL", url).env("WORKER_MODEL", model);
        }
        None => {
            // Blank — not removed: a removed var falls back to the machine's
            // global .mmhmem.json (worker armed on dev hosts during
            // dogfooding), which silently turns "worker unconfigured" tests
            // into configured ones — one test even hit the real worker.
            // Blank wins over the file lookup and is treated as missing.
            cmd.env("WORKER_API_URL", "")
                .env("WORKER_MODEL", "")
                .env("WORKER_API_KEY", "");
        }
    }
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    cmd
}

struct StdioMcp {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<tokio::process::ChildStdout>,
    next_id: u64,
}

impl StdioMcp {
    async fn new(mut cmd: Command) -> Self {
        let mut child = cmd.spawn().expect("spawn mmhmem stdio");
        let stdin = child.stdin.take().expect("stdin pipe");
        let stdout = BufReader::new(child.stdout.take().expect("stdout pipe"));
        Self {
            child,
            stdin,
            stdout,
            next_id: 1,
        }
    }

    async fn send(&mut self, line: &str) {
        self.stdin
            .write_all(line.as_bytes())
            .await
            .expect("write req");
        self.stdin.write_all(b"\n").await.expect("write nl");
        self.stdin.flush().await.expect("flush req");
    }

    async fn wait_for(&mut self, id: u64) -> Value {
        loop {
            let mut line = String::new();
            let n = tokio::time::timeout(READ_TIMEOUT, self.stdout.read_line(&mut line))
                .await
                .expect("timed out waiting for a response line")
                .expect("stdout stream ended before the response arrived");
            assert!(n > 0, "stdout closed while waiting for id {id}");
            let msg: Value = serde_json::from_str(line.trim())
                .unwrap_or_else(|e| panic!("non-JSON stdout frame {line:?}: {e}"));
            if msg.get("id").and_then(Value::as_u64) == Some(id) {
                return msg;
            }
        }
    }

    async fn request(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let id = self.next_id;
        self.send(
            &json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}).to_string(),
        )
        .await;
        self.wait_for(id).await
    }

    async fn handshake(&mut self) {
        self.send(INIT).await;
        let init = self.wait_for(1).await;
        assert!(init.get("error").is_none(), "initialize failed: {init}");
        self.send(INITIALIZED).await;
    }

    async fn call(&mut self, name: &str, arguments: Value) -> Value {
        self.request("tools/call", json!({"name": name, "arguments": arguments}))
            .await
    }

    /// The response text on success, or the error message.
    fn text_of(resp: &Value) -> String {
        resp["result"]["content"][0]["text"]
            .as_str()
            .map(str::to_string)
            .unwrap_or_else(|| {
                resp.get("error")
                    .map(|e| e["message"].as_str().unwrap_or_default().to_string())
                    .unwrap_or_else(|| resp.to_string())
            })
    }

    async fn shutdown(mut self) {
        drop(self.stdin);
        let _ = tokio::time::timeout(READ_TIMEOUT, self.child.wait())
            .await
            .expect("timed out waiting for child exit");
    }
}

// ---------- worker stub (tests/shunt.rs pattern) ----------

fn find_subslice(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// Single-shot HTTP stub: serves `response` to up to `max` connections,
/// counting requests. Returns (url, request_count).
async fn spawn_stub(response: &'static str, max: usize) -> (String, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let count = Arc::new(AtomicUsize::new(0));
    let c_count = Arc::clone(&count);
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
            let resp = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{response}",
                response.len()
            );
            let _ = sock.write_all(resp.as_bytes()).await;
        }
    });
    (format!("http://{addr}/v1/chat/completions"), count)
}

// ---------- helpers ----------

fn write_lines(dir: &Path, name: &str, lines: usize) -> String {
    let path = dir.join(name);
    std::fs::write(
        &path,
        (0..lines)
            .map(|i| format!("line {i}\n"))
            .collect::<String>(),
    )
    .expect("write file");
    path.display().to_string()
}

fn open_store(data_dir: &Path) -> Db {
    Db::open(data_dir).expect("open store")
}

async fn delegated_stats_rows(db: &Db) -> i64 {
    db.with_conn(|c| {
        c.query_row(
            "SELECT COUNT(*) FROM token_stats WHERE kind = 'shunt_delegated'",
            [],
            |r| r.get(0),
        )
    })
    .await
    .expect("count stats rows")
}

// ---------- 6.1: warmup end to end ----------

#[tokio::test]
async fn warmup_delegates_for_real_then_serves_cached() {
    // Req 1.2: a real delegation through the tool lands real cache rows
    // and real stats rows with honest worker token totals; the repeat
    // batch is a pure cache hit (Req 1.3).
    let data = tempfile::tempdir().unwrap();
    let files = tempfile::tempdir().unwrap();
    let big = write_lines(files.path(), "big.rs", 200);
    let (url, calls) = spawn_stub(SUMMARY_CHAT, 4).await;

    let mut mcp = StdioMcp::new(spawn_cmd(data.path(), None, Some((&url, "stub-model")))).await;
    mcp.handshake().await;

    let resp = mcp
        .call(
            "shunt_warmup",
            json!({"project": "p", "agent_harness": "opencode",
                   "paths": [big], "question": QUESTION}),
        )
        .await;
    let text = StdioMcp::text_of(&resp);
    assert!(text.contains("1 delegated"), "delegated count: {text}");
    assert!(
        text.contains("Worker tokens: 500 prompt + 30 completion"),
        "honest worker token totals: {text}"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1, "one real worker call");

    let db = open_store(data.path());
    assert_eq!(
        delegated_stats_rows(&db).await,
        1,
        "a real shunt_delegated stats row exists"
    );

    let resp = mcp
        .call(
            "shunt_warmup",
            json!({"project": "p", "agent_harness": "opencode",
                   "paths": [big], "question": QUESTION}),
        )
        .await;
    let text = StdioMcp::text_of(&resp);
    assert!(text.contains("1 cached"), "repeat batch is cached: {text}");
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "cache hit must not call the worker"
    );

    mcp.shutdown().await;
}

#[tokio::test]
async fn warmup_mixed_batch_counts_every_outcome() {
    // Req 1.4/1.5: cached, below-threshold, security, missing, and denied
    // files are all counted in one response; nothing aborts.
    let data = tempfile::tempdir().unwrap();
    let files = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();

    // Seed a cache row for the big file so it comes back "cached" without
    // a worker (the canonical path is the engine's lookup key).
    let cached_file = write_lines(files.path(), "cached.rs", 200);
    {
        let db = open_store(data.path());
        let body = std::fs::read(&cached_file).expect("read");
        let sha = sha256_hex(&body);
        let canonical = std::fs::canonicalize(&cached_file).expect("canonicalize");
        cache_store(
            &db,
            &canonical,
            &sha,
            &query_fingerprint(QUESTION),
            "seeded",
            200,
        )
        .await
        .expect("seed");
    }
    let small = write_lines(files.path(), "small.rs", 10);
    let secure = write_lines(files.path(), "auth_util.rs", 200);
    let outside_file = write_lines(outside.path(), "outside.rs", 200);

    let mut mcp = StdioMcp::new(spawn_cmd(
        data.path(),
        Some(files.path().to_str().unwrap()),
        None,
    ))
    .await;
    mcp.handshake().await;

    let resp = mcp
        .call(
            "shunt_warmup",
            json!({"project": "p", "agent_harness": "opencode",
                   "paths": [cached_file, small, secure, "missing.rs", outside_file],
                   "question": QUESTION}),
        )
        .await;
    let text = StdioMcp::text_of(&resp);
    assert!(text.contains("1 cached"), "cached: {text}");
    assert!(text.contains("1 below threshold"), "threshold: {text}");
    assert!(text.contains("1 security-skipped"), "security: {text}");
    assert!(text.contains("1 missing"), "missing: {text}");
    assert!(text.contains("1 denied"), "denied: {text}");
    assert!(text.contains("0 delegated"), "no worker configured: {text}");

    mcp.shutdown().await;
}

#[tokio::test]
async fn warmup_worker_unconfigured_error_matches_smart_read() {
    // Req 1.7: the batch aborts with the engine's own actionable error —
    // byte-identical to smart_read's wording.
    let data = tempfile::tempdir().unwrap();
    let files = tempfile::tempdir().unwrap();
    let big = write_lines(files.path(), "big.rs", 200);

    let mut mcp = StdioMcp::new(spawn_cmd(data.path(), None, None)).await;
    mcp.handshake().await;

    let read = mcp
        .call(
            "smart_read",
            json!({"project": "p", "agent_harness": "opencode",
                   "file_path": big, "question": "q"}),
        )
        .await;
    let read_msg = StdioMcp::text_of(&read);
    let warm = mcp
        .call(
            "shunt_warmup",
            json!({"project": "p", "agent_harness": "opencode",
                   "paths": [big], "question": "q"}),
        )
        .await;
    let warm_msg = StdioMcp::text_of(&warm);
    assert_eq!(
        warm_msg, read_msg,
        "worker-unconfigured error must match smart_read's parity"
    );

    mcp.shutdown().await;
}

// ---------- 6.2: prune tool end to end ----------

#[tokio::test]
async fn prune_tool_dry_run_previews_then_real_run_removes() {
    // Req 3.1-3.3, 4.1-4.3: preview counts only; the real run removes
    // exactly the stale rows; fresh rows survive.
    let data = tempfile::tempdir().unwrap();
    let files = tempfile::tempdir().unwrap();

    let fresh = write_lines(files.path(), "fresh.rs", 20);
    let (fresh_sha, fresh_key) = {
        let body = std::fs::read(&fresh).expect("read");
        (sha256_hex(&body), fresh.clone())
    };
    let missing_path = files.path().join("gone.rs");
    let missing_key = missing_path.display().to_string();
    let missing_sha = sha256_hex(b"gone content\n");
    {
        let db = open_store(data.path());
        cache_store(
            &db,
            Path::new(&fresh_key),
            &fresh_sha,
            "fp",
            "fresh summary",
            20,
        )
        .await
        .expect("seed fresh");
        cache_store(
            &db,
            Path::new(&missing_key),
            &missing_sha,
            "fp",
            "stale summary",
            20,
        )
        .await
        .expect("seed missing");
    }

    let mut mcp = StdioMcp::new(spawn_cmd(data.path(), None, None)).await;
    mcp.handshake().await;

    let preview = mcp
        .call(
            "shunt_prune",
            json!({"project": "p", "agent_harness": "opencode", "dry_run": true}),
        )
        .await;
    let text = StdioMcp::text_of(&preview);
    assert!(text.contains("nothing deleted"), "preview label: {text}");
    assert!(text.contains("1 missing"), "preview counts: {text}");
    assert!(text.contains("1 kept fresh"), "preview counts: {text}");
    {
        let db = open_store(data.path());
        assert!(
            cache_lookup(&db, Path::new(&missing_key), &missing_sha, "fp")
                .await
                .expect("lookup")
                .is_some(),
            "preview must not delete"
        );
    }

    let real = mcp
        .call(
            "shunt_prune",
            json!({"project": "p", "agent_harness": "opencode"}),
        )
        .await;
    let text = StdioMcp::text_of(&real);
    assert!(text.contains("Prune complete"), "completion label: {text}");
    assert!(text.contains("1 missing"), "grouped counts: {text}");
    {
        let db = open_store(data.path());
        assert!(
            cache_lookup(&db, Path::new(&missing_key), &missing_sha, "fp")
                .await
                .expect("lookup")
                .is_none(),
            "real run removes the stale row"
        );
        assert!(
            cache_lookup(&db, Path::new(&fresh_key), &fresh_sha, "fp")
                .await
                .expect("lookup")
                .is_some(),
            "fresh row survives"
        );
    }

    mcp.shutdown().await;
}

// ---------- 10.3: breadcrumb search end to end ----------

#[tokio::test]
async fn breadcrumb_search_finds_real_crumbs_end_to_end() {
    // Req 10.1-10.5 through the real binary: real breadcrumbs (a
    // checkpoint plus a smart_read call) are found by keyword and harness
    // filter, newest first, with no stored row modified.
    let data = tempfile::tempdir().unwrap();
    let files = tempfile::tempdir().unwrap();
    let small = write_lines(files.path(), "small.rs", 10);

    let mut mcp = StdioMcp::new(spawn_cmd(data.path(), None, None)).await;
    mcp.handshake().await;

    // Two harnesses make real crumbs: a checkpoint and a smart_read.
    mcp.call(
        "session_resume_or_handoff",
        json!({"project": "p", "agent_harness": "claude-code",
               "updated_working_state_md": "state"}),
    )
    .await;
    mcp.call(
        "smart_read",
        json!({"project": "p", "agent_harness": "opencode",
               "file_path": small, "question": "q"}),
    )
    .await;

    let resp = mcp
        .call(
            "mem_search_breadcrumbs",
            json!({"project": "p", "agent_harness": "opencode", "query": "handoff"}),
        )
        .await;
    let text = StdioMcp::text_of(&resp);
    assert!(
        text.contains("session_resume_or_handoff") || text.contains("session_handoff"),
        "keyword hits the checkpoint crumbs: {text}"
    );
    assert!(text.contains("Newest First"), "render header: {text}");

    // Blank query lists all crumbs newest first (Req 10.2): the later
    // smart_read crumb sorts before the session crumbs.
    let resp = mcp
        .call(
            "mem_search_breadcrumbs",
            json!({"project": "p", "agent_harness": "opencode", "query": ""}),
        )
        .await;
    let text = StdioMcp::text_of(&resp);
    let smart_pos = text.find("smart_read");
    let checkpoint_pos = text.find("session");
    assert!(
        smart_pos
            .map(|s| checkpoint_pos.map(|c| s < c).unwrap_or(false))
            .unwrap_or(false),
        "the later smart_read crumb sorts first: {text}"
    );

    // Harness filter: opencode's crumb only.
    let resp = mcp
        .call(
            "mem_search_breadcrumbs",
            json!({"project": "p", "agent_harness": "opencode",
                   "query": "smart_read", "harness": "opencode"}),
        )
        .await;
    let text = StdioMcp::text_of(&resp);
    assert!(
        text.contains("[opencode]"),
        "filter keeps the opencode crumb: {text}"
    );

    let resp = mcp
        .call(
            "mem_search_breadcrumbs",
            json!({"project": "p", "agent_harness": "opencode",
                   "query": "smart_read", "harness": "claude-code"}),
        )
        .await;
    let text = StdioMcp::text_of(&resp);
    assert!(
        text.contains("No breadcrumbs matched"),
        "other harness's crumbs are filtered out: {text}"
    );

    // No stored row was modified by the searches: total rows = the three
    // tool-call crumbs plus each search's own crumb (mem_search parity).
    let db = open_store(data.path());
    let count: i64 = db
        .with_conn(|c| {
            c.query_row(
                "SELECT COUNT(*) FROM session_breadcrumbs WHERE project = 'p'",
                [],
                |r| r.get(0),
            )
        })
        .await
        .expect("count");
    assert_eq!(count, 7, "3 call crumbs + 4 own search crumbs: {count}");

    mcp.shutdown().await;
}
