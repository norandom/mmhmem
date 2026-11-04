//! Tests for the Claude Code integration (task 3.2): install, re-plan with no
//! changes, the hook on and off, byte-identical removal, user keys inside
//! mmhmem's entry, invalid files and the routing section. Temporary homes
//! only.

use std::path::PathBuf;

use serde_json::{Value, json};

use super::*;
use crate::setup::agents::doubles::TestHome;
use crate::setup::agents::doubles_cli;
use crate::setup::plan::{self, Outcome, Plan, target_file};
use crate::setup::record::{
    Drift, InstallRecord, Os, Probe, RecordStore, UtcTimestamp, detect_drift,
};

/// `~/.claude.json` with unrelated settings, another server and odd
/// formatting.
const CLAUDE_JSON: &str = "{\n  \"numStartups\" : 42,\n\t\"oauthAccount\": {\"emailAddress\": \"user@example.com\"},\n  \"mcpServers\": {\n      \"other\": { \"type\": \"stdio\", \"command\": \"other-server\", \"args\": [], \"env\": {\"TOKEN\": \"secret-value\"} }\n  },\n  \"projects\": {\"C:\\\\work\": {\"allowedTools\": []}}\n}\n";

/// `settings.json` with a user hook group and other settings.
const SETTINGS: &str = "{\n    \"theme\" :  \"dark\",\n  \"hooks\": {\n    \"PreToolUse\": [\n      { \"matcher\": \"Bash\", \"hooks\": [ { \"type\": \"command\", \"command\": \"my-guard\" } ] }\n    ],\n    \"Stop\": []\n  },\n  \"permissions\": {\"allow\": [\"Bash(ls:*)\"]}\n}\n";

struct Rig {
    home: TestHome,
    store: RecordStore,
    record: InstallRecord,
    agent: ClaudeCode,
    runs: u32,
}

fn rig() -> Rig {
    let home = TestHome::new(Os::current());
    let store = RecordStore::new(home.dir.path().join("state"));
    let agent = ClaudeCode::new(home.bin().join("mmhmem.exe"));
    Rig {
        home,
        store,
        record: InstallRecord::new(),
        agent,
        runs: 0,
    }
}

fn all() -> Selection {
    Selection::new().with(AgentId::ClaudeCode, &FEATURES)
}

fn without_hook() -> Selection {
    let mut want = all();
    want.deselect(AgentId::ClaudeCode, Feature::ReadHook);
    want
}

impl Rig {
    fn paths(&self) -> Paths {
        Paths::of(&self.home.env)
    }

    fn rec(&self) -> Option<&AgentRecord> {
        self.record.agents.get(&AgentId::ClaudeCode)
    }

    fn plan(&self, want: &Selection) -> Vec<Change> {
        self.agent
            .plan(&self.home.env, want, self.rec())
            .expect("planned")
    }

    fn apply(&mut self, changes: Vec<Change>) {
        self.runs += 1;
        let run = UtcTimestamp::try_from(format!("2026-11-04T09:{:02}:00Z", self.runs)).unwrap();
        let report = plan::apply(&Plan { run, changes }, &self.store, &mut self.record);
        for agent in &report.agents {
            assert_eq!(agent.outcome, Outcome::Applied, "{:?}", agent.scope);
        }
    }

    fn remove_all(&mut self) {
        let rec = self.rec().cloned().expect("recorded");
        let changes = self.agent.plan_remove(&self.home.env, &rec);
        self.apply(changes);
    }

    fn assert_no_drift(&self) {
        let reports = detect_drift(&self.record, &Probe::default());
        assert!(!reports.is_empty());
        for report in reports {
            assert_eq!(report.drift, Drift::Unchanged, "{:?}", report.target);
        }
    }
}

fn read(file: &std::path::Path) -> String {
    std::fs::read_to_string(file).expect("file")
}

fn parse(file: &std::path::Path) -> Value {
    serde_json::from_str(&read(file)).expect("strict JSON")
}

