//! Types for the shunt engine (split from `shunt.rs` per the Req 13.3
//! 500-LOC-per-src-file cap, split by responsibility).

use std::path::PathBuf;

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
    /// The agent's question.
    pub question: &'a str,
    /// Targeted-read offset (1-based), if given.
    pub offset: Option<u32>,
    /// Targeted-read limit, if given.
    pub limit: Option<u32>,
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
    },
    /// Stage 4: worker-delegated summary (Req 3.5).
    Shunted {
        summary: String,
        latency_ms: u64,
        avoided_input_tokens_est: u64,
    },
}

/// smart_read input/IO errors (Req 3.8 + confinement + worker failures).
#[derive(Debug)]
pub enum ShuntError {
    /// File does not exist (Req 3.8).
    FileNotFound(String),
    /// Canonicalized path lies outside all allowed roots (audited).
    OutsideAllowedRoots(PathBuf),
    /// IO error reading the file.
    Io(String),
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
