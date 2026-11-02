//! Task 10.4 smoke test: the real binary boots with env-only config, serves
//! /healthz (200) and the MCP endpoint (401 without the bearer token, 200
//! initialize with it), writes audit lines, and is killed cleanly.

use std::process::{Child, Command, Stdio};
use std::time::Duration;

/// Kills the child on drop so a failed assert can never leak a daemon process
/// (a leaked process holds target\debug\mmhmem.exe and blocks later builds).
struct KillOnDrop(Child);

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[tokio::test]
async fn daemon_boots_serves_and_audits() {
    let roots = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    let logs = tempfile::tempdir().unwrap();
    // Find a free port (bind 0, drop, reuse).
    let port = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let log_file = logs.path().join("audit.jsonl");
    let child = Command::new(env!("CARGO_BIN_EXE_mmhmem"))
        .env("AGENT_MEM_TOKEN", "smoke-token")
        .env("AGENT_MEM_ALLOWED_ROOTS", roots.path())
        .env("AGENT_MEM_DATA_DIR", data.path())
        .env("AGENT_MEM_LOG_FILE", &log_file)
        .env("AGENT_MEM_HOST", "127.0.0.1")
        .env("AGENT_MEM_PORT", port.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let child = KillOnDrop(child);

    let client = reqwest::Client::new();
    let base = format!("http://127.0.0.1:{port}");
    let mut up = false;
    for _ in 0..100 {
        if let Ok(r) = client
            .get(format!("{base}/healthz"))
            .timeout(Duration::from_millis(200))
            .send()
            .await
            && r.status() == 200
        {
            up = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(up, "daemon did not come up on port {port}");

    // MCP without token -> 401; with token -> initialize 200.
    let init_body = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"t","version":"0"}}}"#;
    let no_auth = client
        .post(format!("{base}/mcp"))
        .header("content-type", "application/json")
        .body(init_body)
        .send()
        .await
        .unwrap();
    assert_eq!(no_auth.status(), 401);
    let with_auth = client
        .post(format!("{base}/mcp"))
        .header("Authorization", "Bearer smoke-token")
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .body(init_body)
        .send()
        .await
        .unwrap();
    assert_eq!(with_auth.status(), 200);
    assert!(with_auth.text().await.unwrap().contains("mmhmem"));

    // Audit trail got the AUTH_FAILURE line.
    let log = std::fs::read_to_string(&log_file).unwrap();
    assert!(log.contains("AUTH_FAILURE"), "log={log}");
    assert!(!log.contains("smoke-token"));

    drop(child); // KillOnDrop kills + reaps
}
