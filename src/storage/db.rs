//! The `Db` component (design.md "storage layer"): one embedded SQLite
//! connection per process in WAL mode, schema applied at open, and an
//! async-friendly accessor that never holds the DB lock across an `.await`.
//!
//! Concurrency model (local-mode supersession of the upstream invariant):
//! EACH PROCESS opens its own connection to the shared store. Within a
//! process, the single connection is owned in an `Arc<std::sync::Mutex<
//! Connection>>` and is ONLY ever touched inside `tokio::task::
//! spawn_blocking` closures run through [`Db::with_conn`]; the mutex guard
//! lives entirely inside the blocking task, so the DB lock is NEVER held
//! across an `.await`, and blocking SQLite work never runs on an async
//! worker thread. ACROSS processes, safety comes from SQLite itself: WAL
//! plus `busy_timeout` (Req 6.2, default 5 s, `AGENT_MEM_BUSY_TIMEOUT_MS`)
//! serialize writers with bounded waiting — no lock files, no coordination
//! service (Req 6.4). Single-writer WAL is correct under concurrent
//! harnesses (Req 9.1) and eliminates locking bugs; local-only durable
//! state (Req 9.3).

use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use anyhow::{Context, Result};
use rusqlite::Connection;

use super::schema;
use crate::config::{Config, DEFAULT_BUSY_TIMEOUT_MS};

/// Database file name inside the configured data dir (`<data_dir>/memory.db`).
pub const DB_FILE_NAME: &str = "memory.db";

/// Shared handle to the embedded database. Cheap to clone (`Arc` inside).
#[derive(Clone)]
pub struct Db {
    conn: Arc<Mutex<Connection>>,
}

impl Db {
    /// Opens `<config.data_dir>/memory.db` (Req 7.5 env-configurable data
    /// dir) with the profile's busy timeout (`AGENT_MEM_BUSY_TIMEOUT_MS`).
    pub fn open_for_config(config: &Config) -> Result<Db> {
        Self::open_with_timeout(&config.data_dir, config.busy_timeout_ms)
    }

    /// Opens the database in `data_dir` with the default busy timeout,
    /// creating the directory if missing.
    pub fn open(data_dir: &Path) -> Result<Db> {
        Self::open_with_timeout(data_dir, DEFAULT_BUSY_TIMEOUT_MS)
    }

    /// Opens `<data_dir>/memory.db`, creating the directory if missing.
    fn open_with_timeout(data_dir: &Path, busy_timeout_ms: u64) -> Result<Db> {
        std::fs::create_dir_all(data_dir)
            .with_context(|| format!("cannot create data dir {}", data_dir.display()))?;
        Self::open_at_with_timeout(&data_dir.join(DB_FILE_NAME), busy_timeout_ms)
    }

    /// Opens the database file at `db_path` (its parent directory must
    /// exist) with the default busy timeout.
    pub fn open_at(db_path: &Path) -> Result<Db> {
        Self::open_at_with_timeout(db_path, DEFAULT_BUSY_TIMEOUT_MS)
    }

