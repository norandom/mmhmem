//! Antigravity (reliable-installers task 3.5; Requirements 3.1 and 7.3):
//! detect it and show the import snippet. Antigravity has no CLI or
//! documented config file mmhmem can edit (research.md §8), so `plan` and
//! `plan_remove` give no file changes; the caller shows [`Antigravity::snippet`]
//! and records [`PendingUserStep::ImportAntigravitySnippet`].

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::{
    Agent, AgentId, AgentProblem, AgentRecord, CheckReport, Detection, Env, Feature,
    PendingUserStep, Selection,
};
use crate::setup::check::Registration;
use crate::setup::check::agent::Steps;
use crate::setup::plan::Change;

/// What the user does to take mmhmem out again.
pub const REMOVE_INSTRUCTION: &str =
    "Remove the mmhmem server in Antigravity's settings (MCP servers).";

/// The Antigravity integration.
#[derive(Debug, Clone)]
pub struct Antigravity {
    /// The installed mmhmem binary the snippet runs.
    pub binary: PathBuf,
}

impl Antigravity {
    /// The Antigravity integration for the installed `binary`.
    pub fn new(binary: impl Into<PathBuf>) -> Self {
        Self {
            binary: binary.into(),
        }
    }

    /// The JSON the user imports in Antigravity.
    pub fn snippet_value(&self) -> Value {
        json!({"mcpServers": {"mmhmem": {
            "command": self.binary.to_string_lossy(),
            "args": ["stdio"],
        }}})
    }

    /// [`Antigravity::snippet_value`] as pretty-printed text to show.
    pub fn snippet(&self) -> String {
        serde_json::to_string_pretty(&self.snippet_value()).expect("a JSON value serializes")
    }

    /// The step only the user can do after install.
    pub fn pending_step() -> PendingUserStep {
        PendingUserStep::ImportAntigravitySnippet
    }

    /// What the user does to remove mmhmem again.
    pub fn removal_instruction() -> &'static str {
        REMOVE_INSTRUCTION
    }

    fn gemini_dir(env: &Env) -> PathBuf {
        env.home().join(".gemini")
    }
}

impl Agent for Antigravity {
    fn id(&self) -> AgentId {
        AgentId::Antigravity
    }

    fn detect(&self, env: &Env) -> Detection {
        let mut paths: Vec<PathBuf> = env.find_program("antigravity").into_iter().collect();
        let dir = Self::gemini_dir(env);
        if dir.is_dir() {
            paths.push(dir.clone());
        }
        if paths.is_empty() {
            let dir: &Path = &dir;
            return Detection::NotFound {
                reason: format!(
                    "antigravity is not on PATH and {} does not exist",
                    dir.display()
                ),
            };
        }
        Detection::Found {
            version: None,
            paths,
        }
    }

    fn features(&self) -> &'static [Feature] {
        &[Feature::Mcp]
    }

    fn plan(
        &self,
        _env: &Env,
        _want: &Selection,
        _rec: Option<&AgentRecord>,
    ) -> Result<Vec<Change>, AgentProblem> {
        Ok(Vec::new())
    }

    /// Antigravity has no CLI or config file mmhmem edits: the check starts
    /// exactly the snippet the user imports; the import stays pending.
    fn check(&self, env: &Env, rec: &AgentRecord) -> CheckReport {
        let snippet = self.snippet();
        let registration = Registration::from_claude_json(snippet.as_bytes(), "mmhmem");
        Steps::start(env, AgentId::Antigravity, registration).finish(rec)
    }

    fn plan_remove(&self, _env: &Env, _rec: &AgentRecord) -> Vec<Change> {
        Vec::new()
    }

    /// The user imports [`Antigravity::snippet`] themselves.
    fn pending_steps(&self, want: &Selection) -> Vec<PendingUserStep> {
        if want.wants(AgentId::Antigravity, Feature::Mcp) {
            vec![Self::pending_step()]
        } else {
            Vec::new()
        }
    }
}

#[cfg(test)]
#[path = "antigravity_tests.rs"]
mod tests;
