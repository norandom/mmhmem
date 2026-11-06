//! Unit tests for `stats_activity.rs` (read-routing-hooks Req 1.1-1.5).
use super::*;
use crate::cli::stats::{TableOptions, aggregate_with_activity, render_json, render_table};
use crate::storage::db::Db;
use std::fs;
use std::path::PathBuf;

/// 2025-01-01T00:00:00Z and 2025-01-02T00:00:00Z in epoch ms.
const DAY1: i64 = 1_735_689_600_000;
const DAY2: i64 = 1_735_776_000_000;

fn event(kind: &str, ms: i64, project: &str, harness: &str) -> String {
    format!(
        "{{\"ts\":\"x\",\"epoch_ms\":{ms},\"event_type\":\"{kind}\",\
         \"agent_harness\":\"{harness}\",\"project\":\"{project}\",\
         \"details\":{{}},\"record_sha256\":\"0\"}}\n"
    )
}

fn write_log(path: &PathBuf, lines: &[String]) {
    fs::write(path, lines.concat()).unwrap();
}

fn key(day: &str, project: &str, harness: &str) -> ActivityKey {
    (day.into(), project.into(), harness.into())
}

fn counts(s: u64, c: u64, sl: u64, d: u64, h: u64) -> ActivityCounts {
    ActivityCounts {
        summary_reads: s,
        cache_hits: c,
        slice_reads: sl,
        direct_reads: d,
        hook_redirects: h,
    }
}

#[test]
fn each_event_type_feeds_its_own_counter() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("audit.jsonl");
    let e = |k: &str| event(k, DAY1 + 5, "mmhmem", "opencode");
    write_log(
        &log,
        &[
            e("IO_SHUNT_DELEGATED"),
            e("IO_SHUNT_DELEGATED"),
            e("IO_SHUNT_CACHE_HIT"),
            e("IO_TARGETED_READ"),
            e("IO_TARGETED_READ"),
            e("IO_TARGETED_READ"),
            e("IO_DIRECT_READ"),
            e("HOOK_REDIRECT"),
            e("HOOK_REDIRECT"),
            e("HOOK_REDIRECT"),
            e("HOOK_REDIRECT"),
            // Not read routes: never counted.
            e("HOOK_PASS"),
            e("TOKEN_STATS"),
            e("SESSION_RESUME"),
        ],
    );
    let got = collect(&log, 5).unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(
        got[&key("2025-01-01", "mmhmem", "opencode")],
        counts(2, 1, 3, 1, 4)
    );
}

#[test]
fn counts_group_by_utc_day_project_and_harness() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("audit.jsonl");
    write_log(
        &log,
        &[
            event("IO_DIRECT_READ", DAY2 - 1, "a", "h1"),
            event("IO_DIRECT_READ", DAY2, "a", "h1"),
            event("IO_DIRECT_READ", DAY2, "b", "h1"),
            event("IO_DIRECT_READ", DAY2, "a", "h2"),
        ],
    );
    let got = collect(&log, 5).unwrap();
    assert_eq!(got[&key("2025-01-01", "a", "h1")], counts(0, 0, 0, 1, 0));
    assert_eq!(got[&key("2025-01-02", "a", "h1")], counts(0, 0, 0, 1, 0));
    assert_eq!(got[&key("2025-01-02", "b", "h1")], counts(0, 0, 0, 1, 0));
    assert_eq!(got[&key("2025-01-02", "a", "h2")], counts(0, 0, 0, 1, 0));
    assert_eq!(got.len(), 4);
}

#[test]
fn rotated_files_are_included_up_to_the_retention_count() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("audit.jsonl");
    let rotated = |n: u32| dir.path().join(format!("audit.jsonl.{n}"));
    write_log(&log, &[event("HOOK_REDIRECT", DAY1, "p", "h")]);
    write_log(&rotated(1), &[event("HOOK_REDIRECT", DAY1, "p", "h")]);
    write_log(&rotated(2), &[event("IO_TARGETED_READ", DAY1, "p", "h")]);
    // Beyond the retention count: not part of the log any more.
    write_log(&rotated(3), &[event("IO_TARGETED_READ", DAY1, "p", "h")]);
    let got = collect(&log, 2).unwrap();
    assert_eq!(got[&key("2025-01-01", "p", "h")], counts(0, 0, 1, 0, 2));
}

#[test]
fn rotated_files_count_even_when_the_active_file_is_gone() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("audit.jsonl");
    write_log(
        &dir.path().join("audit.jsonl.1"),
        &[event("IO_SHUNT_CACHE_HIT", DAY1, "p", "h")],
    );
    let got = collect(&log, 5).unwrap();
    assert_eq!(got[&key("2025-01-01", "p", "h")], counts(0, 1, 0, 0, 0));
}

#[test]
fn missing_or_garbled_logs_yield_nothing_instead_of_failing() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("audit.jsonl");
    assert!(collect(&log, 5).unwrap().is_empty());
    fs::write(
        &log,
        b"not json\n\n{\"event_type\":\"HOOK_REDIRECT\"}\n\xff\xfe\n{\"epoch_ms\":1}\n",
    )
    .unwrap();
    assert!(collect(&log, 5).unwrap().is_empty());
}

