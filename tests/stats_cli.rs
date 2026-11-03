//! Stats CLI integration tests (local-mode Req 1.3, 8.1-8.6; client-usage-stats
//! Req 1-5): run the REAL binary as a child process against a seeded
//! per-user store — table rendering, `--json`, the empty-store contract, and
//! the read-only guarantee. Client-reported rows are written through the
//! production collector (client-usage-stats Req 5.1); only their timestamps
//! are moved afterwards to place them on fixed days.

use std::path::Path;
use std::process::Command;

use mmhmem::services::session_bus::ClientUsage;
use mmhmem::services::shunt_types::CallCtx;
use mmhmem::services::stats::StatsCollector;
use mmhmem::storage::audit::AuditWriter;
use mmhmem::storage::db::Db;
use rusqlite::params;

/// 2025-01-01T00:00:00Z / 2025-01-02T00:00:00Z in epoch ms.
const DAY1: i64 = 1_735_689_600_000;
const DAY2: i64 = 1_735_776_000_000;

fn run_stats(data_dir: &Path, json: bool) -> std::process::Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_mmhmem"));
    cmd.arg("stats")
        .env("AGENT_MEM_DATA_DIR", data_dir)
        .env_remove("AGENT_MEM_TOKEN")
        .env_remove("AGENT_MEM_ALLOWED_ROOTS")
        // Blank — not removed: removed worker vars fall back to the
        // machine's global .mmhmem.json (armed on dev hosts); blank wins
        // over the file lookup and is treated as missing.
        .env("WORKER_API_URL", "")
        .env("WORKER_MODEL", "")
        .env("WORKER_API_KEY", "");
    if json {
        cmd.arg("--json");
    }
    cmd.output().expect("failed to spawn the mmhmem binary")
}

/// (ts_ms, project, harness, kind, avoided_est, worker_prompt, worker_completion)
type EstimateRow = (
    i64,
    &'static str,
    &'static str,
    &'static str,
    i64,
    Option<i64>,
    Option<i64>,
);

async fn seed_estimates(db: &Db) {
    db.with_conn(|conn| {
        let rows: [EstimateRow; 3] = [
            (
                DAY1,
                "mmhmem",
                "opencode",
                "shunt_delegated",
                400,
                Some(50),
                Some(10),
            ),
            (
                DAY2,
                "mmhmem",
                "opencode",
                "shunt_cache_hit",
                100,
                None,
                None,
            ),
            (
                DAY2,
                "other",
                "claude-code",
                "code_write",
                900,
                Some(20),
                Some(30),
            ),
        ];
        for (ts, project, harness, kind, est, wp, wc) in rows {
            conn.execute(
                "INSERT INTO token_stats \
                 (project, agent_harness, kind, ts_epoch_ms, \
                  avoided_input_tokens_est, avoided_output_tokens_est, \
                  worker_prompt_tokens, worker_completion_tokens) \
                 VALUES (?1, ?2, ?3, ?4, ?5, 0, ?6, ?7)",
                params![project, harness, kind, ts, est, wp, wc],
            )?;
        }
        Ok(())
    })
    .await
    .unwrap();
}

/// One `client_usage` report written exactly as the server writes it, then
/// moved to `ts` (the collector stamps the current time).
async fn report(c: &StatsCollector, db: &Db, ts: i64, task: &str, source: &str, usage: (u64, u64)) {
    let ctx = CallCtx {
        project: "mmhmem",
        harness: "opencode",
        task: Some(task),
    };
    let usage = ClientUsage {
        prompt_tokens: usage.0,
        completion_tokens: usage.1,
        source: source.to_string(),
    };
    c.record_client_usage(&ctx, &usage).await.unwrap();
    db.with_conn(move |conn| {
        conn.execute(
            "UPDATE token_stats SET ts_epoch_ms = ?1 WHERE id = (SELECT MAX(id) FROM token_stats)",
            params![ts],
        )
    })
    .await
    .unwrap();
}

/// Seeds estimates plus client reports. Expected client values:
/// - DAY1 mmhmem/opencode: t1 "opencode /status" reported 100, 250, then 400
///   cumulatively (only 400/40 counts), t2 60/6, t1 "opencode-e2e" 30/3:
///   490 prompt, 49 completion, two sources.
/// - DAY2 mmhmem/opencode: t3 90/9 (a report on its own day).
/// - DAY2 other/claude-code: a hand-written row without completion: 11/0.
async fn seed(dir: &Path, audit: &Path) {
    let db = Db::open(dir).unwrap();
    seed_estimates(&db).await;
    let c = StatsCollector::new(db.clone(), AuditWriter::new(audit.to_path_buf()));
    let status = "opencode /status";
    report(&c, &db, DAY1 + 1, "t1", status, (100, 10)).await;
    report(&c, &db, DAY1 + 2, "t1", status, (250, 25)).await;
    report(&c, &db, DAY1 + 3, "t1", status, (400, 40)).await;
    report(&c, &db, DAY1 + 4, "t2", status, (60, 6)).await;
    report(&c, &db, DAY1 + 5, "t1", "opencode-e2e", (30, 3)).await;
    report(&c, &db, DAY2 + 1, "t3", status, (90, 9)).await;
    db.with_conn(|conn| {
        conn.execute(
            "INSERT INTO token_stats (project, agent_harness, kind, ts_epoch_ms, details) \
             VALUES ('other', 'claude-code', 'client_reported', ?1, \
                     '{\"prompt_tokens\":11,\"source\":\"manual\"}')",
            params![DAY2 + 2],
        )
    })
    .await
    .unwrap();
}

