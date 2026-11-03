//! Client-reported frontier usage for `mmhmem stats` (client-usage-stats
//! spec, Requirements 1 and 3; design.md "stats_client").
//!
//! The server stores each `client_usage` report as a `client_reported` row
//! whose token counts live only in its `details` JSON. A report is the
//! harness's CUMULATIVE frontier usage for one workspace task, so only the
//! latest report per project, harness, task and source is counted, on the
//! UTC day of that report. Details are parsed in Rust so a malformed row
//! degrades to zeros instead of failing the whole command. Read-only: this
//! module issues `SELECT` only.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use rusqlite::params;
use serde_json::Value;

use super::stats::KIND_CLIENT_REPORTED;
use crate::storage::db::Db;

/// (day, project, harness) grouping key for [`client_days`].
type DayKey = (String, String, String);
/// (prompt, completion, sources) running totals for one [`DayKey`].
type DayTotals = (i64, i64, BTreeSet<String>);

/// Source label for a report whose details carry no usable `source`.
pub const UNLABELLED_SOURCE: &str = "unlabelled";

const CLIENT_ROWS_SQL: &str = "\
SELECT id, ts_epoch_ms, date(ts_epoch_ms / 1000, 'unixepoch'), \
       project, agent_harness, workspace_task_id, details \
FROM token_stats WHERE kind = ?1";

/// Identifies one stream of cumulative reports (Requirement 3.2, 3.3).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ReportKey {
    pub project: String,
    pub harness: String,
    pub task: Option<String>,
    pub source: String,
}

/// One parsed client-reported row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientReport {
    pub id: i64,
    pub ts_epoch_ms: i64,
    /// UTC day of the report, `YYYY-MM-DD` (Requirement 3.4).
    pub day: String,
    pub key: ReportKey,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
}

/// Client totals for one day × project × harness (Requirement 1.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientDay {
    pub day: String,
    pub project: String,
    pub harness: String,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    /// Sorted, distinct report sources behind the totals (Requirement 2.3).
    pub sources: Vec<String>,
}

/// Parses one row's details into (prompt, completion, source); never fails
/// (Requirement 1.3). Values that are missing, negative, fractional or not
/// numbers count as 0.
pub fn parse_details(details: Option<&str>) -> (i64, i64, String) {
    let value: Option<Value> = details.and_then(|d| serde_json::from_str(d).ok());
    let count = |key: &str| {
        value
            .as_ref()
            .and_then(|v| v.get(key))
            .and_then(Value::as_u64)
            .and_then(|n| i64::try_from(n).ok())
            .unwrap_or(0)
    };
    let source = value
        .as_ref()
        .and_then(|v| v.get("source"))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or(UNLABELLED_SOURCE)
        .to_string();
    (count("prompt_tokens"), count("completion_tokens"), source)
}

/// Keeps the latest report per key: newest timestamp, then the later row
/// (Requirement 3.2). Output is ordered by key.
pub fn latest_per_key(reports: Vec<ClientReport>) -> Vec<ClientReport> {
    let mut latest: HashMap<ReportKey, ClientReport> = HashMap::new();
    for report in reports {
        let newer = latest
            .get(&report.key)
            .is_none_or(|kept| (report.ts_epoch_ms, report.id) > (kept.ts_epoch_ms, kept.id));
        if newer {
            latest.insert(report.key.clone(), report);
        }
    }
    let mut kept: Vec<ClientReport> = latest.into_values().collect();
    kept.sort_by(|a, b| a.key.cmp(&b.key));
    kept
}

/// Sums kept reports per day × project × harness, ordered by that triple.
pub fn client_days(reports: Vec<ClientReport>) -> Vec<ClientDay> {
    let mut days: BTreeMap<DayKey, DayTotals> = BTreeMap::new();
    for r in reports {
        let entry = days
            .entry((r.day, r.key.project, r.key.harness))
            .or_default();
        entry.0 += r.prompt_tokens;
        entry.1 += r.completion_tokens;
        entry.2.insert(r.key.source);
    }
    days.into_iter()
        .map(
            |((day, project, harness), (prompt, completion, sources))| ClientDay {
                day,
                project,
                harness,
                prompt_tokens: prompt,
                completion_tokens: completion,
                sources: sources.into_iter().collect(),
            },
        )
        .collect()
}

