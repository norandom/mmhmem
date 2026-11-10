//! `mmhmem prune` CLI tests (corpus-warmup-prune task 4.1; Req 3.1-3.4).
//! The real binary is spawned exactly like a user would run it.

use std::path::Path;
use std::process::Command;

use mmhmem::services::shunt_cache::{
    ContentCacheRow, SummaryRow, content_cache_lookup, content_cache_store_and_evict,
    summary_lookup, summary_store_and_evict,
};
use mmhmem::storage::audit::sha256_hex;
use mmhmem::storage::db::Db;

/// Runs `mmhmem prune [--dry-run]` against `data_dir`'s store; returns
/// (exit-success, combined output).
fn prune(data_dir: &Path, dry_run: bool) -> (bool, String) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_mmhmem"));
    cmd.arg("prune")
        .env("AGENT_MEM_DATA_DIR", data_dir)
        .env_remove("AGENT_MEM_LOG_FILE")
        // Blank — not removed: removed worker vars fall back to the
        // machine's global .mmhmem.json (armed on dev hosts); blank wins
        // over the file lookup and is treated as missing.
        .env("WORKER_MODEL", "")
        .env("WORKER_API_URL", "")
        .env("WORKER_API_KEY", "");
    if dry_run {
        cmd.arg("--dry-run");
    }
    let out = cmd.output().expect("run mmhmem prune");
    (
        out.status.success(),
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
    )
}

/// Seeds one file-read summary row (kind `file_read`, `last_path` set)
/// exactly as a `smart_read` delegation would.
async fn seed_file_row(db: &Db, path: &Path, sha: &str, summary: &str) {
    let row = SummaryRow {
        sha,
        fingerprint: "fp",
        summary,
        line_count: 10,
        path,
    };
    summary_store_and_evict(db, row, 5000)
        .await
        .expect("seed file row");
}

/// Seeds the store in `data_dir` with one stale-missing row (the file does
/// not exist) and one fresh row (file present, hash matches); returns the
/// (missing_path, missing_sha, fresh_path, fresh_sha) keys.
async fn seed_store(data_dir: &Path) -> (String, String, String, String) {
    let db = Db::open(data_dir).expect("open store");
    let fresh_path = data_dir.join("fresh.rs");
    std::fs::write(&fresh_path, "current content\n").expect("write fresh file");
    let fresh_sha = sha256_hex(b"current content\n");
    seed_file_row(&db, &fresh_path, &fresh_sha, "fresh summary").await;
    let missing_path = data_dir.join("gone.rs");
    let missing_sha = sha256_hex(b"gone content\n");
    seed_file_row(&db, &missing_path, &missing_sha, "stale summary").await;
    drop(db);
    (
        missing_path.display().to_string(),
        missing_sha,
        fresh_path.display().to_string(),
        fresh_sha,
    )
}

#[tokio::test]
async fn missing_store_exits_zero_with_clear_message() {
    // Req 3.4: no store is not an error.
    let data = tempfile::tempdir().unwrap();
    let (ok, out) = prune(data.path(), false);
    assert!(ok, "exit 0 with no store: {out}");
    assert!(out.contains("nothing to prune"), "clear message: {out}");
}

#[tokio::test]
async fn dry_run_previews_counts_and_deletes_nothing() {
    // Req 3.2: grouped preview, zero mutations.
    let data = tempfile::tempdir().unwrap();
    let (missing, missing_sha, fresh, fresh_sha) = seed_store(data.path()).await;
    let (ok, out) = prune(data.path(), true);
    assert!(ok, "dry-run exits 0: {out}");
    assert!(out.contains("nothing deleted"), "preview is labeled: {out}");
    assert!(out.contains("1 missing"), "grouped counts: {out}");
    assert!(
        out.contains("Re-run without dry_run"),
        "actionable next step: {out}"
    );
    let db = Db::open(data.path()).expect("reopen store");
    assert!(
        summary_lookup(&db, &missing_sha, "fp", Path::new(&missing))
            .await
            .expect("lookup")
            .is_some(),
        "dry-run must not delete the stale row"
    );
    assert!(
        summary_lookup(&db, &fresh_sha, "fp", Path::new(&fresh))
            .await
            .expect("lookup")
            .is_some(),
        "dry-run must not touch the fresh row"
    );
}

