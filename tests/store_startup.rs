//! local-mode task 2.1: bounded first-open contention (Requirements 6.1–6.4).

use std::path::Path;
use std::sync::{Arc, Barrier, mpsc};
use std::time::{Duration, Instant};

use mmhmem::config::{Config, ENV_ALLOWED_ROOTS, ENV_BUSY_TIMEOUT_MS, ENV_DATA_DIR, ENV_TOKEN};
use mmhmem::storage::db::{DB_FILE_NAME, Db};
use mmhmem::storage::schema::SCHEMA_VERSION;
use rusqlite::Connection;

fn open_with_timeout(root: &Path, milliseconds: u64) -> anyhow::Result<Db> {
    let config = Config::from_env(&|key| match key {
        ENV_TOKEN => Some("startup-fixture".into()),
        ENV_ALLOWED_ROOTS => Some(root.display().to_string()),
        ENV_DATA_DIR => Some(root.display().to_string()),
        ENV_BUSY_TIMEOUT_MS => Some(milliseconds.to_string()),
        _ => None,
    })?;
    Db::open_for_config(&config)
}

fn hold_rollback_reader(root: &Path) -> Connection {
    let holder = Connection::open(root.join(DB_FILE_NAME)).unwrap();
    holder
        .execute_batch("CREATE TABLE fixture_lock (value); BEGIN; SELECT * FROM fixture_lock;")
        .unwrap();
    holder
}

#[test]
fn wal_transition_waits_for_reader_that_clears_within_timeout() {
    let directory = tempfile::tempdir().unwrap();
    let holder = hold_rollback_reader(directory.path());
    let path = directory.path().to_path_buf();
    let (sender, receiver) = mpsc::channel();
    let child = std::thread::spawn(move || sender.send(open_with_timeout(&path, 1000)).unwrap());
    let early_result = receiver.recv_timeout(Duration::from_millis(100));
    holder.execute_batch("ROLLBACK").unwrap();
    let result = match early_result {
        Ok(result) => {
            child.join().unwrap();
            panic!(
                "WAL initialization returned before its reader cleared: {:?}",
                result.err()
            );
        }
        Err(mpsc::RecvTimeoutError::Timeout) => {
            receiver.recv_timeout(Duration::from_secs(3)).unwrap()
        }
        Err(error) => panic!("initialization worker disconnected: {error}"),
    };
    child.join().unwrap();
    assert!(
        result.is_ok(),
        "initialization should recover: {:?}",
        result.err()
    );
}

#[test]
fn busy_initialization_expires_without_partial_schema() {
    let directory = tempfile::tempdir().unwrap();
    let holder = hold_rollback_reader(directory.path());
    let started = Instant::now();
    let error = open_with_timeout(directory.path(), 120)
        .err()
        .expect("held reader must time out");
    let elapsed = started.elapsed();
    holder.execute_batch("ROLLBACK").unwrap();
    assert!(
        elapsed >= Duration::from_millis(100),
        "failed immediately: {elapsed:?}: {error:#}"
    );
    assert!(
        elapsed < Duration::from_secs(2),
        "configured deadline was exceeded: {elapsed:?}"
    );
    assert!(
        format!("{error:#}").contains("120 ms"),
        "timeout context missing: {error:#}"
    );
    let tables: i64 = holder
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE name='observations'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let version: u32 = holder
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(
        (tables, version),
        (0, 0),
        "timed-out startup partially applied schema"
    );
    drop(open_with_timeout(directory.path(), 1000).unwrap());
}

#[test]
fn newer_schema_is_rejected_without_waiting_or_downgrade() {
    let directory = tempfile::tempdir().unwrap();
    let connection = Connection::open(directory.path().join(DB_FILE_NAME)).unwrap();
    connection.pragma_update(None, "user_version", 999).unwrap();
    let started = Instant::now();
    let error = open_with_timeout(directory.path(), 5000)
        .err()
        .expect("newer schema must fail");
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "non-contention error was retried"
    );
    assert!(format!("{error:#}").contains("newer"));
    let version: u32 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, 999);
}

