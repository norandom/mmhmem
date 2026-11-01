//! Shared integration-test harness (design.md "File Structure Plan"
//! tests/common/mod.rs): ephemeral-port server, env-only config in tempdirs,
//! raw MCP-over-HTTP client helpers with SSE parsing.

use std::collections::HashMap;
use std::sync::Arc;

use mmhmem::config::{Config, ENV_ALLOWED_ROOTS, ENV_DATA_DIR, ENV_LOG_FILE, ENV_TOKEN};
use mmhmem::mcp::server::{router, AppState};

/// Builds an AppState with everything rooted in fresh tempdirs.
/// Returns (roots, data, logs, state).
pub fn test_state(
    token: &str,
) -> (
    tempfile::TempDir,
    tempfile::TempDir,
    tempfile::TempDir,
    Arc<AppState>,
) {
    let roots = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    let logs = tempfile::tempdir().unwrap();
    let mut env: HashMap<String, String> = HashMap::new();
    env.insert(ENV_TOKEN.to_string(), token.to_string());
    env.insert(
        ENV_ALLOWED_ROOTS.to_string(),
        roots.path().display().to_string(),
    );
    env.insert(ENV_DATA_DIR.to_string(), data.path().display().to_string());
    env.insert(
        ENV_LOG_FILE.to_string(),
        logs.path().join("audit.jsonl").display().to_string(),
    );
    let config = Config::from_env(&|k| env.get(k).cloned()).unwrap();
    let state = Arc::new(AppState::build(config).unwrap());
    (roots, data, logs, state)
}

/// Spawns the axum+rmcp server on an ephemeral port; returns (port, join handle).
pub async fn spawn_server(state: Arc<AppState>) -> (u16, tokio::task::JoinHandle<()>) {
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (port, handle)
}

/// Extracts JSON payloads from an SSE response body (`data: {...}` lines).
pub fn sse_json(body: &str) -> Vec<serde_json::Value> {
    body.lines()
        .filter_map(|l| l.strip_prefix("data: "))
        .filter_map(|d| serde_json::from_str(d).ok())
        .collect()
}

/// One MCP request with bearer token + session id.
/// Returns (status, body); updates `session` from the mcp-session-id header.
pub async fn mcp_call(
    client: &reqwest::Client,
    base: &str,
    token: &str,
    session: &mut Option<String>,
    body: &str,
) -> (u16, String) {
    let mut req = client
        .post(format!("{base}/mcp"))
        .header("Authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .body(body.to_string());
    if let Some(s) = session.as_ref() {
        req = req.header("mcp-session-id", s);
    }
    let resp = req.send().await.unwrap();
    if let Some(s) = resp
        .headers()
        .get("mcp-session-id")
        .and_then(|v| v.to_str().ok())
    {
        *session = Some(s.to_string());
    }
    let status = resp.status().as_u16();
    let text = resp.text().await.unwrap();
    (status, text)
}

/// A full MCP session handshake: initialize + notifications/initialized.
/// Returns the session id.
pub async fn mcp_init(client: &reqwest::Client, base: &str, token: &str) -> Option<String> {
    let mut session = None;
    let init = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"t","version":"0"}}}"#;
    let (status, body) = mcp_call(client, base, token, &mut session, init).await;
    assert_eq!(status, 200, "body={body}");
    assert!(body.contains("serverInfo"), "body={body}");
    let (_status, _body) = mcp_call(
        client,
        base,
        token,
        &mut session,
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
    )
    .await;
    session
}

/// Calls a tool; returns the response body string.
pub async fn mcp_tool(
    client: &reqwest::Client,
    base: &str,
    token: &str,
    session: &mut Option<String>,
    id: i64,
    name: &str,
    arguments: &str,
) -> (u16, String) {
    let body = format!(
        r#"{{"jsonrpc":"2.0","id":{id},"method":"tools/call","params":{{"name":"{name}","arguments":{arguments}}}}}"#
    );
    mcp_call(client, base, token, session, &body).await
}

/// Extracts the text content of a tools/call result from an SSE or plain-JSON body.
pub fn tool_text(body: &str) -> String {
    let msgs = sse_json(body);
    let msg = if msgs.is_empty() {
        // Plain JSON response (no SSE framing).
        serde_json::from_str::<serde_json::Value>(body.trim())
            .unwrap_or_else(|_| panic!("unparseable response body: {body}"))
    } else {
        msgs.into_iter().next().unwrap()
    };
    msg["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or("")
        .to_string()
}
