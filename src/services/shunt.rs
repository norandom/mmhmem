//! The `ShuntEngine` component (tasks 5.1/5.2; design.md "services layer" /
//! "Components and Interfaces" ShuntEngine row; requirements.md Req 3.1-3.9,
//! 10.1). Types live in `shunt_types.rs`, cache/payload helpers in
//! `shunt_cache.rs`, the security path rule in `security_path.rs` (splits
//! per the Req 13.3 500-LOC cap).
//!
//! Deterministic four-stage smart_read evaluation (design.md "System Flows"):
//!   1. targeted slice (offset+limit) -> exact `L<n>:` lines, never delegated
//!   2. security-critical path OR below-threshold -> direct line-numbered
//!      read, bypass reason audited (Req 3.2, 3.3)
//!   3. content-addressed cache hit -> cached summary, zero worker calls (Req 3.4, 3.6)
//!   4. cache miss -> worker-model shunt, summary cached (Req 3.5-3.7)
//!
//! Path confinement (design.md "Security Considerations" + amended rule):
//! every agent-supplied path is canonicalized and must lie under one of
//! `AGENT_MEM_ALLOWED_ROOTS`; violations are rejected AND audited.

use std::path::PathBuf;
use std::time::Instant;

use serde_json::Value;

use crate::config::Config;
use crate::services::paths::{AllowedRoots, PathError};
pub use crate::services::security_path::{is_security_critical_in, is_security_critical_path};
use crate::services::session_bus::SessionBus;
use crate::services::shunt_cache as cache;
pub use crate::services::shunt_types::{BypassReason, ReadOutcome, ShuntError};
use crate::services::shunt_types::{CallCtx, Evaluation, FileView, ReadRequest};
use crate::services::stats;
use crate::services::worker::WorkerClient;
use crate::storage::audit::{AuditEventType, AuditWriter, sha256_hex};
use crate::storage::db::Db;

/// Bundled audit metadata for a shunt outcome (keeps audit_shunt under the
/// clippy 7-arg limit, Req 13).
struct ShuntAuditMeta<'a> {
    path: &'a std::path::Path,
    sha: &'a str,
    line_count: usize,
    worker_info: Option<(&'a str, u64, crate::services::worker::WorkerUsage)>,
    avoided_est: u64,
}

/// Cache-miss delegation inputs (bundled to stay under the clippy 7-arg limit).
struct Miss<'a> {
    path: PathBuf,
    sha: String,
    fp: String,
    line_count: usize,
    question: &'a str,
    raw_chars: usize,
}

/// The deterministic read-shunting engine (design.md ShuntEngine).
#[derive(Clone)]
pub struct ShuntEngine {
    db: Db,
    audit: AuditWriter,
    config: Config,
    bus: Option<SessionBus>,
    worker: Option<WorkerClient>,
    roots: AllowedRoots,
    /// Project folders for the security rule: the allowed roots and the
    /// working folder (a harness starts the stdio server in its project).
    projects: Vec<PathBuf>,
}

impl ShuntEngine {
    /// New engine; allowed roots are canonicalized once at construction.
    pub fn new(
        db: Db,
        audit: AuditWriter,
        config: Config,
        bus: Option<SessionBus>,
        worker: Option<WorkerClient>,
    ) -> Self {
        let roots = AllowedRoots::from_config(&config);
        let mut projects = roots.roots().to_vec();
        projects.extend(std::env::current_dir().and_then(std::fs::canonicalize));
        Self {
            db,
            audit,
            roots,
            projects,
            config,
            bus,
            worker,
        }
    }

    /// The configured shunt threshold in lines (Req 3.9, default 150).
    pub fn shunt_min_lines(&self) -> u32 {
        self.config.shunt_min_lines
    }

    /// Canonicalize `raw` and require it to lie under an allowed root;
    /// violations are audited (design.md Security Considerations).
    pub fn confine(&self, raw: &str, ctx: &CallCtx<'_>) -> Result<PathBuf, ShuntError> {
        match self.roots.confine(raw) {
            Ok(path) => Ok(path),
            Err(PathError::NotFound(p)) => Err(ShuntError::FileNotFound(p)),
            Err(PathError::OutsideRoots(path)) => {
                let _ = self.audit.write_event(
                    AuditEventType::Error,
                    ctx.harness,
                    ctx.project,
                    serde_json::json!({
                        "error": "path_outside_allowed_roots",
                        "path": path.display().to_string(),
                    }),
                );
                Err(ShuntError::OutsideAllowedRoots(path))
            }
        }
    }

