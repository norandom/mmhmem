//! Antigravity (reliable-installers task 3.5; Requirements 3.1 and 7.3):
//! detect it and show the import snippet. Antigravity has no CLI or
//! documented config file mmhmem can edit (research.md §8), so `plan`
//! saves the import snippet as a file next to the installed binary
//! (`antigravity-mcp.json`) for the user to paste into Antigravity's MCP
//! settings, and records [`PendingUserStep::ImportAntigravitySnippet`].

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::{
    Agent, AgentId, AgentProblem, AgentRecord, CheckReport, Detection, Env, Feature, FileChain,
    PendingUserStep, Selection,
};
use crate::setup::check::Registration;
use crate::setup::check::agent::Steps;
use crate::setup::plan::{Action, Change};
use crate::setup::record::{Scope, Target};
use crate::setup::ui::text::changes as say;

/// What the user does to take mmhmem out again.
pub const REMOVE_INSTRUCTION: &str =
    "Remove the mmhmem server in Antigravity's settings (MCP servers).";

/// The saved import snippet, next to the installed binary.
pub const SNIPPET_FILE: &str = "antigravity-mcp.json";

const SCOPE: Scope = Scope::Agent(AgentId::Antigravity);

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

    /// The file the snippet is saved to, next to the installed binary.
    pub fn snippet_file(&self) -> PathBuf {
        self.binary
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(SNIPPET_FILE)
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
            tracing::debug!(
                "antigravity is not on PATH and {} does not exist",
                dir.display()
            );
            return Detection::NotFound {
                reason: say::not_installed(AgentId::Antigravity.label()),
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

    /// Saves the import snippet next to the binary (the whole file is
    /// mmhmem's; Antigravity itself has no config to edit).
    fn plan(
        &self,
        _env: &Env,
        want: &Selection,
        rec: Option<&AgentRecord>,
    ) -> Result<Vec<Change>, AgentProblem> {
        if !want.wants(AgentId::Antigravity, Feature::Mcp) {
            return Ok(Vec::new());
        }
        let file = self.snippet_file();
        let mut chain = FileChain::new();
        let before = chain.bytes(&file)?;
        let wanted = self.snippet().into_bytes();
        let recorded = rec.is_some_and(|r| {
            r.managed
                .iter()
                .any(|e| e.target == Target::File { file: file.clone() })
        });
        if before.as_deref() == Some(wanted.as_slice()) && recorded {
            return Ok(Vec::new());
        }
        chain.push(Change {
            scope: SCOPE,
            target: Target::File { file: file.clone() },
            file: file.clone(),
            action: if before.is_some() {
                Action::Update
            } else {
                Action::Add
            },
            before,
            after: Some(wanted),
            owner_only: false,
            hide_diff: false,
            missing_parents: None,
            summary: format!(
                "Save the snippet you paste into Antigravity (adds the file {})",
                file.display()
            ),
        });
        Ok(chain.into_changes())
    }

    /// Antigravity has no CLI or config file mmhmem edits: the check starts
    /// exactly the snippet the user imports; the import stays pending.
    fn check(&self, env: &Env, rec: &AgentRecord) -> CheckReport {
        let snippet = self.snippet();
        let registration = Registration::from_claude_json(snippet.as_bytes(), "mmhmem");
        Steps::start(env, AgentId::Antigravity, registration).finish(rec)
    }

    /// Takes the saved snippet file back out; Antigravity's own settings
    /// are the user's to undo ([`REMOVE_INSTRUCTION`]).
    fn plan_remove(&self, _env: &Env, rec: &AgentRecord) -> Vec<Change> {
        let mut chain = FileChain::new();
        for entry in &rec.managed {
            let Target::File { file } = &entry.target else {
                continue;
            };
            if *file != self.snippet_file() {
                continue;
            }
            let Ok(Some(before)) = chain.bytes(file) else {
                continue;
            };
            chain.push(Change {
                scope: SCOPE,
                target: entry.target.clone(),
                file: file.clone(),
                action: Action::Remove,
                before: Some(before),
                after: None,
                owner_only: false,
                hide_diff: false,
                missing_parents: None,
                summary: say::remove(AgentId::Antigravity.label(), file),
            });
        }
        chain.into_changes()
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
