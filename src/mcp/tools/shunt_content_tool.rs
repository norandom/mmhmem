//! `shunt_content` tool handler (shunt-content-tool task 3.1; design.md
//! "mcp / shunt_content tool" API contract). Declared in its own module
//! file because `tools.rs` sits at the 500-line cap (tests/loc_caps.rs):
//! the `#[tool]` method and its generated `shunt_content_tool_attr`
//! metadata fn live here and are wired into the router from `tools.rs`.
//! Handler duties are exactly the sibling pattern (parse args, call the
//! service, render one text block, map errors, telemetry span) — all
//! policy stays in `services`.

use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock, ErrorData};
use rmcp::tool;

use crate::services::shunt_content::{ContentKind, ContentRequest, MAX_CONTENT_BYTES};

use super::super::params::ShuntContentParams;
use super::super::server::McpServer;
use super::super::tool_support::{ok_text, with_span};

impl McpServer {
    /// Content-based shunt for programmatic consumers (Req 1.1, 1.3, 1.4,
    /// 2.1, 2.3, 2.4). Kind and size are validated BEFORE the service is
    /// called; the success body is exactly one text block holding the wire
    /// JSON contract; every error surfaces as a tool error (`isError`)
    /// whose text is the documented actionable message — the wire form the
    /// mvibe consumer reads (its JSON-RPC error path sees only the code).
    #[tool(
        description = "Content-based shunt for programmatic consumers that hold a tool result string (e.g. an LLM-API proxy). Not for agents reading repository files: use smart_read. Returns a cached or worker-generated structural summary as one JSON object; summaries are not exact source evidence. Security-critical content and short content are bypassed, never summarized. Cache, audit and statistics are written; eviction may replace cached rows.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    pub(super) async fn shunt_content(
        &self,
        Parameters(p): Parameters<ShuntContentParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let state = self.state.clone();
        with_span(
            &state,
            "shunt_content",
            &p.agent_harness,
            &p.project,
            || async {
                // Handler preconditions (design.md "Preconditions"); the
                // service re-checks. Invalid args leave no rows (Req 1.3/1.4).
                if p.kind != ContentKind::FileRead.as_str() {
                    return Ok(tool_error(format!(
                        "unsupported kind '{}'; supported: {}",
                        p.kind,
                        ContentKind::FileRead.as_str()
                    )));
                }
                if p.content.is_empty() {
                    return Ok(tool_error("content is empty".to_string()));
                }
                if p.content.len() > MAX_CONTENT_BYTES {
                    return Ok(tool_error(format!(
                        "content exceeds {MAX_CONTENT_BYTES} bytes"
                    )));
                }
                let req = ContentRequest {
                    project: &p.project,
                    harness: &p.agent_harness,
                    task: None,
                    content: &p.content,
                    kind: ContentKind::FileRead,
                    source_hint: p.source_hint.as_deref(),
                    question: &p.question,
                };
                match state.services.shunt.shunt_content(&req).await {
                    Ok(outcome) => ok_text(outcome.to_contract_json().to_string()),
                    Err(e) => Ok(tool_error(e.to_string())),
                }
            },
        )
        .await
    }
}

/// One `isError` tool result whose single text block is the message
/// (design.md "Error Handling": `unsupported kind '<k>; …`, size guards
/// and the existing worker errors). Distinct from the sibling `err()`
/// JSON-RPC mapping on purpose: parameter-level failures must reach the
/// consumer as readable text, not a bare error code.
fn tool_error(text: String) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(text)])
}