fn own_groups(settings: &Value) -> Vec<Value> {
    let matcher = ElementMatcher::MmhmemHook(HookHarness::Claude);
    settings["hooks"]["PreToolUse"]
        .as_array()
        .map(|a| a.iter().filter(|g| matcher.matches(g)).cloned().collect())
        .unwrap_or_default()
}

#[test]
fn detects_the_cli_with_its_version() {
    let r = rig();
    let program = r.home.install_program("claude");
    doubles_cli::claude(&r.home.runner, &r.home.env);
    let found = r.agent.detect(&r.home.env);
    assert_eq!(
        found,
        Detection::Found {
            version: Some("2.1.302".to_owned()),
            paths: vec![program]
        }
    );
}

#[test]
fn detects_the_folder_alone_and_tolerates_a_failing_cli() {
    let r = rig();
    assert!(matches!(
        r.agent.detect(&r.home.env),
        Detection::NotFound { reason } if reason == "Claude Code is not installed"
    ));
    std::fs::create_dir_all(r.home.home().join(".claude")).unwrap();
    r.home.install_program("claude");
    // Unscripted: running `claude --version` fails.
    let Detection::Found { version, paths } = r.agent.detect(&r.home.env) else {
        panic!("not found");
    };
    assert_eq!(version, None);
    assert_eq!(paths.len(), 2);
}

#[test]
fn install_writes_every_feature_records_it_and_replans_to_nothing() {
    let mut r = rig();
    let p = r.paths();
    r.home.write(&[".claude.json"], CLAUDE_JSON);
    let changes = r.plan(&all());
    assert_eq!(changes.len(), 5);
    assert!(changes.iter().all(|c| c.scope == SCOPE));
    let mcp = changes.iter().find(|c| c.file == p.claude_json).unwrap();
    assert!(!mcp.owner_only, ".claude.json keeps its own permissions");
    assert!(mcp.hide_diff, ".claude.json can hold secrets");
    assert_eq!(mcp.diff(), plan::HIDDEN_DIFF);
    assert_eq!(mcp.missing_parents, Some(0));
    r.apply(changes);
    assert_installed(&r, &p);
    r.assert_no_drift();
    assert!(r.plan(&all()).is_empty(), "a re-plan changes nothing");
}

/// The bytes on disk after installing every feature over [`CLAUDE_JSON`].
fn assert_installed(r: &Rig, p: &Paths) {
    let root = parse(&p.claude_json);
    let bin = r.agent.binary_text();
    let want = json!({"type": "stdio", "command": bin, "args": ["stdio"]});
    assert_eq!(root["mcpServers"]["mmhmem"], want);
    assert_eq!(root["mcpServers"]["other"]["env"]["TOKEN"], "secret-value");
    assert_eq!(root["numStartups"], 42);
    assert_eq!(read(&p.skill), UNIFIED_MEMORY_SKILL.text);
    assert_eq!(read(&p.command), CLAUDE_RECAP_COMMAND.text);
    let rules = read(&p.rules);
    assert!(
        rules.starts_with("<!-- mmhmem:routing:start -->\n<!-- mmhmem:routing:prefix=none -->\n")
    );
    assert!(rules.ends_with("<!-- mmhmem:routing:end -->\n"));
    let settings = parse(&p.settings);
    assert_eq!(own_groups(&settings), vec![hook_group(&bin)]);
    let group = &settings["hooks"]["PreToolUse"][0];
    assert_eq!(group["matcher"], "Read|Bash|PowerShell");
    assert_eq!(group["hooks"][0]["args"], json!(["hook", "claude"]));
    assert_eq!(group["hooks"][0]["timeout"], 10);
}

#[test]
fn mcp_is_planned_with_any_other_feature() {
    let r = rig();
    let want = Selection::new().with(AgentId::ClaudeCode, &[Feature::Skill]);
    let changes = r.plan(&want);
    let targets: Vec<&Target> = changes.iter().map(|c| &c.target).collect();
    assert_eq!(
        targets,
        vec![
            &mcp_target(&r.paths()),
            &Target::File {
                file: r.paths().skill
            }
        ]
    );
    assert!(r.plan(&Selection::new()).is_empty());
}

