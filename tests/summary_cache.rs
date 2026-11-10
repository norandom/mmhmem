//! Content-keyed summary cache and lookup policy (shunt-cache-efficiency
//! tasks 1.1 and 2.1; Requirements 1.1-1.4, 2.1-2.3, 2.5, 5.1, 5.2).
//!
//! File-read summaries live in `shunt_content_cache` with kind
//! `file_read`, keyed by content hash and question fingerprint only; the
//! default question maps to the canonical fingerprint. A custom question
//! is served a cached answer to it, else the canonical summary with a
//! one-line note; only `answer_question: true` asks the worker for it.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use mmhmem::config::{
    Config, ENV_ALLOWED_ROOTS, ENV_SHUNT_CONTENT_CACHE_MAX_ROWS, ENV_SHUNT_MIN_LINES, ENV_TOKEN,
    ENV_WORKER_API_URL, ENV_WORKER_MODEL,
};
use mmhmem::services::shunt::{ReadOutcome, ShuntEngine};
use mmhmem::services::shunt_cache::{
    SummaryRow, canonical_fingerprint, is_canonical_question, query_fingerprint, summary_lookup,
    summary_store_and_evict,
};
use mmhmem::services::shunt_content::DEFAULT_QUESTION;
use mmhmem::services::shunt_types::{QUESTION_NOT_APPLIED, ReadRequest};
use mmhmem::services::worker::WorkerClient;
use mmhmem::storage::audit::{AuditWriter, sha256_hex};
use mmhmem::storage::db::Db;
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const SUMMARY_CHAT: &str = r#"{"choices":[{"message":{"content":"- foo [L1-L9]\n- bar [L10-L20]"}}],"usage":{"prompt_tokens":500,"completion_tokens":30}}"#;
const CUSTOM: &str = "Which functions touch the parser?";
/// Large enough for every threshold this spec ships (150 today, 300 later).
const LINES: usize = 400;

struct Rig {
    _data: tempfile::TempDir,
    logs: tempfile::TempDir,
    db: Db,
    engine: ShuntEngine,
}

impl Rig {
    fn audit(&self) -> Vec<Value> {
        std::fs::read_to_string(self.logs.path().join("audit.jsonl"))
            .unwrap_or_default()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    fn events(&self, event: &str) -> Vec<Value> {
        self.audit()
            .into_iter()
            .filter(|r| r["event_type"] == event)
            .collect()
    }
}

fn config(root: &Path, url: Option<&str>, max_rows: u32) -> Config {
    let mut env: HashMap<String, String> = HashMap::new();
    env.insert(ENV_TOKEN.into(), "tok".into());
    env.insert(ENV_ALLOWED_ROOTS.into(), root.display().to_string());
    env.insert(ENV_SHUNT_MIN_LINES.into(), "150".into());
    env.insert(
        ENV_SHUNT_CONTENT_CACHE_MAX_ROWS.into(),
        max_rows.to_string(),
    );
    // Blank, not absent: an absent worker variable falls back to the
    // machine's .mmhmem.json on dev hosts.
    env.insert(ENV_WORKER_MODEL.into(), String::new());
    env.insert(ENV_WORKER_API_URL.into(), String::new());
    if let Some(url) = url {
        env.insert(ENV_WORKER_API_URL.into(), url.into());
        env.insert(ENV_WORKER_MODEL.into(), "stub-worker".into());
    }
    Config::from_env(&|k| env.get(k).cloned()).unwrap()
}

fn rig(root: &Path, url: Option<&str>, max_rows: u32) -> Rig {
    let cfg = config(root, url, max_rows);
    let worker = WorkerClient::from_config(&cfg).ok();
    let data = tempfile::tempdir().unwrap();
    let logs = tempfile::tempdir().unwrap();
    let db = Db::open(data.path()).unwrap();
    let audit = AuditWriter::new(logs.path().join("audit.jsonl"));
    let engine = ShuntEngine::new(db.clone(), audit, cfg, None, worker);
    Rig {
        _data: data,
        logs,
        db,
        engine,
    }
}

fn read<'a>(path: &'a str, question: &'a str, answer_question: bool) -> ReadRequest<'a> {
    ReadRequest {
        project: "p",
        harness: "h",
        task: None,
        path,
        question,
        offset: None,
        limit: None,
        answer_question,
    }
}

fn write_file(dir: &Path, name: &str, tag: &str) -> PathBuf {
    let path = dir.join(name);
    let body: String = (1..=LINES).map(|i| format!("{tag} line {i}\n")).collect();
    std::fs::write(&path, body).unwrap();
    path
}

fn canon(path: &Path) -> String {
    std::fs::canonicalize(path).unwrap().display().to_string()
}

/// (sha, fingerprint, kind, summary, hits, last_path) for every row.
type Row = (String, String, String, String, i64, Option<String>);

async fn rows(db: &Db) -> Vec<Row> {
    db.with_conn(|c| {
        let mut s = c.prepare(
            "SELECT content_sha256, query_fingerprint, kind, structural_summary, hits, last_path \
               FROM shunt_content_cache ORDER BY created_at_epoch, query_fingerprint",
        )?;
        let mapped = s.query_map([], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
            ))
        })?;
        mapped.collect()
    })
    .await
    .unwrap()
}

