//! Local-mode stdio integration tests (task 3.3; Req 2.1-2.6, 4.1-4.4,
//! 12.1). Spawns the REAL binary as a child process and drives MCP over
//! piped stdio with hand-rolled JSON-RPC lines — no new dev-dependencies
//! (house stub style, like tests/cli.rs and tests/daemon_smoke.rs).

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::time::timeout;

const INIT: &str = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"t","version":"0"}}}"#;
const INITIALIZED: &str = r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;
/// Generous bound: the child must ANSWER within it or the test fails
/// instead of hanging (deterministic suite rule).
const READ_TIMEOUT: Duration = Duration::from_secs(20);

/// A spawned `mmhmem stdio` child with framed-line MCP helpers.
struct StdioMcp {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<tokio::process::ChildStdout>,
    next_id: u64,
}

/// Only the data dir is set — everything else (token, roots, worker, otel)
/// is deliberately absent and must be optional end to end (Req 2.5).
fn spawn_stdio(data_dir: &Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_mmhmem"));
    cmd.arg("stdio")
        .env("AGENT_MEM_DATA_DIR", data_dir)
        .env_remove("AGENT_MEM_TOKEN")
        .env_remove("AGENT_MEM_ALLOWED_ROOTS")
        .env_remove("AGENT_MEM_LOG_FILE")
        .env_remove("WORKER_API_URL")
        .env_remove("WORKER_MODEL")
        .env_remove("WORKER_API_KEY")
        .env_remove("OTEL_EXPORTER_OTLP_ENDPOINT")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    cmd
}

/// Like [`spawn_stdio`] but with a worker model configured (Req 8.2 tests).
fn spawn_stdio_with_worker(data_dir: &Path, model: &str) -> Command {
    let mut cmd = spawn_stdio(data_dir);
    cmd.env("WORKER_MODEL", model);
    cmd
}

