//! Unit tests for `stats.rs` (local-mode Req 8; client-usage-stats Req 1, 2, 4, 5).
use super::*;
use crate::services::session_bus::ClientUsage;
use crate::services::shunt_types::CallCtx;
use crate::services::stats::StatsCollector;
use crate::storage::audit::AuditWriter;
use crate::storage::db::Db;
use std::path::Path;

/// 2025-01-01T00:00:00Z and 2025-01-02T00:00:00Z in epoch ms.
const DAY1: i64 = 1_735_689_600_000;
const DAY1_LATER: i64 = 1_735_690_000_000;
const DAY2: i64 = 1_735_776_000_000;

struct SeedRow {
    ts: i64,
    project: &'static str,
    harness: &'static str,
    kind: &'static str,
    avoided_est: i64,
    worker_prompt: Option<i64>,
    worker_completion: Option<i64>,
}

async fn seed_db(dir: &Path, rows: Vec<SeedRow>) -> Db {
    let db = Db::open(dir).unwrap();
    db.with_conn(move |conn| {
        for r in rows {
            conn.execute(
                "INSERT INTO token_stats \
                 (project, agent_harness, kind, ts_epoch_ms, \
                  avoided_input_tokens_est, avoided_output_tokens_est, \
                  worker_prompt_tokens, worker_completion_tokens) \
                 VALUES (?1, ?2, ?3, ?4, ?5, 0, ?6, ?7)",
                params![
                    r.project,
                    r.harness,
                    r.kind,
                    r.ts,
                    r.avoided_est,
                    r.worker_prompt,
                    r.worker_completion
                ],
            )?;
        }
        Ok(())
    })
    .await
    .unwrap();
    db
}

fn row(
    ts: i64,
    project: &'static str,
    harness: &'static str,
    kind: &'static str,
    avoided_est: i64,
    wp: Option<i64>,
    wc: Option<i64>,
) -> SeedRow {
    SeedRow {
        ts,
        project,
        harness,
        kind,
        avoided_est,
        worker_prompt: wp,
        worker_completion: wc,
    }
}

/// Estimate rows only; client rows go through the production path.
fn sample_rows() -> Vec<SeedRow> {
    vec![
        row(
            DAY1,
            "mmhmem",
            "opencode",
            "shunt_delegated",
            400,
            Some(50),
            Some(10),
        ),
        row(
            DAY2,
            "mmhmem",
            "opencode",
            "shunt_cache_hit",
            100,
            None,
            None,
        ),
        row(
            DAY2,
            "other",
            "claude-code",
            "code_write",
            900,
            Some(20),
            Some(30),
        ),
    ]
}

/// Writes one client report exactly as the server does, then moves it to
/// `ts` (the collector stamps the current time).
async fn report_usage(db: &Db, dir: &Path, ts: i64, prompt: u64, completion: u64) {
    let collector = StatsCollector::new(db.clone(), AuditWriter::new(dir.join("audit.jsonl")));
    let ctx = CallCtx {
        project: "mmhmem",
        harness: "opencode",
        task: Some("t1"),
    };
    let usage = ClientUsage {
        prompt_tokens: prompt,
        completion_tokens: completion,
        source: "opencode /status".to_string(),
    };
    collector.record_client_usage(&ctx, &usage).await.unwrap();
    db.with_conn(move |c| {
        c.execute(
            "UPDATE token_stats SET ts_epoch_ms = ?1 WHERE id = (SELECT MAX(id) FROM token_stats)",
            params![ts],
        )
    })
    .await
    .unwrap();
}

fn day(day: &str, project: &str, harness: &str, est: i64, wp: i64, wc: i64) -> DayAggregate {
    DayAggregate {
        day: day.into(),
        project: project.into(),
        harness: harness.into(),
        avoided_est: est,
        client_prompt: 0,
        client_completion: 0,
        client_sources: Vec::new(),
        worker_prompt: wp,
        worker_completion: wc,
        activity: ActivityCounts::default(),
    }
}

fn expected_aggregates() -> Vec<DayAggregate> {
    let mut day1 = day("2025-01-01", "mmhmem", "opencode", 400, 50, 10);
    day1.client_prompt = 700;
    day1.client_completion = 70;
    day1.client_sources = vec!["opencode /status".to_string()];
    // day DESC, then project, then harness.
    vec![
        day("2025-01-02", "mmhmem", "opencode", 100, 0, 0),
        day("2025-01-02", "other", "claude-code", 900, 20, 30),
        day1,
    ]
}

