//! Stages 3-4 of `smart_read` (shunt-cache-efficiency design.md "Lookup
//! policy"; split from `shunt.rs` per the 500-LOC cap): the content-keyed
//! summary cache lookup, the worker delegation and the store with audited
//! eviction. File-read summaries live in `shunt_content_cache` with kind
//! `file_read`; the path is not part of the key (Req 1.1-1.3).
//!
//! Lookup policy (Req 2.1-2.3): the default (or no) question is served the
//! shared structural summary, produced once per content. A custom question
//! is served a cached answer to it; otherwise the structural summary with
//! the one-line [`QUESTION_NOT_APPLIED`] note — produced when nothing is
//! cached, never the custom answer — unless `answer_question` is set, which
//! asks the worker and caches the answer under the question's fingerprint.
//! The canonical row is never replaced by a custom answer.

use std::path::PathBuf;
use std::time::Instant;

use serde_json::Value;

use super::shunt::ShuntEngine;
use super::shunt_cache::{self as cache, SummaryRow};
use super::shunt_content::DEFAULT_QUESTION;
use super::shunt_types::{CallCtx, QUESTION_NOT_APPLIED, ReadOutcome, ShuntError, ShuntSource};
use super::stats;
use super::worker::WorkerUsage;
use crate::storage::audit::AuditEventType;

/// One stage 3-4 request (the `Evaluation::Shuntable` fields).
pub(crate) struct SummaryAsk {
    pub(crate) path: PathBuf,
    pub(crate) sha: String,
    pub(crate) line_count: usize,
    pub(crate) question: String,
    pub(crate) answer_question: bool,
}

/// Which question was asked and which row answers it (audit `question` /
/// `served`, and whether the response carries the note).
#[derive(Debug, Clone, Copy)]
struct Choice {
    custom_question: bool,
    served_custom: bool,
}

impl Choice {
    fn question_label(self) -> &'static str {
        if self.custom_question {
            "custom"
        } else {
            "default"
        }
    }

    fn served_label(self) -> &'static str {
        if self.served_custom {
            "custom"
        } else {
            "canonical"
        }
    }

    /// The one-line note when a custom question got the shared summary.
    fn note(self) -> Option<String> {
        (self.custom_question && !self.served_custom).then(|| QUESTION_NOT_APPLIED.to_string())
    }
}

/// Bundled audit metadata for a shunt outcome (keeps audit_shunt under the
/// clippy 7-arg limit, Req 13).
struct ShuntAuditMeta<'a> {
    source: &'a ShuntSource,
    sha: &'a str,
    line_count: usize,
    worker_info: Option<(&'a str, u64, WorkerUsage)>,
    avoided_est: u64,
    choice: Choice,
}

/// What a cache miss produces: the row key and the worker question.
struct Target<'a> {
    fingerprint: &'a str,
    worker_question: &'a str,
    choice: Choice,
}

impl ShuntEngine {
    /// Stage 3-4 lookup policy (module docs, design.md flow "smart_read
    /// lookup policy"): custom answer, else the canonical summary, else the
    /// worker.
    pub(crate) async fn shunt(
        &self,
        ctx: &CallCtx<'_>,
        ask: SummaryAsk,
    ) -> Result<ReadOutcome, ShuntError> {
        let raw_chars = cache::raw_chars(&ask.path).await?;
        let custom_question = !cache::is_canonical_question(&ask.question);
        if custom_question {
            let choice = Choice {
                custom_question,
                served_custom: true,
            };
            let fp = cache::query_fingerprint(&ask.question);
            if let Some(summary) = cache::summary_lookup(&self.db, &ask.sha, &fp, &ask.path).await?
            {
                return self
                    .serve_cached(ctx, &ask, raw_chars, summary, choice)
                    .await;
            }
            if ask.answer_question {
                let target = Target {
                    fingerprint: &fp,
                    worker_question: &ask.question,
                    choice,
                };
                return self.delegate(ctx, &ask, raw_chars, target).await;
            }
        }
        let choice = Choice {
            custom_question,
            served_custom: false,
        };
        let canonical = cache::canonical_fingerprint();
        if let Some(summary) =
            cache::summary_lookup(&self.db, &ask.sha, canonical, &ask.path).await?
        {
            return self
                .serve_cached(ctx, &ask, raw_chars, summary, choice)
                .await;
        }
        let target = Target {
            fingerprint: canonical,
            worker_question: DEFAULT_QUESTION,
            choice,
        };
        self.delegate(ctx, &ask, raw_chars, target).await
    }

    /// Cache-hit path (Req 3.4, 2.5): audited, counted as a cache hit
    /// whichever row served, breadcrumbed, no worker call.
    async fn serve_cached(
        &self,
        ctx: &CallCtx<'_>,
        ask: &SummaryAsk,
        raw_chars: usize,
        summary: String,
        choice: Choice,
    ) -> Result<ReadOutcome, ShuntError> {
        let est = stats::estimate_cache_hit(raw_chars);
        let source = ShuntSource::File(ask.path.clone());
        let meta = ShuntAuditMeta {
            source: &source,
            sha: &ask.sha,
            line_count: ask.line_count,
            worker_info: None,
            avoided_est: est,
            choice,
        };
        self.audit_shunt(ctx, AuditEventType::IoShuntCacheHit, &meta);
        self.record_stat(
            ctx,
            stats::kind::SHUNT_CACHE_HIT,
            est,
            &source,
            &ask.sha,
            None,
        )
        .await;
        let line_count = ask.line_count;
        let text = format!("cached summary for {line_count}-line file");
        self.crumb(ctx, "smart_read:cache_hit", &source, &text)
            .await;
        Ok(ReadOutcome::Cached {
            summary,
            avoided_input_tokens_est: est,
            note: choice.note(),
        })
    }

