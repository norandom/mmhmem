//! Bounded initialization of the existing WAL/schema connection contract.

use anyhow::{Context, Result};
use rusqlite::{Connection, ErrorCode, TransactionBehavior};
use std::time::{Duration, Instant};

use super::schema::{self, CacheMigration};

/// Sets WAL and applies the schema in one transaction; returns the v3
/// cache copy's outcome when an older store was migrated.
pub(super) fn initialize(conn: &mut Connection, timeout_ms: u64) -> Result<Option<CacheMigration>> {
    // SQLite may skip its busy handler during journal-mode lock promotion.
    // One outer deadline governs all startup attempts, not one timeout per SQL.
    conn.busy_timeout(Duration::ZERO)?;
    let initialized = retry_initialization(timeout_ms, || initialize_once(conn));
    let restored = conn.busy_timeout(Duration::from_millis(timeout_ms));
    let migrated = initialized?;
    restored.context("restoring SQLite busy timeout")?;
    Ok(migrated)
}

fn retry_initialization<T>(timeout_ms: u64, operation: impl FnMut() -> Result<T>) -> Result<T> {
    let started = Instant::now();
    retry_with_timing(
        timeout_ms,
        operation,
        || started.elapsed(),
        std::thread::sleep,
    )
}

fn retry_with_timing<T>(
    timeout_ms: u64,
    mut operation: impl FnMut() -> Result<T>,
    elapsed: impl Fn() -> Duration,
    mut sleep: impl FnMut(Duration),
) -> Result<T> {
    let budget = Duration::from_millis(timeout_ms);
    loop {
        match operation() {
            Ok(value) => return Ok(value),
            Err(error) => {
                let busy = matches!(error.downcast_ref::<rusqlite::Error>(),
                    Some(rusqlite::Error::SqliteFailure(failure, _)) if failure.code == ErrorCode::DatabaseBusy);
                if !busy {
                    return Err(error);
                }
                let remaining = budget.saturating_sub(elapsed());
                if !remaining.is_zero() {
                    sleep(remaining.min(Duration::from_millis(10)));
                    if elapsed() < budget {
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

fn initialize_once(conn: &mut Connection) -> Result<Option<CacheMigration>> {
    conn.pragma_update(None, "journal_mode", "WAL")
        .context("setting SQLite journal_mode=WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")
        .context("setting SQLite synchronous=NORMAL")?;
    let transaction = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .context("beginning SQLite schema initialization")?;
    let migrated = schema::apply(&transaction).context("applying SQLite schema")?;
    transaction
        .commit()
        .context("committing SQLite schema initialization")?;
    Ok(migrated)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn immediate_busy_is_retried_even_without_sqlite_busy_handler() {
        let mut attempts = 0;
        let elapsed = std::cell::Cell::new(Duration::ZERO);
        let result = retry_with_timing(
            100,
            || {
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
            },
            || elapsed.get(),
            |delay| elapsed.set(elapsed.get() + delay),
        );
        assert!(
            result.is_ok(),
            "immediate busy contention must retry: {result:?}"
        );
        assert_eq!(attempts, 3);
        assert_eq!(elapsed.get(), Duration::from_millis(20));
    }

    #[test]
    fn oversleep_exhausts_budget_without_an_extra_attempt() {
        let elapsed = std::cell::Cell::new(Duration::ZERO);
        let mut attempts = 0;
        let result: Result<()> = retry_with_timing(
            100,
            || {
                attempts += 1;
                Err(rusqlite::Error::SqliteFailure(
                    rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_BUSY),
                    None,
                )
                .into())
            },
            || elapsed.get(),
            |_| elapsed.set(Duration::from_millis(101)),
        );
        assert!(format!("{:#}", result.unwrap_err()).contains("100 ms"));
        assert_eq!(attempts, 1);
    }

    #[test]
    fn zero_timeout_attempts_initialization_once() {
        let mut attempts = 0;
        let result: Result<()> = retry_initialization(0, || {
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
        let result: Result<()> = retry_initialization(5000, || {
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
