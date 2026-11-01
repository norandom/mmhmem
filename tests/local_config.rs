//! Local-configuration integration tests (task 5.2; Req 2.5, 2.6, 3.1-3.4,
//! 12.4, 12.5): exercise the local profile end to end against the real
//! binary — per-OS default paths, overrides, optional confinement with
//! audited denials, redacted config logging, and fail-fast on an unusable
//! data dir. (Self-contained helper: tests/common/mod.rs stays untouched
//! so the pre-existing suite remains byte-identical.)

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::time::timeout;

const INIT: &str = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"t","version":"0"}}}"#;
const INITIALIZED: &str = r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;
const READ_TIMEOUT: Duration = Duration::from_secs(20);

/// A spawned `mmhmem stdio` child with framed-line MCP helpers (same
/// pattern as tests/local_stdio.rs).
struct StdioMcp {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<tokio::process::ChildStdout>,
    next_id: u64,
}

impl StdioMcp {
    fn from_command(mut cmd: Command) -> Self {
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

    async fn shutdown(mut self) -> (bool, String) {
        drop(self.stdin);
        let status = timeout(READ_TIMEOUT, self.child.wait())
            .await
            .expect("timed out waiting for child exit")
            .expect("child wait failed");
        let mut stderr = String::new();
        if let Some(mut err) = self.child.stderr.take() {
            let _ = err.read_to_string(&mut stderr).await;
        }
        (status.success(), stderr)
    }
}

/// Spawns `mmhmem stdio` with full control over the child's user-location
/// environment (tests never touch the real user directories).
fn spawn_with(data_dir: Option<&Path>, env: &[(&str, String)]) -> StdioMcp {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_mmhmem"));
    cmd.arg("stdio")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    if let Some(dir) = data_dir {
        cmd.env("AGENT_MEM_DATA_DIR", dir);
    } else {
        cmd.env_remove("AGENT_MEM_DATA_DIR");
    }
    for (k, v) in env {
        cmd.env(k, v);
    }
    StdioMcp::from_command(cmd)
}

fn redirected_user_env(base: &Path) -> Vec<(&'static str, String)> {
    vec![
        (
            "APPDATA",
            base.join("appdata").to_string_lossy().into_owned(),
        ),
        (
            "XDG_DATA_HOME",
            base.join("xdg").to_string_lossy().into_owned(),
        ),
        ("HOME", base.join("home").to_string_lossy().into_owned()),
    ]
}

/// The expected per-user store location for the CURRENT operating system,
/// mirroring the config resolver's per-OS defaults (Req 3.1).
fn expected_default_store(base: &Path) -> PathBuf {
    if cfg!(windows) {
        base.join("appdata").join("mmhmem")
    } else if cfg!(target_os = "macos") {
        base.join("home")
            .join("Library")
            .join("Application Support")
            .join("mmhmem")
    } else {
        base.join("xdg").join("mmhmem")
    }
}

fn small_file(dir: &Path, name: &str, marker: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, format!("L1: fn a() {{}}\nL2: {marker}\nL3: done\n")).unwrap();
    path
}

#[tokio::test]
async fn per_user_default_paths_resolve_end_to_end() {
    // Req 3.1 + 2.5: with NO AGENT_MEM_DATA_DIR, the store defaults to the
    // per-user location derived from the OS user environment.
    let base = tempfile::tempdir().unwrap();
    let mut child = spawn_with(None, &redirected_user_env(base.path()));
    child.handshake().await;
    // The store is created at startup (Req 2.6): zero setup, zero config.
    let store = expected_default_store(base.path());
    assert!(
        store.join("memory.db").is_file(),
        "store auto-created at the per-user default: {}",
        store.display()
    );
    let (ok, stderr) = child.shutdown().await;
    assert!(ok, "stderr: {stderr}");
}

#[tokio::test]
async fn config_logging_never_leaks_the_token() {
    // Req 12.4: a (serve-only) token set in the environment must never
    // appear in the local-mode diagnostics.
    let data = tempfile::tempdir().unwrap();
    let token = "super-secret-token-value-42";
    let mut env = vec![("AGENT_MEM_TOKEN", token.to_string())];
    env.extend(redirected_user_env(data.path()));
    let mut child = spawn_with(None, &env);
    child.handshake().await;
    let (ok, stderr) = child.shutdown().await;
    assert!(ok);
    assert!(
        !stderr.contains(token),
        "the token must never reach stderr logs: {stderr}"
    );
}

