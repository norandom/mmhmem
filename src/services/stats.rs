//! Token-savings statistics primitives (introduced in task 5.2 for the shunt
//! path; extended in task 8.1 with client_reported rows and OTel counters).
//! requirements.md Req 10.1-10.5; design.md "Data Models" token_stats.
//!
//! The server never sees the frontier API, so server-side numbers are
//! ESTIMATES at ~4 chars/token (Req 10.5): avoided frontier input tokens.
//! True frontier counts arrive only as client_reported deltas (Req 10.4).

use serde_json::Value;

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estimates_use_chars_per_token_and_saturate() {
        assert_eq!(estimate_delegated_read(4000, 400), 900);
        assert_eq!(estimate_delegated_read(100, 400), 0, "saturates at 0");
        assert_eq!(estimate_cache_hit(4000), 1000);
        assert_eq!(estimate_code_write(800, 1200), (200, 300));
    }
}
