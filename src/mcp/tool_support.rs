//! Shared helpers for the MCP tool handlers in `tools.rs`: result and error
//! mapping, the `smart_read` outcome text, and the per-call telemetry span.

use std::time::Instant;

use rmcp::model::{CallToolResult, ContentBlock, ErrorData};

use crate::services::shunt::ReadOutcome;

/// Markdown for one `smart_read` outcome. A summary carries the one-line
/// "question not applied" note when the shared summary answered a custom
/// question (shunt-cache-efficiency Req 2.2).
pub(super) fn render_read(file_path: &str, outcome: ReadOutcome) -> String {
    const EDIT_HINT: &str = "*For edits, call smart_read with offset and limit.*";
    let with_note = |note: Option<String>| note.map(|n| format!("\n\n{n}")).unwrap_or_default();
    match outcome {
        ReadOutcome::Targeted {
            text,
            start,
            end,
            total_lines,
        } => format!(
            "### Targeted Read: {file_path} (L{start}-L{end} of {total_lines})\n```\n{text}\n```"
        ),
        ReadOutcome::Direct {
            text,
            reason,
            total_lines,
        } => format!(
            "### Direct Read ({}): {file_path} ({total_lines} lines)\n```\n{text}\n```",
            reason.as_str()
        ),
        ReadOutcome::Cached {
            summary,
            avoided_input_tokens_est,
            note,
        } => format!(
            "### Cached Shunt Summary\n{summary}{}\n\n{EDIT_HINT} (est. avoided frontier input tokens: {avoided_input_tokens_est})",
            with_note(note)
        ),
        ReadOutcome::Shunted {
            summary,
            latency_ms,
            avoided_input_tokens_est,
            note,
            ..
        } => format!(
            "### Shunted Summary (worker, {latency_ms} ms)\n{summary}{}\n\n{EDIT_HINT} (est. avoided frontier input tokens: {avoided_input_tokens_est})",
            with_note(note)
        ),
    }
}

/// Success result with one text content block.
pub(super) fn ok_text(md: String) -> Result<CallToolResult, ErrorData> {
    Ok(CallToolResult::success(vec![ContentBlock::text(md)]))
}

/// Actionable MCP tool error from any service error.
pub(super) fn err<E: std::fmt::Display>(e: E) -> ErrorData {
    ErrorData::internal_error(e.to_string(), None)
}

/// Runs `f` under a telemetry span named after the tool.
pub(super) async fn with_span<F, Fut, T>(
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