    /// Cache-miss path (Req 3.5): delegate to the worker, store with audited
    /// eviction, then audit, count and breadcrumb.
    async fn delegate(
        &self,
        ctx: &CallCtx<'_>,
        ask: &SummaryAsk,
        raw_chars: usize,
        target: Target<'_>,
    ) -> Result<ReadOutcome, ShuntError> {
        let worker = self
            .worker
            .as_ref()
            .ok_or(ShuntError::WorkerNotConfigured)?;
        let payload = cache::line_numbered_payload(&ask.path).await?;
        let t0 = Instant::now();
        let (summary, usage) = worker
            .summarize_file(&payload, target.worker_question)
            .await
            .map_err(|e| {
                let details = serde_json::json!({
                    "file": ask.path.display().to_string(), "error": e.to_string(),
                });
                self.audit_event(ctx, AuditEventType::WorkerError, details);
                ShuntError::Worker(e.to_string())
            })?;
        let latency_ms = t0.elapsed().as_millis() as u64;
        let est = stats::estimate_delegated_read(raw_chars, summary.len());
        let source = ShuntSource::File(ask.path.clone());
        self.store_summary(ctx, ask, &source, target.fingerprint, &summary)
            .await?;
        let meta = ShuntAuditMeta {
            source: &source,
            sha: &ask.sha,
            line_count: ask.line_count,
            worker_info: Some((worker.model(), latency_ms, usage)),
            avoided_est: est,
            choice: target.choice,
        };
        self.audit_shunt(ctx, AuditEventType::IoShuntDelegated, &meta);
        let kind = stats::kind::SHUNT_DELEGATED;
        self.record_stat(ctx, kind, est, &source, &ask.sha, Some(usage))
            .await;
        let line_count = ask.line_count;
        let text = format!("worker summary for {line_count}-line file ({latency_ms} ms)");
        self.crumb(ctx, "smart_read:delegated", &source, &text)
            .await;
        Ok(ReadOutcome::Shunted {
            summary,
            latency_ms,
            avoided_input_tokens_est: est,
            usage,
            note: target.choice.note(),
        })
    }

    /// Stores a summary under (sha, fingerprint, `file_read`) with the
    /// content cache's cap; an eviction is audited once with its count
    /// (`CONTENT_CACHE_EVICT`, Req 5.2).
    async fn store_summary(
        &self,
        ctx: &CallCtx<'_>,
        ask: &SummaryAsk,
        source: &ShuntSource,
        fingerprint: &str,
        summary: &str,
    ) -> Result<(), ShuntError> {
        let row = SummaryRow {
            sha: &ask.sha,
            fingerprint,
            summary,
            line_count: ask.line_count,
            path: &ask.path,
        };
        let cap = self.config.shunt_content_cache_max_rows;
        let evicted = cache::summary_store_and_evict(&self.db, row, cap).await?;
        if evicted > 0 {
            let mut details = source.audit_details();
            details["kind"] = Value::from(cache::FILE_READ_KIND);
            details["sha256"] = Value::from(ask.sha.as_str());
            details["evicted"] = Value::from(evicted);
            self.audit_event(ctx, AuditEventType::ContentCacheEvict, details);
        }
        Ok(())
    }

    /// One audit line for a shunt outcome (cache hit / delegated). Details
    /// carry the explicit `source` label plus the file path, the sha, the
    /// line count and the estimate (design.md ShuntSource seam).
    fn audit_shunt(&self, ctx: &CallCtx<'_>, event: AuditEventType, meta: &ShuntAuditMeta<'_>) {
        let mut details = meta.source.audit_details();
        details["sha256"] = serde_json::json!(meta.sha);
        details["total_lines"] = serde_json::json!(meta.line_count);
        details["avoided_input_tokens_est"] = serde_json::json!(meta.avoided_est);
        details["question"] = Value::from(meta.choice.question_label());
        details["served"] = Value::from(meta.choice.served_label());
        if let Some((model, latency_ms, usage)) = meta.worker_info {
            details["worker_model"] = Value::from(model);
            details["latency_ms"] = Value::from(latency_ms);
            details["worker_prompt_tokens"] = Value::from(usage.prompt_tokens);
            details["worker_completion_tokens"] = Value::from(usage.completion_tokens);
        }
        self.audit_event(ctx, event, details);
    }

    /// One best-effort audit event in the call's harness and project.
    fn audit_event(&self, ctx: &CallCtx<'_>, event: AuditEventType, details: Value) {
        let _ = self
            .audit_writer()
            .write_event(event, ctx.harness, ctx.project, details);
    }
}
