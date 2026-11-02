//! `mmhmem prune` CLI tests (corpus-warmup-prune task 4.1; Req 3.1-3.4).
//! The real binary is spawned exactly like a user would run it.

use std::path::Path;
use std::process::Command;

use mmhmem::services::shunt_cache::{cache_lookup, cache_store};
use mmhmem::storage::audit::sha256_hex;
use mmhmem::storage::db::Db;

/// Runs `mmhmem prune [--dry-run]` against `data_dir`'s store; returns
/// (exit-success, combined output).
fn prune(data_dir: &Path, dry_run: bool) -> (bool, String) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_mmhmem"));
    cmd.arg("prune")
        .env("AGENT_MEM_DATA_DIR", data_dir)
        .env_remove("AGENT_MEM_LOG_FILE")
        .env_remove("WORKER_MODEL")
        .env_remove("WORKER_API_URL")
        .env_remove("WORKER_API_KEY");
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

/// Seeds the store in `data_dir` with one stale-missing row (the file does
/// not exist) and one fresh row (file present, hash matches); returns the
/// (missing_path, missing_sha, fresh_path, fresh_sha) keys.
async fn seed_store(data_dir: &Path) -> (String, String, String, String) {
    let db = Db::open(data_dir).expect("open store");
    let fresh_path = data_dir.join("fresh.rs");
    std::fs::write(&fresh_path, "current content\n").expect("write fresh file");
    let fresh_sha = sha256_hex(b"current content\n");
    cache_store(&db, &fresh_path, &fresh_sha, "fp", "fresh summary", 10)
        .await
        .expect("seed fresh");
    let missing_path = data_dir.join("gone.rs");
    let missing_sha = sha256_hex(b"gone content\n");
    cache_store(&db, &missing_path, &missing_sha, "fp", "stale summary", 10)
        .await
        .expect("seed missing");
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
        cache_lookup(&db, Path::new(&missing), &missing_sha, "fp")
            .await
            .expect("lookup")
            .is_some(),
        "dry-run must not delete the stale row"
    );
    assert!(
        cache_lookup(&db, Path::new(&fresh), &fresh_sha, "fp")
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
        cache_lookup(&db, Path::new(&missing), &missing_sha, "fp")
            .await
            .expect("lookup")
            .is_none(),
        "stale row must be gone"
    );
    assert!(
        cache_lookup(&db, Path::new(&fresh), &fresh_sha, "fp")
            .await
            .expect("lookup")
            .is_some(),
        "fresh row must survive"
    );
    // One PRUNE audit event landed in the store's audit log.
    let audit = std::fs::read_to_string(data.path().join("audit.jsonl")).expect("audit file");
    assert!(audit.contains("PRUNE"), "audited deletion: {audit}");
}
