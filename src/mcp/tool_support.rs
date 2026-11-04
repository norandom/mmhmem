//! Shared helpers for the MCP tool handlers in `tools.rs`: result and error
//! mapping, and the per-call telemetry span.

use std::time::Instant;

use rmcp::model::{CallToolResult, ContentBlock, ErrorData};

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
