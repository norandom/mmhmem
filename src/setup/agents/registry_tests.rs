//! Tests across the built-in agents (reliable-installers group 3): the
//! registry, the pending user steps, and adoption agreeing with the agents
//! (an adopted install re-planned with the same selection changes nothing).
//! Temporary homes only.

use std::path::Path;

use super::doubles::TestHome;
use super::*;
use crate::setup::adopt::{adoption_plan, scan};
use crate::setup::plan::{self, Outcome, Plan};
use crate::setup::record::{InstallRecord, Os, RecordStore, UtcTimestamp};

fn run(n: u32) -> UtcTimestamp {
    UtcTimestamp::try_from(format!("2026-11-04T09:{n:02}:00Z")).unwrap()
}

fn apply_all(store: &RecordStore, record: &mut InstallRecord, plan: &Plan) {
    let report = plan::apply(plan, store, record);
    for agent in &report.agents {
        assert_eq!(agent.outcome, Outcome::Applied, "{:?}", agent.scope);
    }
}

#[test]
fn the_builtin_registry_lists_every_agent_in_display_order() {
    let registry = Registry::builtin(Path::new("/opt/mmhmem"));
    assert_eq!(registry.ids(), DISPLAY_ORDER);
    for id in DISPLAY_ORDER {
        assert_eq!(registry.get(id).map(|a| a.id()), Some(id));
    }
}

#[test]
fn pending_steps_are_collected_per_selected_agent() {
    let registry = Registry::builtin(Path::new("/opt/mmhmem"));
    let want = Selection::new()
        .with(AgentId::ClaudeCode, &[Feature::Mcp])
        .with(AgentId::ClaudeDesktop, &[Feature::Mcp])
        .with(AgentId::Codex, &[Feature::Mcp, Feature::ReadHook])
        .with(AgentId::Antigravity, &[Feature::Mcp]);
    assert_eq!(
        registry.pending_steps(&want),
        vec![
            (
                AgentId::ClaudeDesktop,
                vec![PendingUserStep::RestartClaudeDesktop]
            ),
            (AgentId::Codex, vec![PendingUserStep::ApproveHookInCodex]),
            (
                AgentId::Antigravity,
                vec![PendingUserStep::ImportAntigravitySnippet]
            ),
        ]
    );
    let codex_mcp = Selection::new().with(AgentId::Codex, &[Feature::Mcp]);
    assert!(registry.pending_steps(&codex_mcp).is_empty());
    assert!(registry.pending_steps(&Selection::new()).is_empty());
}

/// A home whose agent files are exactly what each agent would write, but
/// with no install record, under `CODEX_HOME`, `XDG_CONFIG_HOME` and
/// `CLAUDE_CONFIG_DIR` overrides.
fn home_with_overrides() -> TestHome {
    let mut home = TestHome::new(Os::Windows);
    let root = home.dir.path().to_path_buf();
    home.env.dirs.codex_home = Some(root.join("codex-home"));
    home.env.dirs.xdg_config_home = Some(root.join("xdg"));
    home.env.dirs.claude_config_dir = Some(root.join("claude-config"));
    let profile = home.env.dirs.appdata.clone().unwrap().join("Claude");
    std::fs::create_dir_all(profile).unwrap();
    home
}

#[test]
fn an_adopted_install_replans_to_zero_changes_for_every_agent() {
    let home = home_with_overrides();
    let binary = home.bin().join("mmhmem.exe");
    let registry = Registry::builtin(&binary);
    let mut want = Selection::new();
    for agent in [AgentId::ClaudeCode, AgentId::ClaudeDesktop] {
        want = want.with(agent, registry.get(agent).unwrap().features());
    }
    for agent in [AgentId::Codex, AgentId::OpenCode] {
        want = want.with(agent, registry.get(agent).unwrap().features());
    }
    // Write each agent's files, then forget the record (as after a lost
    // record or an install by another tool with the same output).
    let scratch = RecordStore::new(home.dir.path().join("scratch"));
    for id in want.agents() {
        let changes = registry.get(id).unwrap().plan(&home.env, &want, None);
        let plan = Plan {
            run: run(1),
            changes: changes.unwrap(),
        };
        apply_all(&scratch, &mut InstallRecord::new(), &plan);
    }

    let found = scan(&home.env.dirs);
    let store = RecordStore::new(home.dir.path().join("state"));
    let mut record = InstallRecord::new();
    apply_all(&store, &mut record, &adoption_plan(&found, None, run(2)));
    for id in want.agents() {
        let adopted = found.agent(id).expect("adopted");
        assert!(adopted.can_adopt(), "{:?}", adopted.problems);
        assert_eq!(adopted.features(), want.features(id), "{id:?}");
        let agent = registry.get(id).unwrap();
        let changes = agent.plan(&home.env, &want, record.agents.get(&id));
        let changes = changes.unwrap();
        assert!(changes.is_empty(), "{id:?}: {changes:?}");
    }
}
