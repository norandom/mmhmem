//! Thin MCP tool handlers (tasks 10.2/10.3; design.md "mcp layer" /
//! "Components and Interfaces" ToolHandlers row). Handlers parse args, call
//! services, format markdown, map errors to actionable MCP tool errors, and
//! wrap each call in a telemetry span. All policy lives in services/storage.

use std::time::Instant;

use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock, ErrorData, InitializeResult, ToolsCapability};
use rmcp::{ServerHandler, tool, tool_handler, tool_router};

use super::params::{
    BoilerplateParams, BreadcrumbSearchParams, MemGetParams, MemRecordParams, MemSearchParams,
    PruneParams, ResumeParams, SmartReadParams, WarmupParams,
};
use super::server::McpServer;

// The ServerHandler impl lives in the same module as the tool router (the
// generated `tool_router` fn is module-private in rmcp 3.x).
#[tool_handler]
impl ServerHandler for McpServer {
    fn get_info(&self) -> InitializeResult {
        let mut info = InitializeResult::default();
        info.server_info.name = "mmhmem".into();
        info.server_info.version = env!("CARGO_PKG_VERSION").into();
        // Req 1.6 (amendment 2026-11-02): advertise the tools capability.
        // rmcp's `#[tool_handler]` injects this only into its generated
        // `get_info`; this manual override supersedes it, so set it here.
        // Without it, capability-negotiating clients (OpenCode) enumerate
        // zero tools. No `listChanged` claim: the server never emits them.
        info.capabilities.tools = Some(ToolsCapability::default());
        info.instructions = Some(super::instructions::INSTRUCTIONS.into());
        info
    }
}
use crate::services::boilerplate::WriteRequest;
use crate::services::breadcrumbs::{BreadcrumbSearch, render_rows};
use crate::services::memory::{RecordRequest, SearchRequest};
use crate::services::prune::render_report as render_prune;
use crate::services::session_bus::ClientUsage;
use crate::services::shunt::ReadOutcome;
use crate::services::shunt_types::{CallCtx, ReadRequest as ShuntReadRequest};
use crate::services::warmup::{render_report as render_warmup, run_warmup};

/// Success result with one text content block.
fn ok_text(md: String) -> Result<CallToolResult, ErrorData> {
    Ok(CallToolResult::success(vec![ContentBlock::text(md)]))
}

/// Actionable MCP tool error from any service error.
fn err<E: std::fmt::Display>(e: E) -> ErrorData {
    ErrorData::internal_error(e.to_string(), None)
}

/// Runs `f` under a telemetry span named after the tool.
async fn with_span<F, Fut, T>(
    state: &super::server::AppState,
    tool: &str,
    harness: &str,
    project: &str,
    f: F,
) -> Result<T, ErrorData>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<T, ErrorData>>,
{
    let span = state.telemetry.start_tool_span(tool, harness, project);
    let t0 = Instant::now();
    let result = f().await;
    span.finish(
        if result.is_ok() { "ok" } else { "error" },
        t0.elapsed().as_millis() as u64,
    );
    result
}

