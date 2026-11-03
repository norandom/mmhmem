//! Opt-in adoption foundations. Actual harness adapters are separate tasks.
#[path = "routing_adoption/mod.rs"]
mod foundation;

#[test]
fn infrastructure_preserves_evidence_boundaries() {
    foundation::verify_foundation();
}

#[test]
fn live_adoption_requires_explicit_opt_in() {
    if std::env::var("E2E_LLM").ok().as_deref() != Some("1") {
        return;
    }
    let report = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(claude::run());
    println!("{}", serde_json::to_string_pretty(&report).unwrap());
    assert!(
        matches!(report.evidence.outcome, foundation::Outcome::Pass),
        "Claude adoption FAIL or NOT_VERIFIED"
    );
    assert_eq!(report.repeat_outcome, "PASS", "cache repeat NOT_VERIFIED");
}
#[test]
fn sanitized_events_and_modes() {
    let events =
        serde_json::json!([{"tool":"smart_read","api_key":"SECRET","conversation":"PRIVATE"}]);
    let tools = foundation::extract_tools(&events).unwrap();
    assert_eq!(tools, vec!["smart_read"]);
    assert!(!serde_json::to_string(&tools).unwrap().contains("SECRET"));
    assert_ne!(
        foundation::Mode::IsolatedCi.label(),
        foundation::Mode::AuthenticatedLocal.label()
    );
    assert!(!foundation::capabilities("--help", &["--format"]));
    assert!(foundation::capabilities(
        "  --model, -m string    Model to use\n  --format choice    Output\n  --standalone    Private server",
        &["--model", "--format", "--standalone"]
    ));
    assert!(!foundation::capabilities("--model-extra, -m", &["--model"]));
}
#[test]
fn bounded_child_is_killed_before_delayed_write() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let fixture = foundation::Fixture::new();
        #[cfg(windows)]
        let (program, args) = (
            "powershell",
            vec![
                "-NoProfile",
                "-Command",
                "Start-Sleep -Seconds 2; Set-Content delayed.txt leaked",
            ],
        );
        #[cfg(not(windows))]
        let (program, args) = ("sh", vec!["-c", "sleep 2; printf leaked > delayed.txt"]);
        let result = foundation::bounded_command(
            program,
            &args,
            &fixture.repository,
            std::time::Duration::from_millis(300),
        )
        .await;
        assert_eq!(result.unwrap_err(), "timeout");
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        assert!(!fixture.repository.join("delayed.txt").exists());
    });
}
#[test]
fn bounded_child_drains_both_streams() {
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let fixture = foundation::Fixture::new();
        #[cfg(windows)]
        let (program, args) = (
            "powershell",
            vec![
                "-NoProfile",
                "-Command",
                "$s='x'*200000; [Console]::Out.Write($s); [Console]::Error.Write($s)",
            ],
        );
        #[cfg(not(windows))]
        let (program, args) = (
            "sh",
            vec![
                "-c",
                "head -c 200000 /dev/zero; head -c 200000 /dev/zero >&2",
            ],
        );
        let output = foundation::bounded_command(
            program,
            &args,
            &fixture.repository,
            std::time::Duration::from_secs(5),
        )
        .await
        .expect("large output must drain");
        assert!(output.status.success());
        assert_eq!(output.stdout.len(), 200000);
        assert_eq!(output.stderr.len(), 200000);
    });
}

#[path = "routing_adoption/claude.rs"]
mod claude;
#[test]
fn claude_events_distinguish_adoption_and_private_payloads() {
    let stream = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"mcp__mmhmem__smart_read","input":{"file_path":"transform.rs","question":"explain","private":"SECRET"}}]}}
{"type":"result","is_error":false}"#;
    let trace = claude::parse(stream).unwrap();
    assert!(claude::adopted(&trace));
    assert!(!serde_json::to_string(&trace).unwrap().contains("SECRET"));
    assert!(claude::parse("{}").is_err());
    let native=claude::parse(r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Read","input":{}},{"type":"tool_use","name":"mcp__mmhmem__smart_read","input":{}}]}}
{"type":"result","is_error":false}"#).unwrap();
    assert!(!claude::adopted(&native));
    assert!(!claude::same_identity(&trace, &native));
}

