//! The `ShuntEngine` component (tasks 5.1/5.2; design.md "services layer" /
//! "Components and Interfaces" ShuntEngine row; requirements.md Req 3.1-3.9).
//!
//! Deterministic four-stage smart_read evaluation (design.md "System Flows"):
//!   1. targeted slice (offset+limit) -> exact `L<n>:` lines, never delegated
//!   2. security-critical path OR below-threshold -> direct line-numbered
//!      read, bypass reason audited (Req 3.2, 3.3)
//!   3. content-addressed cache hit -> cached summary (task 5.2)
//!   4. cache miss -> worker-model shunt (task 5.2)
//!
//! Task 5.1 implements stages 1-2 plus the stage-3/4 decision
//! ([`Evaluation::Shuntable`]); task 5.2 consumes it with cache + worker.
//!
//! Path confinement (design.md "Security Considerations" + the amended
//! rule): every agent-supplied path is canonicalized and must lie under one
//! of `AGENT_MEM_ALLOWED_ROOTS`; violations are rejected AND audited.

use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use regex::Regex;

use crate::config::Config;
use crate::services::session_bus::SessionBus;
use crate::services::worker::WorkerClient;
use crate::storage::audit::{sha256_hex, AuditEventType, AuditWriter};
use crate::storage::db::Db;

/// Security-critical path pattern, case-insensitive (Req 3.3; design.md
/// ShuntEngine: auth|crypto|secret|token|rbac|jwt|tls|cert|mutex|lock|
/// concurrency|permission|sudo|pam).
static SECURITY_CRITICAL_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)(auth|crypto|secret|token|rbac|jwt|tls|cert|mutex|lock|concurren|permission|sudo|pam)",
    )
    .expect("valid regex")
});

/// Borrowed view of a file being evaluated (keeps helper arities under the
/// clippy 7-arg limit, Req 13).
struct FileView<'a> {
    path: &'a Path,
    sha: &'a str,
    lines: &'a [&'a str],
    total: usize,
}

/// Why a read bypassed the worker model (Req 3.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BypassReason {
    /// Path matched the security-critical pattern.
    SecurityCritical,
    /// File smaller than the shunt threshold.
    BelowThreshold,
}

impl BypassReason {
    /// Audit string (reference: `security_critical_bypass` / `below_line_threshold`).
    pub fn as_str(self) -> &'static str {
        match self {
            BypassReason::SecurityCritical => "security_critical_bypass",
            BypassReason::BelowThreshold => "below_line_threshold",
        }
    }
}

/// Stage decision for a smart_read call (design.md ShuntEngine).
#[derive(Debug)]
pub enum Evaluation {
    /// Stage 1: exact line-numbered slice for offset+limit (Req 3.1).
    Targeted {
        /// Line-numbered slice text.
        text: String,
        /// 1-based first line returned.
        start: usize,
        /// 1-based last line returned.
        end: usize,
        /// Total lines in the file.
        total_lines: usize,
    },
    /// Stage 2: full line-numbered content, worker bypassed (Req 3.2, 3.3).
    Direct {
        /// Line-numbered full content.
        text: String,
        /// Why the worker was bypassed.
        reason: BypassReason,
        /// Total lines in the file.
        total_lines: usize,
    },
    /// Stages 3-4 input: large, non-critical file (consumed by task 5.2).
    Shuntable {
        /// Confined canonical path.
        path: PathBuf,
        /// SHA-256 of the file content (cache key component, Req 3.4).
        content_sha256: String,
        /// Total lines.
        line_count: usize,
        /// The agent's question (cache fingerprint component, Req 3.4).
        question: String,
    },
}

/// smart_read input/IO errors (Req 3.8 + confinement).
#[derive(Debug)]
pub enum ShuntError {
    /// File does not exist (Req 3.8).
    FileNotFound(String),
    /// Canonicalized path lies outside all allowed roots (audited).
    OutsideAllowedRoots(PathBuf),
    /// IO error reading the file.
    Io(String),
}

impl std::fmt::Display for ShuntError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ShuntError::FileNotFound(p) => write!(f, "file not found: {p}"),
            ShuntError::OutsideAllowedRoots(p) => {
                write!(f, "path outside AGENT_MEM_ALLOWED_ROOTS: {}", p.display())
            }
            ShuntError::Io(e) => write!(f, "io error: {e}"),
        }
    }
}

