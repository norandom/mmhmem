//! Same-machine multi-process sharing tests (task 5.1; Req 5.1-5.5, 6.1,
//! 6.3, 6.4, 2.6): two real `mmhmem stdio` child processes against ONE
//! per-user store — cross-process handoff, shared shunt cache (zero
//! worker calls), cross-process memory, shared token stats, and a
//! concurrent fresh-open race on first run. No coordination beyond the
//! processes themselves (Req 6.4).

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::time::timeout;

use mmhmem::services::shunt_cache;
use mmhmem::storage::audit::sha256_hex;
use mmhmem::storage::db::{DB_FILE_NAME, Db};

const INIT: &str = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"t","version":"0"}}}"#;
const INITIALIZED: &str = r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;
const READ_TIMEOUT: Duration = Duration::from_secs(20);
const BIG_FILE_LINES: usize = 220;
const CACHE_MARKER: &str = "CACHED_SUMMARY_MARKER_9c2";
const WORKING_STATE: &str = "working state: halfway there";

/// A spawned `mmhmem stdio` child with framed-line MCP helpers (same
/// pattern as tests/local_stdio.rs; tests/common/mod.rs is off-limits to
/// keep the pre-existing suite byte-identical).
struct StdioMcp {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<tokio::process::ChildStdout>,
    next_id: u64,
}

impl StdioMcp {
    async fn new(data_dir: &Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_mmhmem"))
            .arg("stdio")
            .env("AGENT_MEM_DATA_DIR", data_dir)
            .env_remove("AGENT_MEM_TOKEN")
            .env_remove("AGENT_MEM_ALLOWED_ROOTS")
            // Blank — not removed: removed worker vars fall back to the
            // machine's global .mmhmem.json (armed on dev hosts); blank
            // wins over the file lookup and is treated as missing.
            .env("WORKER_API_URL", "")
            .env("WORKER_MODEL", "")
            .env("WORKER_API_KEY", "")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .expect("spawn mmhmem stdio");
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
            let n = timeout(READ_TIMEOUT, self.stdout.read_line(&mut line))
                .await
                .expect("timed out waiting for a response line")
                .expect("stdout ended before the response arrived");
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
        let req = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        self.send(&req.to_string()).await;
        self.wait_for(id).await
    }

    /// Handshake + a `tools/call`; returns the response's text field.
    async fn call_tool(&mut self, name: &str, arguments: Value) -> String {
        let resp = self
            .request("tools/call", json!({"name": name, "arguments": arguments}))
            .await;
        resp["result"]["content"][0]["text"]
            .as_str()
            .map(str::to_string)
            .unwrap_or_else(|| resp.to_string())
    }

    async fn handshake(&mut self) {
        self.send(INIT).await;
        let init = self.wait_for(1).await;
        assert!(init.get("error").is_none(), "initialize failed: {init}");
        self.send(INITIALIZED).await;
    }

    async fn shutdown(mut self) -> bool {
        drop(self.stdin);
        let status = timeout(READ_TIMEOUT, self.child.wait())
            .await
            .expect("timed out waiting for child exit")
            .expect("child wait failed");
        let mut stderr = String::new();
        if let Some(mut err) = self.child.stderr.take() {
            let _ = err.read_to_string(&mut stderr).await;
        }
        status.success()
    }
}

fn big_file(dir: &Path) -> PathBuf {
    let path = dir.join("big.rs");
    let body: String = (0..BIG_FILE_LINES)
        .map(|i| format!("line {i} of the shared cache target file\n"))
        .collect();
    std::fs::write(&path, body).expect("write big file");
    path
}

/// Seeds one shunt-cache row via the lib API so BOTH processes hit the
/// cache with ZERO worker calls (Req 5.3) — no worker is configured at
/// all in these tests.
async fn seed_cache_row(data_dir: &Path, file: &Path, question: &str) {
    let content = std::fs::read_to_string(file).unwrap();
    let sha = sha256_hex(content.as_bytes());
    let fp = shunt_cache::query_fingerprint(question);
    let canonical = std::fs::canonicalize(file).unwrap();
    let db = Db::open(data_dir).unwrap();
    db.with_conn(move |conn| {
        conn.execute(
            "INSERT OR REPLACE INTO shunt_file_cache \
             (file_path, content_sha256, query_fingerprint, structural_summary, \
              line_count, created_at_epoch) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![
                canonical.to_string_lossy(),
                sha,
                fp,
                format!("summary text {CACHE_MARKER}"),
                BIG_FILE_LINES as i64,
                0i64
            ],
        )?;
        Ok(())
    })
    .await
    .unwrap();
    drop(db);
}

