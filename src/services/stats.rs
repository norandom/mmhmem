//! Token-savings statistics (task 5.2 primitives; task 8.1 collector).
//! requirements.md Req 10.1-10.5; design.md "Data Models" token_stats.
//!
//! The server never sees the frontier API, so server-side numbers are
//! ESTIMATES at ~4 chars/token (Req 10.5): avoided frontier input tokens.
//! True frontier counts arrive only as client_reported rows (Req 10.4): each
//! report is the harness's cumulative usage for one workspace task, and
//! `mmhmem stats` counts the latest report per task and source.
//! OTel counters go through the injected [`OtelSink`] (no-op by default; the
//! real exporter is task 9.1 behind the `otel` cargo feature, wired in 10.4).

use serde_json::Value;

use crate::services::session_bus::ClientUsage;
use crate::services::shunt_types::CallCtx;
use crate::storage::audit::{AuditEventType, AuditWriter};
use crate::storage::db::Db;

/// `token_stats.kind` values.
pub mod kind {
    /// Worker-delegated shunted read (cache miss).
    pub const SHUNT_DELEGATED: &str = "shunt_delegated";
    /// Cache hit — zero worker calls (Req 3.6).
    pub const SHUNT_CACHE_HIT: &str = "shunt_cache_hit";
    /// boilerplate_write via the worker.
    pub const CODE_WRITE: &str = "code_write";
    /// Harness-reported frontier totals (Req 10.4).
    pub const CLIENT_REPORTED: &str = "client_reported";
}

/// One `token_stats` row (design.md Physical Data Model).
#[derive(Debug, Clone)]
pub struct TokenStatRow {
    /// Project name.
    pub project: String,
    /// Workspace task ID, if known.
    pub task_id: Option<String>,
    /// Calling harness.
    pub harness: String,
    /// `kind` (see [`kind`]).
    pub kind: &'static str,
    /// Estimated avoided frontier INPUT tokens (÷4 chars heuristic).
    pub avoided_input_tokens_est: u64,
    /// Estimated avoided frontier OUTPUT tokens (boilerplate only).
    pub avoided_output_tokens_est: u64,
    /// Worker prompt tokens (from the worker API `usage`), if a worker ran.
    pub worker_prompt_tokens: Option<u64>,
    /// Worker completion tokens, if a worker ran.
    pub worker_completion_tokens: Option<u64>,
    /// Redacted extra details.
    pub details: Value,
}

/// Persists one stats row (Req 10.3). Caller runs this in a short locked
/// section via `Db::with_conn`.
pub async fn record(db: &Db, row: TokenStatRow) -> anyhow::Result<()> {
    let TokenStatRow {
        project,
        task_id,
        harness,
        kind,
        avoided_input_tokens_est,
        avoided_output_tokens_est,
        worker_prompt_tokens,
        worker_completion_tokens,
        details,
    } = row;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    db.with_conn(move |c| {
        c.execute(
            "INSERT INTO token_stats (project, workspace_task_id, agent_harness, kind, \
              ts_epoch_ms, avoided_input_tokens_est, avoided_output_tokens_est, \
              worker_prompt_tokens, worker_completion_tokens, details) \
              VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            rusqlite::params![
                project,
                task_id,
                harness,
                kind,
                now,
                avoided_input_tokens_est as i64,
                avoided_output_tokens_est as i64,
                worker_prompt_tokens.map(|v| v as i64),
                worker_completion_tokens.map(|v| v as i64),
                details.to_string(),
            ],
        )
        .map(|_| ())
    })
    .await
}

/// Chars-per-token heuristic shared with the working-state cap (Req 2.5, 10.5).
pub const CHARS_PER_TOKEN: usize = 4;

/// Estimate for a delegated read (Req 10.1): (raw − returned) chars ÷ 4.
pub fn estimate_delegated_read(raw_chars: usize, returned_chars: usize) -> u64 {
    (raw_chars.saturating_sub(returned_chars) / CHARS_PER_TOKEN) as u64
}

/// Estimate for a cache hit (Req 10.1): the full raw read was avoided ÷ 4.
pub fn estimate_cache_hit(raw_chars: usize) -> u64 {
    (raw_chars / CHARS_PER_TOKEN) as u64
}

