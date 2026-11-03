pub fn assert_metadata(tools: &serde_json::Value) {
    let tools = tools.as_array().expect("tools array");
    assert_eq!(tools.len(), 9);
    for tool in tools {
        let name = tool["name"].as_str().unwrap();
        let destructive = matches!(
            name,
            "session_resume_or_handoff"
                | "smart_read"
                | "boilerplate_write"
                | "shunt_warmup"
                | "shunt_prune"
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
}
