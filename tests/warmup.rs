//! Warmup orchestration tests (corpus-warmup-prune tasks 1.1/1.2;
//! Req 1.2-1.8). Moved here from src/services/warmup.rs to keep the src
//! file under the Req 13.3 500-LOC cap (tests/shunt.rs precedent).

use mmhmem::config::{Config, ENV_ALLOWED_ROOTS, ENV_TOKEN, ENV_WORKER_API_URL, ENV_WORKER_MODEL};
use mmhmem::services::shunt::{ShuntEngine, ShuntError};
use mmhmem::services::shunt_cache::{cache_store, query_fingerprint};
use mmhmem::services::shunt_types::CallCtx;
use mmhmem::services::warmup::{WarmupError, WarmupReport, render_report, run_warmup};
use mmhmem::services::worker::WorkerClient;
use mmhmem::storage::audit::{AuditWriter, sha256_hex};
use mmhmem::storage::db::Db;

const QUESTION: &str = "Summarize exported symbols, signatures, and control flow.";

fn test_config(root: &std::path::Path) -> Config {
    let root = root.display().to_string();
    Config::from_env(&|k| match k {
        ENV_TOKEN => Some("tok".to_string()),
        ENV_ALLOWED_ROOTS => Some(root.clone()),
        _ => None,
    })
    .expect("valid env")
}

/// Keeps the tempdirs and the shared audit writer alive for the engine's
/// lifetime (the engine holds a clone of the writer).
struct Env {
    _data: tempfile::TempDir,
    _logs: tempfile::TempDir,
    engine: ShuntEngine,
    audit: AuditWriter,
    audit_path: std::path::PathBuf,
}

fn setup(root: &std::path::Path, worker: Option<WorkerClient>) -> Env {
    let data = tempfile::tempdir().expect("tempdir");
    let logs = tempfile::tempdir().expect("tempdir");
    let audit_path = logs.path().join("audit.jsonl");
    let db = Db::open(data.path()).expect("db");
    let audit = AuditWriter::new(audit_path.clone());
    let engine = ShuntEngine::new(db, audit.clone(), test_config(root), None, worker);
    Env {
        _data: data,
        _logs: logs,
        engine,
        audit,
        audit_path,
    }
}

fn write_lines(dir: &std::path::Path, name: &str, count: usize) -> String {
    let path = dir.join(name);
    let body: String = (0..count).map(|i| format!("line {i} of many\n")).collect();
    std::fs::write(&path, body).expect("write file");
    path.display().to_string()
}

fn ctx() -> CallCtx<'static> {
    CallCtx {
        project: "p",
        harness: "h",
        task: None,
    }
}

fn audit_text(env: &Env) -> String {
    std::fs::read_to_string(&env.audit_path).unwrap_or_default()
}

#[tokio::test]
async fn cap_exceeded_errors_before_any_work() {
    // Req 1.6: more than 25 paths → error, no side effects, no event.
    let dir = tempfile::tempdir().expect("tempdir");
    let env = setup(dir.path(), None);
    let many: Vec<String> = (0..26).map(|i| format!("missing-{i}.rs")).collect();
    let err = run_warmup(&env.engine, &env.audit, &many, QUESTION, ctx())
        .await
        .expect_err("26 paths must fail");
    assert!(matches!(
        err,
        WarmupError::TooManyPaths { given: 26, cap: 25 }
    ));
    assert!(
        !audit_text(&env).contains("WARMUP"),
        "no audit event may be written for a rejected batch"
    );
}

#[tokio::test]
async fn cap_boundary_of_25_is_accepted() {
    // Req 1.6: exactly 25 paths is a valid batch (all missing → counted).
    let dir = tempfile::tempdir().expect("tempdir");
    let env = setup(dir.path(), None);
    let many: Vec<String> = (0..25).map(|i| format!("missing-{i}.rs")).collect();
    let report = run_warmup(&env.engine, &env.audit, &many, QUESTION, ctx())
        .await
        .expect("25 paths is the cap, not over it");
    assert_eq!(report.missing, 25);
    assert_eq!(report.delegated, 0);
}