async fn token_stats_count(data_dir: &Path, kind: &str) -> i64 {
    let db = Db::open_read_only(&data_dir.join(DB_FILE_NAME))
        .unwrap()
        .expect("store exists");
    let kind = kind.to_string();
    db.with_conn(move |conn| {
        let n: i64 = conn.query_row(
            "SELECT COUNT(*) FROM token_stats WHERE kind = ?1",
            rusqlite::params![kind],
            |r| r.get(0),
        )?;
        Ok(n)
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn two_processes_share_handoff_cache_memory_and_stats() {
    let data = tempfile::tempdir().unwrap();
    let file = big_file(data.path());
    let question = "summarize the shared file";

    // Test-process seeding of the shared shunt cache (Req 5.3 setup).
    seed_cache_row(data.path(), &file, question).await;

    // Harness A (child process 1): new session, observation, cached read,
    // checkpoint.
    let mut a = StdioMcp::new(data.path()).await;
    a.handshake().await;
    let started = a
        .call_tool(
            "session_resume_or_handoff",
            json!({"project": "proj", "agent_harness": "claude-code"}),
        )
        .await;
    assert!(
        started.contains("No active cross-agent session"),
        "fresh project starts a session: {started}"
    );
    let task_id = started
        .split("task ID: ")
        .nth(1)
        .unwrap()
        .trim()
        .to_string();

    let obs = a
        .call_tool(
            "mem_record_observation",
            json!({
                "project": "proj", "agent_harness": "claude-code", "task": task_id,
                "obs_type": "discovery", "title": "multiprocess obs",
                "narrative": "recorded by harness A", "facts": "f", "files": "big.rs",
            }),
        )
        .await;
    assert!(obs.contains("#1"), "observation stored: {obs}");

    let cached_a = a
        .call_tool(
            "smart_read",
            json!({
                "project": "proj", "agent_harness": "claude-code",
                "file_path": file.to_string_lossy(), "question": question,
            }),
        )
        .await;
    assert!(
        cached_a.contains(CACHE_MARKER),
        "process A must hit the shared cache (no worker configured): {cached_a}"
    );

    let checkpoint = a
        .call_tool(
            "session_resume_or_handoff",
            json!({
                "project": "proj", "agent_harness": "claude-code",
                "workspace_task_id": task_id, "active_objective": "ship it",
                "updated_working_state_md": WORKING_STATE,
            }),
        )
        .await;
    assert!(
        checkpoint.contains("Checkpoint saved"),
        "A checkpoints: {checkpoint}"
    );
    assert!(a.shutdown().await, "process A exits 0 on EOF");

    // Harness B (child process 2, same per-user store): resume inherits
    // A's state, finds A's memory, hits the same cache, accumulates stats.
    let mut b = StdioMcp::new(data.path()).await;
    b.handshake().await;
    let resumed = b
        .call_tool(
            "session_resume_or_handoff",
            json!({"project": "proj", "agent_harness": "opencode"}),
        )
        .await;
    assert!(
        resumed.contains(WORKING_STATE),
        "B receives A's working state through the shared store: {resumed}"
    );
    assert!(
        resumed.contains("claude-code"),
        "B sees the last active harness: {resumed}"
    );

    let found = b
        .call_tool(
            "mem_search",
            json!({"project": "proj", "agent_harness": "opencode", "query": "multiprocess"}),
        )
        .await;
    assert!(
        found.contains("multiprocess obs"),
        "B's search finds A's observation immediately (Req 5.4): {found}"
    );

    let cached_b = b
        .call_tool(
            "smart_read",
            json!({
                "project": "proj", "agent_harness": "opencode",
                "file_path": file.to_string_lossy(), "question": question,
            }),
        )
        .await;
    assert!(
        cached_b.contains(CACHE_MARKER),
        "process B hits the same shared cache entry (Req 5.3): {cached_b}"
    );
    assert!(b.shutdown().await, "process B exits 0 on EOF");

    // Req 5.5: token-savings events from BOTH processes accumulated in the
    // one shared store (one cache-hit row per process, zero worker calls).
    let hits = token_stats_count(data.path(), "shunt_cache_hit").await;
    assert_eq!(hits, 2, "one shared token_stats table, two processes");
}

#[tokio::test]
async fn concurrent_fresh_open_is_safe() {
    // Req 6.1/2.6: two processes race on a not-yet-existing store — both
    // must come up, serve, and write without corruption or lost updates.
    let data = tempfile::tempdir().unwrap();

    let mut a = StdioMcp::new(data.path()).await;
    let mut b = StdioMcp::new(data.path()).await;
    a.handshake().await;
    b.handshake().await;

    let cp_a = a
        .call_tool(
            "session_resume_or_handoff",
            json!({
                "project": "proj-a", "agent_harness": "claude-code",
                "workspace_task_id": "task-a", "active_objective": "A",
                "updated_working_state_md": "state A",
            }),
        )
        .await;
    let cp_b = b
        .call_tool(
            "session_resume_or_handoff",
            json!({
                "project": "proj-b", "agent_harness": "opencode",
                "workspace_task_id": "task-b", "active_objective": "B",
                "updated_working_state_md": "state B",
            }),
        )
        .await;
    assert!(
        cp_a.contains("Checkpoint saved") && cp_b.contains("Checkpoint saved"),
        "both concurrent writers succeed: A={cp_a} B={cp_b}"
    );
    assert!(a.shutdown().await);
    assert!(b.shutdown().await);

    // Both projects' sessions survived — no lost updates (Req 6.1).
    let db = Db::open_read_only(&data.path().join(DB_FILE_NAME))
        .unwrap()
        .expect("store exists after concurrent fresh open");
    let (sessions, obs_tables): (i64, i64) = db
        .with_conn(|conn| {
            let s: i64 =
                conn.query_row("SELECT COUNT(*) FROM active_sessions", [], |r| r.get(0))?;
            let o: i64 = conn.query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table'",
                [],
                |r| r.get(0),
            )?;
            Ok((s, o))
        })
        .await
        .unwrap();
    assert_eq!(sessions, 2, "both concurrent checkpoints persisted");
    assert!(
        obs_tables >= 6,
        "schema fully applied exactly once (no torn DDL race)"
    );
}
