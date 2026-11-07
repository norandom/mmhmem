//! Content-based shunt service (shunt-content-tool design.md "services /
//! shunt_content"). Task 1.1 defined the request/kind/outcome types and the
//! wire contract renderer; task 2.1 added the evaluation (size re-check,
//! `CONTENT_SECRET_RE`, hint-path rule, threshold) with hash-only bypass
//! audits; task 2.2 adds the remaining stages: the content-cache hit path
//! (zero worker calls, Req 4.2) and delegation through the EXISTING
//! summarize operation with an `L<n>:`-numbered payload, store-and-evict
//! beyond the configured cap (Req 4.3), stats rows committed before return
//! (Req 5.1/5.2), hash-only audits (5.3), breadcrumbs (5.4) and the
//! documented worker-failure errors with no cache row (5.5).
//! Services may import `config` and `storage`, never `mcp` (steering
//! structure.md) — the MCP handler renders, this module decides.

use std::path::Path;
use std::sync::LazyLock;
use std::time::Instant;

use regex::Regex;
use serde_json::{Value, json};

use super::security_path::is_security_critical_path;
use super::shunt::ShuntEngine;
use super::shunt_cache as cache;
use super::shunt_types::{BypassReason, CallCtx, ShuntError, ShuntSource};
use super::stats;
use super::worker::WorkerUsage;
use crate::storage::audit::{AuditEventType, sha256_hex};

/// Maximum content size `shunt_content` accepts (design.md Service
/// Interface; 1 MiB bounds the worker payload). The handler pre-checks,
/// the service re-checks (Req 1.4) — the same guard rejects both.
pub const MAX_CONTENT_BYTES: usize = 1 << 20;

/// The fixed default question (Req 1.6): same string as the `smart_read`
/// default, so identical content from different callers shares one cache
/// identity. The MCP handler applies it when the caller sends no question.
pub const DEFAULT_QUESTION: &str =
    "Summarize exported symbols, signatures, and control flow with exact line ranges.";

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

/// Base audit details for every content shunt event (Req 5.3): the
/// explicit `source` label (+ trimmed hint when supplied), the kind and
/// the content hash — never the content itself.
fn content_audit_details(source: &ShuntSource, kind: &str, sha: &str) -> Value {
    let mut details = source.audit_details();
    details["kind"] = Value::from(kind);
    details["sha256"] = Value::from(sha);
    details
}

