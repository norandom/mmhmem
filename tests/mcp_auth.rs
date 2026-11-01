//! Task 10.1 integration tests: bearer auth (Req 1.2), /healthz, MCP handshake
//! (Req 1.1), AUTH_FAILURE auditing. Spins the real axum+rmcp server on an
//! ephemeral port.

use std::collections::HashMap;
use std::sync::Arc;

use mmhmem::config::{Config, ENV_ALLOWED_ROOTS, ENV_DATA_DIR, ENV_LOG_FILE, ENV_TOKEN};
use mmhmem::mcp::server::{router, AppState};

fn test_state(token: &str) -> (tempfile::TempDir, tempfile::TempDir, Arc<AppState>) {
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
    (logs, roots, state)
}

async fn spawn_server(state: Arc<AppState>) -> (u16, tokio::task::JoinHandle<()>) {
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (port, handle)
}

const INIT: &str = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2026-07-28","capabilities":{},"clientInfo":{"name":"test","version":"0.1"}}}"#;

#[tokio::test]
async fn missing_or_wrong_token_is_401_and_audited() {
    let (logs, _r, state) = test_state("secret-token");
    let (port, _h) = spawn_server(state).await;
    let client = reqwest::Client::new();
    let url = format!("http://127.0.0.1:{port}/mcp");

    let no_auth = client.post(&url).body(INIT).send().await.unwrap();
    assert_eq!(no_auth.status(), 401);
    let wrong = client
        .post(&url)
        .header("Authorization", "Bearer wrong")
        .body(INIT)
        .send()
        .await
        .unwrap();
    assert_eq!(wrong.status(), 401);

    let log = std::fs::read_to_string(logs.path().join("audit.jsonl")).unwrap();
    let failures = log.matches("AUTH_FAILURE").count();
    assert_eq!(failures, 2, "log={log}");
    assert!(!log.contains("secret-token"), "token must never be logged");
}

#[tokio::test]
async fn healthz_is_open_and_mcp_handshake_works_with_valid_token() {
    let (_logs, _r, state) = test_state("secret-token");
    let (port, _h) = spawn_server(state).await;
    let client = reqwest::Client::new();
    let base = format!("http://127.0.0.1:{port}");

    let health = client.get(format!("{base}/healthz")).send().await.unwrap();
    assert_eq!(health.status(), 200);
    assert_eq!(health.text().await.unwrap(), "ok");

    let init = client
        .post(format!("{base}/mcp"))
        .header("Authorization", "Bearer secret-token")
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .body(INIT)
        .send()
        .await
        .unwrap();
    assert_eq!(
        init.status(),
        200,
        "initialize must succeed with valid bearer"
    );
    let body = init.text().await.unwrap();
    assert!(body.contains("serverInfo"), "body={body}");
    assert!(body.contains("mmhmem"), "body={body}");
}