impl std::error::Error for ShuntError {}

/// The deterministic read-shunting engine (design.md ShuntEngine).
///
/// Task 5.1 stores only what stages 1-2 need; `db`/`bus`/`worker` arrive via
/// the constructor (stable API) and are stored from task 5.2 onward, when the
/// cache/worker stages and breadcrumbs use them.
pub struct ShuntEngine {
    audit: AuditWriter,
    config: Config,
    allowed_roots: Vec<PathBuf>,
}

impl ShuntEngine {
    /// New engine; allowed roots are canonicalized once at construction.
    pub fn new(
        _db: Db,
        audit: AuditWriter,
        config: Config,
        _bus: Option<SessionBus>,
        _worker: Option<WorkerClient>,
    ) -> Self {
        let allowed_roots = config
            .allowed_roots
            .iter()
            .filter_map(|r| std::fs::canonicalize(r).ok())
            .collect();
        Self {
            audit,
            config,
            allowed_roots,
        }
    }

    /// The configured shunt threshold in lines (Req 3.9, default 150).
    pub fn shunt_min_lines(&self) -> u32 {
        self.config.shunt_min_lines
    }

    /// Canonicalize `raw` and require it to lie under an allowed root;
    /// violations are audited (design.md Security Considerations).
    pub fn confine(&self, raw: &str, project: &str, harness: &str) -> Result<PathBuf, ShuntError> {
        let path =
            std::fs::canonicalize(raw).map_err(|_| ShuntError::FileNotFound(raw.to_string()))?;
        let ok = self.allowed_roots.iter().any(|root| path.starts_with(root));
        if !ok {
            let _ = self.audit.write_event(
                AuditEventType::Error,
                harness,
                project,
                serde_json::json!({
                    "error": "path_outside_allowed_roots",
                    "path": path.display().to_string(),
                }),
            );
            return Err(ShuntError::OutsideAllowedRoots(path));
        }
        Ok(path)
    }

    /// The deterministic evaluation (stages 1-2 here; 3-4 decision returned
    /// as [`Evaluation::Shuntable`] for task 5.2).
    pub async fn evaluate(
        &self,
        project: &str,
        harness: &str,
        raw_path: &str,
        question: &str,
        offset: Option<u32>,
        limit: Option<u32>,
    ) -> Result<Evaluation, ShuntError> {
        let path = self.confine(raw_path, project, harness)?;
        let raw = tokio::fs::read_to_string(&path)
            .await
            .map_err(|e| ShuntError::Io(e.to_string()))?;
        let lines: Vec<&str> = raw.lines().collect();
        let sha = sha256_hex(raw.as_bytes());
        let view = FileView {
            path: &path,
            sha: &sha,
            lines: &lines,
            total: lines.len(),
        };
        if let (Some(off), Some(lim)) = (offset, limit) {
            return Ok(self.targeted(&view, off, lim, project, harness));
        }
        if self.is_security_critical(view.path) {
            return self.direct(&view, BypassReason::SecurityCritical, project, harness);
        }
        if (view.total as u32) < self.shunt_min_lines() {
            return self.direct(&view, BypassReason::BelowThreshold, project, harness);
        }
        Ok(Evaluation::Shuntable {
            path: path.clone(), // view borrows path; clone for the owned outcome
            content_sha256: view.sha.to_string(),
            line_count: view.total,
            question: question.to_string(),
        })
    }

    /// Stage 1: exact line-numbered slice (Req 3.1), audited as
    /// IO_TARGETED_READ, never delegated.
    fn targeted(
        &self,
        view: &FileView<'_>,
        offset: u32,
        limit: u32,
        project: &str,
        harness: &str,
    ) -> Evaluation {
        let total = view.total;
        let start = (offset.max(1) as usize).min(total.max(1));
        let end = ((start - 1 + limit as usize).min(total)).max(start);
        let text = view.lines[start - 1..end]
            .iter()
            .enumerate()
            .map(|(i, l)| format!("L{}: {}", start + i, l))
            .collect::<Vec<_>>()
            .join("\n");
        let _ = self.audit.write_event(
            AuditEventType::IoTargetedRead,
            harness,
            project,
            serde_json::json!({
                "file": view.path.display().to_string(), "sha256": view.sha,
                "offset": start, "limit": limit, "total_lines": total,
            }),
        );
        Evaluation::Targeted {
            text,
            start,
            end,
            total_lines: total,
        }
    }