#[tokio::test]
async fn cached_row_counts_as_cached_without_worker() {
    // Req 1.3: a seeded cache row serves the file with zero worker calls.
    let dir = tempfile::tempdir().expect("tempdir");
    let big = write_lines(dir.path(), "big.rs", 200);
    let data = tempfile::tempdir().expect("tempdir");
    let logs = tempfile::tempdir().expect("tempdir");
    let db = Db::open(data.path()).expect("db");
    let audit = AuditWriter::new(logs.path().join("audit.jsonl"));
    // Seed through the same cache API the engine uses. The key must use
    // the canonicalized path — the engine looks up confined paths.
    let fp = query_fingerprint(QUESTION);
    let body = std::fs::read(&big).expect("read");
    let file_sha = sha256_hex(&body);
    let canonical = std::fs::canonicalize(&big).expect("canonicalize");
    cache_store(&db, &canonical, &file_sha, &fp, "seeded summary", 200)
        .await
        .expect("seed");
    let engine = ShuntEngine::new(db, audit.clone(), test_config(dir.path()), None, None);
    let report = run_warmup(&engine, &audit, &[big], QUESTION, ctx())
        .await
        .expect("cache hit needs no worker");
    assert_eq!(report.cached, 1);
    assert_eq!(report.delegated, 0);
}

#[tokio::test]
async fn below_threshold_is_skipped_and_counted() {
    let dir = tempfile::tempdir().expect("tempdir");
    let small = write_lines(dir.path(), "small.rs", 10);
    let env = setup(dir.path(), None);
    let report = run_warmup(&env.engine, &env.audit, &[small], QUESTION, ctx())
        .await
        .expect("small files are skipped, not errors");
    assert_eq!(report.skipped_below_threshold, 1);
    assert_eq!(report.delegated, 0);
}

#[tokio::test]
async fn security_critical_is_skipped_and_counted() {
    let dir = tempfile::tempdir().expect("tempdir");
    let big = write_lines(dir.path(), "auth_utils.rs", 200);
    let env = setup(dir.path(), None);
    let report = run_warmup(&env.engine, &env.audit, &[big], QUESTION, ctx())
        .await
        .expect("security-critical files bypass the worker");
    assert_eq!(report.skipped_security, 1);
    assert_eq!(report.delegated, 0);
}

#[tokio::test]
async fn missing_file_is_counted_and_never_fatal() {
    // Req 1.4: a missing file counts as missing; the batch continues.
    let dir = tempfile::tempdir().expect("tempdir");
    let small = write_lines(dir.path(), "small.rs", 10);
    let batch = [String::from("no-such-file.rs"), small];
    let env = setup(dir.path(), None);
    let report = run_warmup(&env.engine, &env.audit, &batch, QUESTION, ctx())
        .await
        .expect("missing files never abort the batch");
    assert_eq!(report.missing, 1);
    assert_eq!(report.skipped_below_threshold, 1);
}

#[tokio::test]
async fn denied_path_is_counted_and_never_fatal() {
    // Req 1.4: confinement denials are counted, not fatal. The file
    // exists but lies outside the allowed root (the tempdir under it).
    let inside = tempfile::tempdir().expect("tempdir");
    let outside = tempfile::tempdir().expect("tempdir");
    let big = write_lines(outside.path(), "big.rs", 200);
    let env = setup(inside.path(), None);
    let report = run_warmup(&env.engine, &env.audit, &[big], QUESTION, ctx())
        .await
        .expect("denials never abort the batch");
    assert_eq!(report.denied, 1);
    assert_eq!(report.delegated, 0);
}

#[tokio::test]
async fn worker_failure_is_counted_and_never_fatal() {
    // A real-but-dead worker endpoint: the call fails, the file counts
    // as failed, the batch continues.
    let dir = tempfile::tempdir().expect("tempdir");
    let big = write_lines(dir.path(), "big.rs", 200);
    let root = dir.path().display().to_string();
    let cfg = Config::from_env(&|k| match k {
        ENV_TOKEN => Some("tok".to_string()),
        ENV_ALLOWED_ROOTS => Some(root.clone()),
        ENV_WORKER_API_URL => Some("http://127.0.0.1:1/v1/chat/completions".to_string()),
        ENV_WORKER_MODEL => Some("dead-worker".to_string()),
        _ => None,
    })
    .expect("valid env");
    let worker = WorkerClient::from_config(&cfg).ok();
    assert!(worker.is_some(), "client construction must succeed offline");
    let env = setup(dir.path(), worker);
    let report = run_warmup(&env.engine, &env.audit, &[big], QUESTION, ctx())
        .await
        .expect("worker failures never abort the batch");
    assert_eq!(report.failed, 1);
    assert_eq!(report.delegated, 0);
}

