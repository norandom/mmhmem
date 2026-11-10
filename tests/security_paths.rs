//! Task 11.6: security, concurrency, and performance tests over MCP
//! (Req 1.2, 6.3, 9.1, 9.2, path confinement, reference-outside-roots).

mod common;

use common::{mcp_init, mcp_tool, spawn_server, test_state, tool_text};

#[tokio::test]
async fn traversal_and_outside_roots_are_rejected_over_mcp() {
    let (roots, _d, logs, state) = test_state("secret-token");
    std::fs::write(roots.path().join("ok.rs"), "fn ok() {}\n").unwrap();
    let (port, _h) = spawn_server(state).await;
    let client = reqwest::Client::new();
    let base = format!("http://127.0.0.1:{port}");
    let mut session = mcp_init(&client, &base, "secret-token").await;

    // smart_read with a traversal path -> actionable error, audited.
    let trav = roots
        .path()
        .join("inner")
        .join("..")
        .join("..")
        .join("secret.txt");
    let args = serde_json::json!({
        "project": "p", "agent_harness": "opencode",
        "file_path": trav.display().to_string(), "question": "q",
    });
    let (_s, body) = mcp_tool(
        &client,
        &base,
        "secret-token",
        &mut session,
        20,
        "smart_read",
        &args.to_string(),
    )
    .await;
    assert!(
        body.contains("outside AGENT_MEM_ALLOWED_ROOTS") || body.contains("file not found"),
        "body={body}"
    );

    // boilerplate_write with an out-of-roots reference -> rejected BEFORE any
    // worker call (no worker configured here; a worker error would prove wrong
    // ordering — we must see the confinement error instead).
    let outside = tempfile::tempdir().unwrap();
    let secret_ref = outside.path().join("secret.rs");
    std::fs::write(&secret_ref, "top secret\n").unwrap();
    let target = roots.path().join("gen.rs");
    let args = serde_json::json!({
        "project": "p", "agent_harness": "opencode",
        "target_path": target.display().to_string(),
        "reference_path": secret_ref.display().to_string(), "spec": "s",
    });
    let (_s, body) = mcp_tool(
        &client,
        &base,
        "secret-token",
        &mut session,
        21,
        "boilerplate_write",
        &args.to_string(),
    )
    .await;
    assert!(
        body.contains("outside AGENT_MEM_ALLOWED_ROOTS"),
        "body={body}"
    );
    assert!(!target.exists(), "nothing written");
    let log = std::fs::read_to_string(logs.path().join("audit.jsonl")).unwrap();
    assert!(log.contains("reference_outside_allowed_roots"), "log={log}");
    assert!(
        !log.contains("top secret"),
        "secret content must never be logged"
    );
}

#[tokio::test]
async fn concurrent_harness_checkpoints_do_not_corrupt_state() {
    let (_r, _d, _l, state) = test_state("secret-token");
    let (port, _h) = spawn_server(state).await;
    let base = format!("http://127.0.0.1:{port}");
    let client = reqwest::Client::new();

    let mut handles = Vec::new();
    for i in 0..16 {
        let client = client.clone();
        let base = base.clone();
        handles.push(tokio::spawn(async move {
            let mut session = mcp_init(&client, &base, "secret-token").await;
            let harness = format!("harness-{i}");
            let args = serde_json::json!({
                "project": "p", "agent_harness": harness, "workspace_task_id": "t",
                "active_objective": format!("obj-{i}"),
                "updated_working_state_md": format!("state from {i}"),
            });
            let (status, body) = mcp_tool(
                &client,
                &base,
                "secret-token",
                &mut session,
                1,
                "session_resume_or_handoff",
                &args.to_string(),
            )
            .await;
            assert_eq!(status, 200, "body={body}");
            assert!(tool_text(&body).contains("Checkpoint saved"), "body={body}");
        }));
    }
    for h in handles {
        h.await.unwrap();
    }

    // Final resume sees a consistent checkpoint (one of the 16, intact).
    let mut session = mcp_init(&client, &base, "secret-token").await;
    let args = serde_json::json!({"project": "p", "agent_harness": "reader"});
    let (_s, body) = mcp_tool(
        &client,
        &base,
        "secret-token",
        &mut session,
        99,
        "session_resume_or_handoff",
        &args.to_string(),
    )
    .await;
    let text = tool_text(&body);
    assert!(text.contains("state from "), "text={text}");
    assert!(text.contains("obj-"), "text={text}");
}

#[tokio::test]
async fn large_file_reaches_shunt_path_over_mcp() {
    let (roots, _d, _l, state) = test_state("secret-token");
    let big = roots.path().join("big.rs");
    // Above the shipped threshold (shunt-cache-efficiency task 4).
    let lines = mmhmem::config::DEFAULT_SHUNT_MIN_LINES + 50;
    let content: String = (1..=lines).map(|i| format!("line {i}\n")).collect();
    std::fs::write(&big, content).unwrap();
    let (port, _h) = spawn_server(state).await;
    let client = reqwest::Client::new();
    let base = format!("http://127.0.0.1:{port}");
    let mut session = mcp_init(&client, &base, "secret-token").await;

    // No worker configured: a large file must reach the shunt path and error
    // clearly (WorkerNotConfigured), not be returned raw.
    let args = serde_json::json!({
        "project": "p", "agent_harness": "opencode",
        "file_path": big.display().to_string(), "question": "q",
    });
    let (_s, body) = mcp_tool(
        &client,
        &base,
        "secret-token",
        &mut session,
        30,
        "smart_read",
        &args.to_string(),
    )
    .await;
    assert!(body.contains("WORKER_MODEL"), "body={body}");
}