    /// Stage 2: full line-numbered direct read with audited bypass reason
    /// (Req 3.2, 3.3), audited as IO_DIRECT_READ.
    fn direct(
        &self,
        view: &FileView<'_>,
        reason: BypassReason,
        project: &str,
        harness: &str,
    ) -> Result<Evaluation, ShuntError> {
        let text = view
            .lines
            .iter()
            .enumerate()
            .map(|(i, l)| format!("L{}: {}", i + 1, l))
            .collect::<Vec<_>>()
            .join("\n");
        let _ = self.audit.write_event(
            AuditEventType::IoDirectRead,
            harness,
            project,
            serde_json::json!({
                "file": view.path.display().to_string(), "sha256": view.sha,
                "total_lines": view.total, "bypass_reason": reason.as_str(),
            }),
        );
        Ok(Evaluation::Direct {
            text,
            reason,
            total_lines: view.total,
        })
    }

    /// Security-critical path check (Req 3.3, case-insensitive).
    pub fn is_security_critical(&self, path: &Path) -> bool {
        SECURITY_CRITICAL_RE.is_match(&path.to_string_lossy())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn test_config(root: &Path, shunt_min_lines: u32) -> Config {
        let mut env: HashMap<String, String> = HashMap::new();
        env.insert(crate::config::ENV_TOKEN.to_string(), "tok".to_string());
        env.insert(
            crate::config::ENV_ALLOWED_ROOTS.to_string(),
            root.display().to_string(),
        );
        env.insert(
            crate::config::ENV_SHUNT_MIN_LINES.to_string(),
            shunt_min_lines.to_string(),
        );
        Config::from_env(&|k| env.get(k).cloned()).unwrap()
    }

    fn engine(root: &Path) -> (tempfile::TempDir, tempfile::TempDir, ShuntEngine) {
        let data = tempfile::tempdir().unwrap();
        let logs = tempfile::tempdir().unwrap();
        let db = Db::open(data.path()).unwrap();
        let audit = AuditWriter::new(logs.path().join("audit.jsonl"));
        let eng = ShuntEngine::new(db, audit, test_config(root, 150), None, None);
        (data, logs, eng)
    }

    fn write_file(root: &Path, name: &str, lines: usize) -> PathBuf {
        let p = root.join(name);
        let content = (1..=lines)
            .map(|i| format!("line {i}\n"))
            .collect::<String>();
        std::fs::write(&p, content).unwrap();
        p
    }

    #[tokio::test]
    async fn targeted_read_returns_exact_line_numbered_slice() {
        let root = tempfile::tempdir().unwrap();
        let (_d, _l, eng) = engine(root.path());
        let f = write_file(root.path(), "big.rs", 200);
        let ev = eng
            .evaluate("p", "h", f.to_str().unwrap(), "q", Some(10), Some(5))
            .await
            .unwrap();
        match ev {
            Evaluation::Targeted {
                text,
                start,
                end,
                total_lines,
            } => {
                assert_eq!((start, end, total_lines), (10, 14, 200));
                assert!(text.starts_with("L10: line 10"));
                assert!(text.contains("L14: line 14"));
                assert!(!text.contains("L15:"));
            }
            other => panic!("expected Targeted, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn small_file_direct_below_threshold() {
        let root = tempfile::tempdir().unwrap();
        let (_d, _l, eng) = engine(root.path());
        let f = write_file(root.path(), "small.rs", 10);
        let ev = eng
            .evaluate("p", "h", f.to_str().unwrap(), "q", None, None)
            .await
            .unwrap();
        match ev {
            Evaluation::Direct {
                text,
                reason,
                total_lines,
            } => {
                assert_eq!(reason, BypassReason::BelowThreshold);
                assert_eq!(total_lines, 10);
                assert!(text.starts_with("L1: line 1"));
            }
            other => panic!("expected Direct, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn security_critical_path_bypasses_even_when_large() {
        let root = tempfile::tempdir().unwrap();
        let (_d, logs, eng) = engine(root.path());
        let f = write_file(root.path(), "auth_handler.rs", 300);
        assert!(eng.is_security_critical(&f));
        let ev = eng
            .evaluate("p", "h", f.to_str().unwrap(), "q", None, None)
            .await
            .unwrap();
        match ev {
            Evaluation::Direct {
                reason,
                total_lines,
                ..
            } => {
                assert_eq!(reason, BypassReason::SecurityCritical);
                assert_eq!(total_lines, 300);
            }
            other => panic!("expected Direct, got {other:?}"),
        }
        let log = std::fs::read_to_string(logs.path().join("audit.jsonl")).unwrap();
        assert!(log.contains("IO_DIRECT_READ"));
        assert!(log.contains("security_critical_bypass"));
    }

    #[tokio::test]
    async fn large_noncritical_file_is_shuntable_with_sha() {
        let root = tempfile::tempdir().unwrap();
        let (_d, _l, eng) = engine(root.path());
        let f = write_file(root.path(), "big.rs", 200);
        let ev = eng
            .evaluate("p", "h", f.to_str().unwrap(), "what?", None, None)
            .await
            .unwrap();
        match ev {
            Evaluation::Shuntable {
                content_sha256,
                line_count,
                question,
                ..
            } => {
                assert_eq!(line_count, 200);
                assert_eq!(content_sha256.len(), 64);
                assert_eq!(question, "what?");
            }
            other => panic!("expected Shuntable, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn missing_file_is_a_clear_error() {
        let root = tempfile::tempdir().unwrap();
        let (_d, _l, eng) = engine(root.path());
        let err = eng
            .evaluate(
                "p",
                "h",
                root.path().join("nope.rs").to_str().unwrap(),
                "q",
                None,
                None,
            )
            .await
            .unwrap_err();
        assert!(matches!(err, ShuntError::FileNotFound(_)), "{err:?}");
        assert!(err.to_string().contains("file not found"));
    }

    #[tokio::test]
    async fn path_outside_allowed_roots_is_rejected_and_audited() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let (_d, logs, eng) = engine(root.path());
        let f = write_file(outside.path(), "secret.txt", 5);
        let err = eng
            .evaluate("p", "h", f.to_str().unwrap(), "q", None, None)
            .await
            .unwrap_err();
        assert!(matches!(err, ShuntError::OutsideAllowedRoots(_)), "{err:?}");
        let log = std::fs::read_to_string(logs.path().join("audit.jsonl")).unwrap();
        assert!(log.contains("path_outside_allowed_roots"), "log={log}");
    }

    #[tokio::test]
    async fn traversal_attempt_is_rejected() {
        let root = tempfile::tempdir().unwrap();
        let (_d, _l, eng) = engine(root.path());
        let inner = root.path().join("inner");
        std::fs::create_dir(&inner).unwrap();
        write_file(root.path(), "ok.rs", 5);
        let trav = inner.join("..").join("..").join("etc").join("passwd");
        let err = eng
            .evaluate("p", "h", trav.to_str().unwrap(), "q", None, None)
            .await
            .unwrap_err();
        // /etc/passwd exists but is outside the allowed root (or missing).
        assert!(matches!(
            err,
            ShuntError::OutsideAllowedRoots(_) | ShuntError::FileNotFound(_)
        ));
    }

    #[test]
    fn security_regex_covers_all_pattern_classes() {
        for name in [
            "auth.rs",
            "crypto_util.rs",
            "secrets.yaml",
            "token_store.rs",
            "rbac.rs",
            "jwt.rs",
            "tls_config.rs",
            "cert.pem",
            "mutex.rs",
            "lock.rs",
            "concurrency.rs",
            "permissions.rs",
            "sudoers",
            "pam.d",
            "AUTH_UPPER.rs",
        ] {
            assert!(SECURITY_CRITICAL_RE.is_match(name), "must match: {name}");
        }
        for name in ["main.rs", "parser.rs", "readme.md"] {
            assert!(
                !SECURITY_CRITICAL_RE.is_match(name),
                "must NOT match: {name}"
            );
        }
    }
}