    /// Full smart_read: evaluate, then stages 3-4 for shuntable files.
    pub async fn smart_read(&self, req: &ReadRequest<'_>) -> Result<ReadOutcome, ShuntError> {
        let ctx = req.ctx();
        match self.evaluate(req).await? {
            Evaluation::Targeted {
                text,
                start,
                end,
                total_lines,
            } => {
                self.crumb(
                    &ctx,
                    "smart_read:targeted",
                    req.path,
                    &format!("L{start}-L{end} of {total_lines}"),
                )
                .await;
                Ok(ReadOutcome::Targeted {
                    text,
                    start,
                    end,
                    total_lines,
                })
            }
            Evaluation::Direct {
                text,
                reason,
                total_lines,
            } => {
                self.crumb(
                    &ctx,
                    "smart_read:direct",
                    req.path,
                    &format!("direct read ({total_lines} lines, {})", reason.as_str()),
                )
                .await;
                Ok(ReadOutcome::Direct {
                    text,
                    reason,
                    total_lines,
                })
            }
            Evaluation::Shuntable {
                path,
                content_sha256,
                line_count,
                question,
            } => {
                self.shunt(&ctx, path, content_sha256, line_count, question)
                    .await
            }
        }
    }

    /// The deterministic evaluation (stages 1-2; 3-4 decision as Shuntable).
    async fn evaluate(&self, req: &ReadRequest<'_>) -> Result<Evaluation, ShuntError> {
        let ctx = req.ctx();
        let path = self.confine(req.path, &ctx)?;
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
        if let (Some(off), Some(lim)) = (req.offset, req.limit) {
            return Ok(self.targeted(&view, off, lim, &ctx));
        }
        if self.is_security_critical(view.path) {
            return self.direct(&view, BypassReason::SecurityCritical, &ctx);
        }
        if (view.total as u32) < self.shunt_min_lines() {
            return self.direct(&view, BypassReason::BelowThreshold, &ctx);
        }
        Ok(Evaluation::Shuntable {
            path: path.clone(),
            content_sha256: view.sha.to_string(),
            line_count: view.total,
            question: req.question.to_string(),
        })
    }

    /// Stage 3-4: cache lookup, then worker delegation + cache store.
    async fn shunt(
        &self,
        ctx: &CallCtx<'_>,
        path: PathBuf,
        sha: String,
        line_count: usize,
        question: String,
    ) -> Result<ReadOutcome, ShuntError> {
        let fp = cache::query_fingerprint(&question);
        let raw_chars = cache::raw_chars(&path).await?;
        if let Some(summary) = cache::cache_lookup(&self.db, &path, &sha, &fp).await? {
            let est = stats::estimate_cache_hit(raw_chars);
            return self
                .serve_cached(ctx, &path, &sha, line_count, est, summary)
                .await;
        }
        self.delegate(
            ctx,
            Miss {
                path,
                sha,
                fp,
                line_count,
                question: &question,
                raw_chars,
            },
        )
        .await
    }

    /// Cache-hit path (Req 3.4): audited, counted, breadcrumbed, < 50 ms,
    /// zero worker calls.
    async fn serve_cached(
        &self,
        ctx: &CallCtx<'_>,
        path: &std::path::Path,
        sha: &str,
        line_count: usize,
        est: u64,
        summary: String,
    ) -> Result<ReadOutcome, ShuntError> {
        let meta = ShuntAuditMeta {
            path,
            sha,
            line_count,
            worker_info: None,
            avoided_est: est,
        };
        self.audit_shunt(ctx, AuditEventType::IoShuntCacheHit, &meta);
        self.record_stat(ctx, stats::kind::SHUNT_CACHE_HIT, est, path, sha, None)
            .await;
        self.crumb(
            ctx,
            "smart_read:cache_hit",
            &path.display().to_string(),
            &format!("cached summary for {line_count}-line file"),
        )
        .await;
        Ok(ReadOutcome::Cached {
            summary,
            avoided_input_tokens_est: est,
        })
    }

