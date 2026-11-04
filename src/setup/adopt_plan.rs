//! The adoption plan and its plain-language summaries (task 2.6).
//!
//! Each adoptable artefact becomes one [`Action::Update`] change. For all
//! but the old Codex marker block `before == after`, so applying the plan
//! only backs the file up, journals the entry and records its hash; the file
//! is not rewritten. Entries the record already holds are skipped, and an
//! agent with a problem contributes nothing (design.md §3 "Rebuild").

use super::{AgentScan, Artefact, ArtefactKind, Origin, Scan};
use crate::setup::plan::{Action, Change, Plan};
use crate::setup::record::{AgentId, InstallRecord, Scope, UtcTimestamp};
use crate::setup::ui::text::changes::{self as say, What};

/// The agent's name as shown to the user.
pub fn agent_label(agent: AgentId) -> &'static str {
    match agent {
        AgentId::ClaudeCode => "Claude Code",
        AgentId::ClaudeDesktop => "Claude Desktop",
        AgentId::Codex => "Codex",
        AgentId::OpenCode => "OpenCode",
        AgentId::Antigravity => "Antigravity",
    }
}

/// The plan that adopts what `scan` found, for the review screen. `record`
/// is the record that loaded (`None` when it is missing or unreadable); its
/// entries are not planned again. Nothing is applied: the caller shows the
/// plan, and applies it only after the user confirms (Requirement 5.6).
pub fn adoption_plan(scan: &Scan, record: Option<&InstallRecord>, run: UtcTimestamp) -> Plan {
    let mut plan = Plan::new(run);
    for agent in scan.agents.iter().filter(|a| a.can_adopt()) {
        plan.changes.extend(agent_changes(agent, record));
    }
    plan
}

fn agent_changes(agent: &AgentScan, record: Option<&InstallRecord>) -> Vec<Change> {
    let scope = Scope::Agent(agent.agent);
    agent
        .artefacts
        .iter()
        .filter(|a| record.is_none_or(|r| r.find(scope, &a.target).is_none()))
        .map(|a| Change {
            scope,
            target: a.target.clone(),
            file: a.file.clone(),
            action: Action::Update,
            before: Some(a.before.clone()),
            after: Some(a.after.clone()),
            owner_only: false,
            hide_diff: false,
            missing_parents: None,
            summary: a.summary(agent.agent),
        })
        .collect()
}

impl Artefact {
    /// One plain-language line for the review screen, e.g. "Found mmhmem in
    /// Codex's server list (made by the old installer); keep and track it."
    pub fn summary(&self, agent: AgentId) -> String {
        let what = match self.kind {
            ArtefactKind::McpEntry => What::Server,
            ArtefactKind::Skill => What::Guide,
            ArtefactKind::RecapCommand => What::Recap,
            ArtefactKind::RoutingSection => What::Instructions,
            ArtefactKind::ReadHook => What::ReadHook,
            ArtefactKind::ReadPlugin => What::ReadPlugin,
        };
        let old = self.origin == Origin::OldInstaller;
        say::adopt(what, agent_label(agent), old, self.converts_marker_block())
    }
}
