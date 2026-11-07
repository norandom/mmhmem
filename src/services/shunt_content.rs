//! Content-based shunt types (shunt-content-tool design.md "services /
//! shunt_content"). Task 1.1 defines the request, kind and outcome types
//! plus the wire result contract renderer; the evaluation (secret
//! patterns, threshold, size guard) and the cache/delegation orchestration
//! land with tasks 2.1/2.2 in the `ShuntEngine::shunt_content` impl block.
//! Services may import `config` and `storage`, never `mcp` (steering
//! structure.md) — the MCP handler renders, this module decides.

use serde_json::{Value, json};

use super::shunt_types::{BypassReason, CallCtx};
use super::worker::WorkerUsage;

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
