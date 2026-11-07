//! Content-cache storage tests (shunt-content-tool task 1.2;
//! requirements.md Req 4.1/4.3/4.5): the dedicated `shunt_content_cache`
//! table's lookup/touch semantics and the LRU store-and-evict helper.
//! Table creation on a pre-existing store is pinned in
//! `tests/store_startup.rs` (Req 4.5); engine wiring is task 2.2.

use mmhmem::services::shunt_cache::{
    ContentCacheRow, content_cache_lookup, content_cache_store_and_evict,
};
use mmhmem::storage::db::Db;
use rusqlite::OptionalExtension;

const FP: &str = "fp1";
const KIND: &str = "file_read";

fn row<'a>(sha: &'a str, summary: &'a str) -> ContentCacheRow<'a> {
    ContentCacheRow {
        content_sha256: sha,
        query_fingerprint: FP,
        kind: KIND,
        structural_summary: summary,
        line_count: 10,
    }
}

/// `(created_at_epoch, last_hit_epoch, hits)` for the test key columns.
async fn row_state(db: &Db, sha: &str) -> Option<(i64, i64, i64)> {
    let sha = sha.to_string();
    db.with_conn(move |c| {
        c.query_row(
            "SELECT created_at_epoch, last_hit_epoch, hits FROM shunt_content_cache \
              WHERE content_sha256 = ?1 AND query_fingerprint = ?2 AND kind = ?3",
            rusqlite::params![sha, FP, KIND],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
    })
    .await
    .unwrap()
}

async fn count_rows(db: &Db) -> i64 {
    db.with_conn(|c| c.query_row("SELECT COUNT(*) FROM shunt_content_cache", [], |r| r.get(0)))
        .await
        .unwrap()
}

/// Rewrites the LRU clocks on rows stored through the helper so eviction
/// order is deterministic: `(sha, created_at_epoch, last_hit_epoch)`.
async fn seed_clocks(db: &Db, clocks: &[(&str, i64, i64)]) {
    let clocks = clocks
        .iter()
        .map(|(sha, created, last)| (sha.to_string(), *created, *last))
        .collect::<Vec<_>>();
    db.with_conn(move |c| {
        for (sha, created, last) in clocks {
            c.execute(
                "UPDATE shunt_content_cache SET created_at_epoch = ?2, last_hit_epoch = ?3 \
                  WHERE content_sha256 = ?1 AND query_fingerprint = ?4 AND kind = ?5",
                rusqlite::params![sha, created, last, FP, KIND],
            )?;
        }
        Ok(())
    })
    .await
    .unwrap();
}

/// Direct-seeds rows (bypassing the helper) so a store can already exceed
/// the cap when `content_cache_store_and_evict` runs — the lowered-cap
/// case (task 1.3) must evict the whole overflow in one call.
/// `(sha, created_at_epoch, last_hit_epoch)`.
async fn seed_rows(db: &Db, rows: &[(&str, i64, i64)]) {
    let rows = rows
        .iter()
        .map(|(sha, created, last)| (sha.to_string(), *created, *last))
        .collect::<Vec<_>>();
    db.with_conn(move |c| {
        for (sha, created, last) in rows {
            c.execute(
                "INSERT INTO shunt_content_cache (content_sha256, query_fingerprint, kind, \
                  structural_summary, line_count, created_at_epoch, last_hit_epoch, hits) \
                  VALUES (?1, ?2, ?3, 'seeded', 10, ?4, ?5, 0)",
                rusqlite::params![sha, FP, KIND, created, last],
            )?;
        }
        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn lookup_miss_returns_none_and_writes_no_row() {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(dir.path()).unwrap();
    assert_eq!(
        content_cache_lookup(&db, "sha-miss", FP, KIND)
            .await
            .unwrap(),
        None
    );
    assert_eq!(count_rows(&db).await, 0);
}

#[tokio::test]
async fn lookup_returns_summary_touches_last_hit_and_counts_hits() {
    // Req 4.2 storage half: a hit returns the stored summary AND touches
    // last-use AND counts the hit; the created timestamp is write-once.
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(dir.path()).unwrap();
    let evicted = content_cache_store_and_evict(&db, row("sha-a", "summary one"), 8)
        .await
        .unwrap();
    assert_eq!(evicted, 0, "a fresh store must not evict");
    let before = row_state(&db, "sha-a").await.expect("stored row");
    assert_eq!(before.2, 0, "a stored row starts with zero hits");
    std::thread::sleep(std::time::Duration::from_millis(10));
    assert_eq!(
        content_cache_lookup(&db, "sha-a", FP, KIND)
            .await
            .unwrap()
            .as_deref(),
        Some("summary one")
    );
    let after = row_state(&db, "sha-a").await.expect("row still present");
    assert_eq!(after.0, before.0, "created_at_epoch is write-once");
    assert!(after.1 > before.1, "lookup must touch last_hit_epoch");
    assert_eq!(after.2, 1, "first lookup counts one hit");
    std::thread::sleep(std::time::Duration::from_millis(10));
    assert_eq!(
        content_cache_lookup(&db, "sha-a", FP, KIND)
            .await
            .unwrap()
            .as_deref(),
        Some("summary one")
    );
    let again = row_state(&db, "sha-a").await.expect("row still present");
    assert_eq!(again.2, 2, "second lookup counts a second hit");
    assert!(again.1 >= after.1);
}

#[tokio::test]
async fn keys_discriminate_by_sha_fingerprint_and_kind() {
    // Req 4.1: keyed by content hash + query fingerprint + kind — a
    // different fingerprint or kind is a different cache entry.
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(dir.path()).unwrap();
    let stored = ContentCacheRow {
        content_sha256: "sha-a",
        query_fingerprint: "fp1",
        kind: "file_read",
        structural_summary: "one",
        line_count: 5,
    };
    assert_eq!(
        content_cache_store_and_evict(&db, stored, 4).await.unwrap(),
        0
    );
    assert_eq!(
        content_cache_lookup(&db, "sha-a", "fp2", "file_read")
            .await
            .unwrap(),
        None,
        "different fingerprint must miss"
    );
    assert_eq!(
        content_cache_lookup(&db, "sha-a", "fp1", "other_kind")
            .await
            .unwrap(),
        None,
        "different kind must miss"
    );
    assert_eq!(
        content_cache_lookup(&db, "sha-a", "fp1", "file_read")
            .await
            .unwrap()
            .as_deref(),
        Some("one"),
        "the exact key must hit"
    );
    assert_eq!(count_rows(&db).await, 1, "misses write no rows");
}

#[tokio::test]
async fn store_beyond_cap_evicts_exactly_the_oldest_hit_row() {
    // Req 4.3: exceeding the cap evicts the least recently used row and
    // returns the evicted count.
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(dir.path()).unwrap();
    for sha in ["sha-1", "sha-2", "sha-3"] {
        assert_eq!(
            content_cache_store_and_evict(&db, row(sha, "s"), 3)
                .await
                .unwrap(),
            0
        );
    }
    seed_clocks(
        &db,
        &[("sha-1", 10, 100), ("sha-2", 20, 200), ("sha-3", 30, 300)],
    )
    .await;
    let evicted = content_cache_store_and_evict(&db, row("sha-4", "s"), 3)
        .await
        .unwrap();
    assert_eq!(evicted, 1, "one row beyond the cap of 3");
    assert!(
        row_state(&db, "sha-1").await.is_none(),
        "the oldest-hit row must be evicted"
    );
    for kept in ["sha-2", "sha-3", "sha-4"] {
        assert!(row_state(&db, kept).await.is_some(), "{kept} must remain");
    }
    assert_eq!(count_rows(&db).await, 3);
}

#[tokio::test]
async fn tie_on_last_hit_evicts_the_older_created_row() {
    // The LRU order is (last_hit_epoch, created_at_epoch): with equal
    // last-hit clocks the row created earlier is the least recently used.
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(dir.path()).unwrap();
    for sha in ["sha-1", "sha-2"] {
        assert_eq!(
            content_cache_store_and_evict(&db, row(sha, "s"), 2)
                .await
                .unwrap(),
            0
        );
    }
    seed_clocks(&db, &[("sha-1", 10, 500), ("sha-2", 20, 500)]).await;
    let evicted = content_cache_store_and_evict(&db, row("sha-3", "s"), 2)
        .await
        .unwrap();
    assert_eq!(evicted, 1);
    assert!(
        row_state(&db, "sha-1").await.is_none(),
        "equal last_hit: the older created_at_epoch goes first"
    );
    assert!(row_state(&db, "sha-2").await.is_some());
    assert!(row_state(&db, "sha-3").await.is_some());
    assert_eq!(count_rows(&db).await, 2);
}

#[tokio::test]
async fn excess_store_evicts_all_overflow_in_one_call() {
    // A store can already hold more rows than the configured cap (the cap
    // can drop between runs, task 1.3): one store call must evict every
    // row beyond the cap, oldest-hit first, and report the full count.
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(dir.path()).unwrap();
    seed_rows(
        &db,
        &[
            ("sha-1", 1, 1),
            ("sha-2", 2, 2),
            ("sha-3", 3, 3),
            ("sha-4", 4, 4),
            ("sha-5", 5, 5),
            ("sha-6", 6, 6),
            ("sha-7", 7, 7),
        ],
    )
    .await;
    let evicted = content_cache_store_and_evict(&db, row("sha-8", "s"), 5)
        .await
        .unwrap();
    assert_eq!(evicted, 3, "8 rows against a cap of 5 evict 3");
    assert_eq!(count_rows(&db).await, 5);
    for gone in ["sha-1", "sha-2", "sha-3"] {
        assert!(
            row_state(&db, gone).await.is_none(),
            "{gone} must be evicted"
        );
    }
    for kept in ["sha-4", "sha-5", "sha-6", "sha-7", "sha-8"] {
        assert!(row_state(&db, kept).await.is_some(), "{kept} must remain");
    }
}

#[tokio::test]
async fn restoring_an_existing_key_updates_without_row_growth() {
    // Same key stored twice is an update (ON CONFLICT DO UPDATE), not a
    // new row; the fresh summary wins and nothing is evicted.
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(dir.path()).unwrap();
    assert_eq!(
        content_cache_store_and_evict(&db, row("sha-x", "v1"), 4)
            .await
            .unwrap(),
        0
    );
    std::thread::sleep(std::time::Duration::from_millis(10));
    assert_eq!(
        content_cache_store_and_evict(&db, row("sha-x", "v2"), 4)
            .await
            .unwrap(),
        0
    );
    assert_eq!(count_rows(&db).await, 1);
    assert_eq!(
        content_cache_lookup(&db, "sha-x", FP, KIND)
            .await
            .unwrap()
            .as_deref(),
        Some("v2")
    );
}
