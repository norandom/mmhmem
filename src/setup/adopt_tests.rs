//! Tests for `setup::adopt` (task 2.6): homes seeded like the old
//! PowerShell and Bash installers' output give the expected adoption plan,
//! scan and plan change nothing on disk, applying the plan records every
//! entry with no drift and rewrites only the Codex marker block, and an
//! invalid file blocks only its own agent. Temporary folders only.

use std::path::Path;

use serde_json::json;

use super::fixtures::{self as fx, Seeded};
use super::*;
use crate::setup::edit;
use crate::setup::plan::{self, Action, Outcome, Plan};
use crate::setup::record::{
    Drift, InstallRecord, Probe, RecordStore, Scope, UtcTimestamp, detect_drift,
};

fn run() -> UtcTimestamp {
    UtcTimestamp::try_from("2026-11-04T09:30:00Z".to_owned()).unwrap()
}

fn kinds(scan: &Scan, agent: AgentId) -> Vec<ArtefactKind> {
    let found = scan.agent(agent).expect("agent was found");
    found.artefacts.iter().map(|a| a.kind).collect()
}

fn check_expected_scan(home: &Seeded) -> Scan {
    use ArtefactKind::*;
    let scan = scan(&home.env);
    let agents: Vec<AgentId> = scan.agents.iter().map(|a| a.agent).collect();
    assert_eq!(
        agents,
        [
            AgentId::ClaudeCode,
            AgentId::ClaudeDesktop,
            AgentId::Codex,
            AgentId::OpenCode
        ]
    );
    assert!(scan.agents.iter().all(AgentScan::can_adopt));
    let code = [McpEntry, Skill, RecapCommand, RoutingSection, ReadHook];
    assert_eq!(kinds(&scan, AgentId::ClaudeCode), code);
    assert_eq!(kinds(&scan, AgentId::ClaudeDesktop), [McpEntry, Skill]);
    let codex = [McpEntry, Skill, Skill, RecapCommand, ReadHook];
    assert_eq!(kinds(&scan, AgentId::Codex), codex);
    let opencode = [McpEntry, Skill, RecapCommand, RoutingSection, ReadPlugin];
    assert_eq!(kinds(&scan, AgentId::OpenCode), opencode);
    assert_eq!(scan.binary.as_deref(), Some(home.bin.as_path()));
    assert_eq!(scan.path_entry.as_deref(), home.bin.parent());
    scan
}

fn check_expected_plan(scan: &Scan, home: &Seeded) -> Plan {
    let plan = adoption_plan(scan, None, run());
    assert_eq!(plan.changes.len(), 17);
    assert!(plan.changes.iter().all(|c| c.action == Action::Update));
    assert!(plan.changes.iter().all(|c| !c.owner_only));
    let converted: Vec<&Path> = plan
        .changes
        .iter()
        .filter(|c| c.before != c.after)
        .map(|c| c.file.as_path())
        .collect();
    assert_eq!(converted, [home.codex_config().as_path()]);
    assert!(
        plan.changes
            .iter()
            .filter(|c| c.before == c.after)
            .all(|c| c.diff() == plan::NO_DIFF)
    );
    plan
}

#[test]
fn powershell_home_gives_the_expected_scan_and_plan() {
    let home = fx::powershell(false);
    let scan = check_expected_scan(&home);
    assert_eq!(scan.leftovers.len(), 3);
    let plan = check_expected_plan(&scan, &home);
    let codex = plan
        .changes
        .iter()
        .find(|c| c.file == home.codex_config())
        .unwrap();
    assert_eq!(
        codex.summary,
        "Found mmhmem in the Codex config (made by the old installer); keep and track it. \
         Only the old installer's two marker comment lines are removed; \
         the rest of the file stays as it is."
    );
    assert!(
        matches!(&codex.target, Target::TomlTable { managed_keys: Some(k), .. } if k.len() == 2)
    );
}

#[test]
fn bash_home_gives_the_expected_scan_and_plan() {
    let home = fx::bash();
    let scan = check_expected_scan(&home);
    assert_eq!(scan.leftovers.len(), 2);
    let plan = check_expected_plan(&scan, &home);
    let summaries: Vec<&str> = plan.changes.iter().map(|c| c.summary.as_str()).collect();
    assert!(summaries.contains(&"Found mmhmem in the Claude Desktop config; keep and track it."));
    assert!(summaries.contains(&"Found the mmhmem skill for Claude Code; keep and track it."));
    assert!(summaries.contains(
        &"Found mmhmem's reading instructions for OpenCode (made by the old installer); \
          keep and track it."
    ));
    assert!(summaries.contains(&"Found mmhmem's read-routing hook for Codex; keep and track it."));
    let scopes: Vec<Scope> = plan.scopes();
    assert_eq!(scopes.len(), 4);
}