fn json_report(dir: &Path) -> serde_json::Value {
    let out = run_stats(dir, true);
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("--json must parse: {e}\n{stdout}"))
}

#[tokio::test]
async fn stats_table_shows_estimates_and_client_reported_spend() {
    let (dir, logs) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    seed(dir.path(), &logs.path().join("audit.jsonl")).await;
    let out = run_stats(dir.path(), false);
    assert!(out.status.success(), "stats must exit 0");
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(
        stdout.contains("2025-01-01") && stdout.contains("2025-01-02"),
        "{stdout}"
    );
    assert!(stdout.contains("opencode") && stdout.contains("claude-code"));
    assert!(
        stdout.contains("AVOIDED (EST.)")
            && stdout.contains("FRONTIER PROMPT (CLIENT-REPORTED)")
            && stdout.contains("FRONTIER COMPLETION (CLIENT-REPORTED)")
            && !stdout.contains("AVOIDED (CLIENT-REPORTED)"),
        "labels: {stdout}"
    );
    let total = stdout
        .lines()
        .find(|l| l.starts_with("TOTAL"))
        .unwrap_or_else(|| panic!("totals row: {stdout}"));
    assert!(total.contains("1,400"), "estimates total: {total}");
    assert!(
        total.contains("591") && total.contains("58"),
        "client totals: {total}"
    );
}

#[tokio::test]
async fn stats_json_counts_the_latest_report_per_task_and_source() {
    let (dir, logs) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    seed(dir.path(), &logs.path().join("audit.jsonl")).await;
    let v = json_report(dir.path());
    let days = v["days"].as_array().expect("days array");
    assert_eq!(days.len(), 3, "{v}");

    // day DESC, then project, then harness.
    assert_eq!(
        (days[0]["day"].as_str(), days[0]["project"].as_str()),
        (Some("2025-01-02"), Some("mmhmem"))
    );
    assert_eq!(days[0]["avoided_input_tokens_est"], 100);
    assert_eq!(days[0]["frontier_prompt_tokens_client_reported"], 90);
    assert_eq!(days[0]["frontier_completion_tokens_client_reported"], 9);

    assert_eq!(days[1]["project"], "other");
    assert_eq!(days[1]["frontier_prompt_tokens_client_reported"], 11);
    assert_eq!(
        days[1]["frontier_completion_tokens_client_reported"], 0,
        "missing value is 0"
    );
    assert_eq!(
        days[1]["client_reported_sources"],
        serde_json::json!(["manual"])
    );

    assert_eq!(days[2]["day"], "2025-01-01");
    assert_eq!(days[2]["avoided_input_tokens_est"], 400);
    assert_eq!(days[2]["frontier_prompt_tokens_client_reported"], 490);
    assert_eq!(days[2]["frontier_completion_tokens_client_reported"], 49);
    assert_eq!(
        days[2]["client_reported_sources"],
        serde_json::json!(["opencode /status", "opencode-e2e"])
    );
    assert!(
        days[2]
            .get("avoided_input_tokens_client_reported")
            .is_none()
    );

    let totals = &v["totals"];
    assert_eq!(totals["avoided_input_tokens_est"], 1_400);
    assert_eq!(totals["frontier_prompt_tokens_client_reported"], 591);
    assert_eq!(totals["frontier_completion_tokens_client_reported"], 58);
    assert_eq!(totals["worker_prompt_tokens"], 70);
    assert_eq!(totals["worker_completion_tokens"], 40);
}

#[tokio::test]
async fn every_report_is_still_stored_and_audited() {
    let (dir, logs) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let audit = logs.path().join("audit.jsonl");
    seed(dir.path(), &audit).await;
    let db = Db::open(dir.path()).unwrap();
    let stored: i64 = db
        .with_conn(|c| {
            c.query_row(
                "SELECT COUNT(*) FROM token_stats WHERE kind = 'client_reported'",
                [],
                |r| r.get(0),
            )
        })
        .await
        .unwrap();
    assert_eq!(stored, 7, "six collector reports plus the hand-written row");
    let log = std::fs::read_to_string(&audit).unwrap();
    assert_eq!(log.matches("TOKEN_STATS_CLIENT").count(), 6, "{log}");
}

#[test]
fn stats_on_missing_store_prints_message_and_exits_zero() {
    let dir = tempfile::tempdir().unwrap();
    let out = run_stats(dir.path(), false);
    assert!(
        out.status.success(),
        "no store yet is not an error (Req 8.5)"
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(
        stdout.contains("No token-savings statistics"),
        "clear message: {stdout}"
    );
    // And the command invented nothing on disk.
    assert!(
        !dir.path().join("memory.db").exists(),
        "stats must not create the store (Req 8.6)"
    );
}

#[tokio::test]
async fn stats_on_empty_store_prints_message_and_exits_zero() {
    let dir = tempfile::tempdir().unwrap();
    {
        // Store exists (schema applied) but holds zero statistics rows.
        let db = Db::open(dir.path()).unwrap();
        drop(db);
    }
    let out = run_stats(dir.path(), false);
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("No token-savings statistics"), "{stdout}");
}

#[tokio::test]
async fn stats_never_mutates_the_store() {
    let (dir, logs) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    seed(dir.path(), &logs.path().join("audit.jsonl")).await;
    let store = dir.path().join("memory.db");
    let before = std::fs::read(&store).unwrap();

    let table = run_stats(dir.path(), false);
    assert!(table.status.success());
    let json = run_stats(dir.path(), true);
    assert!(json.status.success());

    let after = std::fs::read(&store).unwrap();
    assert_eq!(before, after, "stats must not mutate the store (Req 8.6)");
}