async fn file_cache_rows(db: &Db) -> i64 {
    db.with_conn(|c| c.query_row("SELECT COUNT(*) FROM shunt_file_cache", [], |r| r.get(0)))
        .await
        .unwrap()
}

async fn stat_kinds(db: &Db) -> Vec<String> {
    db.with_conn(|c| {
        let mut s = c.prepare("SELECT kind FROM token_stats ORDER BY id")?;
        let mapped = s.query_map([], |r| r.get(0))?;
        mapped.collect()
    })
    .await
    .unwrap()
}

fn find_subslice(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// Multi-shot HTTP stub (tests/shunt.rs pattern): counts requests and
/// keeps every body so the worker's question can be checked.
async fn spawn_stub(max: usize) -> (String, Arc<AtomicUsize>, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let count = Arc::new(AtomicUsize::new(0));
    let bodies: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let (c_count, c_bodies) = (Arc::clone(&count), Arc::clone(&bodies));
    tokio::spawn(async move {
        for _ in 0..max {
            let Ok((mut sock, _)) = listener.accept().await else {
                break;
            };
            c_count.fetch_add(1, Ordering::SeqCst);
            let mut buf: Vec<u8> = Vec::new();
            let mut tmp = [0u8; 8192];
            let header_end = loop {
                let r = sock.read(&mut tmp).await.unwrap();
                if r == 0 {
                    break buf.len();
                }
                buf.extend_from_slice(&tmp[..r]);
                if let Some(pos) = find_subslice(&buf, b"\r\n\r\n") {
                    break pos + 4;
                }
            };
            let head = String::from_utf8_lossy(&buf[..header_end]).to_lowercase();
            let need: usize = head
                .lines()
                .find_map(|l| l.strip_prefix("content-length:"))
                .and_then(|v| v.trim().parse().ok())
                .unwrap_or(0);
            let mut body = buf[header_end..].to_vec();
            while body.len() < need {
                let r = sock.read(&mut tmp).await.unwrap();
                if r == 0 {
                    break;
                }
                body.extend_from_slice(&tmp[..r]);
            }
            c_bodies
                .lock()
                .unwrap()
                .push(String::from_utf8_lossy(&body).into_owned());
            let resp = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{SUMMARY_CHAT}",
                SUMMARY_CHAT.len()
            );
            let _ = sock.write_all(resp.as_bytes()).await;
        }
    });
    (format!("http://{addr}/v1/chat/completions"), count, bodies)
}

// ---------- 1.1: canonical fingerprint and the shared table ----------

#[test]
fn canonical_fingerprint_is_the_default_question_and_absent_or_empty_is_canonical() {
    assert_eq!(canonical_fingerprint(), query_fingerprint(DEFAULT_QUESTION));
    for q in [
        "",
        "   ",
        DEFAULT_QUESTION,
        &format!("  {}  ", DEFAULT_QUESTION.to_uppercase()),
    ] {
        assert!(is_canonical_question(q), "canonical: {q:?}");
    }
    assert!(!is_canonical_question(CUSTOM));
}

#[tokio::test]
async fn same_content_under_two_paths_is_one_row_and_a_hit_with_last_path_updated() {
    let root = tempfile::tempdir().unwrap();
    let (url, calls, _) = spawn_stub(4).await;
    let rig = rig(root.path(), Some(&url), 5000);
    let first = write_file(root.path(), "a.rs", "same");
    std::fs::create_dir(root.path().join("worktree")).unwrap();
    let copy = root.path().join("worktree").join("a_copy.rs");
    std::fs::copy(&first, &copy).unwrap();

    let out1 = rig
        .engine
        .smart_read(&read(first.to_str().unwrap(), DEFAULT_QUESTION, false))
        .await
        .unwrap();
    assert!(matches!(out1, ReadOutcome::Shunted { .. }), "{out1:?}");
    let out2 = rig
        .engine
        .smart_read(&read(copy.to_str().unwrap(), "", false))
        .await
        .unwrap();
    assert!(matches!(out2, ReadOutcome::Cached { .. }), "{out2:?}");
    assert_eq!(calls.load(Ordering::SeqCst), 1, "the copy is a cache hit");

    let rows = rows(&rig.db).await;
    assert_eq!(rows.len(), 1, "one row per content: {rows:?}");
    let (sha, fp, kind, _, hits, last_path) = &rows[0];
    assert_eq!(sha, &sha256_hex(&std::fs::read(&first).unwrap()));
    assert_eq!(fp, canonical_fingerprint());
    assert_eq!(kind, "file_read");
    assert_eq!(*hits, 1);
    assert_eq!(last_path.as_deref(), Some(canon(&copy).as_str()));
    assert_eq!(file_cache_rows(&rig.db).await, 0, "old table not written");
}