/// Estimates for boilerplate_write (Req 10.1): (avoided input, avoided output)
/// = (reference chars ÷ 4, generated chars ÷ 4).
pub fn estimate_code_write(reference_chars: usize, generated_chars: usize) -> (u64, u64) {
    (
        (reference_chars / CHARS_PER_TOKEN) as u64,
        (generated_chars / CHARS_PER_TOKEN) as u64,
    )
}

/// OTel sink for stats counters (Req 11.3). No-op unless the `otel` feature
/// exporter (task 9.1) is wired in task 10.4.
pub trait OtelSink: Send + Sync {
    /// Tokens-avoided counters.
    fn incr_tokens_avoided(&self, _input_est: u64, _output_est: u64) {}
    /// Worker token counters.
    fn incr_worker_tokens(&self, _prompt: u64, _completion: u64) {}
    /// Shunt cache hit/miss counter.
    fn incr_cache(&self, _hit: bool) {}
}

/// Default no-op sink (OTel disabled).
#[derive(Debug, Default, Clone, Copy)]
pub struct NoopSink;

impl OtelSink for NoopSink {}

/// The stats collector (task 8.1): persists token_stats rows, audits them,
/// and mirrors counters to the OTel sink. Server-side rows are estimates
/// (labelled); client-reported rows carry the harness's true frontier totals.
#[derive(Clone)]
pub struct StatsCollector {
    db: Db,
    audit: AuditWriter,
    sink: std::sync::Arc<dyn OtelSink>,
}

impl StatsCollector {
    /// New collector with the no-op sink.
    pub fn new(db: Db, audit: AuditWriter) -> Self {
        Self {
            db,
            audit,
            sink: std::sync::Arc::new(NoopSink),
        }
    }

    /// New collector with a custom OTel sink (wired in 10.4 when `otel` is on).
    pub fn with_sink(db: Db, audit: AuditWriter, sink: std::sync::Arc<dyn OtelSink>) -> Self {
        Self { db, audit, sink }
    }

    /// Estimated avoided tokens (input + output) over the last `days` days:
    /// returns `(project_tokens, total_tokens)`.
    pub async fn avoided_tokens_last_days(
        &self,
        project: &str,
        days: i64,
    ) -> anyhow::Result<(i64, i64)> {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        let cutoff_ms = now_ms.saturating_sub(days.saturating_mul(86_400_000));
        let project = project.to_owned();
        self.db
            .with_conn(move |c| {
                let total: i64 = c.query_row(
                    "SELECT COALESCE(SUM(avoided_input_tokens_est + avoided_output_tokens_est), 0) \
                     FROM token_stats WHERE kind != ?1 AND ts_epoch_ms >= ?2",
                    rusqlite::params![kind::CLIENT_REPORTED, cutoff_ms],
                    |r| r.get(0),
                )?;
                let proj: i64 = c.query_row(
                    "SELECT COALESCE(SUM(avoided_input_tokens_est + avoided_output_tokens_est), 0) \
                     FROM token_stats WHERE kind != ?1 AND ts_epoch_ms >= ?2 AND project = ?3",
                    rusqlite::params![kind::CLIENT_REPORTED, cutoff_ms, project],
                    |r| r.get(0),
                )?;
                Ok((proj, total))
            })
            .await
    }

    /// Persists a server-side (estimate) row: token_stats insert + TOKEN_STATS
    /// audit + sink counters (Req 10.1-10.3).
    pub async fn record_estimate(&self, row: TokenStatRow) -> anyhow::Result<()> {
        let kind = row.kind;
        let avoided_in = row.avoided_input_tokens_est;
        let avoided_out = row.avoided_output_tokens_est;
        let (wp, wc) = (row.worker_prompt_tokens, row.worker_completion_tokens);
        let harness = row.harness.clone();
        let project = row.project.clone();
        let details = row.details.clone();
        record(&self.db, row).await?;
        let _ = self.audit.write_event(
            AuditEventType::TokenStats,
            &harness,
            &project,
            serde_json::json!({
                "kind": kind, "avoided_input_tokens_est": avoided_in,
                "avoided_output_tokens_est": avoided_out,
                "worker_prompt_tokens": wp, "worker_completion_tokens": wc,
                "label": "estimate", "details": details,
            }),
        );
        self.sink.incr_tokens_avoided(avoided_in, avoided_out);
        if let (Some(p), Some(c)) = (wp, wc) {
            self.sink.incr_worker_tokens(p, c);
        }
        if kind == kind::SHUNT_CACHE_HIT {
            self.sink.incr_cache(true);
        } else if kind == kind::SHUNT_DELEGATED {
            self.sink.incr_cache(false);
        }
        Ok(())
    }

