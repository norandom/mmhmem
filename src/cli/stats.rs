//! Token-savings statistics aggregation and rendering for `mmhmem stats`
//! (local-mode Req 8.1-8.6; design.md "Stats CLI"; amended by the
//! client-usage-stats spec).
//!
//! Reads ONLY the existing `token_stats` table (no schema changes): groups
//! by day (UTC) × project × harness, summing estimated avoided input tokens
//! (all non-client kinds) plus the worker prompt/completion tokens spent
//! (Req 8.2). Client-reported rows are reduced separately by
//! [`super::stats_client`] to the frontier tokens the harness reported it
//! SPENT (latest cumulative report per task and source) and merged in by
//! key. The labeling discipline is preserved end to end: computed values are
//! always marked as estimates, client-reported values keep their source
//! label, and nothing computed is ever presented as measured (Req 8.3).

use std::collections::BTreeMap;

use rusqlite::params;
use serde::Serialize;

use super::stats_activity::{ActivityCounts, ActivityMap};
use super::stats_client::{ClientDay, client_totals};
use crate::storage::db::Db;

/// The one measured (non-estimate) kind in `token_stats`.
pub const KIND_CLIENT_REPORTED: &str = "client_reported";

/// One aggregate row: a day × project × harness triple (Req 8.1/8.2).
/// Serde field names form the `--json` data contract (design.md "Data
/// Contracts & Integration"; client-usage-stats Requirement 4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DayAggregate {
    pub day: String,
    pub project: String,
    pub harness: String,
    #[serde(rename = "avoided_input_tokens_est")]
    pub avoided_est: i64,
    #[serde(rename = "frontier_prompt_tokens_client_reported")]
    pub client_prompt: i64,
    #[serde(rename = "frontier_completion_tokens_client_reported")]
    pub client_completion: i64,
    #[serde(rename = "client_reported_sources")]
    pub client_sources: Vec<String>,
    #[serde(rename = "worker_prompt_tokens")]
    pub worker_prompt: i64,
    #[serde(rename = "worker_completion_tokens")]
    pub worker_completion: i64,
    /// Read-activity counts (read-routing-hooks Req 1.1-1.3).
    #[serde(flatten)]
    pub activity: ActivityCounts,
}

/// Totals across all rows (same field names as [`DayAggregate`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Totals {
    #[serde(rename = "avoided_input_tokens_est")]
    pub avoided_est: i64,
    #[serde(rename = "frontier_prompt_tokens_client_reported")]
    pub client_prompt: i64,
    #[serde(rename = "frontier_completion_tokens_client_reported")]
    pub client_completion: i64,
    #[serde(rename = "worker_prompt_tokens")]
    pub worker_prompt: i64,
    #[serde(rename = "worker_completion_tokens")]
    pub worker_completion: i64,
    #[serde(flatten)]
    pub activity: ActivityCounts,
}

const AGGREGATE_SQL: &str = "\
SELECT date(ts_epoch_ms / 1000, 'unixepoch') AS day, \
       project, \
       agent_harness, \
       SUM(avoided_input_tokens_est), \
       SUM(COALESCE(worker_prompt_tokens, 0)), \
       SUM(COALESCE(worker_completion_tokens, 0)) \
FROM token_stats \
WHERE kind != ?1 \
GROUP BY day, project, agent_harness";

/// (day, project, harness) key shared by both reads.
type RowKey = (String, String, String);

fn empty_row((day, project, harness): RowKey) -> DayAggregate {
    DayAggregate {
        day,
        project,
        harness,
        avoided_est: 0,
        client_prompt: 0,
        client_completion: 0,
        client_sources: Vec::new(),
        worker_prompt: 0,
        worker_completion: 0,
        activity: ActivityCounts::default(),
    }
}

/// Aggregates all token-stats rows (Req 8.1/8.2). Read-only by contract:
/// the caller opens the store with [`Db::open_read_only`] (Req 8.6).
pub async fn aggregate(db: &Db) -> anyhow::Result<Vec<DayAggregate>> {
    aggregate_with_activity(Some(db), ActivityMap::new()).await
}

