//! Agent integrations: the [`Agent`] trait, the registry, [`Env`] and
//! [`Selection`] (reliable-installers task 3.1,
//! `.kiro/specs/reliable-installers/design.md` §5 and §6; Requirements 2.1,
//! 8.1 and 8.4).
//!
//! Each agent module (tasks 3.2–3.5) implements [`Agent`]: it detects the
//! agent, plans its changes as whole-file bytes (chained per file with
//! [`FileChain`]), plans their removal from the install record and, from
//! task 4.2, checks that the agent really sees mmhmem (the shared steps are
//! in `setup::check::agent`). Submodules:
//!
//! - `env.rs`: [`Env`] (folders, `PATH`, OS) and the injectable
//!   [`CommandRunner`] for agent CLIs, with the real [`SystemRunner`].
//! - `problem.rs`: [`AgentProblem`] and [`FileChain`].
//! - `doubles.rs`, `doubles_cli.rs` (tests only): a test home, a replaying
//!   runner and doubles of the `claude`, `codex` and `opencode` CLIs.
//!
//! **Rule for every agent module** (from the 2.2/2.3 reviews): edit leaf keys
//! of mmhmem's own entry (e.g. `mcp.servers.mmhmem.command`), never replace a
//! whole user-visible object or array. Use the JSON array element functions
//! for shared arrays (`hooks.PreToolUse`: mmhmem owns its hook handler, never
//! the group, see `hook_entry.rs`) and keep user-added
//! keys inside mmhmem's entry. Record `missing_parents` before a `set` so
//! removal prunes only parents mmhmem created.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Path, PathBuf};

use crate::setup::adopt::agent_label;
use crate::setup::plan::Change;
use crate::setup::record::{CheckDetail, CheckResult, InstallRecord, LastCheck, UtcTimestamp};

pub mod antigravity;
pub mod claude_code;
pub mod claude_desktop;
pub mod codex;
mod env;
mod hook_entry;
pub mod opencode;
mod problem;

pub use crate::setup::adopt::ScanEnv;
pub use crate::setup::record::{AgentId, AgentRecord, Feature, PendingUserStep};
pub use env::{
    CommandOutput, CommandRunner, DEFAULT_TIMEOUT, Env, Invocation, MAX_OUTPUT, RunError,
    SystemRunner,
};
pub use problem::{AgentProblem, FileChain, read_optional, section_left};

#[cfg(test)]
mod check_more_tests;
#[cfg(test)]
pub(crate) mod check_tests;
#[cfg(test)]
pub(crate) mod doubles;
#[cfg(test)]
pub(crate) mod doubles_cli;
#[cfg(test)]
mod hook_group_tests;
#[cfg(test)]
mod registry_tests;
#[cfg(test)]
mod tests;

/// Every agent in the order the menu and the results list them.
pub const DISPLAY_ORDER: [AgentId; 5] = [
    AgentId::ClaudeCode,
    AgentId::ClaudeDesktop,
    AgentId::Codex,
    AgentId::OpenCode,
    AgentId::Antigravity,
];

impl AgentId {
    /// The stable key used in the install record and on the command line
    /// (the same names as its serde form).
    pub fn key(self) -> &'static str {
        match self {
            Self::ClaudeCode => "claude_code",
            Self::ClaudeDesktop => "claude_desktop",
            Self::Codex => "codex",
            Self::OpenCode => "opencode",
            Self::Antigravity => "antigravity",
        }
    }

    /// The agent for a [`AgentId::key`].
    pub fn from_key(key: &str) -> Option<Self> {
        DISPLAY_ORDER.into_iter().find(|id| id.key() == key)
    }

    /// The agent's name as users know it ("Claude Code").
    pub fn label(self) -> &'static str {
        agent_label(self)
    }

    /// The position in [`DISPLAY_ORDER`].
    pub fn display_position(self) -> usize {
        DISPLAY_ORDER
            .iter()
            .position(|id| *id == self)
            .unwrap_or(DISPLAY_ORDER.len())
    }
}