#[test]
fn managed_keys_follow_the_agent_rule() {
    let scan = scan(&fx::bash().env);
    let managed = |agent: AgentId| -> Vec<String> {
        let found = scan.agent(agent).unwrap();
        match &found.artefacts[0].target {
            Target::JsonEntry { managed_keys, .. } | Target::TomlTable { managed_keys, .. } => {
                let keys = managed_keys.as_ref().unwrap();
                keys.iter().map(|k| k.as_str().to_owned()).collect()
            }
            other => panic!("not an entry: {other:?}"),
        }
    };
    assert_eq!(managed(AgentId::ClaudeCode), ["command", "args"]);
    assert_eq!(managed(AgentId::ClaudeDesktop), ["command", "args"]);
    assert_eq!(managed(AgentId::Codex), ["command", "args"]);
    assert_eq!(managed(AgentId::OpenCode), ["type", "command"]);
    let features = scan.agent(AgentId::Codex).unwrap().features();
    assert_eq!(features.len(), 4);
    assert!(features.contains(&Feature::ReadHook));
}

#[test]
fn scan_and_plan_change_nothing_on_disk() {
    for home in [fx::powershell(false), fx::powershell(true), fx::bash()] {
        let before = fx::tree_hash(&home.root);
        let found = scan(&home.env);
        let plan = adoption_plan(&found, None, run());
        assert_eq!(plan.changes.len(), 17);
        let _ = format!("{found:?} {plan:?}");
        assert_eq!(fx::tree_hash(&home.root), before);
    }
}

#[test]
fn msix_profile_is_found() {
    let home = fx::powershell(true);
    let scan = scan(&home.env);
    let desktop = scan.agent(AgentId::ClaudeDesktop).unwrap();
    assert_eq!(desktop.artefacts.len(), 2);
    assert!(desktop.artefacts[0].file.starts_with(&home.desktop_dir));
    assert!(
        desktop.artefacts[0]
            .file
            .to_string_lossy()
            .contains("Packages")
    );
}

fn apply_plan(home: &Seeded) -> (RecordStore, InstallRecord, tempfile::TempDir, Plan) {
    let state = tempfile::tempdir().unwrap();
    let store = RecordStore::new(state.path().join("record"));
    let plan = adoption_plan(&scan(&home.env), None, run());
    let mut record = InstallRecord::new();
    let report = plan::apply(&plan, &store, &mut record);
    assert_eq!(report.agents.len(), 4);
    assert!(report.agents.iter().all(|a| a.outcome == Outcome::Applied));
    (store, record, state, plan)
}

fn check_apply(home: &Seeded, begin: &str) {
    let before = fx::snapshot(&home.root);
    let (store, record, _state, plan) = apply_plan(home);
    let after = fx::snapshot(&home.root);
    let changed: Vec<_> = after.iter().filter(|(p, b)| before[*p] != **b).collect();
    assert_eq!(changed.len(), 1);
    let (path, bytes) = changed[0];
    assert_eq!(*path, home.codex_config());
    let original = String::from_utf8(before[path].clone()).unwrap();
    let expected = original
        .replace(&format!("{begin}\n"), "")
        .replace(&format!("{}\n", fx::MARKER_END), "");
    assert_eq!(String::from_utf8(bytes.clone()).unwrap(), expected);
    assert_eq!(after.len(), before.len());

    let drift = detect_drift(&record, &Probe::default());
    assert_eq!(drift.len(), 17);
    assert!(drift.iter().all(|d| d.drift == Drift::Unchanged));
    assert_eq!(store.load().unwrap().unwrap(), record);
    let codex = plan.changes.iter().find(|c| c.file == *path).unwrap();
    let entry = record.find(codex.scope, &codex.target).unwrap();
    let backup = entry.backup.as_ref().unwrap().resolve(store.dir());
    assert_eq!(std::fs::read(backup).unwrap(), before[path]);
}

#[test]
fn applying_the_powershell_plan_records_everything_and_only_converts_codex() {
    check_apply(&fx::powershell(false), fx::MARKER_PS1);
}

#[test]
fn applying_the_bash_plan_records_everything_and_only_converts_codex() {
    check_apply(&fx::bash(), fx::MARKER_SH);
}

