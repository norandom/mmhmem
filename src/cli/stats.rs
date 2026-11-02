//! Token-savings statistics aggregation and rendering for `mmhmem stats`
//! (local-mode Req 8.1-8.6; design.md "Stats CLI").
//!
//! Reads ONLY the existing `token_stats` table (no schema changes): groups
//! by day (UTC) × project × harness, summing estimated avoided input tokens
//! (all non-client kinds) SEPARATELY from client-reported values, plus the
//! worker prompt/completion tokens spent (Req 8.2). The labeling discipline
//! is preserved end to end: computed values are always marked as estimates,
//! client-reported values keep their source label, and nothing computed is
//! ever presented as measured (Req 8.3).

use rusqlite::params;
use serde::Serialize;

use crate::storage::db::Db;

/// The one measured (non-estimate) kind in `token_stats`.
pub const KIND_CLIENT_REPORTED: &str = "client_reported";

/// One aggregate row: a day × project × harness triple (Req 8.1/8.2).
/// Serde field names form the `--json` data contract (design.md "Data
/// Contracts & Integration").
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DayAggregate {
    pub day: String,
    pub project: String,
    pub harness: String,
    #[serde(rename = "avoided_input_tokens_est")]
    pub avoided_est: i64,
    #[serde(rename = "avoided_input_tokens_client_reported")]
    pub avoided_client: i64,
    #[serde(rename = "worker_prompt_tokens")]
    pub worker_prompt: i64,
    #[serde(rename = "worker_completion_tokens")]
    pub worker_completion: i64,
}

/// Totals across all rows (same field names as [`DayAggregate`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Totals {
    #[serde(rename = "avoided_input_tokens_est")]
    pub avoided_est: i64,
    #[serde(rename = "avoided_input_tokens_client_reported")]
    pub avoided_client: i64,
    #[serde(rename = "worker_prompt_tokens")]
    pub worker_prompt: i64,
    #[serde(rename = "worker_completion_tokens")]
    pub worker_completion: i64,
}

const AGGREGATE_SQL: &str = "\
SELECT date(ts_epoch_ms / 1000, 'unixepoch') AS day, \
       project, \
       agent_harness, \
       SUM(CASE WHEN kind != ?1 THEN avoided_input_tokens_est ELSE 0 END), \
       SUM(CASE WHEN kind = ?1 THEN avoided_input_tokens_est ELSE 0 END), \
       SUM(COALESCE(worker_prompt_tokens, 0)), \
       SUM(COALESCE(worker_completion_tokens, 0)) \
FROM token_stats \
GROUP BY day, project, agent_harness \
ORDER BY day DESC, project, agent_harness";

