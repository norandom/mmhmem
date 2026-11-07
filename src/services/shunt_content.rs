//! Content-based shunt service (shunt-content-tool design.md "services /
//! shunt_content"). Task 1.1 defined the request, kind and outcome types
//! plus the wire result contract renderer; task 2.1 adds the evaluation:
//! the content size re-check, the documented secret pattern set
//! (`CONTENT_SECRET_RE`), the hint-path security check and the line-count
//! threshold, in the design's order, with hash-only bypass audits and no
//! cache row / no statistics on bypass. The cache-hit and delegation
//! stages land with task 2.2 in the same impl block.
//! Services may import `config` and `storage`, never `mcp` (steering
//! structure.md) — the MCP handler renders, this module decides.

use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;
use serde_json::{Value, json};

use super::security_path::is_security_critical_path;
use super::shunt::ShuntEngine;
use super::shunt_types::{BypassReason, CallCtx, ShuntError, ShuntSource};
use super::worker::WorkerUsage;
use crate::storage::audit::{AuditEventType, sha256_hex};

/// Maximum content size `shunt_content` accepts (design.md Service
/// Interface; 1 MiB bounds the worker payload). The handler pre-checks,
/// the service re-checks (Req 1.4) — the same guard rejects both.
pub const MAX_CONTENT_BYTES: usize = 1 << 20;

/// The content kind discriminator (Req 1.3: validated, not extended — v1
/// summarizes file reads only). `file_read` selects the file-read
/// evaluation order and the bulk-reader prompt reuse.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentKind {
    /// A historical file-read tool result.
    FileRead,
}

impl ContentKind {
    /// Wire name: the supported-kinds error list entry and the
    /// content-cache `kind` column value.
    pub fn as_str(&self) -> &'static str {
        match self {
            ContentKind::FileRead => "file_read",
        }
    }
}

/// One shunt_content call's input (design.md Service Interface; the tool
/// handler pre-checks size/kind and the service re-checks).
#[derive(Debug, Clone)]
pub struct ContentRequest<'a> {
    /// Project name.
    pub project: &'a str,
    /// Calling harness (the proxy sends `mvibe-proxy`).
    pub harness: &'a str,
    /// Workspace task ID, if known.
    pub task: Option<&'a str>,
    /// The caller-supplied tool-result string; never a path to open.
    pub content: &'a str,
    /// The kind discriminator (v1: file reads only).
    pub kind: ContentKind,
    /// Optional source hint: a label for the path-based security regex and
    /// audit lines; never opened or confined (Req 1.5).
    pub source_hint: Option<&'a str>,
    /// The caller's question (one fixed default when absent, Req 1.6).
    pub question: &'a str,
}

impl<'a> ContentRequest<'a> {
    /// The shared call context.
    pub fn ctx(&self) -> CallCtx<'a> {
        CallCtx {
            project: self.project,
            harness: self.harness,
            task: self.task,
        }
    }
}

/// Final shunt_content outcome (design.md Service Interface). The field
/// names ARE the consumer wire contract (Req 2.1; the mvibe
/// proxy-econ-client `ShuntResult` parser builds against them) — any
/// change requires the revalidation note in the spec's tasks.md header.
#[derive(Debug, Clone, PartialEq)]
pub enum ContentOutcome {
    /// Security-critical or below threshold; no worker call, no cache row,
    /// no savings statistics (Req 3.1-3.5).
    Bypassed {
        /// Why the content bypassed.
        reason: BypassReason,
        /// SHA-256 hex of the content (64 chars).
        content_sha256: String,
        /// Content line count.
        line_count: usize,
    },
    /// Served from the content cache; zero worker calls (Req 4.2).
    Cached {
        /// Byte-identical to the bytes stored at delegation time (Req 2.2).
        summary: String,
        /// SHA-256 hex of the content.
        content_sha256: String,
        /// Content line count.
        line_count: usize,
        /// Labelled estimate (never presented as a measurement).
        avoided_input_tokens_est: u64,
        /// Lookup latency.
        latency_ms: u64,
    },
    /// Worker-delegated structural summary (Req 5.1).
    Summarized {
        /// The worker's structural summary.
        summary: String,
        /// SHA-256 hex of the content.
        content_sha256: String,
        /// Content line count.
        line_count: usize,
        /// Labelled estimate (never presented as a measurement).
        avoided_input_tokens_est: u64,
        /// Worker call latency.
        latency_ms: u64,
        /// Worker token usage for this call (honest worker accounting).
        usage: WorkerUsage,
        /// The worker model that produced the summary.
        worker_model: String,
    },
}

impl ContentOutcome {
    /// Renders the wire result contract (Req 2.1) for the single text
    /// block: one flat JSON object — `outcome` always present; `summary`
    /// only for summarized/cached; `bypass_reason` only for bypassed;
    /// `worker_model` only when the worker was called; absent fields are
    /// omitted, never null.
    pub fn to_contract_json(&self) -> Value {
        match self {
            ContentOutcome::Bypassed {
                reason,
                content_sha256,
                line_count,
            } => json!({
                "outcome": "bypassed",
                "bypass_reason": wire_bypass_reason(*reason),
                "content_sha256": content_sha256,
                "line_count": line_count,
            }),
            ContentOutcome::Cached {
                summary,
                content_sha256,
                line_count,
                avoided_input_tokens_est,
                latency_ms,
            } => json!({
                "outcome": "cached",
                "summary": summary,
                "content_sha256": content_sha256,
                "line_count": line_count,
                "avoided_input_tokens_est": avoided_input_tokens_est,
                "latency_ms": latency_ms,
            }),
            ContentOutcome::Summarized {
                summary,
                content_sha256,
                line_count,
                avoided_input_tokens_est,
                latency_ms,
                worker_model,
                ..
            } => json!({
                "outcome": "summarized",
                "summary": summary,
                "content_sha256": content_sha256,
                "line_count": line_count,
                "avoided_input_tokens_est": avoided_input_tokens_est,
                "latency_ms": latency_ms,
                "worker_model": worker_model,
            }),
        }
    }
}

