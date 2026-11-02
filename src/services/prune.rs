//! Stale-cache pruning (corpus-warmup-prune Req 3, 4): a cache row is
//! stale exactly when its file is missing or its content hash changed;
//! only those rows are ever removed. `scan_stale` counts without deleting
//! (preview); `delete_stale` removes exactly the stale rows and records
//! one `PRUNE` audit summary. Identical logic serves the CLI and the
//! tool, so both transports behave the same by construction.

use serde::Serialize;

use super::shunt_types::CallCtx;
use crate::storage::audit::{sha256_hex, AuditEventType, AuditWriter};
use crate::storage::db::Db;

/// Grouped outcome for one prune pass (Req 3.3).
#[derive(Debug, Default, Clone, Copy, Serialize, PartialEq, Eq)]
pub struct PruneReport {
    /// Stale rows removed because their file is gone or unreadable.
    pub removed_missing: u32,
    /// Stale rows removed because the file content hash changed.
    pub removed_changed: u32,
    /// Rows kept because the file still matches its cached hash.
    pub kept_fresh: u32,
}

impl PruneReport {
    /// Total stale rows this pass removed.
    pub fn removed_total(self) -> u32 {
        self.removed_missing + self.removed_changed
    }
}

/// One stale cache row, keyed by (file_path, content_sha256) — every
/// fingerprint variant of a stale file goes together.
struct StaleRow {
    path: String,
    sha: String,
}

/// Preview pass (Req 3.2, 4.2): counts only — nothing is deleted.
pub async fn scan_stale(db: &Db) -> anyhow::Result<PruneReport> {
    let (missing, changed, fresh) = classify(db).await?;
    Ok(report_of(missing.len(), changed.len(), fresh))
}

/// Deletion pass (Req 3.1): removes exactly the stale rows and records
/// one `PRUNE` audit summary. Non-stale rows are never touched (Req 4.3).
pub async fn delete_stale(
    db: &Db,
    audit: &AuditWriter,
    ctx: CallCtx<'_>,
) -> anyhow::Result<PruneReport> {
    let (missing, changed, fresh) = classify(db).await?;
    let report = report_of(missing.len(), changed.len(), fresh);
    if report.removed_total() == 0 {
        return Ok(report); // nothing removed → no summary event
    }
    // Distinct (path, sha) pairs: every fingerprint row under a stale pair
    // goes together, and each pair is deleted exactly once.
    let stale: Vec<(String, String)> = missing
        .into_iter()
        .chain(changed)
        .map(|row| (row.path, row.sha))
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    let removed = db
        .with_conn(move |c| {
            let mut removed = 0usize;
            for (path, sha) in &stale {
                removed += c.execute(
                    "DELETE FROM shunt_file_cache \
                     WHERE file_path = ?1 AND content_sha256 = ?2",
                    rusqlite::params![path, sha],
                )?;
            }
            Ok(removed)
        })
        .await?;
    debug_assert_eq!(removed as u32, report.removed_total());
    let _ = audit.write_event(
        AuditEventType::Prune,
        ctx.harness,
        ctx.project,
        serde_json::json!({
            "removed_missing": report.removed_missing,
            "removed_changed": report.removed_changed,
            "kept_fresh": report.kept_fresh,
        }),
    );
    Ok(report)
}

/// Counts every cache row: stale-missing, stale-changed, or fresh
/// (Req 3.1). Unreadable files count as stale-missing — the row can
/// never be served again, so pruning it is the honest outcome.
async fn classify(db: &Db) -> anyhow::Result<(Vec<StaleRow>, Vec<StaleRow>, u32)> {
    let rows: Vec<(String, String)> = db
        .with_conn(|c| {
            let mut stmt = c.prepare("SELECT file_path, content_sha256 FROM shunt_file_cache")?;
            let mapped =
                stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
            mapped.collect::<Result<Vec<_>, _>>()
        })
        .await?;
    let mut missing = Vec::new();
    let mut changed = Vec::new();
    let mut fresh = 0u32;
    for (path, sha) in rows {
        match tokio::fs::read(&path).await {
            Err(_) => missing.push(StaleRow { path, sha }),
            Ok(bytes) => {
                if sha256_hex(&bytes) == sha {
                    fresh += 1;
                } else {
                    changed.push(StaleRow { path, sha });
                }
            }
        }
    }
    Ok((missing, changed, fresh))
}