    /// Persists a client-reported frontier usage delta (Req 10.4): stored
    /// with kind=client_reported and audited as TOKEN_STATS_CLIENT; labelled
    /// with its source; never required.
    pub async fn record_client_usage(
        &self,
        ctx: &CallCtx<'_>,
        usage: &ClientUsage,
    ) -> anyhow::Result<()> {
        let row = TokenStatRow {
            project: ctx.project.to_string(),
            task_id: ctx.task.map(str::to_string),
            harness: ctx.harness.to_string(),
            kind: kind::CLIENT_REPORTED,
            avoided_input_tokens_est: 0,
            avoided_output_tokens_est: 0,
            worker_prompt_tokens: None,
            worker_completion_tokens: None,
            details: serde_json::json!({
                "prompt_tokens": usage.prompt_tokens,
                "completion_tokens": usage.completion_tokens,
                "source": usage.source,
                "label": "client_reported",
            }),
        };
        let audit_details = row.details.clone();
        record(&self.db, row).await?;
        let _ = self.audit.write_event(
            AuditEventType::TokenStatsClient,
            ctx.harness,
            ctx.project,
            audit_details,
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::audit::AuditWriter;

    #[test]
    fn estimates_use_chars_per_token_and_saturate() {
        assert_eq!(estimate_delegated_read(4000, 400), 900);
        assert_eq!(estimate_delegated_read(100, 400), 0, "saturates at 0");
        assert_eq!(estimate_cache_hit(4000), 1000);
        assert_eq!(estimate_code_write(800, 1200), (200, 300));
    }

    #[tokio::test]
    async fn client_reported_row_is_persisted_and_audited_with_source_label() {
        let data = tempfile::tempdir().unwrap();
        let logs = tempfile::tempdir().unwrap();
        let db = Db::open(data.path()).unwrap();
        let audit = AuditWriter::new(logs.path().join("audit.jsonl"));
        let collector = StatsCollector::new(db.clone(), audit);
        let ctx = CallCtx {
            project: "p",
            harness: "opencode",
            task: Some("t"),
        };
        let usage = ClientUsage {
            prompt_tokens: 12_345,
            completion_tokens: 678,
            source: "opencode /status".to_string(),
        };
        collector.record_client_usage(&ctx, &usage).await.unwrap();

        let rows: Vec<(String, String)> = db
            .with_conn(|c| {
                let mut s = c.prepare("SELECT kind, details FROM token_stats")?;
                let rows =
                    s.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
                rows.collect()
            })
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].0, "client_reported");
        assert!(
            rows[0].1.contains("12345")
                && rows[0].1.contains("client_reported")
                && rows[0].1.contains("opencode /status")
        );

        let log = std::fs::read_to_string(logs.path().join("audit.jsonl")).unwrap();
        assert!(log.contains("TOKEN_STATS_CLIENT"), "log={log}");
        assert!(log.contains("12345"));
    }

    #[tokio::test]
    async fn estimate_row_is_audited_as_estimate() {
        let data = tempfile::tempdir().unwrap();
        let logs = tempfile::tempdir().unwrap();
        let db = Db::open(data.path()).unwrap();
        let audit = AuditWriter::new(logs.path().join("audit.jsonl"));
        let collector = StatsCollector::new(db, audit);
        collector
            .record_estimate(TokenStatRow {
                project: "p".to_string(),
                task_id: None,
                harness: "h".to_string(),
                kind: kind::SHUNT_CACHE_HIT,
                avoided_input_tokens_est: 500,
                avoided_output_tokens_est: 0,
                worker_prompt_tokens: None,
                worker_completion_tokens: None,
                details: serde_json::json!({}),
            })
            .await
            .unwrap();
        let log = std::fs::read_to_string(logs.path().join("audit.jsonl")).unwrap();
        assert!(log.contains("TOKEN_STATS"));
        assert!(log.contains("\"label\":\"estimate\""), "log={log}");
        assert!(log.contains("500"));
    }
}
