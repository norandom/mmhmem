//! Types for the shunt engine (split from `shunt.rs` per the Req 13.3
//! 500-LOC-per-src-file cap, split by responsibility).

use std::path::PathBuf;

use super::worker::WorkerUsage;

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

/// Per-call context shared by all smart_read stages.
#[derive(Debug, Clone, Copy)]
pub struct CallCtx<'a> {
    /// Project name.
    pub project: &'a str,
    /// Calling harness.
    pub harness: &'a str,
    /// Workspace task ID, if known.
    pub task: Option<&'a str>,
}

/// One smart_read call's input (bundled to stay under the clippy 7-arg limit).
#[derive(Debug, Clone)]
pub struct ReadRequest<'a> {
    /// Project name.
    pub project: &'a str,
    /// Calling harness.
    pub harness: &'a str,
    /// Workspace task ID, if known.
    pub task: Option<&'a str>,
    /// Raw (uncanonicalized) agent-supplied path.
    pub path: &'a str,
    /// The agent's question; empty or the default question asks for the
    /// shared structural summary (shunt-cache-efficiency Req 1.1).
    pub question: &'a str,
    /// Targeted-read offset (1-based), if given.
    pub offset: Option<u32>,
    /// Targeted-read limit, if given.
    pub limit: Option<u32>,
    /// Ask the worker to answer a custom `question` when no answer to it is
    /// cached, instead of serving the structural summary (Req 2.3).
    pub answer_question: bool,
}

impl<'a> ReadRequest<'a> {
    /// The shared call context.
    pub fn ctx(&self) -> CallCtx<'a> {
        CallCtx {
            project: self.project,
            harness: self.harness,
            task: self.task,
        }
    }
}

/// Stage decision for a smart_read call (stages 1-2 resolved, 3-4 as input).
#[derive(Debug)]
pub enum Evaluation {
    /// Stage 1 resolved.
    Targeted {
        text: String,
        start: usize,
        end: usize,
        total_lines: usize,
    },
    /// Stage 2 resolved.
    Direct {
        text: String,
        reason: BypassReason,
        total_lines: usize,
    },
    /// Stages 3-4 input.
    Shuntable {
        path: PathBuf,
        content_sha256: String,
        line_count: usize,
        question: String,
        answer_question: bool,
    },
}

/// Final smart_read outcome returned to the tool handler (design.md ReadOutcome).
#[derive(Debug)]
pub enum ReadOutcome {
    /// Stage 1: exact line-numbered slice (Req 3.1).
    Targeted {
        text: String,
        start: usize,
        end: usize,
        total_lines: usize,
    },
    /// Stage 2: full line-numbered direct read (Req 3.2, 3.3).
    Direct {
        text: String,
        reason: BypassReason,
        total_lines: usize,
    },
    /// Stage 3: cached summary, zero worker calls (Req 3.6).
    Cached {
        summary: String,
        avoided_input_tokens_est: u64,
        /// [`QUESTION_NOT_APPLIED`] when a custom question got the shared
        /// structural summary (shunt-cache-efficiency Req 2.2).
        note: Option<String>,
    },
    /// Stage 4: worker-delegated summary (Req 3.5).
    Shunted {
        summary: String,
        latency_ms: u64,
        avoided_input_tokens_est: u64,
        /// Worker token usage for this call (carried so batch warmup can
        /// report honest worker token totals, corpus-warmup-prune Req 1.5).
        usage: WorkerUsage,
        /// [`QUESTION_NOT_APPLIED`] when a custom question got the shared
        /// structural summary (shunt-cache-efficiency Req 2.2).
        note: Option<String>,
    },
}

/// One-line note appended when a custom question was answered with the
/// shared structural summary (shunt-cache-efficiency Req 2.2).
pub const QUESTION_NOT_APPLIED: &str = "Served the shared structural summary; the question was not applied. \
     Call smart_read again with answer_question: true for an answer to it.";

/// smart_read input/IO errors (Req 3.8 + confinement + worker failures).
#[derive(Debug)]
pub enum ShuntError {
    /// File does not exist (Req 3.8).
    FileNotFound(String),
    /// Canonicalized path lies outside all allowed roots (audited).
    OutsideAllowedRoots(PathBuf),
    /// IO error reading the file.
    Io(String),
    /// Invalid request on the requested shunt path — service-level
    /// re-check of handler preconditions such as the content size guard
    /// (shunt-content-tool design.md "Error Handling").
    Invalid(String),
    /// File is shuntable but no worker model is configured (Req 9.4).
    WorkerNotConfigured,
    /// Worker call failed (Req 6.3) — audited as WORKER_ERROR.
    Worker(String),
}