/// Folds the classifier result into the report shape.
fn report_of(missing: usize, changed: usize, fresh: u32) -> PruneReport {
    PruneReport {
        removed_missing: missing as u32,
        removed_changed: changed as u32,
        kept_fresh: fresh,
    }
}

/// Renders the pass result as the response text; PREVIEW marks a
/// scan-only pass (Req 3.3, 4.2).
pub fn render_report(report: PruneReport, preview: bool) -> String {
    let head = if preview {
        "Prune preview (dry run — nothing deleted):"
    } else {
        "Prune complete:"
    };
    let mut md = format!(
        "{head} {} missing + {} changed stale rows, {} kept fresh.",
        report.removed_missing, report.removed_changed, report.kept_fresh
    );
    if preview && report.removed_total() > 0 {
        md.push_str(" Re-run without dry_run to remove the stale rows.");
    }
    md
}

/// Prune service handle for the MCP layer (the CLI constructs its own from
/// config; both call the same free functions, so behavior is identical).
#[derive(Clone)]
pub struct PruneService {
    db: Db,
    audit: AuditWriter,
}

impl PruneService {
    /// New service over db + audit.
    pub fn new(db: Db, audit: AuditWriter) -> Self {
        Self { db, audit }
    }

    /// Preview pass (Req 3.2, 4.2): counts only — nothing is deleted.
    pub async fn scan(&self) -> anyhow::Result<PruneReport> {
        scan_stale(&self.db).await
    }

    /// Deletion pass (Req 3.1): removes exactly the stale rows and audits.
    pub async fn delete(&self, ctx: CallCtx<'_>) -> anyhow::Result<PruneReport> {
        delete_stale(&self.db, &self.audit, ctx).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::services::shunt_cache::{cache_lookup, cache_store};
    use crate::storage::db::Db;

    const FP: &str = "fp0001";

    struct Fixture {
        _data: tempfile::TempDir,
        _logs: tempfile::TempDir,
        db: Db,
        audit: AuditWriter,
        audit_path: std::path::PathBuf,
    }

    fn open() -> Fixture {
        let data = tempfile::tempdir().expect("tempdir");
        let logs = tempfile::tempdir().expect("tempdir");
        let audit_path = logs.path().join("audit.jsonl");
        let db = Db::open(data.path()).expect("db");
        let audit = AuditWriter::new(audit_path.clone());
        Fixture {
            _data: data,
            _logs: logs,
            db,
            audit,
            audit_path,
        }
    }

    fn ctx() -> CallCtx<'static> {
        CallCtx {
            project: "p",
            harness: "h",
            task: None,
        }
    }

    async fn seed(fx: &Fixture, name: &str, sha: &str) -> String {
        let path = fx._data.path().join(name).display().to_string();
        cache_store(&fx.db, std::path::Path::new(&path), sha, FP, "summary", 10)
            .await
            .expect("seed");
        path
    }

    async fn seeded_fixture() -> (Fixture, String, String, String) {
        let fx = open();
        let dir = fx._data.path();
        // Fresh: file present, hash matches the stored one.
        std::fs::write(dir.join("fresh.rs"), "current content\n").expect("write");
        let fresh_sha = sha256_hex(b"current content\n");
        let fresh = seed(&fx, "fresh.rs", &fresh_sha).await;
        // Changed: file present, hash differs from the stored one.
        std::fs::write(dir.join("changed.rs"), "new content\n").expect("write");
        let old_sha = sha256_hex(b"old content\n");
        let changed = seed(&fx, "changed.rs", &old_sha).await;
        // Missing: no file at all.
        let missing = seed(&fx, "missing.rs", &old_sha).await;
        (fx, fresh, changed, missing)
    }

