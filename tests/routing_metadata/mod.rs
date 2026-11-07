pub fn assert_metadata(tools: &serde_json::Value) {
    let tools = tools.as_array().expect("tools array");
    // shunt-content-tool Req 1.1/6.1: the content-based tool makes the
    // count eleven (the spec's "nine → ten" wording predates the status
    // tool, reliable-installers Req 6.1, which made it ten).
    assert_eq!(tools.len(), 11);
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
                | "shunt_warmup"
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
    let read = tools.iter().find(|t| t["name"] == "smart_read").unwrap()["description"]
        .as_str()
        .unwrap();
    for phrase in [
        "ordinary repository inspection",
        "before a full native or shell read",
        "exact",
        "same cache identity",
    ] {
        assert!(read.contains(phrase), "missing {phrase}");
    }
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

/// Installer health check (reliable-installers Req 6.1): read-only hints.
fn assert_status_metadata(tool: &serde_json::Value) {
    assert_eq!(tool["annotations"]["readOnlyHint"], true);
    assert_eq!(tool["annotations"]["destructiveHint"], false);
    assert_eq!(tool["annotations"]["idempotentHint"], true);
    assert_eq!(tool["annotations"]["openWorldHint"], false);
    assert!(tool["inputSchema"]["properties"]["project"].is_object());
    assert!(tool["inputSchema"]["properties"]["agent_harness"].is_object());
}