#[tokio::test]
async fn confinement_enforced_when_roots_set_and_denials_audited() {
    // Req 3.3 + 12.5: configured roots are enforced with audited denials.
    let data = tempfile::tempdir().unwrap();
    let roots = tempfile::tempdir().unwrap();
    let inside = small_file(roots.path(), "inside.rs", "INSIDE_MARKER");
    let outside_dir = tempfile::tempdir().unwrap();
    let outside = small_file(outside_dir.path(), "outside.rs", "OUTSIDE_MARKER");

    let mut env = vec![(
        "AGENT_MEM_ALLOWED_ROOTS",
        roots.path().to_string_lossy().into_owned(),
    )];
    env.extend(redirected_user_env(data.path()));
    let mut child = spawn_with(None, &env);
    child.handshake().await;

    let read_inside = child
        .call_tool(
            "smart_read",
            json!({
                "project": "p", "agent_harness": "opencode",
                "file_path": inside.to_string_lossy(), "question": "q",
            }),
        )
        .await;
    assert!(
        read_inside.contains("INSIDE_MARKER"),
        "reads under the configured root work: {read_inside}"
    );

    let denied = child
        .request(
            "tools/call",
            json!({
                "name": "smart_read",
                "arguments": {
                    "project": "p", "agent_harness": "opencode",
                    "file_path": outside.to_string_lossy(), "question": "q",
                },
            }),
        )
        .await;
    assert!(
        denied.get("error").is_some(),
        "reads outside the roots must be a clear error: {denied}"
    );

    let (ok, _stderr) = child.shutdown().await;
    assert!(ok);

    // Req 12.5: the denial reason is recorded in the audit trail.
    let audit = std::fs::read_to_string(expected_default_store(data.path()).join("audit.jsonl"))
        .expect("audit file exists");
    assert!(
        audit.contains("path_outside_allowed_roots"),
        "denial audited: {audit}"
    );
}

#[tokio::test]
async fn unconfined_reads_when_roots_unset() {
    // Req 3.3: unset roots in local mode mean unconfined — a file outside
    // any root is readable (the spawned child is inside the trust boundary).
    let data = tempfile::tempdir().unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    let file = small_file(elsewhere.path(), "free.rs", "FREE_READ_MARKER");

    let env = redirected_user_env(data.path());
    let mut child = spawn_with(None, &env);
    child.handshake().await;
    let read = child
        .call_tool(
            "smart_read",
            json!({
                "project": "p", "agent_harness": "opencode",
                "file_path": file.to_string_lossy(), "question": "q",
            }),
        )
        .await;
    assert!(
        read.contains("FREE_READ_MARKER"),
        "unconfined local mode reads anywhere in the user session: {read}"
    );
    let (ok, _stderr) = child.shutdown().await;
    assert!(ok);
}

#[tokio::test]
async fn fail_fast_on_unusable_data_dir() {
    // Req 2.5 (fail-fast parity): a data dir that cannot be created is a
    // clear startup error, non-zero exit, no listener, no hang.
    let blockfile = tempfile::tempdir().unwrap();
    let blocker = blockfile.path().join("not-a-dir");
    std::fs::write(&blocker, "x").unwrap();
    let bad_dir = blocker.join("data");

    let mut cmd = Command::new(env!("CARGO_BIN_EXE_mmhmem"));
    cmd.arg("stdio")
        .env("AGENT_MEM_DATA_DIR", &bad_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = cmd.spawn().unwrap();
    drop(child.stdin.take());
    let status = timeout(READ_TIMEOUT, child.wait())
        .await
        .expect("exit in time")
        .expect("child wait failed");
    assert!(
        !status.success(),
        "unusable data dir must exit non-zero, got {status:?}"
    );
    let mut stderr = String::new();
    let _ = child
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut stderr)
        .await;
    assert!(
        stderr.contains("mmhmem stdio"),
        "the error names the entrypoint: {stderr}"
    );
}

#[tokio::test]
async fn data_dir_override_relocates_the_store() {
    // Req 3.4: AGENT_MEM_DATA_DIR wins over the per-user default.
    let data = tempfile::tempdir().unwrap();
    let env = redirected_user_env(data.path());
    let mut child = spawn_with(Some(data.path()), &env);
    child.handshake().await;
    assert!(
        data.path().join("memory.db").is_file(),
        "override wins over the per-user default"
    );
    let (ok, _stderr) = child.shutdown().await;
    assert!(ok);
}
