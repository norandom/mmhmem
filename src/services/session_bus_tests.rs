use super::*;
use crate::storage::audit::AuditWriter;

fn open_bus() -> (tempfile::TempDir, tempfile::TempDir, SessionBus) {
    let db_dir = tempfile::tempdir().unwrap();
    let log_dir = tempfile::tempdir().unwrap();
    let db = Db::open(db_dir.path()).unwrap();
    let audit = AuditWriter::new(log_dir.path().join("audit.jsonl"));
    (db_dir, log_dir, SessionBus::new(db, audit))
}

#[test]
fn truncation_caps_at_last_complete_line() {
    let line = "x".repeat(100);
    let s = format!("{}\n{}\n{}", line, line, "tail".repeat(100));
    let out = truncate_to_budget(&s, 250);
    assert!(out.len() <= 250);
    assert!(out.ends_with(&line), "must cut at a line boundary");
    // No newline within the cap: hard cut at the cap.
    let no_nl = "y".repeat(500);
    assert_eq!(truncate_to_budget(&no_nl, 250).len(), 250);
    // Under the cap: unchanged.
    assert_eq!(truncate_to_budget("short", 6000), "short");
}

#[test]
fn truncation_exact_boundary() {
    let s = "a".repeat(6000);
    assert_eq!(truncate_to_budget(&s, WORKING_STATE_CAP_CHARS).len(), 6000);
    let mut s2 = "a".repeat(5999);
    s2.push('\n');
    s2.push_str(&"b".repeat(100));
    let out = truncate_to_budget(&s2, WORKING_STATE_CAP_CHARS);
    assert_eq!(out.len(), 5999, "newline at 5999 is the last complete line");
}

#[tokio::test]
async fn resume_without_prior_session_initializes_task_id() {
    let (_d, _l, bus) = open_bus();
    let view = bus.resume("proj", "opencode", None).await.unwrap();
    assert!(!view.found_prior_session);
    assert_eq!(view.last_harness, "none");
    assert!(view.task_id.starts_with("proj-"), "{}", view.task_id);
    assert!(view.breadcrumbs.is_empty());
}

#[tokio::test]
async fn checkpoint_then_resume_in_second_harness() {
    let (_d, log_dir, bus) = open_bus();
    let state = "working on it\nsecret api_key=abc123\n";
    let ack = bus
        .checkpoint("proj", "claude-code", "task-1", "obj", state, None)
        .await
        .unwrap();
    assert_eq!(ack.task_id, "task-1");
    assert!(ack.client_usage.is_none());

    let view = bus.resume("proj", "opencode", None).await.unwrap();
    assert!(view.found_prior_session);
    assert_eq!(view.task_id, "task-1");
    assert_eq!(view.last_harness, "claude-code");
    assert_eq!(view.objective, "obj");
    assert!(view.working_state_md.contains("working on it"));
    assert!(
        !view.working_state_md.contains("abc123"),
        "secrets must be redacted from stored state: {}",
        view.working_state_md
    );
    assert!(!view.breadcrumbs.is_empty());
    assert_eq!(view.breadcrumbs[0].tool, "session_handoff");

    // Audit file has SESSION_CHECKPOINT + SESSION_RESUME, hash-verified.
    let raw = std::fs::read_to_string(log_dir.path().join("audit.jsonl")).unwrap();
    let types: Vec<String> = raw
        .lines()
        .map(|l| {
            let v: serde_json::Value = serde_json::from_str(l).unwrap();
            v["event_type"].as_str().unwrap().to_string()
        })
        .collect();
    assert!(types.iter().any(|t| t == "SESSION_CHECKPOINT"));
    assert!(types.iter().any(|t| t == "SESSION_RESUME"));
    for line in raw.lines() {
        let v: serde_json::Value = serde_json::from_str(line).unwrap();
        assert!(v["record_sha256"].is_string());
        assert!(!line.contains("abc123"), "audit leaked secret: {line}");
    }
}

#[tokio::test]
async fn checkpoint_truncates_overlong_state() {
    let (_d, _l, bus) = open_bus();
    let big = "line\n".repeat(2000); // 10_000 chars > 6_000 cap
    let ack = bus
        .checkpoint("p", "h", "t", "o", &big, None)
        .await
        .unwrap();
    assert!(
        ack.stored_chars <= WORKING_STATE_CAP_CHARS,
        "{}",
        ack.stored_chars
    );
    let view = bus.resume("p", "h2", None).await.unwrap();
    assert_eq!(view.working_state_md.len(), ack.stored_chars);
}