#[test]
fn eight_concurrent_first_opens_preserve_a_complete_store() {
    let directory = tempfile::tempdir().unwrap();
    let barrier = Arc::new(Barrier::new(8));
    let workers: Vec<_> = (0..8)
        .map(|_| {
            let path = directory.path().to_path_buf();
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                open_with_timeout(&path, 5000)
            })
        })
        .collect();
    for worker in workers {
        assert!(
            worker.join().unwrap().is_ok(),
            "concurrent first open failed"
        );
    }
    let connection = Connection::open(directory.path().join(DB_FILE_NAME)).unwrap();
    let integrity: String = connection
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .unwrap();
    assert_eq!(integrity, "ok");
    let version: u32 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    // Pinned to the constant, not a literal: shunt-content-tool task 1.2
    // bumps the schema version to add `shunt_content_cache` additatively
    // (design.md "Revalidation Triggers": schema version bump policy →
    // store_startup tests).
    assert_eq!(version, SCHEMA_VERSION);
}

#[test]
fn schema_initialization_failure_rolls_back_all_new_objects() {
    let directory = tempfile::tempdir().unwrap();
    let connection = Connection::open(directory.path().join(DB_FILE_NAME)).unwrap();
    connection
        .execute_batch("CREATE VIEW observations AS SELECT 1 AS id")
        .unwrap();
    let result = open_with_timeout(directory.path(), 1000);
    assert!(result.is_err(), "incompatible schema fixture must fail");
    let count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE name='active_sessions'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let version: u32 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(
        (count, version),
        (0, 0),
        "schema error committed partial initialization"
    );
}

#[tokio::test]
async fn preexisting_store_gains_content_cache_additively() {
    // shunt-content-tool task 1.2 / Req 4.5: opening a v1 store must add
    // `shunt_content_cache` and its LRU index without altering existing
    // tables or data. The fixture hand-builds a v1 store (one real v1
    // table, a seeded row, user_version=1) exactly like an older binary
    // would have left it.
    let directory = tempfile::tempdir().unwrap();
    {
        let connection = Connection::open(directory.path().join(DB_FILE_NAME)).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE shunt_file_cache (file_path TEXT NOT NULL, \
                 content_sha256 TEXT NOT NULL, query_fingerprint TEXT NOT NULL, \
                 structural_summary TEXT NOT NULL, line_count INTEGER NOT NULL, \
                 created_at_epoch INTEGER NOT NULL, \
                 PRIMARY KEY (file_path, content_sha256, query_fingerprint)); \
                 INSERT INTO shunt_file_cache VALUES \
                 ('src/a.rs', 'sha-a', 'fp1', 'summary one', 12, 111); \
                 PRAGMA user_version = 1;",
            )
            .unwrap();
    }
    let db = open_with_timeout(directory.path(), 1000).unwrap();
    let (table, index, seeded, version): (i64, i64, String, u32) = db
        .with_conn(|c| {
            let table: i64 = c.query_row(
                "SELECT COUNT(*) FROM sqlite_master \
                  WHERE type='table' AND name='shunt_content_cache'",
                [],
                |r| r.get(0),
            )?;
            let index: i64 = c.query_row(
                "SELECT COUNT(*) FROM sqlite_master \
                  WHERE type='index' AND name='idx_shunt_content_cache_lru'",
                [],
                |r| r.get(0),
            )?;
            let seeded: String = c.query_row(
                "SELECT structural_summary FROM shunt_file_cache WHERE file_path = 'src/a.rs'",
                [],
                |r| r.get(0),
            )?;
            let version: u32 = c.pragma_query_value(None, "user_version", |r| r.get(0))?;
            Ok((table, index, seeded, version))
        })
        .await
        .unwrap();
    assert_eq!((table, index), (1, 1), "table and LRU index must be added");
    assert_eq!(seeded, "summary one", "pre-existing data must survive");
    assert_eq!(version, SCHEMA_VERSION);
}
