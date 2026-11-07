//! Shunt-cache and payload helpers for the `ShuntEngine` (split out of
//! `shunt.rs` per the Req 13.3 500-LOC-per-file cap, split by responsibility).
//!
//! Two content-addressed caches (Req 4.1): the FILE cache keyed by
//! (file_path, content_sha256, query_fingerprint) — Req 3.4-3.7; a changed
//! file (different SHA-256) never serves a stale summary (Req 3.7) — and
//! the CONTENT cache keyed by (content_sha256, query_fingerprint, kind)
//! with its own bounded LRU eviction (shunt-content-tool task 1.2,
//! Req 4.1-4.3), separate because content rows are ephemeral tool output
//! that prune must never classify by on-disk existence.

use std::path::Path;

use rusqlite::OptionalExtension;

use crate::services::shunt_types::ShuntError;
use crate::storage::audit::sha256_hex;
use crate::storage::db::Db;

/// 16-char fingerprint of the normalized question (cache key component,
/// Req 3.4 — reference: SHA-256 of trimmed lowercase question, first 16).
pub fn query_fingerprint(question: &str) -> String {
    sha256_hex(question.trim().to_lowercase().as_bytes())[..16].to_string()
}

/// Cache lookup by (file_path, content_sha256, query_fingerprint) — Req 3.4.
pub async fn cache_lookup(
    db: &Db,
    path: &Path,
    sha: &str,
    fp: &str,
) -> Result<Option<String>, ShuntError> {
    let key = path.display().to_string();
    let (sha, fp) = (sha.to_string(), fp.to_string());
    db.with_conn(move |c| {
        c.query_row(
            "SELECT structural_summary FROM shunt_file_cache \
              WHERE file_path = ?1 AND content_sha256 = ?2 AND query_fingerprint = ?3",
            rusqlite::params![key, sha, fp],
            |r| r.get::<_, String>(0),
        )
        .optional()
    })
    .await
    .map_err(|e| ShuntError::Io(e.to_string()))
}

/// Cache store — Req 3.6 (repeat reads of the unchanged file are free).
pub async fn cache_store(
    db: &Db,
    path: &Path,
    sha: &str,
    fp: &str,
    summary: &str,
    line_count: usize,
) -> Result<(), ShuntError> {
    let key = path.display().to_string();
    let (sha, fp, summary) = (sha.to_string(), fp.to_string(), summary.to_string());
    db.with_conn(move |c| {
        c.execute(
            "INSERT INTO shunt_file_cache (file_path, content_sha256, query_fingerprint, \
              structural_summary, line_count, created_at_epoch) \
              VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
              ON CONFLICT(file_path, content_sha256, query_fingerprint) DO UPDATE SET \
              structural_summary=excluded.structural_summary, line_count=excluded.line_count, \
              created_at_epoch=excluded.created_at_epoch",
            rusqlite::params![key, sha, fp, summary, line_count as i64, now_ms()],
        )
        .map(|_| ())
    })
    .await
    .map_err(|e| ShuntError::Io(e.to_string()))
}

/// One content-cache row for [`content_cache_store_and_evict`] (design.md
/// "storage / Content cache", shunt-content-tool task 1.2): keyed by
/// content hash, question fingerprint and kind (Req 4.1). The LRU columns
/// (`created_at_epoch`, `last_hit_epoch`, `hits`) are maintained by the
/// helpers, not the caller.
pub struct ContentCacheRow<'a> {
    /// SHA-256 hex of the content (64 chars).
    pub content_sha256: &'a str,
    /// Fingerprint of the normalized question (see [`query_fingerprint`]).
    pub query_fingerprint: &'a str,
    /// Content kind discriminator (`ContentKind::as_str`; v1 `file_read`).
    pub kind: &'a str,
    /// The structural summary to cache (Req 2.2: served byte-identical).
    pub structural_summary: &'a str,
    /// Content line count.
    pub line_count: usize,
}