impl std::fmt::Display for ShuntError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ShuntError::FileNotFound(p) => write!(f, "file not found: {p}"),
            ShuntError::OutsideAllowedRoots(p) => {
                write!(f, "path outside AGENT_MEM_ALLOWED_ROOTS: {}", p.display())
            }
            ShuntError::Io(e) => write!(f, "io error: {e}"),
            ShuntError::Invalid(e) => write!(f, "invalid shunt request: {e}"),
            ShuntError::WorkerNotConfigured => write!(
                f,
                "file exceeds the shunt threshold but no worker model is configured (WORKER_MODEL)"
            ),
            ShuntError::Worker(e) => write!(f, "worker model call failed: {e}"),
        }
    }
}

impl std::error::Error for ShuntError {}

/// Borrowed view of a file being evaluated (keeps helper arities under the
/// clippy 7-arg limit, Req 13).
pub struct FileView<'a> {
    /// Canonical confined path.
    pub path: &'a std::path::Path,
    /// SHA-256 hex of the content.
    pub sha: &'a str,
    /// Lines.
    pub lines: &'a [&'a str],
    /// Total line count.
    pub total: usize,
}

/// Where shunted content comes from (shunt-content-tool design.md
/// "ShuntSource seam"): a confined repository file (`smart_read`) or
/// caller-supplied content with an optional label (`shunt_content`). One
/// enum selects the cache table, payload builder and audit labels while
/// the engine's stages stay shared (Req 6.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShuntSource {
    /// A canonical, confined file path (smart_read path).
    File(PathBuf),
    /// Caller-supplied content; the hint is a label only — never opened,
    /// never confined (shunt_content path, Req 1.5).
    Content {
        /// Optional caller source hint, trimmed to 256 chars at
        /// construction (design.md shunt_content API contract).
        hint: Option<String>,
    },
}

/// Hint length cap for the content source label (design.md `source_hint`).
const MAX_HINT_CHARS: usize = 256;

impl ShuntSource {
    /// Content source from an optional caller hint; the hint is trimmed to
    /// [`MAX_HINT_CHARS`] here so every later use (audit, breadcrumbs) is
    /// already bounded.
    pub fn content(hint: Option<&str>) -> Self {
        ShuntSource::Content {
            hint: hint.map(|h| h.chars().take(MAX_HINT_CHARS).collect()),
        }
    }

    /// The explicit audit label: `"file"` or `"content"`.
    pub fn label(&self) -> &'static str {
        match self {
            ShuntSource::File(_) => "file",
            ShuntSource::Content { .. } => "content",
        }
    }

    /// Base audit details for a shunt outcome: the explicit `source` label
    /// plus the subject — the file path, or the trimmed hint for content
    /// (no `hint` key when the caller supplied none).
    pub fn audit_details(&self) -> serde_json::Value {
        match self {
            ShuntSource::File(path) => serde_json::json!({
                "source": self.label(),
                "file": path.display().to_string(),
            }),
            ShuntSource::Content { hint } => {
                let mut v = serde_json::json!({"source": self.label()});
                if let Some(h) = hint {
                    v["hint"] = serde_json::Value::from(h.as_str());
                }
                v
            }
        }
    }

    /// The `token_stats` details `file` column value: the path for files,
    /// `content:<sha12>` for content — keeps the token_stats columns
    /// unchanged (design.md ShuntSource seam).
    pub fn stat_subject(&self, sha: &str) -> String {
        match self {
            ShuntSource::File(path) => path.display().to_string(),
            ShuntSource::Content { .. } => format!("content:{}", &sha[..12.min(sha.len())]),
        }
    }

    /// Breadcrumb resource label for this source.
    pub fn crumb_resource(&self) -> String {
        match self {
            ShuntSource::File(path) => path.display().to_string(),
            ShuntSource::Content { hint } => {
                hint.clone().unwrap_or_else(|| self.label().to_string())
            }
        }
    }
}