impl StdioMcp {
    async fn from_command(mut cmd: Command) -> Self {
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

    async fn new(data_dir: &Path) -> Self {
        Self::from_command(spawn_stdio(data_dir)).await
    }

    async fn send(&mut self, line: &str) {
        self.stdin
            .write_all(line.as_bytes())
            .await
            .expect("write req");
        self.stdin.write_all(b"\n").await.expect("write nl");
        self.stdin.flush().await.expect("flush req");
    }

    /// Reads stdout lines until the response with `id` arrives. Every line
    /// must parse as JSON — stdout carries ONLY protocol frames (Req 2.2).
    async fn wait_for(&mut self, id: u64) -> Value {
        loop {
            let mut line = String::new();
            let n = timeout(READ_TIMEOUT, self.stdout.read_line(&mut line))
                .await
                .expect("timed out waiting for a response line")
                .expect("stdout stream ended before the response arrived");
            assert!(n > 0, "stdout closed while waiting for id {id}");
            let msg: Value = serde_json::from_str(line.trim()).unwrap_or_else(|e| {
                panic!("stdout must carry only JSON frames (Req 2.2), got {line:?}: {e}")
            });
            if msg.get("id").and_then(Value::as_u64) == Some(id) {
                return msg;
            }
            // Server notifications (no matching id): skip.
        }
    }

    async fn request(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let id = self.next_id;
        let req = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        self.send(&req.to_string()).await;
        self.wait_for(id).await
    }

    /// Full MCP handshake: initialize + initialized notification.
    async fn handshake(&mut self) -> Value {
        self.send(INIT).await;
        let init = self.wait_for(1).await;
        assert!(
            init.get("error").is_none(),
            "initialize must succeed, got: {init}"
        );
        assert!(
            init["result"]["serverInfo"]["name"].as_str().is_some(),
            "serverInfo missing: {init}"
        );
        self.send(INITIALIZED).await;
        init
    }

    /// Closes stdin (EOF), waits for the child to exit, drains and
    /// JSON-parses any remaining stdout lines, returns (exit-success,
    /// stderr).
    async fn shutdown(mut self) -> (bool, String) {
        drop(self.stdin);
        let status = timeout(READ_TIMEOUT, self.child.wait())
            .await
            .expect("timed out waiting for child exit")
            .expect("child wait failed");
        // Drain leftover stdout: must all be JSON frames (Req 2.2).
        let mut rest = String::new();
        let _ = self.stdout.read_to_string(&mut rest).await;
        for line in rest.lines().filter(|l| !l.trim().is_empty()) {
            serde_json::from_str::<Value>(line.trim())
                .unwrap_or_else(|e| panic!("non-JSON stdout frame after shutdown: {line:?}: {e}"));
        }
        let mut stderr = String::new();
        if let Some(mut err) = self.child.stderr.take() {
            let _ = err.read_to_string(&mut stderr).await;
        }
        (status.success(), stderr)
    }
}

fn write_lines_file(path: &Path, lines: usize, marker: &str) {
    let body: String = (0..lines)
        .map(|i| {
            if i == 1 {
                format!("{marker}\n")
            } else {
                format!("line {i} of the local stdio test file\n")
            }
        })
        .collect();
    std::fs::write(path, body).expect("write test file");
}

#[tokio::test]
async fn stdio_handshake_lists_all_nine_tools() {
    let data = tempfile::tempdir().unwrap();
    let mut mcp = StdioMcp::new(data.path()).await;
    mcp.handshake().await;

    let list = mcp.request("tools/list", json!({})).await;
    let names: Vec<&str> = list["result"]["tools"]
        .as_array()
        .expect("tools array")
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    for expected in [
        "session_resume_or_handoff",
        "smart_read",
        "boilerplate_write",
        "mem_search",
        "mem_get_observations",
        "mem_record_observation",
        "shunt_warmup",
        "shunt_prune",
        "mem_search_breadcrumbs",
    ] {
        assert!(
            names.contains(&expected),
            "missing tool {expected}: {names:?}"
        );
    }
    assert_eq!(
        names.len(),
        9,
        "exactly nine tools (corpus-warmup-prune Req 5.3)"
    );

    let (ok, _stderr) = mcp.shutdown().await;
    assert!(ok, "exit 0 after stdin EOF (Req 2.3)");
}

#[tokio::test]
async fn stdio_direct_read_without_worker_and_store_autocreate() {
    let data = tempfile::tempdir().unwrap();
    let file = data.path().join("small.rs");
    write_lines_file(&file, 3, "DIRECT_READ_MARKER_7f3a");

    let mut mcp = StdioMcp::new(data.path()).await;
    mcp.handshake().await;
    let read = mcp
        .request(
            "tools/call",
            json!({"name": "smart_read", "arguments": {
                "project": "p", "agent_harness": "opencode",
                "file_path": file.to_string_lossy(), "question": "what is in here"
            }}),
        )
        .await;
    assert!(
        read.get("error").is_none(),
        "direct read must not error without a worker: {read}"
    );
    let text = read["result"]["content"][0]["text"].as_str().unwrap();
    assert!(
        text.contains("DIRECT_READ_MARKER_7f3a"),
        "direct read must return the file content: {text}"
    );
    // Req 2.6: the store was created on first use, zero setup.
    assert!(
        data.path().join("memory.db").is_file(),
        "per-user store auto-created on first run"
    );
    let (ok, _stderr) = mcp.shutdown().await;
    assert!(ok);
}

#[tokio::test]
async fn stdio_shunt_required_without_worker_is_a_clear_error() {
    let data = tempfile::tempdir().unwrap();
    let file = data.path().join("big.rs");
    write_lines_file(&file, 220, "SHUNT_REQUIRED_MARKER");

    let mut mcp = StdioMcp::new(data.path()).await;
    mcp.handshake().await;
    let read = mcp
        .request(
            "tools/call",
            json!({"name": "smart_read", "arguments": {
                "project": "p", "agent_harness": "opencode",
                "file_path": file.to_string_lossy(), "question": "summarize"
            }}),
        )
        .await;
    // Req 4.4: shunting requires a worker; without one the call returns a
    // clear actionable error naming the missing configuration.
    let msg = read
        .get("error")
        .map(|e| e["message"].as_str().unwrap_or_default().to_string())
        .unwrap_or_else(|| read.to_string());
    assert!(
        msg.to_lowercase().contains("worker"),
        "expected a worker-related actionable error, got: {msg}"
    );
    let (ok, _stderr) = mcp.shutdown().await;
    assert!(ok, "a tool error must not kill the child (Req 2.4 spirit)");
}

#[tokio::test]
async fn stdio_survives_malformed_input_lines() {
    let data = tempfile::tempdir().unwrap();
    let mut mcp = StdioMcp::new(data.path()).await;
    mcp.handshake().await;

    // Req 2.4: garbage must not terminate the process; the next valid
    // request is still answered.
    mcp.send("this is not json-rpc at all").await;
    mcp.send("{ broken json").await;
    let list = mcp.request("tools/list", json!({})).await;
    assert!(
        list.get("error").is_none(),
        "valid request after garbage must be answered: {list}"
    );
    let names = list["result"]["tools"].as_array().unwrap().len();
    assert_eq!(names, 9, "all nine tools still listed after garbage");

    let (ok, _stderr) = mcp.shutdown().await;
    assert!(ok);
}

#[tokio::test]
async fn stdio_exits_cleanly_when_stdin_closes_immediately() {
    // Req 2.3: parent closes stdin right away → flush + clean exit 0.
    let data = tempfile::tempdir().unwrap();
    let mcp = StdioMcp::new(data.path()).await;
    let (ok, _stderr) = mcp.shutdown().await;
    assert!(ok, "immediate EOF must exit 0");
}

#[tokio::test]
async fn resume_notes_missing_worker_when_unconfigured() {
    // Req 8.1: every session-start response (resume view and fresh
    // init) carries the one-line notice naming the missing setting.
    let data = tempfile::tempdir().unwrap();
    let mut mcp = StdioMcp::new(data.path()).await;
    mcp.handshake().await;
    // Checkpoint first so the resume branch has a prior session to show.
    mcp.request(
        "tools/call",
        json!({"name": "session_resume_or_handoff", "arguments": {
            "project": "p", "agent_harness": "opencode",
            "updated_working_state_md": "working state"
        }}),
    )
    .await;
    let resume = mcp
        .request(
            "tools/call",
            json!({"name": "session_resume_or_handoff", "arguments": {
                "project": "p", "agent_harness": "opencode"
            }}),
        )
        .await;
    let text = resume["result"]["content"][0]["text"].as_str().unwrap();
    assert!(
        text.contains("No worker model configured"),
        "resume view must carry the notice: {text}"
    );
    assert!(
        text.contains("worker_model") && text.contains(".mmhmem.json"),
        "notice must name the setting and the config file: {text}"
    );
    // Fresh init (no prior session) carries it too.
    let fresh = mcp
        .request(
            "tools/call",
            json!({"name": "session_resume_or_handoff", "arguments": {
                "project": "fresh-project", "agent_harness": "opencode"
            }}),
        )
        .await;
    let text = fresh["result"]["content"][0]["text"].as_str().unwrap();
    assert!(
        text.contains("No worker model configured"),
        "fresh init must carry the notice: {text}"
    );
    let (ok, _stderr) = mcp.shutdown().await;
    assert!(ok);
}

#[tokio::test]
async fn resume_has_no_worker_notice_when_configured() {
    // Req 8.2: with a worker model configured the notice is absent and
    // the response content is otherwise unchanged.
    let data = tempfile::tempdir().unwrap();
    let mut mcp = StdioMcp::from_command(spawn_stdio_with_worker(data.path(), "stub")).await;
    mcp.handshake().await;
    let resume = mcp
        .request(
            "tools/call",
            json!({"name": "session_resume_or_handoff", "arguments": {
                "project": "p", "agent_harness": "opencode"
            }}),
        )
        .await;
    let text = resume["result"]["content"][0]["text"].as_str().unwrap();
    assert!(
        !text.contains("No worker model configured"),
        "notice must be absent with a worker: {text}"
    );
    assert!(
        text.contains("Initialized new task ID"),
        "the rest of the response is unchanged: {text}"
    );
    let (ok, _stderr) = mcp.shutdown().await;
    assert!(ok);
}