    /// Cache-miss path (Req 3.5): delegate to the worker, store in the
    /// content-addressed cache, audit + count + breadcrumb.
    async fn delegate(&self, ctx: &CallCtx<'_>, miss: Miss<'_>) -> Result<ReadOutcome, ShuntError> {
        let Miss {
            path,
            sha,
            fp,
            line_count,
            question,
            raw_chars,
        } = miss;
        let worker = self
            .worker
            .as_ref()
            .ok_or(ShuntError::WorkerNotConfigured)?;
        let payload = cache::line_numbered_payload(&path).await?;
        let t0 = Instant::now();
        let (summary, usage) = worker
            .summarize_file(&payload, question)
            .await
            .map_err(|e| {
                let _ = self.audit.write_event(
                    AuditEventType::WorkerError,
                    ctx.harness,
                    ctx.project,
                    serde_json::json!({"file": path.display().to_string(), "error": e.to_string()}),
                );
                ShuntError::Worker(e.to_string())
            })?;
        let latency_ms = t0.elapsed().as_millis() as u64;
        let est = stats::estimate_delegated_read(raw_chars, summary.len());
        cache::cache_store(&self.db, &path, &sha, &fp, &summary, line_count).await?;
        let meta = ShuntAuditMeta {
            path: &path,
            sha: &sha,
            line_count,
            worker_info: Some((worker.model(), latency_ms, usage)),
            avoided_est: est,
        };
        self.audit_shunt(ctx, AuditEventType::IoShuntDelegated, &meta);
        self.record_stat(
            ctx,
            stats::kind::SHUNT_DELEGATED,
            est,
            &path,
            &sha,
            Some(usage),
        )
        .await;
        self.crumb(
            ctx,
            "smart_read:delegated",
            &path.display().to_string(),
            &format!("worker summary for {line_count}-line file ({latency_ms} ms)"),
        )
        .await;
        Ok(ReadOutcome::Shunted {
            summary,
            latency_ms,
            avoided_input_tokens_est: est,
            usage,
        })
    }

    /// Stage 1: exact line-numbered slice (Req 3.1), audited, never delegated.
    fn targeted(
        &self,
        view: &FileView<'_>,
        offset: u32,
        limit: u32,
        ctx: &CallCtx<'_>,
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
            ctx.harness,
            ctx.project,
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

    /// Stage 2: full line-numbered direct read with audited bypass reason.
    fn direct(
        &self,
        view: &FileView<'_>,
        reason: BypassReason,
        ctx: &CallCtx<'_>,
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
            ctx.harness,
            ctx.project,
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

    /// Security-critical path check (Req 3.3, case-insensitive) of the part
    /// of a canonical `path` inside its project.
    pub fn is_security_critical(&self, path: &std::path::Path) -> bool {
        is_security_critical_in(path, &self.projects)
    }

    /// One audit line for a shunt outcome (cache hit / delegated).
    fn audit_shunt(&self, ctx: &CallCtx<'_>, event: AuditEventType, meta: &ShuntAuditMeta<'_>) {
        let mut details = serde_json::json!({
            "file": meta.path.display().to_string(), "sha256": meta.sha,
            "total_lines": meta.line_count, "avoided_input_tokens_est": meta.avoided_est,
        });
        if let Some((model, latency_ms, usage)) = meta.worker_info {
            details["worker_model"] = Value::from(model);
            details["latency_ms"] = Value::from(latency_ms);
            details["worker_prompt_tokens"] = Value::from(usage.prompt_tokens);
            details["worker_completion_tokens"] = Value::from(usage.completion_tokens);
        }
        let _ = self
            .audit
            .write_event(event, ctx.harness, ctx.project, details);
    }

    /// One token_stats row for a shunt outcome (Req 10.1, 10.3).
    async fn record_stat(
        &self,
        ctx: &CallCtx<'_>,
        kind: &'static str,
        avoided_in: u64,
        path: &std::path::Path,
        sha: &str,
        usage: Option<crate::services::worker::WorkerUsage>,
    ) {
        let row = stats::TokenStatRow {
            project: ctx.project.to_string(),
            task_id: ctx.task.map(str::to_string),
            harness: ctx.harness.to_string(),
            kind,
            avoided_input_tokens_est: avoided_in,
            avoided_output_tokens_est: 0,
            worker_prompt_tokens: usage.map(|u| u.prompt_tokens),
            worker_completion_tokens: usage.map(|u| u.completion_tokens),
            details: serde_json::json!({"file": path.display().to_string(), "sha256": sha}),
        };
        // Req 10.1 (amendment 2026-11-02): commit the row BEFORE the tool
        // response returns. The previous fire-and-forget spawn raced
        // cross-connection readers (deterministic macOS CI failure) and
        // could lose the row on quick shutdown.
        if let Err(e) = stats::record(&self.db, row).await {
            tracing::warn!("token_stats insert failed: {e}");
        }
    }

    /// Automatic breadcrumb when a session bus is attached (Req 2.4).
    async fn crumb(&self, ctx: &CallCtx<'_>, tool: &str, resource: &str, summary: &str) {
        if let Some(bus) = &self.bus {
            let task = ctx.task.unwrap_or(ctx.project);
            if let Err(e) = bus
                .breadcrumb(
                    ctx.project,
                    task,
                    ctx.harness,
                    tool,
                    Some(resource),
                    summary,
                )
                .await
            {
                tracing::warn!("breadcrumb failed: {e}");
            }
        }
    }
}