#[tokio::test]
async fn worker_unconfigured_aborts_with_engine_parity() {
    // Req 1.7: without a worker, a delegable file aborts the batch with
    // the engine's own actionable error (smart_read parity).
    let dir = tempfile::tempdir().expect("tempdir");
    let big = write_lines(dir.path(), "big.rs", 200);
    let env = setup(dir.path(), None);
    let err = run_warmup(&env.engine, &env.audit, &[big], QUESTION, ctx())
        .await
        .expect_err("no worker configured");
    assert!(matches!(
        err,
        WarmupError::Engine(ShuntError::WorkerNotConfigured)
    ));
    assert_eq!(
        err.to_string(),
        ShuntError::WorkerNotConfigured.to_string(),
        "the error must match smart_read's wording"
    );
}

#[tokio::test]
async fn one_audit_event_summarizes_the_batch() {
    // Req 1.8: exactly one WARMUP event with the counts.
    let dir = tempfile::tempdir().expect("tempdir");
    let small = write_lines(dir.path(), "small.rs", 10);
    let batch = [small, String::from("no-such-file.rs")];
    let env = setup(dir.path(), None);
    let report = run_warmup(&env.engine, &env.audit, &batch, QUESTION, ctx())
        .await
        .expect("mixed batch succeeds");
    assert_eq!(report.skipped_below_threshold, 1);
    assert_eq!(report.missing, 1);
    let warmup_events = audit_text(&env)
        .lines()
        .filter(|l| l.contains("WARMUP"))
        .count();
    assert_eq!(warmup_events, 1, "exactly one batch summary event");
}

#[tokio::test]
async fn notes_collect_per_file_reasons_in_batch_order() {
    // Task 1.2: every non-delegated, non-cached outcome carries its
    // path + reason, in batch order.
    let dir = tempfile::tempdir().expect("tempdir");
    let small = write_lines(dir.path(), "small.rs", 10);
    let auth = write_lines(dir.path(), "auth_utils.rs", 200);
    let batch = [small, String::from("no-such-file.rs"), auth];
    let env = setup(dir.path(), None);
    let report = run_warmup(&env.engine, &env.audit, &batch, QUESTION, ctx())
        .await
        .expect("mixed batch succeeds");
    assert_eq!(report.notes.len(), 3);
    assert!(report.notes[0].contains("small.rs"), "{}", report.notes[0]);
    assert!(
        report.notes[0].contains("below threshold"),
        "{}",
        report.notes[0]
    );
    assert!(
        report.notes[1].contains("no-such-file.rs"),
        "{}",
        report.notes[1]
    );
    assert!(report.notes[1].contains("missing"), "{}", report.notes[1]);
    assert!(
        report.notes[2].contains("auth_utils.rs"),
        "{}",
        report.notes[2]
    );
    assert!(report.notes[2].contains("security"), "{}", report.notes[2]);
}

#[test]
fn render_shows_all_counters_tokens_and_details() {
    // Req 1.5: the response text carries every counter, the worker
    // token totals, and the per-file details list.
    let report = WarmupReport {
        delegated: 2,
        cached: 1,
        skipped_below_threshold: 3,
        skipped_security: 1,
        missing: 1,
        denied: 1,
        failed: 1,
        worker_prompt_tokens: 4242,
        worker_completion_tokens: 1717,
        notes: vec!["big.rs: below threshold (10 lines)".to_string()],
    };
    let md = render_report(&report);
    for expected in ["2", "1", "3", "4242", "1717", "big.rs", "below threshold"] {
        assert!(
            md.contains(expected),
            "render must mention {expected}: {md}"
        );
    }
}

#[test]
fn render_omits_details_when_no_notes() {
    let report = WarmupReport {
        delegated: 1,
        ..WarmupReport::default()
    };
    let md = render_report(&report);
    assert!(!md.contains("Per-file details"), "no notes section: {md}");
    assert!(md.contains("1"), "counters still shown: {md}");
}
