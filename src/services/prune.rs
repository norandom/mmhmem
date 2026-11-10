//! Stale-cache pruning (corpus-warmup-prune Req 3, 4; shunt-cache-efficiency
//! Req 7): the file-read summaries in `shunt_content_cache` (kind
//! `file_read`) are checked at their `last_path`; a row is stale exactly
//! when that file is missing or its content hash differs from the row's;
//! only those rows are ever removed. Rows without a `last_path` (written by
//! `shunt_content`, which has no file) and rows of other kinds are neither
//! counted nor touched (shunt-content-tool Req 4.4). `scan_stale` counts
//! without deleting (preview); `delete_stale` removes exactly the stale
//! rows and records one `PRUNE` audit summary. Identical logic serves the
//! CLI and the tool, so both transports behave the same by construction.

use std::collections::HashMap;

use serde::Serialize;

use super::shunt_cache::FILE_READ_KIND;
use super::shunt_types::CallCtx;
use crate::storage::audit::{AuditEventType, AuditWriter, sha256_hex};
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

/// One stale cache row, by its key within kind `file_read`: every question
/// variant of a stale content is its own row and goes on its own merits.
struct StaleRow {
    sha: String,
    fp: String,
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
    // Each stale row by its key; the key is unique, so each goes once.
    let stale: Vec<StaleRow> = missing.into_iter().chain(changed).collect();
    let removed = db
        .with_conn(move |c| {
            let mut removed = 0usize;
            for row in &stale {
                removed += c.execute(
                    "DELETE FROM shunt_content_cache \
                     WHERE content_sha256 = ?1 AND query_fingerprint = ?2 AND kind = ?3",
                    rusqlite::params![row.sha, row.fp, FILE_READ_KIND],
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

/// Counts every file-read row with a known last path: stale-missing,
/// stale-changed, or fresh (Req 3.1, 7.1, 7.2). Unreadable files count as
/// stale-missing — the row can never be served from there again, so
/// pruning it is the honest outcome. Each path is hashed once.
async fn classify(db: &Db) -> anyhow::Result<(Vec<StaleRow>, Vec<StaleRow>, u32)> {
    let rows: Vec<(String, String, String)> = db
        .with_conn(|c| {
            let mut stmt = c.prepare(
                "SELECT content_sha256, query_fingerprint, last_path FROM shunt_content_cache \
                  WHERE kind = ?1 AND last_path IS NOT NULL",
            )?;
            let mapped =
                stmt.query_map([FILE_READ_KIND], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
            mapped.collect::<Result<Vec<_>, _>>()
        })
        .await?;
    let mut current: HashMap<String, Option<String>> = HashMap::new();
    let (mut missing, mut changed, mut fresh) = (Vec::new(), Vec::new(), 0u32);
    for (sha, fp, path) in rows {
        if !current.contains_key(&path) {
            let hash = tokio::fs::read(&path).await.ok().map(|b| sha256_hex(&b));
            current.insert(path.clone(), hash);
        }
        match current.get(&path).and_then(Option::as_deref) {
            None => missing.push(StaleRow { sha, fp }),
            Some(now) if now == sha => fresh += 1,
            Some(_) => changed.push(StaleRow { sha, fp }),
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

    use std::path::Path;

    use crate::services::shunt_cache::{
        ContentCacheRow, SummaryRow, content_cache_store_and_evict, summary_lookup,
        summary_store_and_evict,
    };
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

    /// Seeds a file-read row under `fp` whose `last_path` is `name` in the
    /// fixture directory (the file may or may not exist).
    async fn seed_fp(fx: &Fixture, name: &str, sha: &str, fp: &str) -> String {
        let path = fx._data.path().join(name).display().to_string();
        let row = SummaryRow {
            sha,
            fingerprint: fp,
            summary: "summary",
            line_count: 10,
            path: Path::new(&path),
        };
        summary_store_and_evict(&fx.db, row, 5000)
            .await
            .expect("seed");
        path
    }

    async fn seed(fx: &Fixture, name: &str, sha: &str) -> String {
        seed_fp(fx, name, sha, FP).await
    }

    /// (fresh, changed, missing) rows with their paths; the changed and
    /// missing rows cache different contents (the key has no path).
    async fn seeded_fixture() -> (Fixture, String, String, String) {
        let fx = open();
        let dir = fx._data.path();
        // Fresh: file present, hash matches the stored one.
        std::fs::write(dir.join("fresh.rs"), "current content\n").expect("write");
        let fresh = seed(&fx, "fresh.rs", &sha256_hex(b"current content\n")).await;
        // Changed: file present, hash differs from the stored one.
        std::fs::write(dir.join("changed.rs"), "new content\n").expect("write");
        let changed = seed(&fx, "changed.rs", &sha256_hex(b"old content\n")).await;
        // Missing: no file at the last path.
        let missing = seed(&fx, "missing.rs", &sha256_hex(b"gone content\n")).await;
        (fx, fresh, changed, missing)
    }

    async fn lookup(fx: &Fixture, path: &str, sha: &str) -> Option<String> {
        summary_lookup(&fx.db, sha, FP, Path::new(path))
            .await
            .expect("lookup")
    }

    async fn row_count(fx: &Fixture) -> i64 {
        fx.db
            .with_conn(|c| {
                c.query_row("SELECT COUNT(*) FROM shunt_content_cache", [], |r| r.get(0))
            })
            .await
            .expect("count")
    }

    #[tokio::test]
    async fn scan_counts_three_kinds_without_deleting() {
        // Req 3.2: the preview counts and touches nothing.
        let (fx, _fresh, _changed, _missing) = seeded_fixture().await;
        let report = scan_stale(&fx.db).await.expect("scan");
        assert_eq!(
            report,
            PruneReport {
                removed_missing: 1,
                removed_changed: 1,
                kept_fresh: 1,
            }
        );
        assert_eq!(row_count(&fx).await, 3, "the preview deletes nothing");
    }

    #[tokio::test]
    async fn delete_removes_exactly_the_stale_rows() {
        // Req 3.1, 4.3; shunt-cache-efficiency 7.1: stale rows go, fresh stay.
        let (fx, fresh, changed, missing) = seeded_fixture().await;
        let report = delete_stale(&fx.db, &fx.audit, ctx())
            .await
            .expect("delete");
        assert_eq!(report.removed_missing, 1);
        assert_eq!(report.removed_changed, 1);
        assert_eq!(report.kept_fresh, 1);
        assert!(
            lookup(&fx, &fresh, &sha256_hex(b"current content\n"))
                .await
                .is_some()
        );
        assert!(
            lookup(&fx, &changed, &sha256_hex(b"old content\n"))
                .await
                .is_none(),
            "changed row must be gone"
        );
        assert!(
            lookup(&fx, &missing, &sha256_hex(b"gone content\n"))
                .await
                .is_none(),
            "missing row must be gone"
        );
    }

    #[tokio::test]
    async fn a_row_last_read_at_a_path_that_still_holds_its_content_is_kept() {
        // shunt-cache-efficiency 7.2: the content moved; the last read at
        // the new path updated `last_path`, so the row is fresh.
        let fx = open();
        let sha = sha256_hex(b"moved content\n");
        let old = seed(&fx, "old_place.rs", &sha).await;
        let new_path = fx._data.path().join("new_place.rs");
        std::fs::write(&new_path, "moved content\n").expect("write");
        assert!(!Path::new(&old).exists());
        let hit = summary_lookup(&fx.db, &sha, FP, &new_path).await;
        assert!(hit.expect("lookup").is_some());
        let report = delete_stale(&fx.db, &fx.audit, ctx())
            .await
            .expect("delete");
        assert_eq!(report, report_of(0, 0, 1));
        assert_eq!(row_count(&fx).await, 1);
    }

    #[tokio::test]
    async fn rows_without_a_last_path_and_rows_of_other_kinds_are_untouched() {
        // shunt-content-tool Req 4.4: content rows have no file to check,
        // so prune neither counts nor deletes them; other kinds likewise.
        let fx = open();
        let row = ContentCacheRow {
            content_sha256: &sha256_hex(b"tool output\n"),
            query_fingerprint: FP,
            kind: "file_read",
            structural_summary: "content summary",
            line_count: 200,
        };
        content_cache_store_and_evict(&fx.db, row, 5000)
            .await
            .expect("content row");
        fx.db
            .with_conn(|c| {
                c.execute(
                    "INSERT INTO shunt_content_cache (content_sha256, query_fingerprint, kind, \
                      structural_summary, line_count, created_at_epoch, last_hit_epoch, hits, \
                      last_path) VALUES ('sha', 'fp', 'other_kind', 's', 1, 1, 1, 0, '/gone.rs')",
                    [],
                )
            })
            .await
            .expect("other kind");
        seed(&fx, "missing.rs", &sha256_hex(b"gone content\n")).await;
        let report = delete_stale(&fx.db, &fx.audit, ctx())
            .await
            .expect("delete");
        assert_eq!(report, report_of(1, 0, 0), "only the file-read row counts");
        assert_eq!(
            row_count(&fx).await,
            2,
            "content and other-kind rows survive"
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
        // One stale content cached under two questions: both rows go.
        let fx = open();
        let old_sha = sha256_hex(b"old content\n");
        let path = seed(&fx, "gone.rs", &old_sha).await;
        seed_fp(&fx, "gone.rs", &old_sha, "fp0002").await;
        let report = delete_stale(&fx.db, &fx.audit, ctx())
            .await
            .expect("delete");
        assert_eq!(
            report.removed_missing, 2,
            "counts are per cache row (one per fingerprint)"
        );
        assert!(lookup(&fx, &path, &old_sha).await.is_none());
        assert_eq!(row_count(&fx).await, 0, "both fingerprint rows are gone");
    }
}