/// Content-cache lookup by (content_sha256, query_fingerprint, kind) —
/// Req 4.1/4.2. A hit returns the stored summary AND touches the row
/// (`last_hit_epoch = now`, the LRU clock) and counts it (`hits + 1`);
/// a miss returns `None` and writes nothing.
pub async fn content_cache_lookup(
    db: &Db,
    sha: &str,
    fp: &str,
    kind: &str,
) -> Result<Option<String>, ShuntError> {
    let (sha, fp, kind) = (sha.to_string(), fp.to_string(), kind.to_string());
    db.with_conn(move |c| {
        let summary = c
            .query_row(
                "SELECT structural_summary FROM shunt_content_cache \
                  WHERE content_sha256 = ?1 AND query_fingerprint = ?2 AND kind = ?3",
                rusqlite::params![sha, fp, kind],
                |r| r.get::<_, String>(0),
            )
            .optional()?;
        if summary.is_some() {
            c.execute(
                "UPDATE shunt_content_cache SET last_hit_epoch = ?4, hits = hits + 1 \
                  WHERE content_sha256 = ?1 AND query_fingerprint = ?2 AND kind = ?3",
                rusqlite::params![sha, fp, kind, now_ms()],
            )?;
        }
        Ok(summary)
    })
    .await
    .map_err(|e| ShuntError::Io(e.to_string()))
}

/// Content-cache store with LRU eviction — Req 4.3 (shunt-content-tool
/// task 1.2; the cap itself is threaded in by the caller, config key
/// task 1.3). Upserts `row`, then — in the SAME transaction, so no partial
/// state survives a failure — deletes the least-recently-used rows beyond
/// `max_rows` (LRU order `(last_hit_epoch, created_at_epoch)`, served by
/// `idx_shunt_content_cache_lru`) and returns how many rows were evicted
/// (one count for the caller's audit event, never per row). The stored
/// row carries the newest `last_hit_epoch`, so its own pass never evicts
/// it.
pub async fn content_cache_store_and_evict(
    db: &Db,
    row: ContentCacheRow<'_>,
    max_rows: u32,
) -> Result<u32, ShuntError> {
    let (sha, fp, kind, summary, line_count) = (
        row.content_sha256.to_string(),
        row.query_fingerprint.to_string(),
        row.kind.to_string(),
        row.structural_summary.to_string(),
        row.line_count,
    );
    let now = now_ms();
    db.with_conn(move |c| {
        let tx = c.unchecked_transaction()?;
        tx.execute(
            "INSERT INTO shunt_content_cache (content_sha256, query_fingerprint, kind, \
              structural_summary, line_count, created_at_epoch, last_hit_epoch, hits) \
              VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6, 0) \
              ON CONFLICT(content_sha256, query_fingerprint, kind) DO UPDATE SET \
              structural_summary=excluded.structural_summary, line_count=excluded.line_count, \
              last_hit_epoch=excluded.last_hit_epoch",
            rusqlite::params![sha, fp, kind, summary, line_count as i64, now],
        )?;
        let evicted = tx.execute(
            "DELETE FROM shunt_content_cache WHERE rowid IN ( \
               SELECT rowid FROM shunt_content_cache \
                ORDER BY last_hit_epoch DESC, created_at_epoch DESC LIMIT -1 OFFSET ?1 )",
            rusqlite::params![i64::from(max_rows)],
        )?;
        tx.commit()?;
        Ok(u32::try_from(evicted).unwrap_or(u32::MAX))
    })
    .await
    .map_err(|e| ShuntError::Io(e.to_string()))
}

/// Raw file size in chars (for estimates, Req 10.1).
pub async fn raw_chars(path: &Path) -> Result<usize, ShuntError> {
    tokio::fs::read_to_string(path)
        .await
        .map(|s| s.len())
        .map_err(|e| ShuntError::Io(e.to_string()))
}

/// Line-numbered file wrapped in XML-ish tags for the worker prompt
/// (research doc: `<file path="..." lines="...">`).
pub async fn line_numbered_payload(path: &Path) -> Result<String, ShuntError> {
    let raw = tokio::fs::read_to_string(path)
        .await
        .map_err(|e| ShuntError::Io(e.to_string()))?;
    let count = raw.lines().count();
    let numbered = raw
        .lines()
        .enumerate()
        .map(|(i, l)| format!("L{}: {}", i + 1, l))
        .collect::<Vec<_>>()
        .join("\n");
    Ok(format!(
        "<file path=\"{}\" lines=\"{}\">\n{}\n</file>",
        path.display(),
        count,
        numbered
    ))
}

/// Millisecond epoch now.
fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
