//! Corpus warmup (corpus-warmup-prune Req 1): batch orchestration over the
//! existing per-file shunt stages. Pure orchestration — no engine refactor:
//! every file runs the real `smart_read` path (audits, statistics rows,
//! breadcrumbs land exactly as on a live read), and this module only counts
//! outcomes and reports them honestly. A bad path never aborts the batch
//! (Req 1.4); a missing worker aborts it (Req 1.7) with the engine's own
//! actionable error so the tool surface stays consistent.

use serde::Serialize;

use super::shunt::{ShuntEngine, ShuntError};
use super::shunt_types::{BypassReason, CallCtx, ReadOutcome, ReadRequest};
use crate::storage::audit::{AuditEventType, AuditWriter};

/// Outcome counters for one warmup batch (Req 1.5).
#[derive(Debug, Default, Clone, Copy, Serialize, PartialEq, Eq)]
pub struct WarmupReport {
    /// Files delegated to the worker (real calls, real cache rows).
    pub delegated: u32,
    /// Files served from the content-addressed cache (zero worker calls).
    pub cached: u32,
    /// Files below `SHUNT_MIN_LINES`.
    pub skipped_below_threshold: u32,
    /// Files matching the security-critical pattern.
    pub skipped_security: u32,
    /// Files missing or unreadable (counted, never fatal).
    pub missing: u32,
    /// Files denied by the allowed-roots confinement (counted, never fatal).
    pub denied: u32,
    /// Files whose worker call failed (counted so one flaky call cannot
    /// waste the tokens already spent on the rest of the batch).
    pub failed: u32,
    /// Worker prompt tokens across delegated calls (Req 1.5).
    pub worker_prompt_tokens: u64,
    /// Worker completion tokens across delegated calls (Req 1.5).
    pub worker_completion_tokens: u64,
}

/// Maximum number of paths in one warmup batch (Req 1.6).
pub const MAX_WARMUP_PATHS: usize = 25;

/// Whole-batch failures (Req 1.6, 1.7).
#[derive(Debug)]
pub enum WarmupError {
    /// More than [`MAX_WARMUP_PATHS`] paths given; nothing was processed.
    TooManyPaths { given: usize, cap: usize },
    /// Engine error that aborts the whole batch (Req 1.7: the
    /// worker-unconfigured error propagates with `smart_read` parity).
    Engine(ShuntError),
}

impl std::fmt::Display for WarmupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WarmupError::TooManyPaths { given, cap } => write!(
                f,
                "warmup batch too large: {given} paths (cap {cap}) — warm fewer files per batch"
            ),
            WarmupError::Engine(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for WarmupError {}

/// Runs each path through the real shunt stages and counts the outcomes
/// (Req 1.2-1.5). Per-file misses/denials/failures are counted, never fatal;
/// a missing worker aborts the batch (Req 1.7). One `WARMUP` audit event
/// summarizes the counts (Req 1.8) — per-file events already come from the
/// reused engine path.
pub async fn run_warmup(
    engine: &ShuntEngine,
    audit: &AuditWriter,
    paths: &[String],
    question: &str,
    ctx: CallCtx<'_>,
) -> Result<WarmupReport, WarmupError> {
    if paths.len() > MAX_WARMUP_PATHS {
        return Err(WarmupError::TooManyPaths {
            given: paths.len(),
            cap: MAX_WARMUP_PATHS,
        });
    }
    let mut report = WarmupReport::default();
    for path in paths {
        let req = ReadRequest {
            project: ctx.project,
            harness: ctx.harness,
            task: ctx.task,
            path: path.as_str(),
            question,
            offset: None,
            limit: None,
        };
        match engine.smart_read(&req).await {
            // Not reachable in warmup (no offset/limit), counted defensively.
            Ok(ReadOutcome::Targeted { .. }) => report.skipped_below_threshold += 1,
            Ok(ReadOutcome::Direct { reason, .. }) => match reason {
                BypassReason::SecurityCritical => report.skipped_security += 1,
                BypassReason::BelowThreshold => report.skipped_below_threshold += 1,
            },
            Ok(ReadOutcome::Cached { .. }) => report.cached += 1,
            Ok(ReadOutcome::Shunted { usage, .. }) => {
                report.delegated += 1;
                report.worker_prompt_tokens += usage.prompt_tokens;
                report.worker_completion_tokens += usage.completion_tokens;
            }
            Err(ShuntError::FileNotFound(_)) | Err(ShuntError::Io(_)) => report.missing += 1,
            Err(ShuntError::OutsideAllowedRoots(_)) => report.denied += 1,
            Err(ShuntError::Worker(_)) => report.failed += 1,
            // Req 1.7: worker-unconfigured aborts the whole batch with the
            // engine's own actionable error (smart_read parity).
            Err(e @ ShuntError::WorkerNotConfigured) => return Err(WarmupError::Engine(e)),
        }
    }
    // Req 1.8: one batch summary event (per-file events came from the
    // reused engine path). Best-effort, like every audit write.
    let _ = audit.write_event(
        AuditEventType::Warmup,
        ctx.harness,
        ctx.project,
        serde_json::json!({
            "paths": paths.len(),
            "delegated": report.delegated,
            "cached": report.cached,
            "skipped_below_threshold": report.skipped_below_threshold,
            "skipped_security": report.skipped_security,
            "missing": report.missing,
            "denied": report.denied,
            "failed": report.failed,
            "worker_prompt_tokens": report.worker_prompt_tokens,
            "worker_completion_tokens": report.worker_completion_tokens,
        }),
    );
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::config::{
        Config, ENV_ALLOWED_ROOTS, ENV_TOKEN, ENV_WORKER_API_URL, ENV_WORKER_MODEL,
    };
    use crate::services::shunt_cache::{cache_store, query_fingerprint};
    use crate::services::worker::WorkerClient;
    use crate::storage::audit::sha256_hex;
    use crate::storage::db::Db;

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
}
