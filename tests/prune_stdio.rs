//! Prune and breadcrumb-search tool integration through a real stdio child
//! (corpus-warmup-prune tasks 6.2/10.3; Req 3.1-3.6, 4.1-4.3, 10.1-10.5;
//! shunt-content-tool Req 4.4). No worker: pruning and breadcrumb search
//! never call one.

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};

use mmhmem::services::shunt_cache::{
    ContentCacheRow, SummaryRow, content_cache_lookup, content_cache_store_and_evict,
    summary_lookup, summary_store_and_evict,
};
use mmhmem::storage::audit::sha256_hex;
use mmhmem::storage::db::Db;

const READ_TIMEOUT: Duration = Duration::from_secs(10);

// ---------- stdio child plumbing (tests/local_stdio.rs pattern) ----------

const INIT: &str = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test","version":"0"}}}"#;
const INITIALIZED: &str = r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;

/// Spawns `mmhmem stdio` with the data dir, no confinement roots and no
/// worker.
fn spawn_cmd(data_dir: &Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_mmhmem"));
    cmd.arg("stdio")
        .env("AGENT_MEM_DATA_DIR", data_dir)
        .env("MMHMEM_NO_UPDATE_CHECK", "1")
        .env_remove("AGENT_MEM_LOG_FILE")
        .env_remove("OTEL_EXPORTER_OTLP_ENDPOINT")
        .env_remove("AGENT_MEM_ALLOWED_ROOTS")
        // Blank — not removed: a removed var falls back to the machine's
        // global .mmhmem.json (worker armed on dev hosts during
        // dogfooding), which silently turns "worker unconfigured" tests
        // into configured ones — one test even hit the real worker.
        // Blank wins over the file lookup and is treated as missing.
        .env("WORKER_API_URL", "")
        .env("WORKER_MODEL", "")
        .env("WORKER_API_KEY", "");
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

/// Seeds one file-read summary row (kind `file_read`, `last_path` set)
/// exactly as a `smart_read` delegation would.
async fn seed_file_row(db: &Db, path: &Path, sha: &str, fp: &str, summary: &str) {
    let row = SummaryRow {
        sha,
        fingerprint: fp,
        summary,
        line_count: 20,
        path,
    };
    summary_store_and_evict(db, row, 5000)
        .await
        .expect("seed file row");
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
        seed_file_row(
            &db,
            Path::new(&fresh_key),
            &fresh_sha,
            "fp",
            "fresh summary",
        )
        .await;
        seed_file_row(&db, Path::new(&missing_key), &missing_sha, "fp", "stale").await;
    }

    let mut mcp = StdioMcp::new(spawn_cmd(data.path())).await;
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
            summary_lookup(&db, &missing_sha, "fp", Path::new(&missing_key))
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
            summary_lookup(&db, &missing_sha, "fp", Path::new(&missing_key))
                .await
                .expect("lookup")
                .is_none(),
            "real run removes the stale row"
        );
        assert!(
            summary_lookup(&db, &fresh_sha, "fp", Path::new(&fresh_key))
                .await
                .expect("lookup")
                .is_some(),
            "fresh row survives"
        );
    }

    mcp.shutdown().await;
}

#[tokio::test]
async fn prune_tool_leaves_content_rows_untouched_and_counts_only_file_rows() {
    // shunt-content-tool Req 4.4: with content-cache rows present, the
    // shunt_prune TOOL's preview and real run count ONLY file-cache rows
    // and never delete a content row — content rows have no file to
    // re-check on disk, so classifying them would be dishonest.
    let data = tempfile::tempdir().unwrap();
    let files = tempfile::tempdir().unwrap();

    let fresh = write_lines(files.path(), "fresh.rs", 20);
    let (fresh_sha, fresh_key) = {
        let body = std::fs::read(&fresh).expect("read");
        (sha256_hex(&body), fresh.clone())
    };
    let missing_key = files.path().join("gone.rs").display().to_string();
    let missing_sha = sha256_hex(b"gone content\n");
    let (content_one, content_two) = (
        sha256_hex(b"ephemeral one\n"),
        sha256_hex(b"ephemeral two\n"),
    );
    {
        let db = open_store(data.path());
        seed_file_row(&db, Path::new(&fresh_key), &fresh_sha, "fp", "fresh").await;
        seed_file_row(&db, Path::new(&missing_key), &missing_sha, "fp", "stale").await;
        for (sha, summary) in [(&content_one, "one"), (&content_two, "two")] {
            let row = ContentCacheRow {
                content_sha256: sha,
                query_fingerprint: "fp",
                kind: "file_read",
                structural_summary: summary,
                line_count: 200,
            };
            content_cache_store_and_evict(&db, row, 5000)
                .await
                .expect("seed content row");
        }
    }

    let mut mcp = StdioMcp::new(spawn_cmd(data.path())).await;
    mcp.handshake().await;

    for (args, labels) in [
        (
            json!({"project": "p", "agent_harness": "opencode", "dry_run": true}),
            "preview",
        ),
        (
            json!({"project": "p", "agent_harness": "opencode"}),
            "real run",
        ),
    ] {
        let resp = mcp.call("shunt_prune", args).await;
        let text = StdioMcp::text_of(&resp);
        assert!(
            text.contains("1 missing") && text.contains("1 kept fresh"),
            "{labels} counts ONLY file-cache rows: {text}"
        );
        let db = open_store(data.path());
        for sha in [&content_one, &content_two] {
            assert!(
                content_cache_lookup(&db, sha, "fp", "file_read")
                    .await
                    .expect("content lookup")
                    .is_some(),
                "{labels} must leave content row {sha} untouched"
            );
        }
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

    let mut mcp = StdioMcp::new(spawn_cmd(data.path())).await;
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
