//! Audit rotation integration tests (task 5.3; Req 7.1-7.4): drive REAL
//! audit traffic through a local-mode child with a tiny rotation
//! threshold, then verify the rotated file set stays within the retention
//! count and every record across all files is complete, digest-verifiable,
//! and identical in shape to serve-mode records.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::time::timeout;

const INIT: &str = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"t","version":"0"}}}"#;
const INITIALIZED: &str = r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;
const READ_TIMEOUT: Duration = Duration::from_secs(20);

struct StdioMcp {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<tokio::process::ChildStdout>,
    next_id: u64,
}

impl StdioMcp {
    async fn new(data_dir: &Path, max_bytes: u32, keep: u32) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_mmhmem"))
            .arg("stdio")
            .env("AGENT_MEM_DATA_DIR", data_dir)
            .env("AGENT_MEM_AUDIT_MAX_BYTES", max_bytes.to_string())
            .env("AGENT_MEM_AUDIT_ROTATE_KEEP", keep.to_string())
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

    async fn call_tool(&mut self, name: &str, arguments: Value) {
        self.next_id += 1;
        let id = self.next_id;
        let req = json!({"jsonrpc": "2.0", "id": id, "method": "tools/call",
                          "params": {"name": name, "arguments": arguments}});
        self.send(&req.to_string()).await;
        let resp = self.wait_for(id).await;
        assert!(
            resp.get("error").is_none(),
            "tool {name} must succeed: {resp}"
        );
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
        status.success()
    }
}

/// Every line across the active + rotated files must parse, carry the
/// per-record digest, and keep the serve-mode record shape (Req 7.1/7.2).
fn assert_records_intact(audit_path: &Path, keep: u32) -> usize {
    let mut total = 0;
    let mut files = vec![audit_path.to_path_buf()];
    for i in 1..=keep {
        files.push(PathBuf::from(format!("{}.{}", audit_path.display(), i)));
    }
    for f in &files {
        if !f.is_file() {
            continue;
        }
        for line in std::fs::read_to_string(f).unwrap().lines() {
            if line.trim().is_empty() {
                continue;
            }
            let v: Value = serde_json::from_str(line).unwrap_or_else(|e| {
                panic!("corrupt audit record in {}: {line:?}: {e}", f.display())
            });
            assert!(
                v.get("record_sha256").is_some(),
                "digest missing in {}: {line}",
                f.display()
            );
            assert!(
                v.get("ts").is_some() && v.get("event_type").is_some(),
                "serve-mode record shape (ts + event_type): {line}"
            );
            total += 1;
        }
    }
    total
}

#[tokio::test]
async fn local_audit_rotates_within_retention_and_records_stay_verifiable() {
    let data = tempfile::tempdir().unwrap();
    let audit_path = data.path().join("audit.jsonl");
    const MAX_BYTES: u32 = 800; // rotate every few events
    const KEEP: u32 = 2;

    let mut child = StdioMcp::new(data.path(), MAX_BYTES, KEEP).await;
    child.handshake().await;
    // Enough real audit-generating traffic to force several rotations.
    for i in 0..12 {
        child
            .call_tool(
                "mem_record_observation",
                json!({
                    "project": "proj", "agent_harness": "opencode", "task": format!("task-{i}"),
                    "obs_type": "discovery", "title": format!("rotation obs {i}"),
                    "narrative": "drives real audit events through the local writer",
                    "facts": "f", "files": "big.rs",
                }),
            )
            .await;
    }
    assert!(child.shutdown().await, "child exits 0 after EOF");

    // Rotation happened…
    assert!(
        PathBuf::from(format!("{}.1", audit_path.display())).is_file(),
        "at least one rotated audit file must exist (threshold {MAX_BYTES} bytes)"
    );
    // …retention is respected…
    assert!(
        !PathBuf::from(format!("{}.3", audit_path.display())).exists(),
        "at most KEEP=2 rotated files may remain (Req 7.3)"
    );
    // …every surviving record is complete, digest-verifiable, and
    // serve-mode shaped…
    let records = assert_records_intact(&audit_path, KEEP);
    assert!(
        records >= 3,
        "records survive across the active + rotated files (found {records})"
    );
    // …and the newest audit traffic is always in the ACTIVE file — the
    // local writer never rotates fresh records away.
    let active = std::fs::read_to_string(&audit_path).unwrap();
    assert!(
        active.contains("rotation obs 11"),
        "the most recent records must be in the active file: {active}"
    );
}

#[tokio::test]
async fn local_audit_redacts_secrets_in_every_rotated_file() {
    let data = tempfile::tempdir().unwrap();
    let audit_path = data.path().join("audit.jsonl");
    const MAX_BYTES: u32 = 600;
    const KEEP: u32 = 3;

    let mut child = StdioMcp::new(data.path(), MAX_BYTES, KEEP).await;
    child.handshake().await;
    for i in 0..10 {
        child
            .call_tool(
                "mem_record_observation",
                json!({
                    "project": "proj", "agent_harness": "opencode", "task": format!("task-{i}"),
                    "obs_type": "discovery", "title": format!("redaction obs {i}"),
                    "narrative": "the api_key is sk-live-secret-abcdef and it must never land on disk",
                    "facts": "f", "files": "big.rs",
                }),
            )
            .await;
    }
    assert!(child.shutdown().await);

    let secret = "sk-live-secret-abcdef";
    let mut files = vec![audit_path.to_path_buf()];
    for i in 1..=KEEP {
        files.push(PathBuf::from(format!("{}.{}", audit_path.display(), i)));
    }
    for f in &files {
        if !f.is_file() {
            continue;
        }
        let content = std::fs::read_to_string(f).unwrap();
        assert!(
            !content.contains(secret),
            "redaction must hold across rotated files (Req 7.2): {}",
            f.display()
        );
    }
}