/// Aggregates all token-stats rows (Req 8.1/8.2). Read-only by contract:
/// the caller opens the store with [`Db::open_read_only`] (Req 8.6).
pub async fn aggregate(db: &Db) -> anyhow::Result<Vec<DayAggregate>> {
    db.with_conn(|conn| {
        let mut stmt = conn.prepare(AGGREGATE_SQL)?;
        let rows = stmt
            .query_map(params![KIND_CLIENT_REPORTED], |r| {
                Ok(DayAggregate {
                    day: r.get(0)?,
                    project: r.get(1)?,
                    harness: r.get(2)?,
                    avoided_est: r.get(3)?,
                    avoided_client: r.get(4)?,
                    worker_prompt: r.get(5)?,
                    worker_completion: r.get(6)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    })
    .await
}

/// Sums the totals across rows (used by both renderers).
pub fn totals(rows: &[DayAggregate]) -> Totals {
    rows.iter().fold(
        Totals {
            avoided_est: 0,
            avoided_client: 0,
            worker_prompt: 0,
            worker_completion: 0,
        },
        |acc, r| Totals {
            avoided_est: acc.avoided_est + r.avoided_est,
            avoided_client: acc.avoided_client + r.avoided_client,
            worker_prompt: acc.worker_prompt + r.worker_prompt,
            worker_completion: acc.worker_completion + r.worker_completion,
        },
    )
}

/// Human-readable fixed-width table (Req 8.1) with an estimates-labeled
/// column, a client-reported column, and a TOTAL row (Req 8.3).
pub fn render_table(rows: &[DayAggregate]) -> String {
    let mut table: Vec<[String; 7]> = vec![TABLE_HEADERS.map(human_label)];
    table.extend(rows.iter().map(row_cells));
    table.push(total_cells(&totals(rows)));
    let widths = column_widths(&table);
    let mut out = String::new();
    for row in &table {
        let line = row
            .iter()
            .enumerate()
            .map(|(i, cell)| pad_to(cell, widths[i]))
            .collect::<Vec<_>>()
            .join("  ");
        out.push_str(&line);
        out.push('\n');
    }
    out
}

/// Machine-readable JSON with the same aggregates (Req 8.4), shaped as
/// `{"days": [...], "totals": {...}}` (design.md data contract).
pub fn render_json(rows: &[DayAggregate]) -> String {
    let report = JsonReport {
        days: rows,
        totals: totals(rows),
    };
    serde_json::to_string_pretty(&report).expect("DayAggregate always serializes")
}

const TABLE_HEADERS: [&str; 7] = [
    "DAY",
    "PROJECT",
    "HARNESS",
    "AVOIDED (EST.)",
    "AVOIDED (CLIENT-REPORTED)",
    "WORKER PROMPT",
    "WORKER COMPLETION",
];

fn human_label(s: &str) -> String {
    s.to_string()
}

fn row_cells(r: &DayAggregate) -> [String; 7] {
    [
        r.day.clone(),
        r.project.clone(),
        r.harness.clone(),
        thousands(r.avoided_est),
        thousands(r.avoided_client),
        thousands(r.worker_prompt),
        thousands(r.worker_completion),
    ]
}

fn total_cells(t: &Totals) -> [String; 7] {
    [
        "TOTAL".to_string(),
        String::new(),
        String::new(),
        thousands(t.avoided_est),
        thousands(t.avoided_client),
        thousands(t.worker_prompt),
        thousands(t.worker_completion),
    ]
}

fn column_widths(table: &[[String; 7]]) -> [usize; 7] {
    let mut widths = [0usize; 7];
    for row in table {
        for (i, cell) in row.iter().enumerate() {
            widths[i] = widths[i].max(cell.chars().count());
        }
    }
    widths
}

fn pad_to(cell: &str, width: usize) -> String {
    let mut out = cell.to_string();
    let len = out.chars().count();
    if len < width {
        out.push_str(&" ".repeat(width - len));
    }
    out
}

/// `1400` → `1,400` (human-readable table numbers).
fn thousands(n: i64) -> String {
    let digits = n.unsigned_abs().to_string();
    let len = digits.len();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (len - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    if n < 0 { format!("-{out}") } else { out }
}

#[derive(Serialize)]
struct JsonReport<'a> {
    days: &'a [DayAggregate],
    totals: Totals,
}

#[cfg(test)]
mod tests {
    use super::*;
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
                DAY1_LATER,
                "mmhmem",
                "opencode",
                "client_reported",
                700,
                None,
                None,
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

    fn expected_aggregates() -> Vec<DayAggregate> {
        vec![
            // day DESC, then project, then harness.
            DayAggregate {
                day: "2025-01-02".into(),
                project: "mmhmem".into(),
                harness: "opencode".into(),
                avoided_est: 100,
                avoided_client: 0,
                worker_prompt: 0,
                worker_completion: 0,
            },
            DayAggregate {
                day: "2025-01-02".into(),
                project: "other".into(),
                harness: "claude-code".into(),
                avoided_est: 900,
                avoided_client: 0,
                worker_prompt: 20,
                worker_completion: 30,
            },
            DayAggregate {
                day: "2025-01-01".into(),
                project: "mmhmem".into(),
                harness: "opencode".into(),
                avoided_est: 400,
                avoided_client: 700,
                worker_prompt: 50,
                worker_completion: 10,
            },
        ]
    }

    #[tokio::test]
    async fn aggregate_groups_by_day_and_splits_estimates_from_client_reports() {
        let dir = tempfile::tempdir().unwrap();
        let db = seed_db(dir.path(), sample_rows()).await;
        let got = aggregate(&db).await.unwrap();
        assert_eq!(got, expected_aggregates());
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
        assert_eq!(t.avoided_client, 700);
        assert_eq!(t.worker_prompt, 50 + 20);
        assert_eq!(t.worker_completion, 10 + 30);
    }

    #[test]
    fn table_marks_estimates_and_labels_client_reports_with_totals() {
        let rows = expected_aggregates();
        let table = render_table(&rows);
        assert!(
            table.contains("AVOIDED (EST.)"),
            "estimates must be labeled: {table}"
        );
        assert!(
            table.contains("CLIENT-REPORTED"),
            "client-reported values keep their source label: {table}"
        );
        assert!(table.contains("DAY"), "day column: {table}");
        assert!(table.contains("2025-01-01"), "day rows: {table}");
        assert!(table.contains("opencode") && table.contains("claude-code"));
        assert!(table.contains("TOTAL"), "totals row: {table}");
        assert!(
            table.contains("1,400"),
            "summed estimates in the total: {table}"
        );
        assert!(table.contains("700"), "client-reported total: {table}");
    }

    #[test]
    fn table_renders_zero_totals_for_empty_input() {
        let table = render_table(&[]);
        assert!(table.contains("TOTAL"));
        assert!(table.contains('0'));
    }

    #[test]
    fn json_matches_the_data_contract() {
        let rows = expected_aggregates();
        let raw = render_json(&rows);
        let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
        let days = v["days"].as_array().unwrap();
        assert_eq!(days.len(), 3);
        for key in [
            "day",
            "project",
            "harness",
            "avoided_input_tokens_est",
            "avoided_input_tokens_client_reported",
            "worker_prompt_tokens",
            "worker_completion_tokens",
        ] {
            assert!(
                days[0].get(key).is_some(),
                "days[0] lacks {key}: {}",
                days[0]
            );
        }
        assert_eq!(days[0]["avoided_input_tokens_est"], 100);
        assert_eq!(days[2]["avoided_input_tokens_client_reported"], 700);
        assert_eq!(v["totals"]["avoided_input_tokens_est"], 1_400);
        assert_eq!(v["totals"]["worker_prompt_tokens"], 70);
    }
}
