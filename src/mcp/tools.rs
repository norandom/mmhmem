//! Thin MCP tool handlers (tasks 10.2/10.3; design.md "mcp layer" /
//! "Components and Interfaces" ToolHandlers row). Handlers parse args, call
//! services, format markdown, map errors to actionable MCP tool errors, and
//! wrap each call in a telemetry span. All policy lives in services/storage.

use std::time::Instant;

use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock, ErrorData, InitializeResult};
use rmcp::{tool, tool_handler, tool_router, ServerHandler};

use super::params::{
    BoilerplateParams, MemGetParams, MemRecordParams, MemSearchParams, ResumeParams,
    SmartReadParams,
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
        info.instructions = Some(
            "mmhmem: cross-agent memory + I/O shunt server. Call session_resume_or_handoff \
             first; prefer smart_read over raw reads of large files (offset/limit for edits); \
             search the memory index before fetching details; checkpoint before switching \
             harnesses."
                .into(),
        );
        info
    }
}
use crate::services::boilerplate::{BoilerplateError, WriteRequest};
use crate::services::memory::{RecordRequest, SearchRequest};
use crate::services::session_bus::ClientUsage;
use crate::services::shunt::ReadOutcome;
use crate::services::shunt_types::ReadRequest as ShuntReadRequest;

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
        description = "Cross-agent session bus. Call WITHOUT updated_working_state_md at session start to inherit the live in-flight session from another harness (returns working state, last harness, objective, last 15 breadcrumbs). Call WITH it to checkpoint for cross-agent handoff. Optional client_usage reports harness frontier token totals for server-side stats."
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
        description = "Deterministic shunted read. offset+limit returns an exact L<n>: line slice (for edits). Files below the shunt threshold or on security-critical paths return full line-numbered content directly. Larger files return a cached or worker-generated structural summary (bullets with symbol + [Lstart-Lend]); repeats of an unchanged file are served from the SHA-256 cache with zero worker calls."
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
                Ok(ReadOutcome::Shunted { summary, latency_ms, avoided_input_tokens_est }) => ok_text(format!(
                    "### Shunted Summary (worker, {latency_ms} ms)\n{summary}\n\n*For edits, call smart_read with offset and limit.* (est. avoided frontier input tokens: {avoided_input_tokens_est})"
                )),
                Err(e) => Err(err(e)),
            }
        })
        .await
    }

    #[tool(
        description = "Generate boilerplate via the worker model and write it straight to target_path (both paths confined to AGENT_MEM_ALLOWED_ROOTS). Returns a one-line confirmation with sha256, line count, worker model, and latency — zero frontier input/output tokens for reference reading and generation."
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
                let Some(bp) = &state.services.boilerplate else {
                    return Err(err(BoilerplateError::WorkerNotConfigured));
                };
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
        description = "Search the project's long-term memory index (FTS5). Returns a compact table of matching observations (ID, type, harness, title, files) — never full narratives. Use obs_type to filter (decision, bugfix, feature, refactor, discovery, change)."
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
        description = "Fetch full narratives, facts, and files for validated observation IDs (project-scoped) from mem_search results."
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
        description = "Record a durable, immediately searchable observation in the project's long-term memory (type: decision, bugfix, feature, refactor, discovery, change). Redacted before persist; secrets and <private> spans are scrubbed."
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
            return ok_text(format!(
                "Checkpoint saved for project '{}' (task: {}). {} chars stored. Ready for cross-agent pickup.",
                p.project, ack.task_id, ack.stored_chars
            ));
        }
        if !view.found_prior_session {
            return ok_text(format!(
                "No active cross-agent session found for '{}'. Initialized new task ID: {}.",
                p.project, view.task_id
            ));
        }
        let mut md = format!(
            "### Active Cross-Agent Session (Task: {})\n**Last Active Agent:** {} | **Objective:** {}\n\n#### Live Working Context\n{}\n\n#### Recent Cross-Agent Tool Breadcrumbs (Newest First)\n",
            view.task_id, view.last_harness, view.objective, view.working_state_md
        );
        if view.breadcrumbs.is_empty() {
            md.push_str("(none)\n");
        } else {
            for b in &view.breadcrumbs {
                md.push_str(&format!(
                    "- [{}] {}({}): {}\n",
                    b.harness,
                    b.tool,
                    b.resource.clone().unwrap_or_default(),
                    b.summary
                ));
            }
        }
        ok_text(md)
    }
}