    async fn lookup(fx: &Fixture, path: &str, sha: &str) -> Option<String> {
        cache_lookup(&fx.db, std::path::Path::new(path), sha, FP)
            .await
            .expect("lookup")
    }

    #[tokio::test]
    async fn scan_counts_three_kinds_without_deleting() {
        // Req 3.2: the preview counts and touches nothing.
        let (fx, fresh, changed, missing) = seeded_fixture().await;
        let report = scan_stale(&fx.db).await.expect("scan");
        assert_eq!(
            report,
            PruneReport {
                removed_missing: 1,
                removed_changed: 1,
                kept_fresh: 1,
            }
        );
        assert!(lookup(&fx, &fresh, &sha256_hex(b"current content\n"))
            .await
            .is_some());
        assert!(lookup(&fx, &changed, &sha256_hex(b"old content\n"))
            .await
            .is_some());
        assert!(lookup(&fx, &missing, &sha256_hex(b"old content\n"))
            .await
            .is_some());
    }

    #[tokio::test]
    async fn delete_removes_exactly_the_stale_rows() {
        // Req 3.1, 4.3: stale rows go, fresh rows stay.
        let (fx, fresh, changed, missing) = seeded_fixture().await;
        let old_sha = sha256_hex(b"old content\n");
        let report = delete_stale(&fx.db, &fx.audit, ctx())
            .await
            .expect("delete");
        assert_eq!(report.removed_missing, 1);
        assert_eq!(report.removed_changed, 1);
        assert_eq!(report.kept_fresh, 1);
        assert!(lookup(&fx, &fresh, &sha256_hex(b"current content\n"))
            .await
            .is_some());
        assert!(
            lookup(&fx, &changed, &old_sha).await.is_none(),
            "changed row must be gone"
        );
        assert!(
            lookup(&fx, &missing, &old_sha).await.is_none(),
            "missing row must be gone"
        );
    }

    #[tokio::test]
    async fn delete_writes_one_prune_audit_event() {
        // Req 3.5: one PRUNE summary event for a real deletion.
        let (fx, _fresh, _changed, _missing) = seeded_fixture().await;
        delete_stale(&fx.db, &fx.audit, ctx())
            .await
            .expect("delete");
        let events = std::fs::read_to_string(&fx.audit_path)
            .expect("audit file")
            .lines()
            .filter(|l| l.contains("PRUNE"))
            .count();
        assert_eq!(events, 1, "exactly one PRUNE summary event");
    }

    #[tokio::test]
    async fn empty_pass_removes_nothing_and_audits_nothing() {
        let fx = open();
        let report = scan_stale(&fx.db).await.expect("scan");
        assert_eq!(report, PruneReport::default());
        let report = delete_stale(&fx.db, &fx.audit, ctx())
            .await
            .expect("delete");
        assert_eq!(report, PruneReport::default());
        let audit = std::fs::read_to_string(&fx.audit_path).unwrap_or_default();
        assert!(
            !audit.contains("PRUNE"),
            "nothing removed → no event: {audit}"
        );
    }

    #[tokio::test]
    async fn fingerprint_variants_go_together() {
        // Same (path, sha) cached under two questions: one stale file
        // version takes all its fingerprint rows.
        let fx = open();
        let old_sha = sha256_hex(b"old content\n");
        let path = seed(&fx, "gone.rs", &old_sha).await;
        cache_store(
            &fx.db,
            std::path::Path::new(&path),
            &old_sha,
            "fp0002",
            "other summary",
            10,
        )
        .await
        .expect("second fingerprint");
        let report = delete_stale(&fx.db, &fx.audit, ctx())
            .await
            .expect("delete");
        assert_eq!(
            report.removed_missing, 2,
            "counts are per cache row (one per fingerprint)"
        );
        assert!(
            lookup(&fx, &path, &old_sha).await.is_none(),
            "both fingerprint rows must be gone"
        );
        let still: Option<String> =
            cache_lookup(&fx.db, std::path::Path::new(&path), &old_sha, "fp0002")
                .await
                .expect("lookup");
        assert!(still.is_none(), "second fingerprint row must be gone too");
    }
}
