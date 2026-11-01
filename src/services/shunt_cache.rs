//! Shunt-cache and payload helpers for the `ShuntEngine` (split out of
//! `shunt.rs` per the Req 13.3 500-LOC-per-file cap, split by responsibility).
//!
//! Content-addressed cache keyed by (file_path, content_sha256,
//! query_fingerprint) — Req 3.4-3.7; a changed file (different SHA-256) never
//! serves a stale summary (Req 3.7).

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
