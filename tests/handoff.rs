//! Task 11.1: session handoff E2E over real MCP HTTP (Req 2.1-2.6, 1.2):
//! harness A resumes (new task), works (mem_record + smart_read), checkpoints;
//! harness B resumes and inherits state + breadcrumbs with zero LLM calls.

mod common;

use common::{mcp_init, mcp_tool, spawn_server, test_state, tool_text};

#[tokio::test]
async fn full_cross_agent_handoff_flow_over_mcp() {
    let (roots, _data, logs, state) = test_state("secret-token");
    std::fs::write(roots.path().join("small.rs"), "fn a() {}\nfn b() {}\n").unwrap();
    let (port, _h) = spawn_server(state).await;
    let client = reqwest::Client::new();
    let base = format!("http://127.0.0.1:{port}");
    let mut session = mcp_init(&client, &base, "secret-token").await;

    // Harness A: resume with no prior session -> new task id.
    let args = serde_json::json!({"project": "proj", "agent_harness": "claude-code"});
    let (status, body) = mcp_tool(
        &client,
        &base,
        "secret-token",
        &mut session,
        10,
        "session_resume_or_handoff",
        &args.to_string(),
    )
    .await;
    assert_eq!(status, 200, "body={body}");
    let text = tool_text(&body);
    assert!(
        text.contains("No active cross-agent session"),
        "text={text}"
    );
    let task_id = text.split("task ID: ").nth(1).unwrap().trim().to_string();

    // Harness A works: record an observation + read a file.
    let args = serde_json::json!({
        "project": "proj", "agent_harness": "claude-code", "task": task_id,
        "obs_type": "discovery", "title": "found it", "narrative": "saw the thing",
        "facts": "f", "files": "small.rs",
    });
    let (_s, body) = mcp_tool(
        &client,
        &base,
        "secret-token",
        &mut session,
        11,
        "mem_record_observation",
        &args.to_string(),
    )
    .await;
    assert!(tool_text(&body).contains("#1"), "body={body}");
    let args = serde_json::json!({
        "project": "proj", "agent_harness": "claude-code",
        "file_path": roots.path().join("small.rs").display().to_string(), "question": "q",
    });
    let (_s, body) = mcp_tool(
        &client,
        &base,
        "secret-token",
        &mut session,
        12,
        "smart_read",
        &args.to_string(),
    )
    .await;
    assert!(tool_text(&body).contains("L1: fn a()"), "body={body}");

    // Harness A checkpoints.
    let args = serde_json::json!({
        "project": "proj", "agent_harness": "claude-code", "workspace_task_id": task_id,
        "active_objective": "ship it", "updated_working_state_md": "working state: halfway there",
    });
    let (_s, body) = mcp_tool(
        &client,
        &base,
        "secret-token",
        &mut session,
        13,
        "session_resume_or_handoff",
        &args.to_string(),
    )
    .await;
    assert!(tool_text(&body).contains("Checkpoint saved"), "body={body}");

    // Harness B resumes: inherits state + breadcrumbs.
    let args = serde_json::json!({"project": "proj", "agent_harness": "opencode"});
    let (_s, body) = mcp_tool(
        &client,
        &base,
        "secret-token",
        &mut session,
        14,
        "session_resume_or_handoff",
        &args.to_string(),
    )
    .await;
    let text = tool_text(&body);
    assert!(text.contains(&task_id), "task id inherited: {text}");
    assert!(text.contains("claude-code"), "last harness: {text}");
    assert!(text.contains("ship it"), "objective: {text}");
    assert!(text.contains("halfway there"), "working state: {text}");
    assert!(text.contains("smart_read"), "breadcrumbs: {text}");
    assert!(
        text.contains("mem_record_observation"),
        "breadcrumbs: {text}"
    );

    // Zero LLM calls on handoff: no worker is configured in this test and
    // resume/checkpoint succeeded — proving no worker dependency (Req 2.6, 9.4).

    // Audit trail recorded both events; every line hash-verified shape.
    let log = std::fs::read_to_string(logs.path().join("audit.jsonl")).unwrap();
    assert!(log.contains("SESSION_RESUME"));
    assert!(log.contains("SESSION_CHECKPOINT"));
    for line in log.lines() {
        let v: serde_json::Value = serde_json::from_str(line).unwrap();
        assert!(v["record_sha256"].is_string());
    }
}
