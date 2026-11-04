//! Read-only health check behind the `status` tool (reliable-installers
//! Req 6.1, task 1.2). The installer's functional check calls it to prove a
//! harness can reach a working server. It reads the store, writes one
//! STATUS_CHECK audit record (steering: every action is audited), and never
//! writes breadcrumbs, sessions or observations.

use anyhow::Result;

use crate::storage::audit::{AuditEventType, AuditWriter};
use crate::storage::db::Db;

/// What one status call found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusReport {
    /// Server version (`CARGO_PKG_VERSION`).
    pub version: &'static str,
    /// Number of tables in the store; zero means an empty or broken store.
    pub tables: i64,
    /// SQLite journal mode; `wal` is the expected value.
    pub journal_mode: String,
}

impl StatusReport {
    /// True when the store looks usable: tables exist and WAL is on.
    pub fn store_ok(&self) -> bool {
        self.tables > 0 && self.journal_mode.eq_ignore_ascii_case("wal")
    }

    /// One-line markdown answer for the tool result.
    pub fn render(&self) -> String {
        let state = if self.store_ok() { "ok" } else { "not ready" };
        format!(
            "mmhmem {}: store {state} ({} tables, journal {})",
            self.version, self.tables, self.journal_mode
        )
    }
}

/// Status service: shared store handle plus the audit writer.
#[derive(Clone)]
pub struct StatusService {
    db: Db,
    audit: AuditWriter,
}

impl StatusService {
    pub fn new(db: Db, audit: AuditWriter) -> Self {
        Self { db, audit }
    }

    /// Reads store health and records one STATUS_CHECK audit event.
    pub async fn check(&self, harness: &str, project: &str) -> Result<StatusReport> {
        let (tables, journal_mode) = self
            .db
            .with_conn(|c| {
                let tables: i64 = c.query_row(
                    "SELECT count(*) FROM sqlite_master WHERE type = 'table'",
                    [],
                    |r| r.get(0),
                )?;
                let mode: String = c.query_row("PRAGMA journal_mode", [], |r| r.get(0))?;
                Ok((tables, mode))
            })
            .await?;
        let report = StatusReport {
            version: env!("CARGO_PKG_VERSION"),
            tables,
            journal_mode,
        };
        let _ = self.audit.write_event(
            AuditEventType::StatusCheck,
            harness,
            project,
            serde_json::json!({"version": report.version, "store_ok": report.store_ok()}),
        );
        Ok(report)
    }
}
