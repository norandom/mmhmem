//! Bounded initialization of the existing WAL/schema connection contract.

use anyhow::{Context, Result};
use rusqlite::{Connection, ErrorCode, TransactionBehavior};
use std::time::{Duration, Instant};

use super::schema;

pub(super) fn initialize(conn: &mut Connection, timeout_ms: u64) -> Result<()> {
    // SQLite may skip its busy handler during journal-mode lock promotion.
    // One outer deadline governs all startup attempts, not one timeout per SQL.
    conn.busy_timeout(Duration::ZERO)?;
    let initialized = retry_initialization(timeout_ms, || initialize_once(conn));
    let restored = conn.busy_timeout(Duration::from_millis(timeout_ms));
    initialized?;
    restored.context("restoring SQLite busy timeout")
}

fn retry_initialization(timeout_ms: u64, mut operation: impl FnMut() -> Result<()>) -> Result<()> {
    let started = Instant::now();
    let budget = Duration::from_millis(timeout_ms);
    loop {
        match operation() {
            Ok(()) => return Ok(()),
            Err(error) => {
                let busy = matches!(error.downcast_ref::<rusqlite::Error>(),
                    Some(rusqlite::Error::SqliteFailure(failure, _)) if failure.code == ErrorCode::DatabaseBusy);
                if !busy {
                    return Err(error);
                }
                let remaining = budget.saturating_sub(started.elapsed());
                if !remaining.is_zero() {
                    std::thread::sleep(remaining.min(Duration::from_millis(10)));
                    if started.elapsed() < budget {
                        continue;
                    }
                }
                return Err(error.context(format!(
                    "SQLite initialization lock-wait timeout after {timeout_ms} ms"
                )));
            }
        }
    }
}

fn initialize_once(conn: &mut Connection) -> Result<()> {
    conn.pragma_update(None, "journal_mode", "WAL")
        .context("setting SQLite journal_mode=WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")
        .context("setting SQLite synchronous=NORMAL")?;
    let transaction = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .context("beginning SQLite schema initialization")?;
    schema::apply(&transaction).context("applying SQLite schema")?;
    transaction
        .commit()
        .context("committing SQLite schema initialization")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn immediate_busy_is_retried_even_without_sqlite_busy_handler() {
        let mut attempts = 0;
        let result = retry_initialization(100, || {
            attempts += 1;
            if attempts < 3 {
                Err(rusqlite::Error::SqliteFailure(
                    rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_BUSY),
                    None,
                )
                .into())
            } else {
                Ok(())
            }
        });
        assert!(
            result.is_ok(),
            "immediate busy contention must retry: {result:?}"
        );
        assert_eq!(attempts, 3);
    }

    #[test]
    fn zero_timeout_attempts_initialization_once() {
        let mut attempts = 0;
        let result = retry_initialization(0, || {
            attempts += 1;
            Err(rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_BUSY),
                None,
            )
            .into())
        });
        assert!(format!("{:#}", result.unwrap_err()).contains("0 ms"));
        assert_eq!(attempts, 1);
    }

    #[test]
    fn nonbusy_errors_are_not_retried() {
        let mut attempts = 0;
        let result = retry_initialization(5000, || {
            attempts += 1;
            Err(rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_CORRUPT),
                None,
            )
            .into())
        });
        assert!(result.is_err());
        assert_eq!(attempts, 1);
    }
}