    /// Read-only sibling of [`Db::open_at`] for reporting surfaces (Req
    /// 8.5/8.6): never creates files, never applies the schema. `Ok(None)`
    /// reports a store that does not exist yet. Bounded busy-waiting still
    /// applies (Req 6.2) so a read alongside an active writer waits rather
    /// than failing.
    pub fn open_read_only(db_path: &Path) -> Result<Option<Db>> {
        if !db_path.is_file() {
            return Ok(None);
        }
        let conn = rusqlite::Connection::open_with_flags(
            db_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .with_context(|| format!("cannot open sqlite db read-only {}", db_path.display()))?;
        conn.busy_timeout(Duration::from_millis(DEFAULT_BUSY_TIMEOUT_MS))
            .with_context(|| format!("cannot set busy timeout on {}", db_path.display()))?;
        Ok(Some(Db {
            conn: Arc::new(Mutex::new(conn)),
        }))
    }

    /// Opens the database file at `db_path`, sets WAL, `synchronous=NORMAL`
    /// and `busy_timeout`, and applies the schema idempotently. The timeout
    /// is set BEFORE the schema batch so a concurrent first-open by another
    /// process waits instead of failing (Req 6.1/6.2).
    fn open_at_with_timeout(db_path: &Path, busy_timeout_ms: u64) -> Result<Db> {
        let conn = Connection::open(db_path)
            .with_context(|| format!("cannot open sqlite db {}", db_path.display()))?;
        conn.busy_timeout(Duration::from_millis(busy_timeout_ms))
            .with_context(|| format!("cannot set busy timeout on {}", db_path.display()))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        schema::apply(&conn)?;
        Ok(Db {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Runs `f` against the connection on a blocking thread and returns its
    /// result. The mutex guard lives only inside the `spawn_blocking`
    /// closure, so callers can never hold the DB lock across an `.await`;
    /// each call is one short locked section (design.md "Db" component).
    pub async fn with_conn<R>(
        &self,
        f: impl FnOnce(&Connection) -> rusqlite::Result<R> + Send + 'static,
    ) -> Result<R>
    where
        R: Send + 'static,
    {
        let conn = Arc::clone(&self.conn);
        let outcome = tokio::task::spawn_blocking(move || {
            let guard = conn.lock().unwrap_or_else(PoisonError::into_inner);
            f(&guard)
        })
        .await
        .context("storage blocking task panicked")?;
        outcome.map_err(anyhow::Error::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn open_temp() -> (tempfile::TempDir, Db) {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(dir.path()).unwrap();
        (dir, db)
    }

    fn insert_observation(conn: &Connection) -> rusqlite::Result<i64> {
        conn.execute(
            "INSERT INTO observations (project, workspace_task_id, agent_harness, type, \
             title, narrative, facts, files_touched, created_at_epoch) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            rusqlite::params![
                "proj",
                "task-1",
                "opencode",
                "discovery",
                "obs_ai trigger smoke",
                "the AFTER INSERT trigger feeds observations_fts immediately",
                "fact one",
                "src/db.rs",
                1_700_000_000_i64,
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    async fn fts_rowids(db: &Db, phrase: &str) -> Vec<i64> {
        let phrase = phrase.to_string();
        db.with_conn(move |c| {
            let mut stmt =
                c.prepare("SELECT rowid FROM observations_fts WHERE observations_fts MATCH ?1")?;
            let rows = stmt.query_map(rusqlite::params![phrase], |r| r.get(0))?;
            rows.collect()
        })
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn fresh_db_has_all_tables_indexes_and_trigger() {
        let (_dir, db) = open_temp();
        let objects: BTreeSet<(String, String)> = db
            .with_conn(|c| {
                let mut stmt = c.prepare("SELECT type, name FROM sqlite_master")?;
                let rows =
                    stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
                let mut out = BTreeSet::new();
                for row in rows {
                    out.insert(row?);
                }
                Ok(out)
            })
            .await
            .unwrap();
        let has = |typ: &str, name: &str| objects.iter().any(|(t, n)| t == typ && n == name);
        for name in [
            "active_sessions",
            "session_breadcrumbs",
            "observations",
            "observations_fts",
            "shunt_file_cache",
            "token_stats",
        ] {
            assert!(has("table", name), "missing table: {name}");
        }
        assert!(has("index", "idx_breadcrumbs_proj"));
        assert!(has("index", "idx_token_stats_proj"));
        assert!(has("trigger", "obs_ai"));
        drop(db);
    }

    #[tokio::test]
    async fn fts5_smoke_query_works() {
        let (_dir, db) = open_temp();
        db.with_conn(|c| {
            c.query_row("SELECT fts5(?)", ["x"], |r| {
                r.get::<_, rusqlite::types::Value>(0)
            })
        })
        .await
        .unwrap();
        drop(db);
    }

    #[tokio::test]
    async fn pragmas_are_wal_normal_and_user_version_is_stamped() {
        let (_dir, db) = open_temp();
        let (journal, sync, version): (String, i64, u32) = db
            .with_conn(|c| {
                let journal: String = c.query_row("PRAGMA journal_mode", [], |r| r.get(0))?;
                let sync: i64 = c.query_row("PRAGMA synchronous", [], |r| r.get(0))?;
                let version: u32 = c.pragma_query_value(None, "user_version", |r| r.get(0))?;
                Ok((journal, sync, version))
            })
            .await
            .unwrap();
        assert!(journal.eq_ignore_ascii_case("wal"), "journal={journal}");
        assert_eq!(sync, 1, "synchronous=NORMAL is pragma value 1");
        assert_eq!(version, schema::SCHEMA_VERSION);
        drop(db);
    }

    #[tokio::test]
    async fn busy_timeout_is_set_at_open() {
        // Req 6.2: multi-process safety — the connection waits for a busy
        // store up to the configurable timeout (default 5 s) instead of
        // failing immediately.
        let (_dir, db) = open_temp();
        let timeout_ms: i64 = db
            .with_conn(|c| c.query_row("PRAGMA busy_timeout", [], |r| r.get(0)))
            .await
            .unwrap();
        assert_eq!(
            timeout_ms,
            i64::try_from(crate::config::DEFAULT_BUSY_TIMEOUT_MS).unwrap()
        );
        drop(db);
    }

    #[tokio::test]
    async fn busy_timeout_config_override_applies() {
        let tmp = tempfile::tempdir().unwrap();
        let mut env = std::collections::HashMap::new();
        env.insert(crate::config::ENV_TOKEN.to_string(), "tok".to_string());
        env.insert(
            crate::config::ENV_ALLOWED_ROOTS.to_string(),
            "/ws".to_string(),
        );
        env.insert(
            crate::config::ENV_BUSY_TIMEOUT_MS.to_string(),
            "12000".to_string(),
        );
        env.insert(
            crate::config::ENV_DATA_DIR.to_string(),
            tmp.path().to_string_lossy().into_owned(),
        );
        let cfg = Config::from_env(&|k| env.get(k).cloned()).unwrap();
        let db = Db::open_for_config(&cfg).unwrap();
        let timeout_ms: i64 = db
            .with_conn(|c| c.query_row("PRAGMA busy_timeout", [], |r| r.get(0)))
            .await
            .unwrap();
        assert_eq!(timeout_ms, 12_000);
        drop(db);
    }

    #[tokio::test]
    async fn observation_insert_is_immediately_fts_searchable() {
        let (_dir, db) = open_temp();
        let id = db.with_conn(insert_observation).await.unwrap();
        let hits = fts_rowids(&db, "\"trigger\"").await;
        assert_eq!(hits, vec![id]);
        drop(db);
    }

    #[tokio::test]
    async fn with_conn_surfaces_sqlite_errors() {
        let (_dir, db) = open_temp();
        let res = db
            .with_conn(|c| c.execute_batch("INSERT INTO no_such_table VALUES (1)"))
            .await;
        assert!(res.is_err());
        drop(db);
    }

    #[tokio::test]
    async fn reopen_is_idempotent_and_rows_survive() {
        let dir = tempfile::tempdir().unwrap();
        let id = {
            let db = Db::open(dir.path()).unwrap();
            let id = db.with_conn(insert_observation).await.unwrap();
            drop(db);
            id
        };
        let db = Db::open(dir.path()).unwrap();
        let count: i64 = db
            .with_conn(|c| c.query_row("SELECT COUNT(*) FROM observations", [], |r| r.get(0)))
            .await
            .unwrap();
        assert_eq!(count, 1);
        let version: u32 = db
            .with_conn(|c| c.pragma_query_value(None, "user_version", |r| r.get(0)))
            .await
            .unwrap();
        assert_eq!(version, schema::SCHEMA_VERSION);
        assert_eq!(fts_rowids(&db, "\"trigger\"").await, vec![id]);
        drop(db);
    }

    #[tokio::test]
    async fn read_only_open_reports_missing_store_without_creating_it() {
        // Req 8.5/8.6: `mmhmem stats` must not invent a store.
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join(DB_FILE_NAME);
        let res = Db::open_read_only(&db_path).unwrap();
        assert!(res.is_none(), "missing store must be Ok(None)");
        assert!(!db_path.exists(), "no store file may be created");
        assert!(
            !dir.path().join(format!("{DB_FILE_NAME}-wal")).exists()
                && !dir.path().join(format!("{DB_FILE_NAME}-shm")).exists(),
            "no wal/shm side files may be created"
        );
    }

    #[tokio::test]
    async fn read_only_open_reads_seeded_rows_and_refuses_writes() {
        let dir = tempfile::tempdir().unwrap();
        {
            let db = Db::open(dir.path()).unwrap();
            db.with_conn(insert_observation).await.unwrap();
            drop(db);
        }
        let ro = Db::open_read_only(&dir.path().join(DB_FILE_NAME))
            .unwrap()
            .expect("seeded store must open read-only");
        let count: i64 = ro
            .with_conn(|c| c.query_row("SELECT COUNT(*) FROM observations", [], |r| r.get(0)))
            .await
            .unwrap();
        assert_eq!(count, 1, "reads work through the read-only connection");
        let write = ro
            .with_conn(|c| c.execute("DELETE FROM observations", []))
            .await;
        assert!(write.is_err(), "writes must be refused (Req 8.6)");
        // The refused write left the data intact.
        let count: i64 = ro
            .with_conn(|c| c.query_row("SELECT COUNT(*) FROM observations", [], |r| r.get(0)))
            .await
            .unwrap();
        assert_eq!(count, 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_with_conn_calls_are_serialized_without_corruption() {
        let (_dir, db) = open_temp();
        let mut handles = Vec::new();
        for i in 0..16_i64 {
            let db = db.clone();
            handles.push(tokio::spawn(async move {
                db.with_conn(move |c| {
                    c.execute(
                        "INSERT INTO session_breadcrumbs (project, workspace_task_id, \
                         agent_harness, tool_name, target_resource, action_summary, \
                         created_at_epoch) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                        rusqlite::params![
                            "proj",
                            "task-1",
                            "harness",
                            "tool",
                            None::<&str>,
                            format!("call {i}"),
                            i,
                        ],
                    )
                })
                .await
                .unwrap();
            }));
        }
        for h in handles {
            h.await.unwrap();
        }
        let count: i64 = db
            .with_conn(|c| {
                c.query_row("SELECT COUNT(*) FROM session_breadcrumbs", [], |r| r.get(0))
            })
            .await
            .unwrap();
        assert_eq!(count, 16);
        drop(db);
    }

    #[test]
    fn open_for_config_uses_agent_mem_data_dir_and_creates_it() {
        let tmp = tempfile::tempdir().unwrap();
        let data_dir = tmp.path().join("nested").join("data");
        let mut env = std::collections::HashMap::new();
        env.insert(crate::config::ENV_TOKEN.to_string(), "tok".to_string());
        env.insert(
            crate::config::ENV_ALLOWED_ROOTS.to_string(),
            "/ws".to_string(),
        );
        env.insert(
            crate::config::ENV_DATA_DIR.to_string(),
            data_dir.to_string_lossy().into_owned(),
        );
        let cfg = Config::from_env(&|k| env.get(k).cloned()).unwrap();
        let db = Db::open_for_config(&cfg).unwrap();
        assert!(data_dir.join(DB_FILE_NAME).is_file());
        drop(db);
    }
}