async fn sample_db(dir: &Path) -> Db {
    let db = seed_db(dir, sample_rows()).await;
    report_usage(&db, dir, DAY1_LATER, 300, 30).await;
    report_usage(&db, dir, DAY1_LATER + 1, 700, 70).await;
    db
}

#[tokio::test]
async fn aggregate_merges_estimates_with_the_latest_client_report() {
    let dir = tempfile::tempdir().unwrap();
    let db = sample_db(dir.path()).await;
    let got = aggregate(&db).await.unwrap();
    assert_eq!(got, expected_aggregates());
}

#[tokio::test]
async fn estimates_never_include_client_rows() {
    let dir = tempfile::tempdir().unwrap();
    // A client row with a value in the estimate column must not count.
    let db = seed_db(
        dir.path(),
        vec![row(
            DAY1,
            "mmhmem",
            "opencode",
            "client_reported",
            700,
            None,
            None,
        )],
    )
    .await;
    let got = aggregate(&db).await.unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].avoided_est, 0);
    assert_eq!((got[0].client_prompt, got[0].client_completion), (0, 0));
    assert_eq!(got[0].client_sources, vec!["unlabelled".to_string()]);
}

#[tokio::test]
async fn aggregate_on_empty_store_returns_no_rows() {
    let dir = tempfile::tempdir().unwrap();
    let db = seed_db(dir.path(), vec![]).await;
    let got = aggregate(&db).await.unwrap();
    assert!(got.is_empty());
}

#[test]
fn totals_sum_every_column() {
    let rows = expected_aggregates();
    let t = totals(&rows);
    assert_eq!(t.avoided_est, 400 + 100 + 900);
    assert_eq!((t.client_prompt, t.client_completion), (700, 70));
    assert_eq!(t.worker_prompt, 50 + 20);
    assert_eq!(t.worker_completion, 10 + 30);
}

/// `--all` at 2025-01-02, no width limit: every row and column.
fn all_opts() -> TableOptions {
    TableOptions {
        width: None,
        today: time::Date::from_calendar_date(2025, time::Month::January, 2).unwrap(),
        all: true,
    }
}

#[test]
fn table_labels_spent_and_estimated_tokens_with_totals() {
    let table = render_table(&expected_aggregates(), &all_opts());
    let header = table.lines().nth(2).unwrap();
    assert!(header.contains("frontier in (client)"), "{header}");
    assert!(header.contains("frontier out (client)"), "{header}");
    assert!(!header.contains("avoided (client)"), "{header}");
    assert!(header.contains("avoided (est.)"), "estimates stay labeled");
    let total = table.lines().find(|l| l.starts_with("│ total")).unwrap();
    assert!(total.contains("1,400") && total.contains("700") && total.contains("70"));
}

#[test]
fn table_renders_zero_totals_for_empty_input() {
    let table = render_table(&[], &all_opts());
    assert!(table.contains("│ total"));
    assert!(table.contains(" 0 "));
}

#[test]
fn json_matches_the_data_contract() {
    let raw = render_json(&expected_aggregates());
    let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
    let days = v["days"].as_array().unwrap();
    assert_eq!(days.len(), 3);
    for key in [
        "day",
        "project",
        "harness",
        "avoided_input_tokens_est",
        "frontier_prompt_tokens_client_reported",
        "frontier_completion_tokens_client_reported",
        "client_reported_sources",
        "worker_prompt_tokens",
        "worker_completion_tokens",
    ] {
        assert!(
            days[0].get(key).is_some(),
            "days[0] lacks {key}: {}",
            days[0]
        );
    }
    assert!(
        days[0]
            .get("avoided_input_tokens_client_reported")
            .is_none()
    );
    assert_eq!(days[2]["frontier_prompt_tokens_client_reported"], 700);
    assert_eq!(days[2]["frontier_completion_tokens_client_reported"], 70);
    assert_eq!(days[2]["client_reported_sources"][0], "opencode /status");
    let totals = &v["totals"];
    assert_eq!(totals["avoided_input_tokens_est"], 1_400);
    assert_eq!(totals["frontier_prompt_tokens_client_reported"], 700);
    assert_eq!(totals["frontier_completion_tokens_client_reported"], 70);
    assert_eq!(totals["worker_prompt_tokens"], 70);
    assert!(totals.get("avoided_input_tokens_client_reported").is_none());
}