#[test]
fn rescan_with_a_record_plans_nothing_and_user_keys_are_not_drift() {
    let home = fx::bash();
    let (_store, record, _state, _plan) = apply_plan(&home);
    let again = scan(&home.env);
    assert_eq!(adoption_plan(&again, Some(&record), run()).changes.len(), 0);
    let fresh = adoption_plan(&again, None, run());
    assert_eq!(fresh.changes.len(), 17);
    assert!(fresh.changes.iter().all(|c| c.before == c.after));

    let path = home.opencode_config();
    let text = std::fs::read_to_string(&path).unwrap();
    let keys = ["mcp", "servers", "mmhmem", "enabled"];
    let edited = edit::json::set(&text, &keys, &json!(false)).unwrap();
    std::fs::write(&path, edited).unwrap();
    let drift = detect_drift(&record, &Probe::default());
    assert!(drift.iter().all(|d| d.drift == Drift::Unchanged));
    let command = ["mcp", "servers", "mmhmem", "command"];
    let edited = edit::json::set(&text, &command, &json!(["other", "stdio"])).unwrap();
    std::fs::write(&path, edited).unwrap();
    let changed = detect_drift(&record, &Probe::default())
        .into_iter()
        .filter(|d| d.drift != Drift::Unchanged)
        .count();
    assert_eq!(changed, 1);
}

#[test]
fn invalid_opencode_json_blocks_only_opencode() {
    let home = fx::powershell(false);
    let secret = "{ \"apiKey\": \"sk-secret-123\" oops";
    fx::write(&home.opencode_config(), secret);
    let found = scan(&home.env);
    let opencode = found.agent(AgentId::OpenCode).unwrap();
    assert!(!opencode.can_adopt());
    let problem = Problem {
        file: home.opencode_config(),
        kind: ProblemKind::NotValid(Format::Json),
    };
    assert_eq!(opencode.problems.as_slice(), std::slice::from_ref(&problem));
    let text = format!(
        "cannot adopt: {} is not valid JSON",
        home.opencode_config().display()
    );
    assert_eq!(problem.to_string(), text);
    assert_eq!(opencode.problem_lines(), [format!("OpenCode: {text}")]);
    let others = found.agents.iter().filter(|a| a.agent != AgentId::OpenCode);
    assert!(others.clone().all(AgentScan::can_adopt));
    let plan = adoption_plan(&found, None, run());
    assert_eq!(plan.changes.len(), 12);
    assert!(
        plan.changes_for(Scope::Agent(AgentId::OpenCode))
            .next()
            .is_none()
    );
    assert!(!format!("{found:?} {plan:?} {problem}").contains("sk-secret"));
}

#[test]
fn bad_codex_files_block_codex_without_echoing_contents() {
    let home = fx::bash();
    let config = home.codex_config();
    let good = std::fs::read_to_string(&config).unwrap();
    let unmatched = good.replace(&format!("{}\n", fx::MARKER_END), "");
    std::fs::write(&config, unmatched).unwrap();
    let codex = scan(&home.env).agent(AgentId::Codex).unwrap().clone();
    assert_eq!(codex.problems.len(), 1);
    let line = codex.problems[0].to_string();
    assert!(line.contains("marker lines that do not match"), "{line}");
    assert!(!line.contains("mcp_servers"));

    std::fs::write(&config, "token = \"sk-secret-9\n[broken").unwrap();
    let codex = scan(&home.env).agent(AgentId::Codex).unwrap().clone();
    let expected = Problem {
        file: config,
        kind: ProblemKind::NotValid(Format::Toml),
    };
    assert_eq!(codex.problems, [expected]);
    assert!(!format!("{codex:?}").contains("sk-secret"));
}

#[test]
fn an_entry_that_is_not_a_table_blocks_its_agent() {
    let home = fx::bash();
    let path = home.desktop_dir.join("claude_desktop_config.json");
    fx::write(&path, "{\"mcpServers\": {\"mmhmem\": \"oops\"}}");
    let found = scan(&home.env);
    let desktop = found.agent(AgentId::ClaudeDesktop).unwrap();
    assert_eq!(desktop.problems.len(), 1);
    assert!(
        desktop.problems[0]
            .to_string()
            .starts_with("cannot adopt: ")
    );
}

#[test]
fn a_same_named_plugin_that_is_not_ours_is_left_alone() {
    let home = fx::bash();
    let plugin = home
        .home()
        .join(".config/opencode/plugins/mmhmem-read-routing.js");
    fx::write(&plugin, "export default {};\n");
    let found = scan(&home.env);
    let opencode = found.agent(AgentId::OpenCode).unwrap();
    assert_eq!(opencode.foreign, [plugin]);
    assert!(opencode.can_adopt());
    assert!(!kinds(&found, AgentId::OpenCode).contains(&ArtefactKind::ReadPlugin));
}

#[test]
fn an_empty_home_gives_an_empty_plan() {
    for os in [Os::Windows, Os::Linux, Os::MacOs] {
        let home = fx::empty(os);
        let before = fx::tree_hash(&home.root);
        let found = scan(&home.env);
        assert!(found.is_empty(), "{found:?}");
        let plan = adoption_plan(&found, None, run());
        assert!(plan.changes.is_empty());
        assert_eq!(fx::tree_hash(&home.root), before);
    }
}