#[tokio::test]
async fn summary_lookup_touches_hits_and_last_path_and_a_miss_writes_nothing() {
    let data = tempfile::tempdir().unwrap();
    let db = Db::open(data.path()).unwrap();
    let sha = sha256_hex(b"content");
    let (p1, p2) = (Path::new("/repo/a.rs"), Path::new("/worktree/a.rs"));
    let miss = summary_lookup(&db, &sha, canonical_fingerprint(), p1)
        .await
        .unwrap();
    assert!(miss.is_none());
    assert!(rows(&db).await.is_empty(), "a miss writes nothing");
    let row = SummaryRow {
        sha: &sha,
        fingerprint: canonical_fingerprint(),
        summary: "canonical",
        line_count: 400,
        path: p1,
    };
    assert_eq!(summary_store_and_evict(&db, row, 5000).await.unwrap(), 0);
    assert_eq!(rows(&db).await[0].5.as_deref(), Some("/repo/a.rs"));
    let hit = summary_lookup(&db, &sha, canonical_fingerprint(), p2)
        .await
        .unwrap();
    assert_eq!(hit.as_deref(), Some("canonical"));
    let rows = rows(&db).await;
    assert_eq!(
        (rows[0].4, rows[0].5.as_deref()),
        (1, Some("/worktree/a.rs"))
    );
}

#[tokio::test]
async fn a_custom_question_answer_is_a_separate_row() {
    let data = tempfile::tempdir().unwrap();
    let db = Db::open(data.path()).unwrap();
    let sha = sha256_hex(b"content");
    let custom_fp = query_fingerprint(CUSTOM);
    for (fp, summary) in [
        (canonical_fingerprint(), "canonical"),
        (custom_fp.as_str(), "custom"),
    ] {
        let row = SummaryRow {
            sha: &sha,
            fingerprint: fp,
            summary,
            line_count: 400,
            path: Path::new("/repo/a.rs"),
        };
        summary_store_and_evict(&db, row, 5000).await.unwrap();
    }
    assert_eq!(rows(&db).await.len(), 2);
    let path = Path::new("/repo/a.rs");
    let canonical = summary_lookup(&db, &sha, canonical_fingerprint(), path).await;
    let custom = summary_lookup(&db, &sha, &custom_fp, path).await;
    assert_eq!(canonical.unwrap().as_deref(), Some("canonical"));
    assert_eq!(custom.unwrap().as_deref(), Some("custom"));
}

#[tokio::test]
async fn eviction_beyond_the_cap_is_audited_with_the_count() {
    let root = tempfile::tempdir().unwrap();
    let (url, calls, _) = spawn_stub(4).await;
    let rig = rig(root.path(), Some(&url), 1);
    for (name, tag) in [("a.rs", "alpha"), ("b.rs", "beta")] {
        let path = write_file(root.path(), name, tag);
        rig.engine
            .smart_read(&read(path.to_str().unwrap(), DEFAULT_QUESTION, false))
            .await
            .unwrap();
    }
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    let rows = rows(&rig.db).await;
    assert_eq!(rows.len(), 1, "cap of one row: {rows:?}");
    let evictions = rig.events("CONTENT_CACHE_EVICT");
    assert_eq!(evictions.len(), 1, "one eviction event: {evictions:?}");
    assert_eq!(evictions[0]["details"]["evicted"], 1);
    assert_eq!(evictions[0]["details"]["source"], "file");
    assert_eq!(evictions[0]["details"]["kind"], "file_read");
}

// ---------- 2.1: serve canonical first, custom answers on request ----------