/// Like [`aggregate`], also merging the audit-log read activity by key: a
/// day x project x harness with activity but no savings row still gets a
/// row (read-routing-hooks Req 1.2).
pub async fn aggregate_with_activity(
    db: Option<&Db>,
    activity: ActivityMap,
) -> anyhow::Result<Vec<DayAggregate>> {
    let Some(db) = db else {
        return Ok(merge(Vec::new(), Vec::new(), activity));
    };
    let estimates = db
        .with_conn(|conn| {
            let mut stmt = conn.prepare(AGGREGATE_SQL)?;
            let rows = stmt
                .query_map(params![KIND_CLIENT_REPORTED], |r| {
                    let mut row = empty_row((r.get(0)?, r.get(1)?, r.get(2)?));
                    row.avoided_est = r.get(3)?;
                    row.worker_prompt = r.get(4)?;
                    row.worker_completion = r.get(5)?;
                    Ok(row)
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
        .await?;
    Ok(merge(estimates, client_totals(db).await?, activity))
}

/// Merges estimate rows with client totals by key; a side a key lacks
/// contributes zeros. Ordered day DESC, then project, then harness.
fn merge(
    estimates: Vec<DayAggregate>,
    clients: Vec<ClientDay>,
    activity: ActivityMap,
) -> Vec<DayAggregate> {
    let mut rows: BTreeMap<RowKey, DayAggregate> = estimates
        .into_iter()
        .map(|r| ((r.day.clone(), r.project.clone(), r.harness.clone()), r))
        .collect();
    for c in clients {
        let key = (c.day, c.project, c.harness);
        let row = rows.entry(key.clone()).or_insert_with(|| empty_row(key));
        row.client_prompt = c.prompt_tokens;
        row.client_completion = c.completion_tokens;
        row.client_sources = c.sources;
    }
    for (key, counts) in activity {
        rows.entry(key.clone())
            .or_insert_with(|| empty_row(key))
            .activity = counts;
    }
    let mut merged: Vec<DayAggregate> = rows.into_values().collect();
    merged.sort_by(|a, b| {
        b.day
            .cmp(&a.day)
            .then_with(|| a.project.cmp(&b.project))
            .then_with(|| a.harness.cmp(&b.harness))
    });
    merged
}

/// Sums the totals across rows (used by both renderers).
pub fn totals(rows: &[DayAggregate]) -> Totals {
    rows.iter().fold(
        Totals {
            avoided_est: 0,
            client_prompt: 0,
            client_completion: 0,
            worker_prompt: 0,
            worker_completion: 0,
            activity: ActivityCounts::default(),
        },
        |acc, r| Totals {
            avoided_est: acc.avoided_est + r.avoided_est,
            client_prompt: acc.client_prompt + r.client_prompt,
            client_completion: acc.client_completion + r.client_completion,
            worker_prompt: acc.worker_prompt + r.worker_prompt,
            worker_completion: acc.worker_completion + r.worker_completion,
            activity: acc.activity.plus(&r.activity),
        },
    )
}

/// Number of table columns.
const COLUMNS: usize = 13;

/// Human-readable fixed-width table (Req 8.1) with an estimates-labeled
/// column, two client-reported spent-token columns, and a TOTAL row (Req 8.3).
pub fn render_table(rows: &[DayAggregate]) -> String {
    let mut table: Vec<[String; COLUMNS]> = vec![TABLE_HEADERS.map(str::to_string)];
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

const TABLE_HEADERS: [&str; COLUMNS] = [
    "DAY",
    "PROJECT",
    "HARNESS",
    "AVOIDED (EST.)",
    "FRONTIER PROMPT (CLIENT-REPORTED)",
    "FRONTIER COMPLETION (CLIENT-REPORTED)",
    "WORKER PROMPT",
    "WORKER COMPLETION",
    "SUMMARY READS",
    "CACHE HITS",
    "SLICE READS",
    "DIRECT READS",
    "HOOK REDIRECTS",
];

fn row_cells(r: &DayAggregate) -> [String; COLUMNS] {
    with_activity(
        vec![
            r.day.clone(),
            r.project.clone(),
            r.harness.clone(),
            thousands(r.avoided_est),
            thousands(r.client_prompt),
            thousands(r.client_completion),
            thousands(r.worker_prompt),
            thousands(r.worker_completion),
        ],
        &r.activity,
    )
}

fn total_cells(t: &Totals) -> [String; COLUMNS] {
    with_activity(
        vec![
            "TOTAL".to_string(),
            String::new(),
            String::new(),
            thousands(t.avoided_est),
            thousands(t.client_prompt),
            thousands(t.client_completion),
            thousands(t.worker_prompt),
            thousands(t.worker_completion),
        ],
        &t.activity,
    )
}

/// Appends the five read-activity cells to the savings cells.
fn with_activity(mut cells: Vec<String>, a: &ActivityCounts) -> [String; COLUMNS] {
    cells.extend(
        [
            a.summary_reads,
            a.cache_hits,
            a.slice_reads,
            a.direct_reads,
            a.hook_redirects,
        ]
        .map(|n| n.to_string()),
    );
    cells.try_into().expect("one cell per column")
}

fn column_widths(table: &[[String; COLUMNS]]) -> [usize; COLUMNS] {
    let mut widths = [0usize; COLUMNS];
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
#[path = "stats_tests.rs"]
mod tests;