#[test]
fn the_hook_goes_on_and_off_leaving_settings_byte_identical() {
    let mut r = rig();
    let p = r.paths();
    r.home.write(&[".claude", "settings.json"], SETTINGS);
    let first = r.plan(&without_hook());
    assert!(first.iter().all(|c| c.file != p.settings));
    r.apply(first);

    let on = r.plan(&all());
    assert_eq!(on.len(), 1);
    assert_eq!(
        (on[0].target.clone(), on[0].action),
        (hook_target(&p), Action::Add)
    );
    r.apply(on);
    let settings = parse(&p.settings);
    assert_eq!(own_groups(&settings).len(), 1);
    assert_eq!(
        settings["hooks"]["PreToolUse"][0]["hooks"][0]["command"],
        "my-guard"
    );
    r.assert_no_drift();
    assert!(r.plan(&all()).is_empty());

    let off = r.plan(&without_hook());
    assert_eq!(off.len(), 1);
    assert_eq!(off[0].action, Action::Remove);
    r.apply(off);
    assert_eq!(read(&p.settings), SETTINGS);
    assert!(r.plan(&without_hook()).is_empty());
}

#[test]
fn an_old_hook_group_is_replaced_and_user_groups_are_kept() {
    let r = rig();
    let p = r.paths();
    let old = hook_group("C:/old/mmhmem.exe");
    let mut seeded: Value = serde_json::from_str(SETTINGS).unwrap();
    seeded["hooks"]["PreToolUse"]
        .as_array_mut()
        .unwrap()
        .push(old);
    r.home
        .write(&[".claude", "settings.json"], &seeded.to_string());
    let changes = r.plan(&all());
    let hook = changes.iter().find(|c| c.file == p.settings).unwrap();
    assert_eq!(hook.action, Action::Update);
    let after: Value = serde_json::from_slice(hook.after.as_deref().unwrap()).unwrap();
    assert_eq!(own_groups(&after), vec![hook_group(&r.agent.binary_text())]);
    assert_eq!(after["hooks"]["PreToolUse"].as_array().unwrap().len(), 2);
    assert_eq!(
        after["hooks"]["PreToolUse"][0],
        seeded["hooks"]["PreToolUse"][0]
    );
}

#[test]
fn removal_restores_seeded_files_byte_identical() {
    let mut r = rig();
    let p = r.paths();
    r.home.write(&[".claude.json"], CLAUDE_JSON);
    r.home.write(&[".claude", "settings.json"], SETTINGS);
    let changes = r.plan(&all());
    r.apply(changes);
    r.remove_all();
    assert_eq!(read(&p.claude_json), CLAUDE_JSON);
    assert_eq!(read(&p.settings), SETTINGS);
    assert!(!p.skill.exists() && !p.command.exists());
    // A Markdown section cannot delete its file, so the rules file stays empty.
    assert_eq!(read(&p.rules), "");
    assert!(r.rec().is_none_or(|rec| rec.managed.is_empty()));
}

#[test]
fn removal_prunes_only_the_parents_mmhmem_created() {
    let mut r = rig();
    let p = r.paths();
    r.home.write(&[".claude.json"], "{\"numStartups\": 1}\n");
    let changes = r.plan(&all());
    let hook = changes.iter().find(|c| c.file == p.settings).unwrap();
    assert_eq!(hook.missing_parents, Some(2));
    let mcp = changes.iter().find(|c| c.file == p.claude_json).unwrap();
    assert_eq!(mcp.missing_parents, Some(1));
    r.apply(changes);
    r.remove_all();
    // `edit::json` lays a one-line object out on several lines when it
    // appends to it, so only the value is restored here.
    assert_eq!(parse(&p.claude_json), json!({"numStartups": 1}));
    assert_eq!(parse(&p.settings), json!({}));
}