#[test]
fn collecting_never_modifies_the_log_files() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("audit.jsonl");
    let rotated = dir.path().join("audit.jsonl.1");
    write_log(&log, &[event("HOOK_REDIRECT", DAY1, "p", "h")]);
    write_log(&rotated, &[event("IO_DIRECT_READ", DAY1, "p", "h")]);
    let before = (fs::read(&log).unwrap(), fs::read(&rotated).unwrap());
    collect(&log, 5).unwrap();
    assert_eq!(
        (fs::read(&log).unwrap(), fs::read(&rotated).unwrap()),
        before
    );
}

#[tokio::test]
async fn a_day_with_activity_but_no_savings_rows_still_gets_a_row() {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(dir.path()).unwrap();
    let mut activity = ActivityMap::new();
    activity.insert(key("2025-01-01", "p", "h"), counts(1, 2, 3, 4, 5));
    let rows = aggregate_with_activity(Some(&db), activity.clone())
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].activity, counts(1, 2, 3, 4, 5));
    assert_eq!((rows[0].avoided_est, rows[0].worker_prompt), (0, 0));
    // Even without a store at all.
    let rows = aggregate_with_activity(None, activity).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].activity.hook_redirects, 5);
}

#[tokio::test]
async fn activity_merges_into_existing_savings_rows_and_totals() {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(dir.path()).unwrap();
    db.with_conn(|c| {
        c.execute(
            "INSERT INTO token_stats (project, agent_harness, kind, ts_epoch_ms, \
             avoided_input_tokens_est, avoided_output_tokens_est) \
             VALUES ('p', 'h', 'shunt_delegated', ?1, 400, 0)",
            [DAY1],
        )
    })
    .await
    .unwrap();
    let mut activity = ActivityMap::new();
    activity.insert(key("2025-01-01", "p", "h"), counts(1, 0, 0, 0, 2));
    activity.insert(key("2025-01-02", "p", "h"), counts(0, 1, 1, 1, 1));
    let rows = aggregate_with_activity(Some(&db), activity).await.unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].day, "2025-01-02");
    assert_eq!(rows[1].avoided_est, 400);
    assert_eq!(rows[1].activity, counts(1, 0, 0, 0, 2));
    assert_eq!(
        crate::cli::stats::totals(&rows).activity,
        counts(1, 1, 1, 1, 3)
    );
}

fn sample_rows() -> Vec<crate::cli::stats::DayAggregate> {
    let mut row = crate::cli::stats::DayAggregate {
        day: "2025-01-01".into(),
        project: "p".into(),
        harness: "h".into(),
        avoided_est: 400,
        client_prompt: 7,
        client_completion: 8,
        client_sources: vec!["s".into()],
        worker_prompt: 5,
        worker_completion: 6,
        activity: counts(1, 2, 3, 4, 5),
    };
    let mut second = row.clone();
    second.day = "2025-01-02".into();
    second.activity = counts(10, 20, 30, 40, 50);
    row.avoided_est = 400;
    vec![second, row]
}

#[test]
fn json_exposes_stable_activity_fields_and_keeps_savings_fields() {
    let v: serde_json::Value = serde_json::from_str(&render_json(&sample_rows())).unwrap();
    let activity_fields = [
        "summary_reads",
        "cache_hits",
        "slice_reads",
        "direct_reads",
        "hook_redirects",
    ];
    let savings_fields = [
        "avoided_input_tokens_est",
        "frontier_prompt_tokens_client_reported",
        "frontier_completion_tokens_client_reported",
        "worker_prompt_tokens",
        "worker_completion_tokens",
    ];
    let day = v["days"][0].as_object().unwrap();
    let mut expected: Vec<&str> = ["day", "project", "harness", "client_reported_sources"]
        .into_iter()
        .chain(savings_fields)
        .chain(activity_fields)
        .collect();
    expected.sort_unstable();
    let mut actual: Vec<&str> = day.keys().map(String::as_str).collect();
    actual.sort_unstable();
    assert_eq!(actual, expected, "exact days field set");
    assert_eq!(v["days"][0]["summary_reads"], 10);
    assert_eq!(v["days"][0]["hook_redirects"], 50);
    assert_eq!(v["days"][1]["cache_hits"], 2);
    let totals = v["totals"].as_object().unwrap();
    for f in activity_fields.into_iter().chain(savings_fields) {
        assert!(totals.contains_key(f), "totals lacks {f}");
    }
    assert_eq!(totals.len(), 10);
    assert_eq!(totals["summary_reads"], 11);
    assert_eq!(totals["slice_reads"], 33);
    assert_eq!(totals["avoided_input_tokens_est"], 800);
}

#[test]
fn table_keeps_savings_columns_and_adds_activity_columns() {
    let opts = TableOptions {
        width: None,
        today: time::Date::from_calendar_date(2025, time::Month::January, 2).unwrap(),
        all: false,
    };
    let table = render_table(&sample_rows(), &opts);
    let header = table.lines().nth(2).unwrap();
    for label in [
        "avoided (est.)",
        "frontier in (client)",
        "frontier out (client)",
        "worker in",
        "worker out",
        "summaries",
        "cache hits",
        "slices",
        "direct",
        "redirects",
    ] {
        assert!(header.contains(label), "missing {label}: {header}");
    }
    let total = table.lines().find(|l| l.starts_with("│ total")).unwrap();
    let cells: Vec<&str> = total
        .split(['│', '┆'])
        .map(str::trim)
        .filter(|c| !c.is_empty())
        .collect();
    assert_eq!(&cells[cells.len() - 5..], ["11", "22", "33", "44", "55"]);
}
