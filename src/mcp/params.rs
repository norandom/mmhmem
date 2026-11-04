//! Tool parameter structs (tasks 10.2/10.3). rmcp tool args are
//! `Parameters<T>` with `T: Deserialize + JsonSchema` (schemars re-exported by
//! rmcp — no extra dependency).

use rmcp::schemars::{self, JsonSchema};
use serde::Deserialize;

use crate::services::session_bus::ClientUsage;

impl From<ClientUsageParam> for ClientUsage {
    fn from(p: ClientUsageParam) -> Self {
        ClientUsage {
            prompt_tokens: p.prompt_tokens,
            completion_tokens: p.completion_tokens,
            source: p.source,
        }
    }
}

/// Optional harness-reported frontier token totals, cumulative for the
/// workspace task so far (Req 10.4; client-usage-stats Req 3.1). Stats count
/// only the latest report per task and source.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct ClientUsageParam {
    /// Cumulative frontier prompt tokens for the task, reported by the harness.
    pub prompt_tokens: u64,
    /// Cumulative frontier completion tokens for the task, reported by the harness.
    pub completion_tokens: u64,
    /// Source label, e.g. "opencode /status".
    pub source: String,
}

/// `session_resume_or_handoff` arguments.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct ResumeParams {
    /// Project name (session key).
    pub project: String,
    /// Calling harness, e.g. "opencode", "claude-code".
    pub agent_harness: String,
    /// New working state to checkpoint (absent = resume).
    #[serde(default)]
    pub updated_working_state_md: Option<String>,
    /// Active objective (checkpoint).
    #[serde(default)]
    pub active_objective: Option<String>,
    /// Workspace task ID (optional; inherited or newly initialized).
    #[serde(default)]
    pub workspace_task_id: Option<String>,
    /// Optional harness-reported frontier usage, cumulative for this
    /// workspace task, for stats (Req 10.4).
    #[serde(default)]
    pub client_usage: Option<ClientUsageParam>,
}

/// `smart_read` arguments.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct SmartReadParams {
    /// Project name.
    pub project: String,
    /// Calling harness.
    pub agent_harness: String,
    /// File path (confined to AGENT_MEM_ALLOWED_ROOTS).
    pub file_path: String,
    /// Question for the worker on shunted reads.
    #[serde(default = "default_question")]
    pub question: String,
    /// Targeted-read offset (1-based); with `limit` returns an exact slice.
    #[serde(default)]
    pub offset: Option<u32>,
    /// Targeted-read limit.
    #[serde(default)]
    pub limit: Option<u32>,
}

fn default_question() -> String {
    "Summarize exported symbols, signatures, and control flow with exact line ranges.".to_string()
}

/// `boilerplate_write` arguments.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct BoilerplateParams {
    /// Project name.
    pub project: String,
    /// Calling harness.
    pub agent_harness: String,
    /// Target path to write (confined).
    pub target_path: String,
    /// Reference file path (confined; sent to the worker).
    pub reference_path: String,
    /// Specification prompt for the worker.
    pub spec: String,
}

/// `mem_search` arguments.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct MemSearchParams {
    /// Project name.
    pub project: String,
    /// Calling harness.
    pub agent_harness: String,
    /// Free-text query (FTS5 phrase-escaped server-side).
    pub query: String,
    /// Optional type filter: decision, bugfix, feature, refactor, discovery, change.
    #[serde(default)]
    pub obs_type: Option<String>,
    /// Max results (default 20, capped at 100).
    #[serde(default = "default_limit")]
    pub limit: u16,
}

fn default_limit() -> u16 {
    20
}

/// `mem_get_observations` arguments.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct MemGetParams {
    /// Project name.
    pub project: String,
    /// Calling harness.
    pub agent_harness: String,
    /// Observation IDs (validated project-scoped, Req 5.4).
    pub ids: Vec<i64>,
}

/// `mem_record_observation` arguments.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct MemRecordParams {
    /// Project name.
    pub project: String,
    /// Calling harness.
    pub agent_harness: String,
    /// Workspace task ID.
    #[serde(default)]
    pub task: Option<String>,
    /// Type: decision, bugfix, feature, refactor, discovery, change.
    pub obs_type: String,
    /// Short title.
    pub title: String,
    /// Full narrative.
    pub narrative: String,
    /// Facts (optional).
    #[serde(default)]
    pub facts: Option<String>,
    /// Files touched, comma-separated (optional).
    #[serde(default)]
    pub files: Option<String>,
}

/// `shunt_warmup` arguments (corpus-warmup-prune Req 1.1).
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct WarmupParams {
    /// Project name.
    pub project: String,
    /// Calling harness.
    pub agent_harness: String,
    /// Key files to summarize (max 25 per batch, Req 1.6).
    pub paths: Vec<String>,
    /// The question summaries answer; defaults to the canonical
    /// summarization question so warmup rows align with later reads.
    #[serde(default = "default_question")]
    pub question: String,
}

/// `shunt_prune` arguments (corpus-warmup-prune Req 4.1).
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct PruneParams {
    /// Project name.
    pub project: String,
    /// Calling harness.
    pub agent_harness: String,
    /// Preview only — nothing is deleted (Req 4.2).
    #[serde(default)]
    pub dry_run: bool,
}

/// `mem_search_breadcrumbs` arguments (corpus-warmup-prune Req 10.1).
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct BreadcrumbSearchParams {
    /// Project name.
    pub project: String,
    /// Calling harness.
    pub agent_harness: String,
    /// Literal substring across tool name, target resource, and action
    /// summary; blank lists the project's crumbs newest first.
    pub query: String,
    /// Restrict matches to one calling harness (Req 10.3).
    #[serde(default)]
    pub harness: Option<String>,
    /// Max results (default 20, capped at 100, Req 10.2).
    #[serde(default = "default_limit")]
    pub limit: u16,
}

/// `status` arguments (reliable-installers Req 6.1).
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct StatusParams {
    /// Project name.
    pub project: String,
    /// Calling harness.
    pub agent_harness: String,
}