/// Wire bypass-reason vocabulary (consumer contract:
/// `security_critical` | `below_threshold`) — distinct from
/// [`BypassReason::as_str`], which carries the audit-line names
/// (`security_critical_bypass` / `below_line_threshold`).
fn wire_bypass_reason(reason: BypassReason) -> &'static str {
    match reason {
        BypassReason::SecurityCritical => "security_critical",
        BypassReason::BelowThreshold => "below_threshold",
    }
}

/// The documented secret pattern set for caller-supplied content (Req 3.2;
/// research.md "Content secret patterns live in mmhmem"): PEM private-key
/// headers (`-----BEGIN … PRIVATE KEY-----`), cloud access-key identifiers
/// (`AKIA` + 16 chars), bearer or API-key assignments
/// (`api_key = …8+ chars`, `Bearer …16+`), and environment assignments
/// whose key contains SECRET/TOKEN/PASSWORD/KEY (line-anchored via `(?m)`).
///
/// Any match bypasses with `security_critical` before the worker is ever
/// considered. False positives are acceptable (a bypass never leaks);
/// false negatives are not (research.md Risks).
static CONTENT_SECRET_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r"-----BEGIN [A-Z0-9 ]*PRIVATE KEY-----",
        r"|AKIA[0-9A-Z]{16}",
        r"|(?i:(?:api[_-]?key|secret|token|password|passwd|pwd)\s*[:=]\s*\S{8,})",
        r"|(?i:bearer\s+[a-z0-9._-]{16,})",
        r"|(?im:^[A-Z_]*(?:SECRET|TOKEN|PASSWORD|KEY)[A-Z_]*=)",
    ))
    .expect("valid content secret regex")
});

/// Whether `content` matches the documented secret pattern set (Req 3.2).
/// The environment-assignment branch is line-anchored; the rest match
/// anywhere in the content.
pub fn is_secret_content(content: &str) -> bool {
    CONTENT_SECRET_RE.is_match(content)
}

impl ShuntEngine {
    /// Content-based shunt entry point (design.md Service Interface).
    ///
    /// Evaluation order (Req 1.2, same ordering as a shunted read):
    /// size re-check → hint path regex (Req 3.1) → content secret patterns
    /// (Req 3.2) → line-count threshold vs `shunt_min_lines` (Req 3.3) →
    /// [task 2.2: content-cache lookup → worker delegation]. A bypass is
    /// audited with the reason and the content hash only (Req 3.4) and
    /// writes no cache row and no savings statistics (Req 3.5).
    pub async fn shunt_content(
        &self,
        req: &ContentRequest<'_>,
    ) -> Result<ContentOutcome, ShuntError> {
        // Service re-check of the handler's preconditions (design.md
        // "Error Handling": invalid args error out with no side effects).
        if req.content.is_empty() {
            return Err(ShuntError::Invalid("content is empty".to_string()));
        }
        if req.content.len() > MAX_CONTENT_BYTES {
            return Err(ShuntError::Invalid(format!(
                "content exceeds {MAX_CONTENT_BYTES} bytes"
            )));
        }
        let sha = sha256_hex(req.content.as_bytes());
        let line_count = req.content.lines().count();
        // The hint is a label only — never opened, never confined (Req 1.5);
        // the existing path rule applied to it decides (Req 3.1).
        let hint_critical = req
            .source_hint
            .is_some_and(|h| is_security_critical_path(Path::new(h)));
        if hint_critical || is_secret_content(req.content) {
            return self.content_bypass(req, BypassReason::SecurityCritical, &sha, line_count);
        }
        if (line_count as u32) < self.shunt_min_lines() {
            return self.content_bypass(req, BypassReason::BelowThreshold, &sha, line_count);
        }
        // Stages 3-4 (content-cache lookup and worker delegation) land with
        // task 2.2; until then threshold-passing content keeps the engine's
        // interim Content-arm guard.
        Err(ShuntError::Invalid(
            "content delegation requires the content cache".to_string(),
        ))
    }

    /// One bypass outcome: exactly one audit event carrying the reason and
    /// the content hash — never the content (Req 3.4) — and no cache row,
    /// no savings statistics (Req 3.5). `IO_DIRECT_READ` is reused
    /// deliberately: like a file bypass, the content is read directly (by
    /// the consumer), never summarized, and `details.source = "content"`
    /// plus `details.kind` distinguish the path.
    fn content_bypass(
        &self,
        req: &ContentRequest<'_>,
        reason: BypassReason,
        sha: &str,
        line_count: usize,
    ) -> Result<ContentOutcome, ShuntError> {
        let source = ShuntSource::content(req.source_hint);
        let mut details = source.audit_details();
        details["kind"] = Value::from(req.kind.as_str());
        details["sha256"] = Value::from(sha);
        details["total_lines"] = Value::from(line_count);
        details["bypass_reason"] = Value::from(reason.as_str());
        let _ = self.audit_writer().write_event(
            AuditEventType::IoDirectRead,
            req.harness,
            req.project,
            details,
        );
        Ok(ContentOutcome::Bypassed {
            reason,
            content_sha256: sha.to_string(),
            line_count,
        })
    }
}
