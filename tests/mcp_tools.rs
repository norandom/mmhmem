//! MCP tool-surface E2E tests (tasks 10.2/10.3 core, extended in 11.1): real
//! axum+rmcp server on an ephemeral port, full initialize -> tools/list ->
//! tools/call flow over Streamable HTTP (SSE responses parsed from `data:`).

mod routing_metadata;

use std::collections::HashMap;
use std::sync::Arc;

use mmhmem::config::{Config, ENV_ALLOWED_ROOTS, ENV_DATA_DIR, ENV_LOG_FILE, ENV_TOKEN};
use mmhmem::mcp::server::{AppState, router};

fn test_state(token: &str) -> (tempfile::TempDir, Arc<AppState>) {
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
    (roots, state)
}

/// Extracts the JSON payload(s) from an SSE response body (`data: {...}` lines).
fn sse_json(body: &str) -> Vec<serde_json::Value> {
    body.lines()
        .filter_map(|l| l.strip_prefix("data: "))
        .filter_map(|d| serde_json::from_str(d).ok())
        .collect()
}

/// One MCP request with the bearer token + session id; returns (status, body, session_id).
async fn mcp_call(
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

#[tokio::test]
async fn tools_list_exposes_the_six_tools_and_call_round_trips_memory() {
    let (_roots, state) = test_state("secret-token");
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let client = reqwest::Client::new();
    let base = format!("http://127.0.0.1:{port}");
    let mut session = None;

    // initialize
    let init = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"t","version":"0"}}}"#;
    let (status, body) = mcp_call(&client, &base, "secret-token", &mut session, init).await;
    assert_eq!(status, 200);
    assert!(body.contains("serverInfo"));
    // Req 1.6 (amendment 2026-11-02): the initialize response must advertise
    // the tools capability for capability-negotiating clients.
    assert!(
        body.contains("\"tools\""),
        "tools capability not advertised: {body}"
    );
    // initialized notification (no id)
    let (status, _) = mcp_call(
        &client,
        &base,
        "secret-token",
        &mut session,
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
    )
    .await;
    assert!(status == 200 || status == 202, "status={status}");

    // tools/list
    let (status, body) = mcp_call(
        &client,
        &base,
        "secret-token",
        &mut session,
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}"#,
    )
    .await;
    assert_eq!(status, 200);
    let msgs = sse_json(&body);
    let list = &msgs[0];
    routing_metadata::assert_metadata(&list["result"]["tools"]);
    let names: Vec<&str> = list["result"]["tools"]
        .as_array()
        .unwrap()
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
    ] {
        assert!(
            names.contains(&expected),
            "missing tool {expected}: {names:?}"
        );
    }

    // tools/call mem_record_observation then mem_search (round trip)
    let record = r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"mem_record_observation","arguments":{"project":"p","agent_harness":"opencode","obs_type":"discovery","title":"e2e obs","narrative":"recorded over MCP","facts":"f","files":"src/x.rs"}}}"#;
    let (status, body) = mcp_call(&client, &base, "secret-token", &mut session, record).await;
    assert_eq!(status, 200, "body={body}");
    assert!(
        sse_json(&body)[0]["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("#1")
    );

    let search = r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"mem_search","arguments":{"project":"p","agent_harness":"opencode","query":"e2e"}}}"#;
    let (status, body) = mcp_call(&client, &base, "secret-token", &mut session, search).await;
    assert_eq!(status, 200);
    let msgs = sse_json(&body);
    let text = msgs[0]["result"]["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("e2e obs"), "text={text}");
    assert!(text.contains("| ID |"), "index table expected: {text}");

    // tools/call smart_read on a missing file -> actionable tool error, not a crash
    let read = r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"smart_read","arguments":{"project":"p","agent_harness":"opencode","file_path":"nope.rs","question":"q"}}}"#;
    let (status, body) = mcp_call(&client, &base, "secret-token", &mut session, read).await;
    assert_eq!(status, 200);
    // Tool errors surface as a JSON-RPC error with an actionable message.
    assert!(body.contains("file not found"), "body={body}");
}