#[tokio::test]
async fn resume_returns_at_most_15_breadcrumbs_newest_first() {
    let (_d, _l, bus) = open_bus();
    for i in 0..20 {
        bus.breadcrumb(
            "p",
            "t",
            "h",
            "smart_read",
            Some("f.rs"),
            &format!("call {i}"),
        )
        .await
        .unwrap();
    }
    let view = bus.resume("p", "h", Some("t")).await.unwrap();
    assert_eq!(view.breadcrumbs.len(), 15);
    assert_eq!(view.breadcrumbs[0].summary, "call 19", "newest first");
}

#[tokio::test]
async fn checkpoint_echoes_client_usage_without_persisting_stats() {
    let (_d, _l, bus) = open_bus();
    let usage = ClientUsage {
        prompt_tokens: 1000,
        completion_tokens: 42,
        source: "opencode /status".to_string(),
    };
    let ack = bus
        .checkpoint("p", "h", "t", "o", "state", Some(usage.clone()))
        .await
        .unwrap();
    assert_eq!(ack.client_usage, Some(usage));
}

#[tokio::test]
async fn breadcrumb_failure_preserves_previous_checkpoint() {
    let (_d, _l, bus) = open_bus();
    bus.checkpoint("p", "h", "old", "old objective", "old state", None)
        .await
        .unwrap();
    let before = bus.load_session("p").await.unwrap();
    let count = bus.load_breadcrumbs("p").await.unwrap().len();
    bus.db.with_conn(|c| c.execute_batch("CREATE TRIGGER reject_checkpoint BEFORE INSERT ON session_breadcrumbs WHEN NEW.tool_name = 'session_handoff' BEGIN SELECT RAISE(ABORT, 'fixture failure'); END;")).await.unwrap();
    assert!(
        bus.checkpoint("p", "h", "new", "new objective", "new state", None)
            .await
            .is_err()
    );
    assert_eq!(bus.load_session("p").await.unwrap(), before);
    assert_eq!(bus.load_breadcrumbs("p").await.unwrap().len(), count);
}

#[tokio::test]
async fn audit_failure_preserves_previous_checkpoint_and_breadcrumbs() {
    let (_d, log, bus) = open_bus();
    bus.checkpoint("p", "h", "old", "old objective", "old state", None)
        .await
        .unwrap();
    let before = bus.load_session("p").await.unwrap();
    let count = bus.load_breadcrumbs("p").await.unwrap().len();
    let failed = SessionBus::new(bus.db.clone(), AuditWriter::new(log.path()));
    assert!(
        failed
            .checkpoint("p", "h", "new", "new objective", "new state", None)
            .await
            .is_err()
    );
    assert_eq!(bus.load_session("p").await.unwrap(), before);
    assert_eq!(bus.load_breadcrumbs("p").await.unwrap().len(), count);
}

#[tokio::test]
async fn save_time_matches_row_and_checkpoint_breadcrumb() {
    let (_d, _l, bus) = open_bus();
    let ack = bus
        .checkpoint("p", "h", "t", "o", "state", None)
        .await
        .unwrap();
    let view = bus.resume("p", "h2", None).await.unwrap();
    assert_eq!(view.checkpoint_saved_at_epoch, Some(ack.saved_at_epoch));
    assert_eq!(view.breadcrumbs[0].created_at_epoch, ack.saved_at_epoch);
    assert_eq!(
        bus.load_session("p").await.unwrap().unwrap().4,
        ack.saved_at_epoch
    );
}

#[tokio::test]
async fn history_and_task_initialization_do_not_create_checkpoint() {
    let (_d, _l, bus) = open_bus();
    let first = bus.resume("p", "h", None).await.unwrap();
    assert_eq!(first.checkpoint_saved_at_epoch, None);
    assert!(first.breadcrumbs.is_empty());
    let second = bus.resume("p", "h2", None).await.unwrap();
    assert_eq!(second.checkpoint_saved_at_epoch, None);
    assert!(!second.found_prior_session);
    assert_eq!(second.breadcrumbs.len(), 1);
    assert_eq!(second.breadcrumbs[0].harness, "h");
    assert!(bus.load_session("p").await.unwrap().is_none());
}
