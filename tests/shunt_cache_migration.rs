//! Schema v2 -> v3 migration of the file-read summaries (shunt-cache-efficiency
//! task 1.2; Requirements 1.5, 4.3).
//!
//! `tests/fixtures/store/v2-file-cache.db` is a store written with the v2
//! DDL (`user_version = 2`, rollback journal) holding four `shunt_file_cache`
//! rows and one observation:
//!
//! | file_path                    | content | question                   | summary                                 | created           |
//! |------------------------------|---------|----------------------------|-----------------------------------------|-------------------|
//! | /old/checkout/src/alpha.rs   | alpha   | default                    | MIGRATED CANONICAL alpha (older copy)   | 1_700_000_000_000 |
//! | /old/worktree/src/alpha.rs   | alpha   | default                    | MIGRATED CANONICAL alpha                | 1_700_000_500_000 |
//! | /old/checkout/src/alpha.rs   | alpha   | "Explain responsibilities" | MIGRATED CUSTOM alpha                   | 1_700_000_100_000 |
//! | /old/checkout/src/beta.rs    | beta    | default                    | MIGRATED CANONICAL beta                 | 1_700_000_200_000 |
//!
//! where "alpha"/"beta" are 400 lines of `"<tag> fixture line <n>\n"`. The
//! two alpha copies collapse into one row (the newest wins), so three rows
//! are copied. Tests always work on a copy of the fixture.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use mmhmem::config::{
    Config, ENV_ALLOWED_ROOTS, ENV_DATA_DIR, ENV_LOG_FILE, ENV_SHUNT_MIN_LINES, ENV_TOKEN,
    ENV_WORKER_API_URL, ENV_WORKER_MODEL,
};
use mmhmem::services::shunt::{ReadOutcome, ShuntEngine};
use mmhmem::services::shunt_types::ReadRequest;
use mmhmem::storage::audit::AuditWriter;
use mmhmem::storage::db::{DB_FILE_NAME, Db};
use mmhmem::storage::schema::SCHEMA_VERSION;
use serde_json::Value;

const FIXTURE: &str = "tests/fixtures/store/v2-file-cache.db";

struct Store {
    _temp: tempfile::TempDir,
    repo: PathBuf,
    config: Config,
}

impl Store {
    /// A copy of the v2 fixture plus the two files whose content it caches.
    fn from_fixture() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let data = temp.path().join("data");
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&data).unwrap();
        std::fs::create_dir_all(&repo).unwrap();
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join(FIXTURE);
        std::fs::copy(fixture, data.join(DB_FILE_NAME)).unwrap();
        for tag in ["alpha", "beta"] {
            let body: String = (1..=400)
                .map(|i| format!("{tag} fixture line {i}\n"))
                .collect();
            std::fs::write(repo.join(format!("{tag}.rs")), body).unwrap();
        }
        let config = store_config(temp.path(), &repo);
        Self {
            _temp: temp,
            repo,
            config,
        }
    }

    fn open(&self) -> Db {
        Db::open_for_config(&self.config).unwrap()
    }

    fn engine(&self, db: Db) -> ShuntEngine {
        let audit = AuditWriter::from_config(&self.config);
        ShuntEngine::new(db, audit, self.config.clone(), None, None)
    }

    fn file(&self, name: &str) -> String {
        self.repo.join(name).display().to_string()
    }

    fn migrations(&self) -> Vec<Value> {
        std::fs::read_to_string(&self.config.log_file)
            .unwrap_or_default()
            .lines()
            .map(|l| serde_json::from_str::<Value>(l).unwrap())
            .filter(|r| r["event_type"] == "CACHE_MIGRATED")
            .collect()
    }
}

fn store_config(root: &Path, repo: &Path) -> Config {
    let env: HashMap<&str, String> = [
        (ENV_TOKEN, "tok".to_string()),
        (ENV_DATA_DIR, root.join("data").display().to_string()),
        (ENV_LOG_FILE, root.join("audit.jsonl").display().to_string()),
        (ENV_ALLOWED_ROOTS, repo.display().to_string()),
        (ENV_SHUNT_MIN_LINES, "150".to_string()),
        // Blank, not absent: no worker may be configured, so any worker
        // call would fail the test with WorkerNotConfigured.
        (ENV_WORKER_API_URL, String::new()),
        (ENV_WORKER_MODEL, String::new()),
    ]
    .into_iter()
    .collect();
    Config::from_env(&|k| env.get(k).cloned()).unwrap()
}

fn read<'a>(path: &'a str, question: &'a str) -> ReadRequest<'a> {
    ReadRequest {
        project: "p",
        harness: "h",
        task: None,
        path,
        question,
        offset: None,
        limit: None,
        answer_question: false,
    }
}

async fn cached_summary(engine: &ShuntEngine, path: &str, question: &str) -> String {
    match engine.smart_read(&read(path, question)).await {
        Ok(ReadOutcome::Cached { summary, note, .. }) => {
            assert!(note.is_none(), "served row answers the question: {path}");
            summary
        }
        other => panic!("expected a cache hit for {path}, got {other:?}"),
    }
}