/// Whether an agent is installed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Detection {
    /// Found.
    Found {
        /// Its version, when the agent reports one.
        version: Option<String>,
        /// The program or folders that showed it is there.
        paths: Vec<PathBuf>,
    },
    /// Not found.
    NotFound {
        /// A plain reason, naming programs and folders only.
        reason: String,
    },
}

impl Detection {
    /// The agent was found.
    pub fn is_found(&self) -> bool {
        matches!(self, Self::Found { .. })
    }
}

/// The features the user selected, per agent.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Selection {
    per_agent: BTreeMap<AgentId, BTreeSet<Feature>>,
}

impl Selection {
    /// Nothing selected.
    pub fn new() -> Self {
        Self::default()
    }

    /// This selection plus `features` for `agent`.
    pub fn with(mut self, agent: AgentId, features: &[Feature]) -> Self {
        for feature in features {
            self.select(agent, *feature);
        }
        self
    }

    /// Selects one feature.
    pub fn select(&mut self, agent: AgentId, feature: Feature) {
        self.per_agent.entry(agent).or_default().insert(feature);
    }

    /// Deselects one feature.
    pub fn deselect(&mut self, agent: AgentId, feature: Feature) {
        if let Some(features) = self.per_agent.get_mut(&agent) {
            features.remove(&feature);
            if features.is_empty() {
                self.per_agent.remove(&agent);
            }
        }
    }

    /// Whether `feature` is selected for `agent`.
    pub fn wants(&self, agent: AgentId, feature: Feature) -> bool {
        self.per_agent
            .get(&agent)
            .is_some_and(|f| f.contains(&feature))
    }

    /// The features selected for `agent`.
    pub fn features(&self, agent: AgentId) -> BTreeSet<Feature> {
        self.per_agent.get(&agent).cloned().unwrap_or_default()
    }

    /// Agents with at least one feature selected, in [`DISPLAY_ORDER`].
    pub fn agents(&self) -> Vec<AgentId> {
        DISPLAY_ORDER
            .into_iter()
            .filter(|id| self.per_agent.contains_key(id))
            .collect()
    }

    /// The features switched on in an install record (for updates).
    pub fn from_record(record: &InstallRecord) -> Self {
        let mut selection = Self::new();
        for (agent, rec) in &record.agents {
            for (feature, on) in &rec.features {
                if *on {
                    selection.select(*agent, *feature);
                }
            }
        }
        selection
    }
}

/// The result of one agent's functional check (design.md §6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckReport {
    /// The agent.
    pub agent: AgentId,
    /// False when no check ran (this agent's check is not implemented yet).
    pub ran: bool,
    /// The overall result.
    pub result: CheckResult,
    /// Per-step outcomes.
    pub details: Vec<CheckDetail>,
    /// The result line, e.g. `✔ Codex — mmhmem connected, 11 tools`.
    pub line: String,
    /// One `● Codex — <step>` line per step only the user can do (only
    /// when everything else passed).
    pub pending: Vec<String>,
    /// Plain notes that are neither a failure nor a user step to do now,
    /// e.g. a read hook that stays inactive without a worker model.
    pub notes: Vec<String>,
}

/// The reason shown while an agent's check is not implemented (task 4.2).
pub const CHECK_NOT_IMPLEMENTED: &str = "the check is not available in this build";

impl CheckReport {
    /// The report of an agent whose check is not implemented yet.
    pub fn not_implemented(agent: AgentId) -> Self {
        Self {
            agent,
            ran: false,
            result: CheckResult::Fail,
            details: Vec::new(),
            line: format!("✘ {} — {CHECK_NOT_IMPLEMENTED}", agent.label()),
            pending: Vec::new(),
            notes: Vec::new(),
        }
    }

    /// The result line followed by the pending-step and note lines.
    pub fn lines(&self) -> Vec<String> {
        std::iter::once(self.line.clone())
            .chain(self.pending.iter().cloned())
            .chain(self.notes.iter().cloned())
            .collect()
    }

    /// The record's `last_check` for this report.
    pub fn last_check(&self, at: UtcTimestamp) -> LastCheck {
        LastCheck {
            at,
            result: self.result,
            details: self.details.clone(),
        }
    }
}