/// Reads every client-reported row (SELECT only) and returns per-day totals
/// of the latest report per key (Requirements 1.1, 1.2, 1.5, 3.2-3.4).
pub async fn client_totals(db: &Db) -> anyhow::Result<Vec<ClientDay>> {
    let reports = db
        .with_conn(|conn| {
            let mut stmt = conn.prepare(CLIENT_ROWS_SQL)?;
            let rows = stmt
                .query_map(params![KIND_CLIENT_REPORTED], |r| {
                    let details: Option<String> = r.get(6)?;
                    let (prompt, completion, source) = parse_details(details.as_deref());
                    Ok(ClientReport {
                        id: r.get(0)?,
                        ts_epoch_ms: r.get(1)?,
                        day: r.get(2)?,
                        key: ReportKey {
                            project: r.get(3)?,
                            harness: r.get(4)?,
                            task: r.get(5)?,
                            source,
                        },
                        prompt_tokens: prompt,
                        completion_tokens: completion,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
        .await?;
    Ok(client_days(latest_per_key(reports)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::session_bus::ClientUsage;
    use crate::services::shunt_types::CallCtx;
    use crate::services::stats::StatsCollector;
    use crate::storage::audit::AuditWriter;

    fn report(id: i64, ts: i64, task: &str, source: &str, prompt: i64) -> ClientReport {
        ClientReport {
            id,
            ts_epoch_ms: ts,
            day: "2026-11-03".to_string(),
            key: ReportKey {
                project: "p".to_string(),
                harness: "h".to_string(),
                task: Some(task.to_string()),
                source: source.to_string(),
            },
            prompt_tokens: prompt,
            completion_tokens: prompt / 10,
        }
    }

    #[test]
    fn complete_details_are_parsed() {
        let d = r#"{"prompt_tokens":1200,"completion_tokens":34,"source":"opencode /status","label":"client_reported"}"#;
        assert_eq!(
            parse_details(Some(d)),
            (1200, 34, "opencode /status".to_string())
        );
    }

    #[test]
    fn missing_or_bad_values_count_as_zero() {
        assert_eq!(
            parse_details(Some(r#"{"prompt_tokens":5,"source":"s"}"#)),
            (5, 0, "s".to_string())
        );
        assert_eq!(
            parse_details(Some(
                r#"{"prompt_tokens":"9","completion_tokens":-3,"source":"s"}"#
            )),
            (0, 0, "s".to_string())
        );
        assert_eq!(
            parse_details(Some(r#"{"prompt_tokens":1.5,"completion_tokens":2}"#)).0,
            0
        );
    }

    #[test]
    fn malformed_or_empty_details_degrade_without_failing() {
        let unlabelled = (0, 0, UNLABELLED_SOURCE.to_string());
        assert_eq!(parse_details(Some("not json")), unlabelled);
        assert_eq!(parse_details(Some("")), unlabelled);
        assert_eq!(parse_details(None), unlabelled);
        assert_eq!(
            parse_details(Some(r#"{"prompt_tokens":3,"source":""}"#)).2,
            UNLABELLED_SOURCE
        );
    }

    #[test]
    fn latest_report_per_key_wins_by_timestamp_then_row() {
        let kept = latest_per_key(vec![
            report(1, 100, "t", "s", 10),
            report(3, 300, "t", "s", 30),
            report(2, 200, "t", "s", 20),
        ]);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].prompt_tokens, 30);

        let tied = latest_per_key(vec![
            report(7, 500, "t", "s", 70),
            report(8, 500, "t", "s", 80),
        ]);
        assert_eq!(
            tied[0].prompt_tokens, 80,
            "same millisecond: the later row wins"
        );
    }

    #[test]
    fn different_tasks_and_sources_are_each_counted() {
        let kept = latest_per_key(vec![
            report(1, 100, "t1", "s", 10),
            report(2, 100, "t2", "s", 20),
            report(3, 100, "t1", "other", 30),
        ]);
        assert_eq!(kept.len(), 3);
        let days = client_days(kept);
        assert_eq!(days.len(), 1);
        assert_eq!(days[0].prompt_tokens, 60);
        assert_eq!(days[0].sources, vec!["other".to_string(), "s".to_string()]);
    }

    fn collector(dir: &std::path::Path) -> (Db, StatsCollector) {
        let db = Db::open(&dir.join("data")).unwrap();
        let audit = AuditWriter::new(dir.join("audit.jsonl"));
        (db.clone(), StatsCollector::new(db, audit))
    }

    async fn report_usage(
        c: &StatsCollector,
        task: &str,
        source: &str,
        prompt: u64,
        completion: u64,
    ) {
        let ctx = CallCtx {
            project: "mmhmem",
            harness: "opencode",
            task: Some(task),
        };
        let usage = ClientUsage {
            prompt_tokens: prompt,
            completion_tokens: completion,
            source: source.to_string(),
        };
        c.record_client_usage(&ctx, &usage).await.unwrap();
    }

    #[tokio::test]
    async fn totals_count_the_latest_cumulative_report_on_its_own_day() {
        let dir = tempfile::tempdir().unwrap();
        let (db, c) = collector(dir.path());
        report_usage(&c, "t1", "opencode /status", 100, 10).await;
        report_usage(&c, "t1", "opencode /status", 250, 25).await;
        report_usage(&c, "t2", "opencode /status", 40, 4).await;
        // Move t2's report to an earlier UTC day after writing it.
        db.with_conn(|conn| {
            conn.execute(
                "UPDATE token_stats SET ts_epoch_ms = 86400000 WHERE workspace_task_id = 't2'",
                [],
            )
        })
        .await
        .unwrap();

        let days = client_totals(&db).await.unwrap();
        assert_eq!(days.len(), 2);
        let earlier = days
            .iter()
            .find(|d| d.day == "1970-01-02")
            .expect("t2 on its own day");
        assert_eq!((earlier.prompt_tokens, earlier.completion_tokens), (40, 4));
        let today = days.iter().find(|d| d.day != "1970-01-02").unwrap();
        assert_eq!((today.prompt_tokens, today.completion_tokens), (250, 25));
        assert_eq!(today.sources, vec!["opencode /status".to_string()]);
    }

    #[tokio::test]
    async fn rows_with_unusual_details_do_not_fail_the_read() {
        let dir = tempfile::tempdir().unwrap();
        let (db, c) = collector(dir.path());
        report_usage(&c, "t", "s", 7, 1).await;
        db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO token_stats (project, agent_harness, kind, ts_epoch_ms, details) \
                 VALUES ('mmhmem', 'opencode', 'client_reported', 1, 'not json')",
                [],
            )
        })
        .await
        .unwrap();
        let days = client_totals(&db).await.unwrap();
        let total: i64 = days.iter().map(|d| d.prompt_tokens).sum();
        assert_eq!(total, 7);
        assert!(
            days.iter()
                .any(|d| d.sources == vec![UNLABELLED_SOURCE.to_string()])
        );
    }
}
