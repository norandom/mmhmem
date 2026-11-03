//! Stats CLI integration tests (task 4.3; Req 1.3, 8.1-8.6): run the REAL
//! binary as a child process against a seeded per-user store — table
//! rendering, `--json`, the empty-store contract, and the read-only
//! guarantee (no new dev-dependencies, house style like tests/cli.rs).

use std::path::Path;
use std::process::Command;

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
type SeedRow = (
    i64,
    &'static str,
    &'static str,
    &'static str,
    i64,
    Option<i64>,
    Option<i64>,
);

/// Seeds the store with a known multi-day, multi-kind history via the lib
/// API (schema + inserts with controlled timestamps).
async fn seed(dir: &Path) {
    let db = Db::open(dir).unwrap();
    db.with_conn(move |conn| {
        let rows: [SeedRow; 4] = [
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
                DAY1,
                "mmhmem",
                "opencode",
                "client_reported",
                700,
                None,
                None,
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

#[tokio::test]
async fn stats_table_shows_seeded_savings() {
    let dir = tempfile::tempdir().unwrap();
    seed(dir.path()).await;
    let out = run_stats(dir.path(), false);
    assert!(out.status.success(), "stats must exit 0");
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(
        !stdout.contains("No token-savings statistics") && stdout.contains("2025-01-01"),
        "day rows must appear: {stdout}"
    );
    assert!(stdout.contains("2025-01-02"), "both days: {stdout}");
    assert!(stdout.contains("opencode") && stdout.contains("claude-code"));
    assert!(
        stdout.contains("AVOIDED (EST.)")
            && stdout.contains("FRONTIER PROMPT (CLIENT-REPORTED)")
            && !stdout.contains("AVOIDED (CLIENT-REPORTED)"),
        "labels (Req 8.3; client-usage-stats Req 2.1): {stdout}"
    );
    assert!(stdout.contains("TOTAL"), "totals row: {stdout}");
    assert!(stdout.contains("1,400"), "estimates total: {stdout}");
}

#[tokio::test]
async fn stats_json_matches_the_data_contract() {
    let dir = tempfile::tempdir().unwrap();
    seed(dir.path()).await;
    let out = run_stats(dir.path(), true);
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    let v: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("--json output must parse: {e}\n{stdout}"));
    let days = v["days"].as_array().expect("days array");
    assert_eq!(days.len(), 3, "two day2 rows + one day1 row: {stdout}");
    assert_eq!(days[0]["day"], "2025-01-02");
    assert_eq!(days[0]["project"], "mmhmem");
    assert_eq!(days[0]["avoided_input_tokens_est"], 100);
    // The seeded client row has no details, so it reports nothing and its
    // estimate-column value is never counted (client-usage-stats Req 1.3, 1.4).
    assert_eq!(days[2]["frontier_prompt_tokens_client_reported"], 0);
    assert!(
        days[2]
            .get("avoided_input_tokens_client_reported")
            .is_none()
    );
    assert_eq!(v["totals"]["avoided_input_tokens_est"], 1_400);
    assert_eq!(v["totals"]["worker_prompt_tokens"], 70);
    assert_eq!(v["totals"]["worker_completion_tokens"], 40);
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
    let dir = tempfile::tempdir().unwrap();
    seed(dir.path()).await;
    let store = dir.path().join("memory.db");
    let before = std::fs::read(&store).unwrap();

    let table = run_stats(dir.path(), false);
    assert!(table.status.success());
    let json = run_stats(dir.path(), true);
    assert!(json.status.success());

    let after = std::fs::read(&store).unwrap();
    assert_eq!(
        before, after,
        "the store file must be byte-identical after stats runs (Req 8.6)"
    );
}