#[test]
fn claude_rejects_failed_and_targeted_only_adoption() {
    assert!(claude::parse(r#"{"type":"result","is_error":true}"#).is_err());
    let stream = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"mcp__mmhmem__smart_read","input":{"file_path":"transform.rs","question":"one","offset":1,"limit":3}}]}}
{"type":"result","is_error":false}"#;
    let first = claude::parse(stream).unwrap();
    assert!(!claude::adopted(&first));
    let second = claude::parse(&stream.replace("one", "two")).unwrap();
    assert!(!claude::same_identity(&first, &second));
    assert!(claude::parse(r#"{"type":"assistant","message":{"content":[]}}"#).is_err());
}

#[test]
fn claude_worker_error_fallback_is_bounded() {
    let stream = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"mcp__mmhmem__smart_read","input":{}}]}}
{"type":"user","message":{"content":[{"type":"tool_result","is_error":true,"content":"SECRET"}]}}
{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Read","input":{}}]}}
{"type":"result","is_error":false}"#;
    let trace = claude::parse(stream).unwrap();
    assert!(claude::bounded_fallback(&trace));
    assert!(!serde_json::to_string(&trace).unwrap().contains("SECRET"));
}

#[test]
fn claude_failed_or_unintended_read_cannot_pass() {
    let stream = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"mcp__mmhmem__smart_read","input":{"file_path":"transform.rs"}}]}}
{"type":"user","message":{"content":[{"type":"tool_result","is_error":true}]}}
{"type":"result","is_error":false}"#;
    assert!(!claude::adopted(&claude::parse(stream).unwrap()));
    for path in ["other.rs", "", "/other/transform.rs"] {
        let good = stream
            .lines()
            .filter(|l| !l.contains("tool_result"))
            .collect::<Vec<_>>()
            .join("\n")
            .replace("transform.rs", path);
        assert!(!claude::adopted(&claude::parse(&good).unwrap()));
    }
}

#[test]
fn claude_partial_failure_retains_only_allowlisted_trace() {
    let stream = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Read","input":{"secret":"PRIVATE"}}]}}
{"type":"unknown","secret":"SECRET"}"#;
    let (trace, result) = claude::parse_partial(stream);
    assert!(result.is_err());
    assert_eq!(trace.tools, vec!["Read"]);
    let serialized = serde_json::to_string(&trace).unwrap();
    assert!(!serialized.contains("PRIVATE"));
    assert!(!serialized.contains("SECRET"));
}

#[path = "routing_adoption/opencode.rs"]
mod opencode;
#[test]
fn opencode_events_require_successful_fixture_inspection() {
    let good = r#"{"type":"tool_use","part":{"tool":"mmhmem_smart_read","state":{"status":"completed","input":{"file_path":"transform.rs","question":"explain"},"output":"PRIVATE"}}}
{"type":"step_finish","part":{"reason":"stop"}}"#;
    let trace = opencode::parse_partial(good).0;
    assert!(claude::adopted(&trace));
    assert!(!serde_json::to_string(&trace).unwrap().contains("PRIVATE"));
    for bad in [
        good.replace("completed", "error"),
        good.replace("transform.rs", "other.rs"),
        good.replace("mmhmem_smart_read", "read"),
    ] {
        assert!(!claude::adopted(&opencode::parse_partial(&bad).0));
    }
    assert!(opencode::parse_partial("{}").1.is_err());
    assert!(
        opencode::parse_partial(good.lines().next().unwrap())
            .1
            .is_err()
    );
}
#[test]
fn live_opencode_adoption_requires_explicit_opt_in() {
    if std::env::var("E2E_LLM").ok().as_deref() != Some("1") {
        return;
    }
    let report = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(opencode::run());
    println!("{}", serde_json::to_string_pretty(&report).unwrap());
    assert!(
        matches!(report.evidence.outcome, foundation::Outcome::Pass),
        "OpenCode adoption FAIL or NOT_VERIFIED"
    );
    assert_eq!(
        report.repeat_outcome, "PASS",
        "OpenCode cache repeat NOT_VERIFIED"
    );
}

#[test]
fn opencode_partial_errors_targeted_and_identity_are_honest() {
    let good = r#"{"type":"tool_use","part":{"tool":"mmhmem_smart_read","state":{"status":"completed","input":{"file_path":"transform.rs","question":"one"}}}}
{"type":"step_finish","part":{"reason":"stop"}}"#;
    let first = opencode::parse_partial(good).0;
    let second = opencode::parse_partial(&good.replace("one", "two")).0;
    assert!(!claude::same_identity(&first, &second));
    let targeted = good.replace("\"question\":\"one\"", "\"offset\":1,\"limit\":2");
    assert!(!claude::adopted(&opencode::parse_partial(&targeted).0));
    let partial =
        good.lines().next().unwrap().to_owned() + "\n{\"type\":\"error\",\"secret\":\"PRIVATE\"}";
    let (trace, result) = opencode::parse_partial(&partial);
    assert!(result.is_err());
    assert_eq!(trace.tools.len(), 1);
    assert!(!serde_json::to_string(&trace).unwrap().contains("PRIVATE"));
    let native =
        r#"{"type":"tool_use","part":{"tool":"read","state":{"status":"completed","input":{}}}}"#;
    assert!(!claude::adopted(
        &opencode::parse_partial(&(native.to_owned() + "\n" + good)).0
    ));
}