#[test]
fn a_user_key_inside_mmhmem_survives_replan_and_removal() {
    let mut r = rig();
    let p = r.paths();
    r.home.write(&[".claude.json"], CLAUDE_JSON);
    let changes = r.plan(&all());
    r.apply(changes);
    let path = ["mcpServers", "mmhmem", "env"];
    let edited = edit_json::set(&read(&p.claude_json), &path, &json!({"LOG": "debug"})).unwrap();
    std::fs::write(&p.claude_json, &edited).unwrap();
    r.assert_no_drift();
    assert!(r.plan(&all()).is_empty(), "the user key is not drift");

    // The user's key as written, from `"env"` to its closing brace.
    let at = edited.rfind("\"env\"").unwrap();
    let user_text = &edited[at..=at + edited[at..].find('}').unwrap()];
    r.remove_all();
    let left = read(&p.claude_json);
    assert!(
        left.contains(user_text),
        "the user key stays byte-identical"
    );
    assert!(left.contains(
        "	\"oauthAccount\": {\"emailAddress\": \"user@example.com\"},
"
    ));
    // Only mmhmem's keys go; the object stays while it holds the user's key.
    let after: Value = serde_json::from_str(&left).unwrap();
    assert_eq!(
        after["mcpServers"]["mmhmem"],
        json!({"env": {"LOG": "debug"}})
    );
    assert_eq!(after["mcpServers"]["other"]["command"], "other-server");
    let rec = r.rec().cloned().unwrap_or_default();
    assert!(
        !rec.managed
            .iter()
            .any(|e| matches!(e.target, Target::JsonEntry { .. }))
    );
}

#[test]
fn an_invalid_settings_file_skips_claude_code_without_echoing_it() {
    let r = rig();
    let p = r.paths();
    r.home.write(
        &[".claude", "settings.json"],
        "{\"apiKeyHelper\": \"sk-secret-123\",,",
    );
    let problem = r.agent.plan(&r.home.env, &all(), None).unwrap_err();
    assert!(matches!(&problem, AgentProblem::Json { file, .. } if *file == p.settings));
    let message = problem.to_string();
    assert!(
        message.contains("settings.json") && !message.contains("sk-secret"),
        "{message}"
    );
    // Without the hook the file is not touched, so Claude Code can be planned.
    assert!(!r.plan(&without_hook()).is_empty());
}

#[test]
fn an_invalid_claude_json_is_a_problem() {
    let r = rig();
    r.home.write(&[".claude.json"], "{\"mcpServers\": [");
    let problem = r.agent.plan(&r.home.env, &all(), None).unwrap_err();
    assert!(matches!(problem, AgentProblem::Json { file, .. } if file == r.paths().claude_json));
}

#[test]
fn removal_skips_a_file_that_no_longer_parses() {
    let mut r = rig();
    let p = r.paths();
    let changes = r.plan(&all());
    r.apply(changes);
    std::fs::write(&p.settings, "{ broken").unwrap();
    let rec = r.rec().cloned().unwrap();
    let removal = r.agent.plan_remove(&r.home.env, &rec);
    assert_eq!(removal.len(), 4);
    assert!(removal.iter().all(|c| c.file != p.settings));
    r.apply(removal);
    let left: Vec<PathBuf> = r
        .rec()
        .unwrap()
        .managed
        .iter()
        .filter_map(|e| target_file(&e.target).map(PathBuf::from))
        .collect();
    assert_eq!(left, vec![p.settings.clone()], "the entry stays recorded");
}

#[test]
fn ambiguous_routing_markers_are_a_problem() {
    let r = rig();
    let twice = format!("{SECTION_START}\n{SECTION_START}\n");
    r.home.write(&[".claude", "rules", "mmhmem.md"], &twice);
    let problem = r.agent.plan(&r.home.env, &all(), None).unwrap_err();
    assert_eq!(
        problem,
        AgentProblem::Markdown {
            file: r.paths().rules,
            reason: markdown::AMBIGUOUS_MARKERS
        }
    );
    assert_eq!(
        problem.to_string(),
        format!(
            "{}: {}",
            r.paths().rules.display(),
            markdown::AMBIGUOUS_MARKERS
        )
    );
}

const SECTION_START: &str = "<!-- mmhmem:routing:start -->";
