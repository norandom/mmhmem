//! Tests of the Claude Desktop agent (task 3.3): profiles per OS, plan,
//! re-plan, apply with drift check, removal and invalid configs. Temporary
//! homes only.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::*;
use crate::setup::agents::doubles::TestHome;
use crate::setup::plan::{self, Plan};
use crate::setup::record::{Drift, InstallRecord, Probe, RecordStore, UtcTimestamp, detect_drift};

const ID: AgentId = AgentId::ClaudeDesktop;
const OTHER_SERVERS: &str = "{\r\n\t\"theme\" : \"dark\",\r\n\t\"mcpServers\": {\r\n\t\t\"files\":{\"command\":\"npx\",\"args\":[\"-y\", \"fs\"]}\r\n\t}\r\n}";

fn agent(home: &TestHome) -> ClaudeDesktop {
    ClaudeDesktop::new(home.home().join("bin").join("mmhmem"))
}

fn want(features: &[Feature]) -> Selection {
    Selection::new().with(ID, features)
}

fn both() -> Selection {
    want(&[Feature::Mcp, Feature::Skill])
}

fn appdata_profile(home: &TestHome) -> PathBuf {
    let dir = home.env.dirs.appdata.clone().unwrap().join("Claude");
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn msix_profile(home: &TestHome) -> PathBuf {
    let local = home.env.dirs.local_app_data.clone().unwrap();
    let dir = local.join("Packages/Claude_pzs8sxrjxfjjc/LocalCache/Roaming/Claude");
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn store(home: &TestHome) -> RecordStore {
    RecordStore::new(home.dir.path().join("state"))
}

fn run_apply(home: &TestHome, changes: Vec<Change>, record: &mut InstallRecord) {
    let run = UtcTimestamp::try_from("2026-11-04T09:30:00Z".to_owned()).unwrap();
    let report = plan::apply(&Plan { run, changes }, &store(home), record);
    let outcome = format!("{:?}", report.agent(SCOPE).map(|a| &a.outcome));
    assert!(outcome.contains("Applied"), "{outcome}");
}

fn drift_is_clean(record: &InstallRecord) {
    assert!(record.applying().is_empty());
    for report in detect_drift(record, &Probe::default()) {
        assert_eq!(report.drift, Drift::Unchanged, "{:?}", report.target);
    }
}

/// Installs both features and returns the record.
fn install(home: &TestHome, agent: &ClaudeDesktop) -> InstallRecord {
    let changes = agent.plan(&home.env, &both(), None).unwrap();
    let mut record = InstallRecord::new();
    run_apply(home, changes, &mut record);
    record
}

fn read_json(file: &Path) -> Value {
    serde_json::from_slice(&std::fs::read(file).unwrap()).unwrap()
}

fn config_of(profile: &Path) -> PathBuf {
    profile.join(CONFIG_FILE)
}

#[test]
fn nothing_found_is_not_found_with_a_plain_reason() {
    let home = TestHome::new(Os::Windows);
    let Detection::NotFound { reason } = agent(&home).detect(&home.env) else {
        panic!("expected NotFound");
    };
    assert!(reason.contains("Claude Desktop"));
    assert!(agent(&home).plan(&home.env, &both(), None).is_err());
}

#[test]
fn appdata_profile_installs_mcp_and_skill_and_replans_to_nothing() {
    let home = TestHome::new(Os::Windows);
    let profile = appdata_profile(&home);
    let desktop = agent(&home);
    assert_eq!(
        desktop.detect(&home.env),
        Detection::Found {
            version: None,
            paths: vec![profile.clone()]
        }
    );
    let record = install(&home, &desktop);
    let config = read_json(&config_of(&profile));
    let binary = desktop.binary.to_string_lossy().into_owned();
    let expected = json!({"command": binary, "args": ["stdio"]});
    assert_eq!(config["mcpServers"]["mmhmem"], expected);
    let skill = profile.join("skills/unified-memory/SKILL.md");
    assert_eq!(std::fs::read(skill).unwrap(), UNIFIED_MEMORY_SKILL.bytes());
    drift_is_clean(&record);
    let rec = record.agents.get(&ID);
    assert!(desktop.plan(&home.env, &both(), rec).unwrap().is_empty());
    assert_eq!(PENDING_STEPS, [PendingUserStep::RestartClaudeDesktop]);
    assert_eq!(desktop.features(), [Feature::Mcp, Feature::Skill]);
}

#[test]
fn msix_profile_is_found_and_used() {
    let home = TestHome::new(Os::Windows);
    let profile = msix_profile(&home);
    let record = install(&home, &agent(&home));
    assert!(read_json(&config_of(&profile))["mcpServers"]["mmhmem"].is_object());
    assert!(profile.join("skills/unified-memory/SKILL.md").is_file());
    drift_is_clean(&record);
}

#[test]
fn several_profiles_stop_only_until_one_is_chosen() {
    let home = TestHome::new(Os::Windows);
    let appdata = appdata_profile(&home);
    let msix = msix_profile(&home);
    let desktop = agent(&home);
    let Detection::Found { paths, .. } = desktop.detect(&home.env) else {
        panic!("expected Found");
    };
    assert_eq!(paths, vec![msix.clone(), appdata.clone()]);
    let Err(AgentProblem::SeveralProfiles { profiles }) = desktop.plan(&home.env, &both(), None)
    else {
        panic!("expected SeveralProfiles");
    };
    assert_eq!(profiles, paths);
    let record = install(&home, &agent(&home).with_profile(&appdata));
    assert!(config_of(&appdata).is_file() && !config_of(&msix).exists());
    // An update without a choice keeps the recorded profile.
    let rec = record.agents.get(&ID);
    assert!(desktop.plan(&home.env, &both(), rec).unwrap().is_empty());
    // A choice that is not a profile does not guess.
    let unknown = agent(&home).with_profile(home.home().join("elsewhere"));
    assert!(matches!(
        unknown.plan(&home.env, &both(), None),
        Err(AgentProblem::SeveralProfiles { .. })
    ));
}

#[test]
fn macos_and_linux_layouts() {
    let mac = TestHome::new(Os::MacOs);
    let support = mac.home().join("Library/Application Support/Claude");
    std::fs::create_dir_all(&support).unwrap();
    install(&mac, &agent(&mac));
    assert!(config_of(&support).is_file());
    let skill = mac
        .home()
        .join("Library/Claude/skills/unified-memory/SKILL.md");
    assert!(skill.is_file());

    let linux = TestHome::new(Os::Linux);
    let dir = linux.home().join(".config/Claude");
    std::fs::create_dir_all(&dir).unwrap();
    install(&linux, &agent(&linux));
    assert!(config_of(&dir).is_file());
    assert!(dir.join("skills/unified-memory/SKILL.md").is_file());

    std::fs::create_dir_all(mac.home().join(".config/Claude")).unwrap();
    let plan = agent(&mac).plan(&mac.env, &both(), None);
    assert!(matches!(plan, Err(AgentProblem::SeveralProfiles { .. })));
}

#[test]
fn removal_restores_a_seeded_config_byte_for_byte() {
    let home = TestHome::new(Os::Windows);
    let profile = appdata_profile(&home);
    std::fs::write(config_of(&profile), OTHER_SERVERS).unwrap();
    let desktop = agent(&home);
    let mut record = install(&home, &desktop);
    let installed = std::fs::read_to_string(config_of(&profile)).unwrap();
    assert!(installed.contains("mmhmem") && installed.contains("\"files\""));
    drift_is_clean(&record);

    let rec = record.agents.get(&ID).unwrap().clone();
    let removal = desktop.plan_remove(&home.env, &rec);
    assert!(removal.iter().all(|c| c.action == Action::Remove));
    run_apply(&home, removal, &mut record);
    let restored = std::fs::read_to_string(config_of(&profile)).unwrap();
    assert_eq!(restored, OTHER_SERVERS);
    assert!(!profile.join("skills/unified-memory/SKILL.md").exists());
}

#[test]
fn an_absent_config_is_created_and_removal_leaves_an_empty_object() {
    let home = TestHome::new(Os::Windows);
    let profile = appdata_profile(&home);
    let desktop = agent(&home);
    let changes = desktop.plan(&home.env, &both(), None).unwrap();
    let mcp = changes
        .iter()
        .find(|c| c.file == config_of(&profile))
        .unwrap();
    assert_eq!(mcp.action, Action::Add);
    assert!(mcp.before.is_none());
    assert_eq!(mcp.missing_parents, Some(2));
    let mut record = InstallRecord::new();
    run_apply(&home, changes, &mut record);
    let rec = record.agents.get(&ID).unwrap().clone();
    run_apply(&home, desktop.plan_remove(&home.env, &rec), &mut record);
    // Documented: mmhmem created the file, so removal leaves a plain `{}`
    // (the file is kept: it is Desktop's own config).
    assert_eq!(read_json(&config_of(&profile)), json!({}));
}

#[test]
fn user_keys_inside_the_entry_and_other_servers_survive() {
    let home = TestHome::new(Os::Windows);
    let profile = appdata_profile(&home);
    let seeded = "{\n  \"mcpServers\": {\n    \"mmhmem\": { \"env\": { \"A\": \"1\" } },\n    \"x\": {}\n  }\n}\n";
    std::fs::write(config_of(&profile), seeded).unwrap();
    let desktop = agent(&home);
    let mut record = install(&home, &desktop);
    let config = read_json(&config_of(&profile));
    assert_eq!(config["mcpServers"]["mmhmem"]["env"], json!({"A": "1"}));
    assert_eq!(config["mcpServers"]["x"], json!({}));
    drift_is_clean(&record);
    let rec = record.agents.get(&ID).unwrap().clone();
    let removal = desktop.plan_remove(&home.env, &rec);
    let mcp = removal
        .iter()
        .find(|c| c.file == config_of(&profile))
        .unwrap();
    assert_eq!(mcp.action, Action::Remove);
    run_apply(&home, removal, &mut record);
    let left = std::fs::read_to_string(config_of(&profile)).unwrap();
    assert!(
        left.contains("\"env\": { \"A\": \"1\" }"),
        "user key byte-identical"
    );
    assert!(left.contains(
        "
    \"x\": {}
"
    ));
    let config: Value = serde_json::from_str(&left).unwrap();
    assert_eq!(config["mcpServers"]["mmhmem"], json!({"env": {"A": "1"}}));
    let rec = record.agents.get(&ID).cloned().unwrap_or_default();
    assert!(rec.managed.is_empty(), "the entry left the record");
}

#[test]
fn a_moved_binary_updates_and_a_deselected_skill_is_removed() {
    let home = TestHome::new(Os::Windows);
    let profile = appdata_profile(&home);
    let mut record = install(&home, &agent(&home));
    let moved = ClaudeDesktop::new(home.home().join("new").join("mmhmem"));
    let rec = record.agents.get(&ID).cloned();
    let only_mcp = want(&[Feature::Mcp]);
    let changes = moved.plan(&home.env, &only_mcp, rec.as_ref()).unwrap();
    assert_eq!(changes.len(), 2);
    assert_eq!(changes[0].action, Action::Remove);
    assert_eq!(changes[1].action, Action::Update);
    run_apply(&home, changes, &mut record);
    let config = read_json(&config_of(&profile));
    let command = config["mcpServers"]["mmhmem"]["command"].as_str().unwrap();
    assert!(command.contains("new"));
    assert!(!profile.join("skills/unified-memory/SKILL.md").exists());
}

#[test]
fn an_invalid_config_is_a_problem_without_echoing_content() {
    let home = TestHome::new(Os::Windows);
    let profile = appdata_profile(&home);
    std::fs::write(config_of(&profile), "{ \"secret-token-123\": ").unwrap();
    let problem = agent(&home).plan(&home.env, &both(), None).unwrap_err();
    assert!(matches!(problem, AgentProblem::Json { .. }));
    let text = problem.to_string();
    assert!(text.contains(CONFIG_FILE) && !text.contains("secret-token-123"));
    std::fs::write(config_of(&profile), "[1]").unwrap();
    assert!(agent(&home).plan(&home.env, &both(), None).is_err());
    // Nothing was written.
    assert_eq!(std::fs::read_to_string(config_of(&profile)).unwrap(), "[1]");
}
