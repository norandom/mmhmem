//! Shunt-cache and payload helpers for the `ShuntEngine` (split out of
//! `shunt.rs` per the Req 13.3 500-LOC-per-file cap, split by responsibility).
//!
//! One content-addressed table, `shunt_content_cache`, keyed by
//! (content_sha256, query_fingerprint, kind) with hits, an LRU clock and a
//! bounded, audited eviction (shunt-content-tool Req 4.1-4.3). File-read
//! summaries live there with kind `file_read` (shunt-cache-efficiency
//! Req 1): the path is not part of the key, so a copy, a worktree or a
//! moved file hits the same row, and `last_path` remembers where the
//! content was last read (prune checks it). A changed file (different
//! SHA-256) never serves a stale summary. The old `shunt_file_cache`
//! table is no longer read or written; schema v3 copies its rows once.

use std::path::Path;
use std::sync::LazyLock;

use rusqlite::OptionalExtension;

use crate::services::shunt_content::DEFAULT_QUESTION;
use crate::services::shunt_types::ShuntError;
use crate::storage::audit::sha256_hex;
use crate::storage::db::Db;

/// The `kind` value of file-read summaries (`ContentKind::FileRead`).
pub const FILE_READ_KIND: &str = "file_read";

/// 16-char fingerprint of the normalized question (cache key component,
/// Req 3.4 — reference: SHA-256 of trimmed lowercase question, first 16).
pub fn query_fingerprint(question: &str) -> String {
    sha256_hex(question.trim().to_lowercase().as_bytes())[..16].to_string()
}

static CANONICAL_FINGERPRINT: LazyLock<String> =
    LazyLock::new(|| query_fingerprint(DEFAULT_QUESTION));

/// Fingerprint of the shared structural summary: the default question's
/// fingerprint (shunt-cache-efficiency Req 1.1).
pub fn canonical_fingerprint() -> &'static str {
    CANONICAL_FINGERPRINT.as_str()
}

/// Whether `question` asks for the shared structural summary: empty, blank,
/// or equal to the default question after the fingerprint normalization.
pub fn is_canonical_question(question: &str) -> bool {
    question.trim().is_empty() || query_fingerprint(question) == canonical_fingerprint()
}

/// One file-read summary for [`summary_store_and_evict`] (kind `file_read`).
pub struct SummaryRow<'a> {
    /// SHA-256 hex of the file content.
    pub sha: &'a str,
    /// [`canonical_fingerprint`] or the fingerprint of a custom question.
    pub fingerprint: &'a str,
    /// The worker's summary or answer, served byte-identical.
    pub summary: &'a str,
    /// File line count.
    pub line_count: usize,
    /// Canonical path the content was read from (stored as `last_path`).
    pub path: &'a Path,
}

/// File-read summary lookup by (sha, fingerprint, `file_read`): a hit
/// returns the summary and touches the row (`hits + 1`, `last_hit_epoch`,
/// `last_path = path`); a miss writes nothing (Req 1.1, 1.3, 5.1).
pub async fn summary_lookup(
    db: &Db,
    sha: &str,
    fp: &str,
    path: &Path,
) -> Result<Option<String>, ShuntError> {
    let path = Some(path.display().to_string());
    lookup_and_touch(db, sha, fp, FILE_READ_KIND, path).await
}

/// File-read summary store through the content cache's store-and-evict
/// (same cap, same LRU order); sets `last_path` and returns the evicted
/// count for the caller's `CONTENT_CACHE_EVICT` audit (Req 1.2, 5.2).
pub async fn summary_store_and_evict(
    db: &Db,
    row: SummaryRow<'_>,
    max_rows: u32,
) -> Result<u32, ShuntError> {
    let path = Some(row.path.display().to_string());
    let content = ContentCacheRow {
        content_sha256: row.sha,
        query_fingerprint: row.fingerprint,
        kind: FILE_READ_KIND,
        structural_summary: row.summary,
        line_count: row.line_count,
    };
    store_and_evict(db, content, path, max_rows).await
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
    lookup_and_touch(db, sha, fp, kind, None).await
}

/// Shared lookup: select, and on a hit touch the LRU clock, the hit count
/// and (when given) `last_path`, under one connection lock.
async fn lookup_and_touch(
    db: &Db,
    sha: &str,
    fp: &str,
    kind: &str,
    path: Option<String>,
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
                "UPDATE shunt_content_cache SET last_hit_epoch = ?4, hits = hits + 1, \
                  last_path = COALESCE(?5, last_path) \
                  WHERE content_sha256 = ?1 AND query_fingerprint = ?2 AND kind = ?3",
                rusqlite::params![sha, fp, kind, now_ms(), path],
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
    store_and_evict(db, row, None, max_rows).await
}

/// Shared upsert plus LRU eviction in one transaction. A store without a
/// path keeps any `last_path` a file read already recorded for the row.
async fn store_and_evict(
    db: &Db,
    row: ContentCacheRow<'_>,
    last_path: Option<String>,
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
              structural_summary, line_count, created_at_epoch, last_hit_epoch, hits, last_path) \
              VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6, 0, ?7) \
              ON CONFLICT(content_sha256, query_fingerprint, kind) DO UPDATE SET \
              structural_summary=excluded.structural_summary, line_count=excluded.line_count, \
              last_hit_epoch=excluded.last_hit_epoch, \
              last_path=COALESCE(excluded.last_path, shunt_content_cache.last_path)",
            rusqlite::params![sha, fp, kind, summary, line_count as i64, now, last_path],
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

/// Line-numbered caller content for the worker payload (shunt-content-tool
/// design.md "Payload"; research.md "Worker payload for pre-numbered
/// content"): every line gets the same `L<n>: ` prefix as
/// [`line_numbered_payload`] regardless of any numbering already in the
/// tool output, so worker `[L<a>-L<b>]` ranges refer to the original line
/// positions 1:1. The wrapper mirrors the file payload's shape with
/// `content` as the subject — the hint is an audit label, never a payload
/// component (Req 1.5).
pub fn payload_from_content(content: &str) -> String {
    let count = content.lines().count();
    let numbered = content
        .lines()
        .enumerate()
        .map(|(i, l)| format!("L{}: {}", i + 1, l))
        .collect::<Vec<_>>()
        .join("\n");
    format!("<content lines=\"{count}\">\n{numbered}\n</content>")
}

/// Millisecond epoch now.
fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