impl ShuntEngine {
    /// Content-based shunt entry point (design.md Service Interface).
    ///
    /// Evaluation order (Req 1.2, same ordering as a shunted read):
    /// size re-check → hint path regex (Req 3.1) → content secret patterns
    /// (Req 3.2) → line-count threshold vs `shunt_min_lines` (Req 3.3) →
    /// content-cache lookup (Req 4.2) → worker delegation + store/evict
    /// (Req 4.3, 5.1). A bypass audits the reason and content hash only
    /// (Req 3.4) and writes no cache row and no statistics (Req 3.5).
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
            return self
                .content_bypass(req, BypassReason::SecurityCritical, &sha, line_count)
                .await;
        }
        if (line_count as u32) < self.shunt_min_lines() {
            return self
                .content_bypass(req, BypassReason::BelowThreshold, &sha, line_count)
                .await;
        }
        // Stage 3: content-cache lookup keyed by (sha, question
        // fingerprint, kind) — a hit serves the stored bytes with ZERO
        // worker calls (Req 4.2).
        let source = ShuntSource::content(req.source_hint);
        let fp = cache::query_fingerprint(req.question);
        let raw_chars = req.content.len();
        let t0 = Instant::now();
        if let Some(summary) =
            cache::content_cache_lookup(&self.db, &sha, &fp, req.kind.as_str()).await?
        {
            let latency_ms = t0.elapsed().as_millis() as u64;
            let est = stats::estimate_cache_hit(raw_chars);
            return self
                .content_serve_cached(req, &sha, line_count, est, latency_ms, summary)
                .await;
        }
        // Stage 4: worker delegation (Req 5.1).
        self.content_delegate(req, &source, &sha, &fp, line_count, raw_chars)
            .await
    }

    /// Cache-hit path (Req 4.2, 5.2, 5.3, 5.4): one audit event carrying
    /// the kind, hash, line count and latency, one cache-hit statistics
    /// row (`record_stat` commits it before the response returns), one
    /// breadcrumb naming the tool, the outcome and the hash prefix.
    async fn content_serve_cached(
        &self,
        req: &ContentRequest<'_>,
        sha: &str,
        line_count: usize,
        est: u64,
        latency_ms: u64,
        summary: String,
    ) -> Result<ContentOutcome, ShuntError> {
        let source = ShuntSource::content(req.source_hint);
        let mut details = content_audit_details(&source, req.kind.as_str(), sha);
        details["total_lines"] = Value::from(line_count);
        details["avoided_input_tokens_est"] = Value::from(est);
        details["latency_ms"] = Value::from(latency_ms);
        self.audit_content(req, AuditEventType::IoShuntCacheHit, details);
        self.record_stat(
            &req.ctx(),
            stats::kind::SHUNT_CACHE_HIT,
            est,
            &source,
            sha,
            None,
        )
        .await;
        let text = format!(
            "cached summary for {line_count}-line content sha {}",
            &sha[..12]
        );
        self.crumb(&req.ctx(), "shunt_content:cache_hit", &source, &text)
            .await;
        Ok(ContentOutcome::Cached {
            summary,
            content_sha256: sha.to_string(),
            line_count,
            avoided_input_tokens_est: est,
            latency_ms,
        })
    }

    /// Delegation path (Req 5.1): the worker call runs in
    /// [`Self::content_call_worker`]; on success this records
    /// store-and-evict beyond the configured cap (Req 4.3, one count
    /// audit event), the delegated audit, stats row and breadcrumb.
    async fn content_delegate(
        &self,
        req: &ContentRequest<'_>,
        source: &ShuntSource,
        sha: &str,
        fp: &str,
        line_count: usize,
        raw_chars: usize,
    ) -> Result<ContentOutcome, ShuntError> {
        let (summary, usage, latency_ms, model) = self.content_call_worker(req, sha).await?;
        let est = stats::estimate_delegated_read(raw_chars, summary.len());
        let row = cache::ContentCacheRow {
            content_sha256: sha,
            query_fingerprint: fp,
            kind: req.kind.as_str(),
            structural_summary: &summary,
            line_count,
        };
        let evicted = cache::content_cache_store_and_evict(
            &self.db,
            row,
            self.config.shunt_content_cache_max_rows,
        )
        .await?;
        if evicted > 0 {
            let mut details = content_audit_details(source, req.kind.as_str(), sha);
            details["evicted"] = Value::from(evicted);
            self.audit_content(req, AuditEventType::ContentCacheEvict, details);
        }
        let mut details = content_audit_details(source, req.kind.as_str(), sha);
        details["total_lines"] = Value::from(line_count);
        details["avoided_input_tokens_est"] = Value::from(est);
        details["latency_ms"] = Value::from(latency_ms);
        details["worker_model"] = Value::from(model.as_str());
        details["worker_prompt_tokens"] = Value::from(usage.prompt_tokens);
        details["worker_completion_tokens"] = Value::from(usage.completion_tokens);
        self.audit_content(req, AuditEventType::IoShuntDelegated, details);
        self.record_stat(
            &req.ctx(),
            stats::kind::SHUNT_DELEGATED,
            est,
            source,
            sha,
            Some(usage),
        )
        .await;
        self.crumb(
            &req.ctx(),
            "shunt_content:delegated",
            source,
            &format!(
                "worker summary for {line_count}-line content sha {} ({latency_ms} ms)",
                &sha[..12]
            ),
        )
        .await;
        Ok(ContentOutcome::Summarized {
            summary,
            content_sha256: sha.to_string(),
            line_count,
            avoided_input_tokens_est: est,
            latency_ms,
            usage,
            worker_model: model,
        })
    }

    /// The worker call of a delegation (Req 5.1, 5.5): the EXISTING
    /// `summarize_file` over the `L<n>:`-numbered payload; failures map
    /// to the existing actionable errors with a hash-only WORKER_ERROR
    /// audit. Returns (summary, usage, latency_ms, worker_model).
    async fn content_call_worker(
        &self,
        req: &ContentRequest<'_>,
        sha: &str,
    ) -> Result<(String, WorkerUsage, u64, String), ShuntError> {
        let worker = match self.worker.as_ref() {
            Some(w) => w,
            None => {
                self.content_worker_error(
                    req,
                    sha,
                    &super::worker::WorkerError::NotConfigured.to_string(),
                );
                return Err(ShuntError::WorkerNotConfigured);
            }
        };
        let payload = cache::payload_from_content(req.content);
        let t0 = Instant::now();
        let (summary, usage) = match worker.summarize_file(&payload, req.question).await {
            Ok(ok) => ok,
            Err(e) => {
                let text = e.to_string();
                self.content_worker_error(req, sha, &text);
                return Err(ShuntError::Worker(text));
            }
        };
        let model = worker.model().to_string();
        Ok((summary, usage, t0.elapsed().as_millis() as u64, model))
    }

    /// One hash-only WORKER_ERROR audit (Req 5.5) — the content hash, kind
    /// and error text, never the content or summary.
    fn content_worker_error(&self, req: &ContentRequest<'_>, sha: &str, error: &str) {
        let mut details = content_audit_details(
            &ShuntSource::content(req.source_hint),
            req.kind.as_str(),
            sha,
        );
        details["error"] = Value::from(error);
        self.audit_content(req, AuditEventType::WorkerError, details);
    }

    /// One audit event for a content outcome; `details` are hash-only.
    fn audit_content(&self, req: &ContentRequest<'_>, event: AuditEventType, details: Value) {
        let _ = self
            .audit_writer()
            .write_event(event, req.harness, req.project, details);
    }

    /// One bypass outcome: exactly one audit event with the reason and the
    /// content hash — never the content (Req 3.4) — no cache row and no
    /// savings statistics (Req 3.5), plus one breadcrumb naming the tool,
    /// the outcome and the hash prefix (Req 5.4). `IO_DIRECT_READ` +
    /// `details.source`/`kind` distinguish it from a file bypass.
    async fn content_bypass(
        &self,
        req: &ContentRequest<'_>,
        reason: BypassReason,
        sha: &str,
        line_count: usize,
    ) -> Result<ContentOutcome, ShuntError> {
        let source = ShuntSource::content(req.source_hint);
        let mut details = content_audit_details(&source, req.kind.as_str(), sha);
        details["total_lines"] = Value::from(line_count);
        details["bypass_reason"] = Value::from(reason.as_str());
        self.audit_content(req, AuditEventType::IoDirectRead, details);
        let text = format!(
            "{} for {line_count}-line content sha {}",
            reason.as_str(),
            &sha[..12]
        );
        self.crumb(&req.ctx(), "shunt_content:bypassed", &source, &text)
            .await;
        Ok(ContentOutcome::Bypassed {
            reason,
            content_sha256: sha.to_string(),
            line_count,
        })
    }
}