#[tokio::test]
async fn real_run_removes_stale_keeps_fresh_exits_zero() {
    // Req 3.1, 3.3: exactly the stale rows go.
    let data = tempfile::tempdir().unwrap();
    let (missing, missing_sha, fresh, fresh_sha) = seed_store(data.path()).await;
    let (ok, out) = prune(data.path(), false);
    assert!(ok, "prune exits 0: {out}");
    assert!(out.contains("Prune complete"), "completion label: {out}");
    assert!(out.contains("1 missing"), "grouped counts: {out}");
    assert!(out.contains("1 kept fresh"), "grouped counts: {out}");
    let db = Db::open(data.path()).expect("reopen store");
    assert!(
        summary_lookup(&db, &missing_sha, "fp", Path::new(&missing))
            .await
            .expect("lookup")
            .is_none(),
        "stale row must be gone"
    );
    assert!(
        summary_lookup(&db, &fresh_sha, "fp", Path::new(&fresh))
            .await
            .expect("lookup")
            .is_some(),
        "fresh row must survive"
    );
    // One PRUNE audit event landed in the store's audit log.
    let audit = std::fs::read_to_string(data.path().join("audit.jsonl")).expect("audit file");
    assert!(audit.contains("PRUNE"), "audited deletion: {audit}");
}

/// Seeds two content-cache rows exactly as `shunt_content` delegations
/// would (shunt-content-tool Req 4.1); returns their content hashes.
async fn seed_content_rows(data_dir: &Path) -> (String, String) {
    let db = Db::open(data_dir).expect("open store");
    let one = sha256_hex(b"ephemeral tool output one\n");
    let two = sha256_hex(b"ephemeral tool output two\n");
    for (sha, summary) in [(&one, "content summary one"), (&two, "content summary two")] {
        let row = ContentCacheRow {
            content_sha256: sha,
            query_fingerprint: "fp",
            kind: "file_read",
            structural_summary: summary,
            line_count: 200,
        };
        let evicted = content_cache_store_and_evict(&db, row, 5000)
            .await
            .expect("seed content row");
        assert_eq!(evicted, 0, "seeding within the cap must not evict");
    }
    drop(db);
    (one, two)
}

#[tokio::test]
async fn prune_command_leaves_content_rows_untouched_and_counts_only_file_rows() {
    // shunt-content-tool Req 4.4: content-cache rows are ephemeral tool
    // output with no file to re-check on disk, so the `mmhmem prune`
    // COMMAND must neither delete nor count them — the report still
    // describes only the file cache.
    let data = tempfile::tempdir().unwrap();
    seed_store(data.path()).await; // one stale-missing + one fresh file row
    let (one, two) = seed_content_rows(data.path()).await;

    let (ok, out) = prune(data.path(), false);
    assert!(ok, "prune exits 0: {out}");
    assert!(
        out.contains("1 missing") && out.contains("1 kept fresh"),
        "report counts ONLY the file-cache rows: {out}"
    );

    let db = Db::open(data.path()).expect("reopen store");
    for sha in [&one, &two] {
        assert!(
            content_cache_lookup(&db, sha, "fp", "file_read")
                .await
                .expect("content lookup")
                .is_some(),
            "prune must leave content row {sha} untouched"
        );
    }
    // The table is shared with file-read rows (shunt-cache-efficiency):
    // content rows are the ones without a last path.
    let rows: i64 = db
        .with_conn(|c| {
            c.query_row(
                "SELECT COUNT(*) FROM shunt_content_cache WHERE last_path IS NULL",
                [],
                |r| r.get(0),
            )
        })
        .await
        .expect("count content rows");
    assert_eq!(rows, 2, "both content rows survive the prune command");
}