/// (user_version, shared rows, old rows, has last_path, observations).
async fn shape(db: &Db) -> (u32, i64, i64, bool, i64) {
    db.with_conn(|c| {
        let version: u32 = c.pragma_query_value(None, "user_version", |r| r.get(0))?;
        let count = |table: &str| -> rusqlite::Result<i64> {
            c.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
        };
        let has_column = c
            .prepare(
                "SELECT 1 FROM pragma_table_info('shunt_content_cache') WHERE name = 'last_path'",
            )?
            .exists([])?;
        Ok((
            version,
            count("shunt_content_cache")?,
            count("shunt_file_cache")?,
            has_column,
            count("observations")?,
        ))
    })
    .await
    .unwrap()
}

#[test]
fn the_committed_fixture_is_a_v2_store_with_file_cache_rows() {
    let conn = rusqlite::Connection::open_with_flags(
        Path::new(env!("CARGO_MANIFEST_DIR")).join(FIXTURE),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let version: u32 = conn
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap();
    let rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM shunt_file_cache", [], |r| r.get(0))
        .unwrap();
    assert_eq!((version, rows), (2, 4));
}

#[tokio::test]
async fn a_v2_store_opens_at_v3_and_serves_copied_rows_without_a_worker() {
    let store = Store::from_fixture();
    let db = store.open();
    assert_eq!(SCHEMA_VERSION, 3);
    assert_eq!(
        shape(&db).await,
        (3, 3, 4, true, 1),
        "copied, old table kept"
    );
    let engine = store.engine(db);
    let alpha = store.file("alpha.rs");
    let beta = store.file("beta.rs");
    assert_eq!(
        cached_summary(&engine, &alpha, "").await,
        "MIGRATED CANONICAL alpha",
        "the newest of two copies wins"
    );
    assert_eq!(
        cached_summary(&engine, &beta, "").await,
        "MIGRATED CANONICAL beta"
    );
    assert_eq!(
        cached_summary(&engine, &alpha, "Explain responsibilities").await,
        "MIGRATED CUSTOM alpha"
    );
    let migrations = store.migrations();
    assert_eq!(migrations.len(), 1, "audited once: {migrations:?}");
    assert_eq!(migrations[0]["details"]["rows"], 3);
    assert_eq!(migrations[0]["details"]["from_version"], 2);
}

#[tokio::test]
async fn reopening_a_migrated_store_copies_nothing_and_audits_nothing() {
    let store = Store::from_fixture();
    drop(store.open());
    let db = store.open();
    assert_eq!(shape(&db).await, (3, 3, 4, true, 1));
    assert_eq!(store.migrations().len(), 1, "no second CACHE_MIGRATED");
}

#[tokio::test]
async fn copied_rows_keep_their_old_path_as_last_path() {
    let store = Store::from_fixture();
    let db = store.open();
    let paths: Vec<(String, Option<String>, i64)> = db
        .with_conn(|c| {
            let mut s = c.prepare(
                "SELECT structural_summary, last_path, hits FROM shunt_content_cache \
                  WHERE kind = 'file_read' ORDER BY structural_summary",
            )?;
            let mapped = s.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
            mapped.collect()
        })
        .await
        .unwrap();
    let expected = [
        ("MIGRATED CANONICAL alpha", "/old/worktree/src/alpha.rs"),
        ("MIGRATED CANONICAL beta", "/old/checkout/src/beta.rs"),
        ("MIGRATED CUSTOM alpha", "/old/checkout/src/alpha.rs"),
    ];
    let got: Vec<(&str, Option<&str>, i64)> = paths
        .iter()
        .map(|(s, p, h)| (s.as_str(), p.as_deref(), *h))
        .collect();
    let want: Vec<(&str, Option<&str>, i64)> =
        expected.iter().map(|(s, p)| (*s, Some(*p), 0)).collect();
    assert_eq!(got, want);
}

#[tokio::test]
async fn a_store_that_already_has_the_column_still_migrates() {
    // Guarded ALTER: a partially upgraded v2 store (column present) opens.
    let store = Store::from_fixture();
    {
        let path = store.config.data_dir.join(DB_FILE_NAME);
        let conn = rusqlite::Connection::open(path).unwrap();
        conn.execute_batch("ALTER TABLE shunt_content_cache ADD COLUMN last_path TEXT")
            .unwrap();
    }
    let db = store.open();
    assert_eq!(shape(&db).await, (3, 3, 4, true, 1));
}

#[tokio::test]
async fn a_fresh_store_records_no_migration() {
    let temp = tempfile::tempdir().unwrap();
    let repo = temp.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    let config = store_config(temp.path(), &repo);
    let db = Db::open_for_config(&config).unwrap();
    assert_eq!(shape(&db).await, (3, 0, 0, true, 0));
    let log = std::fs::read_to_string(&config.log_file).unwrap_or_default();
    assert!(!log.contains("CACHE_MIGRATED"), "{log}");
}
