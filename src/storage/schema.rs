//! SQLite DDL and idempotent schema application (task 2.1).
//!
//! The DDL is the "Physical Data Model (SQLite, WAL)" block from design.md,
//! made idempotent with `IF NOT EXISTS`. Schema versioning is a single
//! `PRAGMA user_version` — there is deliberately no migration framework
//! (design.md "Complexity Budget"); a database stamped with a NEWER schema
//! version is refused, never silently downgraded.

use std::cmp::Ordering;

use rusqlite::Connection;
use rusqlite::ffi;

/// Schema version stored in `PRAGMA user_version`. v2 (shunt-content-tool
/// task 1.2, Req 4.1/4.5) adds the `shunt_content_cache` table and its LRU
/// index additatively: the DDL below is idempotent (`IF NOT EXISTS`), so an
/// on-disk v1 store is upgraded in place without touching existing tables
/// or rows — `apply` re-runs the batch because the stamped version is
/// older, then stamps the current version. v3 (shunt-cache-efficiency
/// task 1.2) keeps file-read summaries in `shunt_content_cache`: it adds
/// `last_path` and copies every `shunt_file_cache` row over once
/// ([`migrate_file_cache`]); the old table stays but is no longer used.
pub const SCHEMA_VERSION: u32 = 3;

/// First version whose file-read summaries live in `shunt_content_cache`.
const SHARED_SUMMARY_VERSION: u32 = 3;

/// Copies the old file cache into the shared table (design.md "Summary
/// cache"): kind `file_read`, the LRU clock starts at the creation time,
/// no hits, the old path becomes `last_path`. When copies of one content
/// share a question, the newest row wins.
const COPY_FILE_CACHE: &str = "INSERT OR IGNORE INTO shunt_content_cache \
    (content_sha256, query_fingerprint, kind, structural_summary, line_count, \
     created_at_epoch, last_hit_epoch, hits, last_path) \
    SELECT content_sha256, query_fingerprint, 'file_read', structural_summary, line_count, \
     created_at_epoch, created_at_epoch, 0, file_path \
    FROM shunt_file_cache ORDER BY created_at_epoch DESC, file_path";

/// Outcome of the one-time v3 copy, for the caller's `CACHE_MIGRATED`
/// audit record (the audit log is written outside the SQL transaction).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CacheMigration {
    /// The on-disk version the store was opened at.
    pub from_version: u32,
    /// Rows copied into `shunt_content_cache`.
    pub rows: u64,
}

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
CREATE TABLE IF NOT EXISTS shunt_content_cache (
  content_sha256 TEXT NOT NULL,
  query_fingerprint TEXT NOT NULL,
  kind TEXT NOT NULL,               -- v1: file_read (ContentKind::as_str)
  structural_summary TEXT NOT NULL,
  line_count INTEGER NOT NULL,
  created_at_epoch INTEGER NOT NULL,
  last_hit_epoch INTEGER NOT NULL,  -- LRU clock: set at store, touched at hit
  hits INTEGER NOT NULL DEFAULT 0,
  last_path TEXT,                   -- v3: last file read path (prune, audit); NULL for content rows
  PRIMARY KEY (content_sha256, query_fingerprint, kind)
);
CREATE INDEX IF NOT EXISTS idx_shunt_content_cache_lru ON shunt_content_cache (last_hit_epoch, created_at_epoch);
"#;

/// Applies the schema idempotently and stamps `PRAGMA user_version`. The
/// caller runs it inside one transaction, so the DDL, the v3 copy and the
/// stamp land together or not at all. Returns the copy's outcome when an
/// existing older store was migrated (a fresh store reports none).
pub fn apply(conn: &Connection) -> rusqlite::Result<Option<CacheMigration>> {
    let current: u32 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
    match current.cmp(&SCHEMA_VERSION) {
        Ordering::Equal => Ok(None),
        Ordering::Greater => Err(newer_schema_error(current)),
        Ordering::Less => {
            conn.execute_batch(SCHEMA_DDL)?;
            let migrated = migrate_file_cache(conn, current)?;
            conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
            Ok(migrated)
        }
    }
}

/// The v3 step for a store below [`SHARED_SUMMARY_VERSION`]: add
/// `last_path` when missing (an old table predates it), then copy the old
/// file cache. Idempotent: the column check and `INSERT OR IGNORE` make a
/// repeated run a no-op.
fn migrate_file_cache(conn: &Connection, from: u32) -> rusqlite::Result<Option<CacheMigration>> {
    if from >= SHARED_SUMMARY_VERSION {
        return Ok(None);
    }
    let has_last_path = conn
        .prepare("SELECT 1 FROM pragma_table_info('shunt_content_cache') WHERE name = 'last_path'")?
        .exists([])?;
    if !has_last_path {
        conn.execute_batch("ALTER TABLE shunt_content_cache ADD COLUMN last_path TEXT")?;
    }
    let rows = conn.execute(COPY_FILE_CACHE, [])? as u64;
    // A brand-new store (version 0, empty old table) migrated nothing.
    Ok((from > 0 || rows > 0).then_some(CacheMigration {
        from_version: from,
        rows,
    }))
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