#[tokio::test]
async fn custom_question_without_the_flag_produces_the_canonical_with_the_note() {
    let root = tempfile::tempdir().unwrap();
    let (url, calls, bodies) = spawn_stub(4).await;
    let rig = rig(root.path(), Some(&url), 5000);
    let path = write_file(root.path(), "a.rs", "alpha");
    let out = rig
        .engine
        .smart_read(&read(path.to_str().unwrap(), CUSTOM, false))
        .await
        .unwrap();
    match &out {
        ReadOutcome::Shunted { note, .. } => {
            assert_eq!(note.as_deref(), Some(QUESTION_NOT_APPLIED));
        }
        other => panic!("expected Shunted, got {other:?}"),
    }
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let body = bodies.lock().unwrap()[0].clone();
    assert!(
        body.contains("control flow with exact line ranges"),
        "{body}"
    );
    assert!(!body.contains(CUSTOM), "the custom question is not sent");
    let rows = rows(&rig.db).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].1, canonical_fingerprint(), "canonical, not custom");
}

#[tokio::test]
async fn custom_question_without_the_flag_serves_the_cached_canonical_with_no_worker_call() {
    let root = tempfile::tempdir().unwrap();
    let (url, calls, _) = spawn_stub(4).await;
    let rig = rig(root.path(), Some(&url), 5000);
    let path = write_file(root.path(), "a.rs", "alpha");
    let path = path.to_str().unwrap();
    let first = rig.engine.smart_read(&read(path, "", false)).await.unwrap();
    match &first {
        ReadOutcome::Shunted { note, .. } => assert!(note.is_none(), "default: no note"),
        other => panic!("expected Shunted, got {other:?}"),
    }
    let out = rig
        .engine
        .smart_read(&read(path, CUSTOM, false))
        .await
        .unwrap();
    match &out {
        ReadOutcome::Cached { summary, note, .. } => {
            assert_eq!(summary, "- foo [L1-L9]\n- bar [L10-L20]");
            assert_eq!(note.as_deref(), Some(QUESTION_NOT_APPLIED));
        }
        other => panic!("expected Cached, got {other:?}"),
    }
    assert_eq!(calls.load(Ordering::SeqCst), 1, "no worker call");
    let hit = &rig.events("IO_SHUNT_CACHE_HIT")[0]["details"];
    assert_eq!(
        (&hit["question"], &hit["served"]),
        (&"custom".into(), &"canonical".into())
    );
    let delegated = &rig.events("IO_SHUNT_DELEGATED")[0]["details"];
    assert_eq!(
        (&delegated["question"], &delegated["served"]),
        (&"default".into(), &"canonical".into())
    );
}

#[tokio::test]
async fn answer_question_caches_a_custom_row_that_is_served_next_time() {
    let root = tempfile::tempdir().unwrap();
    let (url, calls, bodies) = spawn_stub(4).await;
    let rig = rig(root.path(), Some(&url), 5000);
    let path = write_file(root.path(), "a.rs", "alpha");
    let path = path.to_str().unwrap();
    rig.engine.smart_read(&read(path, "", false)).await.unwrap();
    let canonical_before = rows(&rig.db).await;

    let asked = rig
        .engine
        .smart_read(&read(path, CUSTOM, true))
        .await
        .unwrap();
    match &asked {
        ReadOutcome::Shunted { note, .. } => assert!(note.is_none()),
        other => panic!("expected Shunted, got {other:?}"),
    }
    assert_eq!(
        calls.load(Ordering::SeqCst),
        2,
        "one worker call for the answer"
    );
    assert!(
        bodies.lock().unwrap()[1].contains(CUSTOM),
        "the question is asked"
    );
    let rows_now = rows(&rig.db).await;
    assert_eq!(rows_now.len(), 2, "{rows_now:?}");
    assert_eq!(rows_now[0], canonical_before[0], "canonical row untouched");
    assert_eq!(rows_now[1].1, query_fingerprint(CUSTOM));

    let again = rig
        .engine
        .smart_read(&read(path, CUSTOM, false))
        .await
        .unwrap();
    match &again {
        ReadOutcome::Cached { note, .. } => assert!(note.is_none(), "custom row served"),
        other => panic!("expected Cached, got {other:?}"),
    }
    assert_eq!(
        calls.load(Ordering::SeqCst),
        2,
        "served without a worker call"
    );
    assert_eq!(
        rows(&rig.db).await[0],
        canonical_before[0],
        "canonical survives"
    );
    assert_eq!(
        stat_kinds(&rig.db).await,
        ["shunt_delegated", "shunt_delegated", "shunt_cache_hit"]
    );
    let delegated = &rig.events("IO_SHUNT_DELEGATED")[1]["details"];
    assert_eq!(
        (&delegated["question"], &delegated["served"]),
        (&"custom".into(), &"custom".into())
    );
    let hit = &rig.events("IO_SHUNT_CACHE_HIT")[0]["details"];
    assert_eq!(
        (&hit["question"], &hit["served"]),
        (&"custom".into(), &"custom".into())
    );
}
