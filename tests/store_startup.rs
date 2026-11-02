//! local-mode task 2.1: bounded first-open contention (Requirements 6.1–6.4).

use std::path::Path;
use std::sync::{Arc, Barrier, mpsc};
use std::time::{Duration, Instant};

use mmhmem::config::{Config, ENV_ALLOWED_ROOTS, ENV_BUSY_TIMEOUT_MS, ENV_DATA_DIR, ENV_TOKEN};
use mmhmem::storage::db::{DB_FILE_NAME, Db};
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
    assert_eq!(version, 1);
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
