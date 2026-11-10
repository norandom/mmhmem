pub fn assert_metadata(tools: &serde_json::Value) {
    let tools = tools.as_array().expect("tools array");
    // shunt-content-tool Req 1.1/6.1: the content-based tool made the
    // count eleven (the spec's "nine → ten" wording predates the status
    // tool, reliable-installers Req 6.1); the 2026-11-10 restore removed
    // the batch summary tool, leaving ten.
    assert_eq!(tools.len(), 10);
    for tool in tools {
        let name = tool["name"].as_str().unwrap();
        if name == "status" {
            assert_status_metadata(tool);
            continue;
        }
        let destructive = matches!(
            name,
            "session_resume_or_handoff"
                | "smart_read"
                | "boilerplate_write"
                | "shunt_prune"
                | "shunt_content"
        );
        assert_eq!(tool["annotations"]["readOnlyHint"], false, "{name}");
        assert_eq!(
            tool["annotations"]["destructiveHint"], destructive,
            "{name}"
        );
        assert_eq!(tool["annotations"]["idempotentHint"], false, "{name}");
        assert_eq!(tool["annotations"]["openWorldHint"], true, "{name}");
        assert!(tool["inputSchema"]["properties"]["project"].is_object());
        assert!(tool["inputSchema"]["properties"]["agent_harness"].is_object());
    }
    assert_smart_read_metadata(tools.iter().find(|t| t["name"] == "smart_read").unwrap());
    // shunt-content-tool Req 2.4: the content tool's description states it
    // is for programmatic consumers with content in hand, that summaries
    // are not exact evidence, and that cache, audit and statistics are
    // written.
    let sc = tools.iter().find(|t| t["name"] == "shunt_content").unwrap()["description"]
        .as_str()
        .unwrap();
    for phrase in [
        "programmatic consumers",
        "not exact source evidence",
        "Cache, audit and statistics are written",
    ] {
        assert!(sc.contains(phrase), "shunt_content missing {phrase}");
    }
}

/// shunt-cache-efficiency Req 2.4: omit the question for the shared
/// summary; ask one with the optional `answer_question` only when the
/// summary is not enough.
fn assert_smart_read_metadata(smart_read: &serde_json::Value) {
    let read = smart_read["description"].as_str().unwrap();
    for phrase in [
        "ordinary repository inspection",
        "before a full native or shell read",
        "exact",
        "Omit `question` to get the shared structural summary (cached per file content for every agent).",
        "Pass `question` with `answer_question: true` only when the summary is not enough.",
    ] {
        assert!(read.contains(phrase), "missing {phrase}");
    }
    assert!(!read.contains("including question"), "stale cache wording");
    let answer = &smart_read["inputSchema"]["properties"]["answer_question"];
    assert_eq!(answer["type"], "boolean", "answer_question param: {answer}");
    let required = smart_read["inputSchema"]["required"].as_array();
    let required: Vec<&str> = required
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str())
        .collect();
    assert!(
        !required.contains(&"answer_question"),
        "optional: {required:?}"
    );
    assert!(!required.contains(&"question"), "optional: {required:?}");
}

/// Installer health check (reliable-installers Req 6.1): read-only hints.
fn assert_status_metadata(tool: &serde_json::Value) {
    assert_eq!(tool["annotations"]["readOnlyHint"], true);
    assert_eq!(tool["annotations"]["destructiveHint"], false);
    assert_eq!(tool["annotations"]["idempotentHint"], true);
    assert_eq!(tool["annotations"]["openWorldHint"], false);
    assert!(tool["inputSchema"]["properties"]["project"].is_object());
    assert!(tool["inputSchema"]["properties"]["agent_harness"].is_object());
}