#[tool_router]
impl McpServer {
    #[tool(
        description = "Cross-agent session bus: resume at session start or after clearing without updated_working_state_md; supply it to replace the saved checkpoint for handoff. Returns checkpoint state and recent breadcrumbs, not imported chat history. Use mem_search_breadcrumbs for older activity. Optional client_usage accepts only actual harness-reported frontier totals.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    async fn session_resume_or_handoff(
        &self,
        Parameters(p): Parameters<ResumeParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let state = self.state.clone();
        with_span(
            &state,
            "session_resume_or_handoff",
            &p.agent_harness,
            &p.project,
            || async { self.resume_or_handoff(&p).await },
        )
        .await
    }

    #[tool(
        description = "Use for ordinary repository inspection before a full native or shell read when available and authorized. Large files return cached or worker-generated structural summaries, not exact source evidence. Use offset+limit for exact line slices when editing or verifying; small/security-critical files return direct content. Unchanged content with the same cache identity (including question) hits cache without a worker call. Cache, audit and stats may be written. On worker failure use bounded authorized targeted/native fallback, not repeated retries or invented savings.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    async fn smart_read(
        &self,
        Parameters(p): Parameters<SmartReadParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let state = self.state.clone();
        with_span(&state, "smart_read", &p.agent_harness, &p.project, || async {
            let req = ShuntReadRequest {
                project: &p.project,
                harness: &p.agent_harness,
                task: None,
                path: &p.file_path,
                question: &p.question,
                offset: p.offset,
                limit: p.limit,
            };
            match state.services.shunt.smart_read(&req).await {
                Ok(ReadOutcome::Targeted { text, start, end, total_lines }) => ok_text(format!(
                    "### Targeted Read: {} (L{start}-L{end} of {total_lines})\n```\n{text}\n```",
                    p.file_path
                )),
                Ok(ReadOutcome::Direct { text, reason, total_lines }) => ok_text(format!(
                    "### Direct Read ({}): {} ({total_lines} lines)\n```\n{text}\n```",
                    reason.as_str(),
                    p.file_path
                )),
                Ok(ReadOutcome::Cached { summary, avoided_input_tokens_est }) => ok_text(format!(
                    "### Cached Shunt Summary\n{summary}\n\n*For edits, call smart_read with offset and limit.* (est. avoided frontier input tokens: {avoided_input_tokens_est})"
                )),
                Ok(ReadOutcome::Shunted { summary, latency_ms, avoided_input_tokens_est, .. }) => ok_text(format!(
                    "### Shunted Summary (worker, {latency_ms} ms)\n{summary}\n\n*For edits, call smart_read with offset and limit.* (est. avoided frontier input tokens: {avoided_input_tokens_est})"
                )),
                Err(e) => Err(err(e)),
            }
        })
        .await
    }

    #[tool(
        description = "Generate requested boilerplate with the worker and write target_path, potentially overwriting it; both paths stay within allowed roots. Use only for intended generation, not source inspection (use smart_read). Returns a compact write acknowledgement; worker access, cache/audit and token accounting have side effects. Does not guarantee measured frontier savings.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    async fn boilerplate_write(
        &self,
        Parameters(p): Parameters<BoilerplateParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let state = self.state.clone();
        with_span(
            &state,
            "boilerplate_write",
            &p.agent_harness,
            &p.project,
            || async {
                let bp = &state.services.boilerplate;
                let req = WriteRequest {
                    project: &p.project,
                    harness: &p.agent_harness,
                    task: None,
                    target: &p.target_path,
                    reference: &p.reference_path,
                    spec: &p.spec,
                };
                match bp.write(&req).await {
                    Ok(ack) => ok_text(format!(
                        "Wrote {} ({} lines, sha256 {}, worker {}, {} ms)",
                        p.target_path,
                        ack.lines,
                        &ack.sha256[..12],
                        ack.model,
                        ack.latency_ms
                    )),
                    Err(e) => Err(err(e)),
                }
            },
        )
        .await
    }

    #[tool(
        description = "Search the project long-term memory index before fetching observation details. Returns compact matching IDs, types, titles and files, not full narratives; use mem_get_observations for validated IDs. Optional obs_type filters results. Search appends audit/breadcrumb activity.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    async fn mem_search(
        &self,
        Parameters(p): Parameters<MemSearchParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let state = self.state.clone();
        with_span(
            &state,
            "mem_search",
            &p.agent_harness,
            &p.project,
            || async {
                let req = SearchRequest {
                    project: &p.project,
                    harness: &p.agent_harness,
                    task: None,
                    query: &p.query,
                    obs_type: p.obs_type.as_deref(),
                    limit: p.limit,
                };
                let rows = state.services.memory.search(&req).await.map_err(err)?;
                if rows.is_empty() {
                    return ok_text("No observations matched your query.".to_string());
                }
                let mut md =
                    "| ID | Type | Harness | Title | Files |\n|---|---|---|---|---|\n".to_string();
                for r in &rows {
                    md.push_str(&format!(
                        "| {} | {} | {} | {} | {} |\n",
                        r.id,
                        r.obs_type,
                        r.harness,
                        r.title,
                        r.files_touched.clone().unwrap_or_default()
                    ));
                }
                ok_text(format!(
                    "{md}\n*Fetch full narratives with mem_get_observations by ID.*"
                ))
            },
        )
        .await
    }

    #[tool(
        description = "Fetch full narratives, facts and files for project-scoped observation IDs validated through mem_search. Prefer the compact search first; this retrieves memory, not current exact source. Appends audit/breadcrumb activity.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    async fn mem_get_observations(
        &self,
        Parameters(p): Parameters<MemGetParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let state = self.state.clone();
        with_span(&state, "mem_get_observations", &p.agent_harness, &p.project, || async {
            let rows = state.services.memory.get(&p.project, &p.agent_harness, &p.ids).await.map_err(err)?;
            if rows.is_empty() {
                return ok_text("No observations found for those IDs in this project.".to_string());
            }
            let mut md = String::new();
            for o in &rows {
                md.push_str(&format!(
                    "### #{}: {} ({})\n**Harness:** {} | **Task:** {} | **At:** {}\n\n{}\n\n**Facts:** {}\n**Files:** {}\n\n---\n",
                    o.id, o.title, o.obs_type, o.harness, o.task_id, o.created_at_epoch, o.narrative,
                    o.facts.clone().unwrap_or_default(), o.files_touched.clone().unwrap_or_default()
                ));
            }
            ok_text(md)
        })
        .await
    }

    #[tool(
        description = "Record a durable searchable project observation for a meaningful decision, bugfix, feature, refactor, discovery or change. Prefer session_resume_or_handoff for in-flight checkpoints. Appends observation and audit/breadcrumb records; configured secret/private-span redaction is applied before persistence.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    async fn mem_record_observation(
        &self,
        Parameters(p): Parameters<MemRecordParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let state = self.state.clone();
        with_span(
            &state,
            "mem_record_observation",
            &p.agent_harness,
            &p.project,
            || async {
                let req = RecordRequest {
                    project: &p.project,
                    harness: &p.agent_harness,
                    task: p.task.as_deref().unwrap_or("default"),
                    obs_type: &p.obs_type,
                    title: &p.title,
                    narrative: &p.narrative,
                    facts: p.facts.as_deref().unwrap_or(""),
                    files_touched: p.files.as_deref().unwrap_or(""),
                };
                let id = state.services.memory.record(&req).await.map_err(err)?;
                ok_text(format!(
                    "Recorded observation #{id} ({}) in project '{}'.",
                    p.obs_type, p.project
                ))
            },
        )
        .await
    }

    #[tool(
        description = "Warm selected corpus files only when prebuilding summaries is intended; for ordinary inspection use smart_read. Processes at most 25 paths per batch through cached/worker summary reads, potentially updating cache and stats. Threshold/security files are skipped; missing, denied and failed paths are counted. Requires worker access for uncached summaries; does not guarantee measured frontier savings.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    async fn shunt_warmup(
        &self,
        Parameters(p): Parameters<WarmupParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let state = self.state.clone();
        with_span(
            &state,
            "shunt_warmup",
            &p.agent_harness,
            &p.project,
            || async {
                let ctx = CallCtx {
                    project: &p.project,
                    harness: &p.agent_harness,
                    task: None,
                };
                match run_warmup(
                    &state.services.shunt,
                    &state.audit,
                    &p.paths,
                    &p.question,
                    ctx,
                )
                .await
                {
                    Ok(report) => ok_text(render_warmup(&report)),
                    Err(e) => Err(err(e)),
                }
            },
        )
        .await
    }

    #[tool(
        description = "Remove stale cache rows whose files are missing or content hashes changed, only when cache cleanup is intended. Use dry_run to preview grouped counts; fresh rows remain. Deletes cache entries, not source files or durable observations; use smart_read for inspection.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    async fn shunt_prune(
        &self,
        Parameters(p): Parameters<PruneParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let state = self.state.clone();
        with_span(
            &state,
            "shunt_prune",
            &p.agent_harness,
            &p.project,
            || async {
                let ctx = CallCtx {
                    project: &p.project,
                    harness: &p.agent_harness,
                    task: None,
                };
                let report = if p.dry_run {
                    state.services.prune.scan().await
                } else {
                    state.services.prune.delete(ctx).await
                };
                match report {
                    Ok(report) => ok_text(render_prune(report, p.dry_run)),
                    Err(e) => Err(err(e)),
                }
            },
        )
        .await
    }

    #[tool(
        description = "Search older project breadcrumb activity beyond the recent resume view. Literal substring matches tool, resource and action summary, newest first; blank query lists activity and harness optionally filters. This is recorded activity, not a saved checkpoint or imported chat history; use session_resume_or_handoff for saved state. Appends audit/breadcrumb activity.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    async fn mem_search_breadcrumbs(
        &self,
        Parameters(p): Parameters<BreadcrumbSearchParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let state = self.state.clone();
        with_span(
            &state,
            "mem_search_breadcrumbs",
            &p.agent_harness,
            &p.project,
            || async {
                let req = BreadcrumbSearch {
                    project: &p.project,
                    harness: &p.agent_harness,
                    task: None,
                    query: &p.query,
                    filter_harness: p.harness.as_deref(),
                    limit: p.limit,
                };
                let rows = state.services.breadcrumbs.search(&req).await.map_err(err)?;
                ok_text(render_rows(&rows))
            },
        )
        .await
    }
}

