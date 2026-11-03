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
    panic!("NOT_VERIFIED: live harness adapters are not installed yet");
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
