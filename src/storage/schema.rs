//! SQLite DDL and idempotent schema application (task 2.1).
//!
//! The DDL is the "Physical Data Model (SQLite, WAL)" block from design.md,
//! made idempotent with `IF NOT EXISTS`. Schema versioning is a single
//! `PRAGMA user_version` — there is deliberately no migration framework
//! (design.md "Complexity Budget"); a database stamped with a NEWER schema
//! version is refused, never silently downgraded.

use std::cmp::Ordering;

use rusqlite::ffi;
use rusqlite::Connection;

/// Schema version stored in `PRAGMA user_version`.
pub const SCHEMA_VERSION: u32 = 1;

/// Full DDL from design.md "Physical Data Model (SQLite, WAL)", made
/// idempotent with `IF NOT EXISTS`. Kept as one `const &str` so the Req 13.4
/// non-test LOC cap counts it once (tasks.md 2.1). `observations_fts` is an
/// FTS5 external-content table over `observations` (`content_rowid='id'`),
/// kept in sync by the `obs_ai` AFTER INSERT trigger (Req 5.5: a recorded
/// observation is searchable immediately).
pub const SCHEMA_DDL: &str = r#"
CREATE TABLE IF NOT EXISTS active_sessions (
  project TEXT PRIMARY KEY,
  workspace_task_id TEXT NOT NULL,
  active_objective TEXT,
  working_state_md TEXT NOT NULL,
  last_harness TEXT NOT NULL,
  updated_at_epoch INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS session_breadcrumbs (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  project TEXT NOT NULL,
  workspace_task_id TEXT NOT NULL,
  agent_harness TEXT NOT NULL,
  tool_name TEXT NOT NULL,
  target_resource TEXT,
  action_summary TEXT NOT NULL,
  created_at_epoch INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_breadcrumbs_proj ON session_breadcrumbs(project, created_at_epoch DESC);
CREATE TABLE IF NOT EXISTS observations (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  project TEXT NOT NULL,
  workspace_task_id TEXT NOT NULL,
  agent_harness TEXT NOT NULL,
  type TEXT NOT NULL,
  title TEXT NOT NULL,
  narrative TEXT NOT NULL,
  facts TEXT,
  files_touched TEXT,
  created_at_epoch INTEGER NOT NULL
);
CREATE VIRTUAL TABLE IF NOT EXISTS observations_fts USING fts5(
  title, narrative, facts, files_touched,
  content='observations', content_rowid='id'
);
CREATE TRIGGER IF NOT EXISTS obs_ai AFTER INSERT ON observations BEGIN
  INSERT INTO observations_fts(rowid, title, narrative, facts, files_touched)
  VALUES (new.id, new.title, new.narrative, new.facts, new.files_touched);
END;
CREATE TABLE IF NOT EXISTS shunt_file_cache (
  file_path TEXT NOT NULL,
  content_sha256 TEXT NOT NULL,
  query_fingerprint TEXT NOT NULL,
  structural_summary TEXT NOT NULL,
  line_count INTEGER NOT NULL,
  created_at_epoch INTEGER NOT NULL,
  PRIMARY KEY (file_path, content_sha256, query_fingerprint)
);
CREATE TABLE IF NOT EXISTS token_stats (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  project TEXT NOT NULL,
  workspace_task_id TEXT,
  agent_harness TEXT NOT NULL,
  kind TEXT NOT NULL,               -- shunt_delegated | shunt_cache_hit | code_write | client_reported
  ts_epoch_ms INTEGER NOT NULL,
  avoided_input_tokens_est INTEGER NOT NULL DEFAULT 0,   -- chars/4 heuristic, labelled estimate
  avoided_output_tokens_est INTEGER NOT NULL DEFAULT 0,
  worker_prompt_tokens INTEGER,     -- from worker API `usage`, NULL for client_reported
  worker_completion_tokens INTEGER,
  details TEXT                      -- redacted JSON (paths, hashes, source label)
  -- `id INTEGER PRIMARY KEY AUTOINCREMENT` above is the PK; the design
  -- block's redundant table-level `PRIMARY KEY (id)` is dropped because
  -- SQLite rejects a table with two PRIMARY KEY clauses.
);
CREATE INDEX IF NOT EXISTS idx_token_stats_proj ON token_stats(project, agent_harness, ts_epoch_ms);
"#;

/// Applies the schema idempotently and stamps `PRAGMA user_version`.
pub fn apply(conn: &Connection) -> rusqlite::Result<()> {
    let current: u32 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
    match current.cmp(&SCHEMA_VERSION) {
        Ordering::Equal => Ok(()),
        Ordering::Greater => Err(newer_schema_error(current)),
        Ordering::Less => {
            conn.execute_batch(SCHEMA_DDL)?;
            conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
            Ok(())
        }
    }
}

/// Error for an on-disk schema newer than this binary understands.
fn newer_schema_error(found: u32) -> rusqlite::Error {
    rusqlite::Error::SqliteFailure(
        ffi::Error::new(ffi::SQLITE_ERROR),
        Some(format!(
            "on-disk schema user_version {found} is newer than this binary supports \
             ({SCHEMA_VERSION}); refusing to open"
        )),
    )
}