impl McpServer {
    /// session_resume_or_handoff core: resume (and maybe checkpoint).
    async fn resume_or_handoff(&self, p: &ResumeParams) -> Result<CallToolResult, ErrorData> {
        let bus = &self.state.services.bus;
        let view = bus
            .resume(&p.project, &p.agent_harness, p.workspace_task_id.as_deref())
            .await
            .map_err(err)?;
        if let Some(state_md) = &p.updated_working_state_md {
            let objective = p.active_objective.as_deref().unwrap_or(&view.objective);
            let usage = p.client_usage.clone().map(ClientUsage::from);
            let ack = bus
                .checkpoint(
                    &p.project,
                    &p.agent_harness,
                    &view.task_id,
                    objective,
                    state_md,
                    usage.clone(),
                )
                .await
                .map_err(err)?;
            if let Some(cu) = usage {
                let ctx = crate::services::shunt_types::CallCtx {
                    project: &p.project,
                    harness: &p.agent_harness,
                    task: Some(&view.task_id),
                };
                if let Err(e) = self
                    .state
                    .services
                    .stats
                    .record_client_usage(&ctx, &cu)
                    .await
                {
                    tracing::warn!("client_usage stats failed: {e}");
                }
            }
            return ok_text(super::session_response::checkpoint(&p.project, &ack));
        }
        let mut md = super::session_response::resume(&p.project, &view);
        md.push_str(&self.worker_notice());
        ok_text(md)
    }

    /// One-line worker-unconfigured notice for session-start responses
    /// (Req 8.1); empty when a worker model is configured (Req 8.2).
    fn worker_notice(&self) -> String {
        if self.state.worker_configured {
            String::new()
        } else {
            "\n\nNo worker model configured — smart_read shunting and \
             shunt_warmup stay off. Set worker_model in .mmhmem.json \
             (or the WORKER_MODEL variable) to enable them."
                .to_string()
        }
    }
}