/// One agent integration (design.md §5).
pub trait Agent: Send + Sync {
    /// Which agent.
    fn id(&self) -> AgentId;

    /// Whether the agent is installed: never writes anything.
    fn detect(&self, env: &Env) -> Detection;

    /// The features this agent supports, in menu order.
    fn features(&self) -> &'static [Feature];

    /// The changes that install `want`'s features for this agent, chained
    /// per file. `rec` is the agent's install record, if any, so a re-plan
    /// of an unchanged install gives no changes and deselected features are
    /// removed. Reading only; nothing is written.
    fn plan(
        &self,
        env: &Env,
        want: &Selection,
        rec: Option<&AgentRecord>,
    ) -> Result<Vec<Change>, AgentProblem>;

    /// Problems that leave one selected feature out while [`Agent::plan`]
    /// still plans the others (Antigravity's read hook when its command
    /// cannot be written); the review lists them. Reading only; none by
    /// default.
    fn feature_problems(&self, env: &Env, want: &Selection) -> Vec<AgentProblem> {
        let _ = (env, want);
        Vec::new()
    }

    /// Whether the agent really sees mmhmem (design.md §6; task 4.2).
    fn check(&self, env: &Env, rec: &AgentRecord) -> CheckReport {
        let _ = (env, rec);
        CheckReport::not_implemented(self.id())
    }

    /// The changes that take out everything `rec` lists for this agent.
    fn plan_remove(&self, env: &Env, rec: &AgentRecord) -> Vec<Change>;

    /// The steps only the user can do after installing `want` (approve the
    /// Codex hook, restart Claude Desktop); the caller stores them in the
    /// agent's record. None by default.
    fn pending_steps(&self, want: &Selection) -> Vec<PendingUserStep> {
        let _ = want;
        Vec::new()
    }
}

/// The same agent was registered twice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DuplicateAgent(pub AgentId);

impl fmt::Display for DuplicateAgent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} is registered twice", self.0.label())
    }
}

impl std::error::Error for DuplicateAgent {}

/// The agents in [`DISPLAY_ORDER`].
pub struct Registry {
    agents: Vec<Box<dyn Agent>>,
}

impl fmt::Debug for Registry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.ids()).finish()
    }
}

impl Registry {
    /// A registry of `agents`, sorted into [`DISPLAY_ORDER`].
    pub fn new(mut agents: Vec<Box<dyn Agent>>) -> Result<Self, DuplicateAgent> {
        agents.sort_by_key(|a| a.id().display_position());
        if let Some(pair) = agents.windows(2).find(|w| w[0].id() == w[1].id()) {
            return Err(DuplicateAgent(pair[0].id()));
        }
        Ok(Self { agents })
    }

    /// The agents built into this release, in [`DISPLAY_ORDER`], each wired
    /// to the installed mmhmem `binary`.
    pub fn builtin(binary: &Path) -> Self {
        Self {
            agents: vec![
                Box::new(claude_code::ClaudeCode::new(binary)),
                Box::new(claude_desktop::ClaudeDesktop::new(binary)),
                Box::new(codex::Codex::new(binary)),
                Box::new(opencode::OpenCode::new(binary)),
                Box::new(antigravity::Antigravity::new(binary)),
            ],
        }
    }

    /// Each selected agent's [`Agent::pending_steps`] for `want`, in display
    /// order; agents without steps are left out.
    pub fn pending_steps(&self, want: &Selection) -> Vec<(AgentId, Vec<PendingUserStep>)> {
        self.agents()
            .filter(|a| want.agents().contains(&a.id()))
            .map(|a| (a.id(), a.pending_steps(want)))
            .filter(|(_, steps)| !steps.is_empty())
            .collect()
    }

    /// The agents, in display order.
    pub fn agents(&self) -> impl Iterator<Item = &dyn Agent> {
        self.agents.iter().map(|a| a.as_ref())
    }

    /// The agent ids, in display order.
    pub fn ids(&self) -> Vec<AgentId> {
        self.agents.iter().map(|a| a.id()).collect()
    }

    /// One agent.
    pub fn get(&self, id: AgentId) -> Option<&dyn Agent> {
        self.agents().find(|a| a.id() == id)
    }
}
