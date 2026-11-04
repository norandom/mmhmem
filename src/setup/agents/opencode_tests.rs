//! Tests of the OpenCode integration (reliable-installers task 3.5). Temp
//! homes only; the real OpenCode folders are never read.

use std::path::PathBuf;

use super::*;
use crate::setup::agents::doubles::TestHome;
use crate::setup::agents::doubles_cli;
use crate::setup::plan::{Outcome, Plan, apply};
use crate::setup::record::{
    Drift, InstallRecord, Os, Probe, RecordStore, UtcTimestamp, detect_drift,
};

const JSON_SEED: &str = "{\n  \"theme\": \"dark\",\n  \"plugin\": [\"a-plugin\"],\n  \"permission\": {\"bash\": \"ask\"},\n  \"mcp\": {\n    \"servers\": {\n      \"other\": {\"type\": \"local\", \"command\": [\"x\"]}\n    }\n  }\n}\n";
const JSONC_SEED: &str = "// my opencode settings\n{\n  /* theme */\n  \"theme\": \"dark\", // keep\n  \"plugin\": [\"a-plugin\",],\n}\n";
const USER_MD: &str = "# My rules\n\nBe kind.\n";

struct Fx {
    home: TestHome,
    store: RecordStore,
    record: InstallRecord,
    agent: OpenCode,
}

fn fx() -> Fx {
    let home = TestHome::new(Os::Windows);
    let store = RecordStore::new(home.dir.path().join("state"));
    let agent = OpenCode::new(home.bin().join("mmhmem.exe"));
    Fx {
        home,
        store,
        record: InstallRecord::new(),
        agent,
    }
}

fn all() -> Selection {
    Selection::new().with(AgentId::OpenCode, FEATURES)
}

fn mcp_only() -> Selection {
    Selection::new().with(AgentId::OpenCode, &[Feature::Mcp])
}

impl Fx {
    fn dir(&self) -> PathBuf {
        self.home.home().join(".config").join("opencode")
    }

    fn put(&self, name: &str, text: &str) -> PathBuf {
        self.home.write(&[".config", "opencode", name], text)
    }

    fn text(&self, name: &str) -> String {
        std::fs::read_to_string(self.dir().join(name)).unwrap()
    }

    fn plan(&self, want: &Selection) -> Result<Vec<Change>, AgentProblem> {
        let rec = self.record.agents.get(&AgentId::OpenCode);
        self.agent.plan(&self.home.env, want, rec)
    }

    fn install(&mut self, want: &Selection) -> Vec<Change> {
        let changes = self.plan(want).expect("plans");
        self.apply(changes.clone());
        changes
    }

    fn apply(&mut self, changes: Vec<Change>) {
        let run = UtcTimestamp::try_from("2026-11-04T09:30:00Z".to_owned()).unwrap();
        let plan = Plan { run, changes };
        let report = apply(&plan, &self.store, &mut self.record);
        assert!(
            report.agents.iter().all(|a| a.outcome == Outcome::Applied),
            "{:?}",
            report.agents
        );
    }

    fn remove_all(&mut self) {
        let rec = self.record.agents.get(&AgentId::OpenCode).unwrap().clone();
        let changes = self.agent.plan_remove(&self.home.env, &rec);
        self.apply(changes);
    }

    fn assert_no_drift(&self) {
        let reports = detect_drift(&self.record, &Probe::default());
        assert!(!reports.is_empty());
        assert!(reports.iter().all(|r| r.drift == Drift::Unchanged));
    }
}

fn files_of(changes: &[Change]) -> Vec<String> {
    changes
        .iter()
        .map(|c| c.file.file_name().unwrap().to_string_lossy().into_owned())
        .collect()
}

#[test]
fn json_only_gets_the_entry_in_opencode_json() {
    let mut fx = fx();
    fx.put("opencode.json", JSON_SEED);
    let changes = fx.install(&mcp_only());
    assert_eq!(files_of(&changes), ["opencode.json"]);
    let value: Value = serde_json::from_str(&fx.text("opencode.json")).unwrap();
    let entry = &value["mcp"]["servers"]["mmhmem"];
    assert_eq!(entry["type"], "local");
    let bin = fx.agent.binary.to_string_lossy().into_owned();
    assert_eq!(entry["command"], json!([bin, "stdio"]));
    assert_eq!(value["mcp"]["servers"]["other"]["command"], json!(["x"]));
    assert!(!fx.dir().join("opencode.jsonc").exists());
    fx.assert_no_drift();
}

#[test]
fn jsonc_only_keeps_comments_and_trailing_commas() {
    let mut fx = fx();
    fx.put("opencode.jsonc", JSONC_SEED);
    let changes = fx.install(&mcp_only());
    assert_eq!(files_of(&changes), ["opencode.jsonc"]);
    let text = fx.text("opencode.jsonc");
    for kept in [
        "// my opencode settings",
        "/* theme */",
        "// keep",
        "[\"a-plugin\",]",
    ] {
        assert!(text.contains(kept), "{kept} lost in {text}");
    }
    assert!(text.contains("\"mmhmem\""));
    assert!(!fx.dir().join("opencode.json").exists());
    fx.assert_no_drift();
}

#[test]
fn both_files_edit_the_one_holding_the_entry_else_the_jsonc() {
    let mut fx = fx();
    fx.put("opencode.json", JSON_SEED);
    fx.put("opencode.jsonc", JSONC_SEED);
    let first = fx.plan(&mcp_only()).unwrap();
    assert_eq!(files_of(&first), ["opencode.jsonc"], "no entry yet: jsonc");
    let old =
        "{\"mcp\":{\"servers\":{\"mmhmem\":{\"type\":\"local\",\"command\":[\"old\",\"stdio\"]}}}}";
    fx.put("opencode.json", old);
    let second = fx.plan(&mcp_only()).unwrap();
    assert_eq!(files_of(&second), ["opencode.json"], "entry holder wins");
    fx.apply(second);
    assert_eq!(fx.text("opencode.jsonc"), JSONC_SEED);
}

#[test]
fn an_entry_in_both_files_is_a_duplicate_problem() {
    let fx = fx();
    let entry =
        "{\"mcp\":{\"servers\":{\"mmhmem\":{\"type\":\"local\",\"command\":[\"m\",\"stdio\"]}}}}";
    fx.put("opencode.json", entry);
    fx.put("opencode.jsonc", entry);
    match fx.plan(&all()) {
        Err(AgentProblem::DuplicateEntry { files }) => assert_eq!(files.len(), 2),
        other => panic!("expected a duplicate problem, got {other:?}"),
    }
}

#[test]
fn a_new_install_creates_json_and_replans_with_no_changes() {
    let mut fx = fx();
    fx.install(&all());
    assert!(fx.dir().join("opencode.json").exists());
    assert!(fx.dir().join("skills/unified-memory/SKILL.md").exists());
    assert!(fx.dir().join("commands/mmhmem-recap.md").exists());
    assert!(fx.dir().join("plugins/mmhmem-read-routing.js").exists());
    assert!(fx.text("AGENTS.md").contains("mmhmem:routing:start"));
    let plugin = fx.text("plugins/mmhmem-read-routing.js");
    assert!(plugin.starts_with(PLUGIN_HEADER));
    assert!(!plugin.contains(BINARY_PLACEHOLDER));
    fx.assert_no_drift();
    assert!(fx.plan(&all()).unwrap().is_empty());
}

#[test]
fn removal_leaves_seeded_configs_byte_identical() {
    for (name, seed) in [("opencode.json", JSON_SEED), ("opencode.jsonc", JSONC_SEED)] {
        let mut fx = fx();
        fx.put(name, seed);
        fx.put("AGENTS.md", USER_MD);
        fx.install(&all());
        assert_ne!(fx.text(name), seed);
        fx.remove_all();
        assert_eq!(fx.text(name), seed, "{name}");
        assert_eq!(fx.text("AGENTS.md"), USER_MD);
        assert!(!fx.dir().join("plugins/mmhmem-read-routing.js").exists());
        assert!(!fx.dir().join("commands/mmhmem-recap.md").exists());
        assert!(!fx.dir().join("skills/unified-memory/SKILL.md").exists());
    }
}

#[test]
fn removal_of_a_fresh_install_prunes_only_what_mmhmem_created() {
    let mut fx = fx();
    fx.put("opencode.json", "{\"mcp\": {\"keep\": 1}}\n");
    fx.install(&mcp_only());
    fx.remove_all();
    let value: Value = serde_json::from_str(&fx.text("opencode.json")).unwrap();
    assert_eq!(value, json!({"mcp": {"keep": 1}}));
}

#[test]
fn an_agents_file_mmhmem_created_goes_unless_the_user_wrote_in_it() {
    let mut fx = fx();
    fx.install(&all());
    fx.remove_all();
    assert!(!fx.dir().join("AGENTS.md").exists());

    let mut fx = self::fx();
    fx.install(&all());
    let installed = fx.text("AGENTS.md");
    fx.put("AGENTS.md", &format!("{installed}\nMy note.\n"));
    fx.remove_all();
    assert_eq!(fx.text("AGENTS.md"), "\nMy note.\n");
}

#[test]
fn user_keys_inside_our_entry_are_kept() {
    let mut fx = fx();
    let seed = "{\"mcp\":{\"servers\":{\"mmhmem\":{\"type\":\"local\",\"command\":[\"old\",\"stdio\"],\"enabled\":false,\"environment\":{\"A\":\"b\"}}}}}";
    fx.put("opencode.json", seed);
    fx.install(&mcp_only());
    let value: Value = serde_json::from_str(&fx.text("opencode.json")).unwrap();
    let entry = &value["mcp"]["servers"]["mmhmem"];
    assert_eq!(entry["enabled"], false);
    assert_eq!(entry["environment"], json!({"A": "b"}));
    assert_ne!(entry["command"][0], "old");
    // Only mmhmem's keys go; the user's keys stay byte-identical.
    fx.remove_all();
    let left = fx.text("opencode.json");
    let user = "\"enabled\":false,\"environment\":{\"A\":\"b\"}";
    assert!(left.contains(user), "{left}");
    let value: Value = serde_json::from_str(&left).unwrap();
    let entry = &value["mcp"]["servers"]["mmhmem"];
    assert_eq!(entry, &json!({"enabled": false, "environment": {"A": "b"}}));
    let rec = fx.record.agents.get(&AgentId::OpenCode).cloned();
    assert!(rec.unwrap_or_default().managed.is_empty());
}

#[test]
fn an_old_installer_entry_is_kept_and_tracked_not_rewritten() {
    let mut fx = fx();
    let bin = fx.agent.binary.to_string_lossy().into_owned();
    let seed =
        json!({"mcp": {"servers": {"mmhmem": {"type": "local", "command": [bin, "stdio"]}}}});
    let text = serde_json::to_string(&seed).unwrap();
    fx.put("opencode.json", &text);
    let changes = fx.install(&mcp_only());
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].before, changes[0].after);
    assert_eq!(fx.text("opencode.json"), text);
    assert!(fx.plan(&mcp_only()).unwrap().is_empty());
}

#[test]
fn an_old_command_folder_copy_is_adopted_not_duplicated() {
    let mut fx = fx();
    let recap = assets::OPENCODE_RECAP_COMMAND.text;
    fx.put("command/mmhmem-recap.md", recap);
    let want = Selection::new().with(AgentId::OpenCode, &[Feature::RecapCommand]);
    let changes = fx.install(&want);
    assert_eq!(changes.len(), 1);
    assert!(
        changes[0].file.ends_with("command/mmhmem-recap.md")
            || changes[0].file.ends_with("command\\mmhmem-recap.md")
    );
    assert!(changes[0].summary.starts_with("Keep"));
    assert!(!fx.dir().join("commands").exists());
    assert!(fx.plan(&want).unwrap().is_empty());
}

#[test]
fn an_invalid_config_skips_only_opencode_and_echoes_nothing() {
    let fx = fx();
    fx.put("opencode.jsonc", "{ \"secret\": \"hunter2\", oops");
    let problem = fx.plan(&all()).unwrap_err();
    assert!(matches!(problem, AgentProblem::Json { .. }));
    assert!(!problem.to_string().contains("hunter2"));
    assert!(!fx.dir().join("opencode.json").exists());
    assert!(!fx.dir().join("skills").exists());
}

#[test]
fn a_plugin_without_our_header_is_left_alone() {
    let mut fx = fx();
    let mine = "// my own plugin\nexport default {}\n";
    let file = fx.put("plugins/mmhmem-read-routing.js", mine);
    let want = Selection::new().with(AgentId::OpenCode, &[Feature::ReadHook]);
    assert!(fx.plan(&want).unwrap().is_empty());
    // Even a recorded entry never takes out a file without the header.
    fx.install(&all());
    assert_eq!(std::fs::read_to_string(&file).unwrap(), mine);
    let other = "// someone else's now\n";
    std::fs::write(&file, other).unwrap();
    fx.remove_all();
    assert_eq!(std::fs::read_to_string(&file).unwrap(), other);
}

#[test]
fn deselecting_a_feature_removes_only_its_entry() {
    let mut fx = fx();
    fx.install(&all());
    let changes = fx.plan(&mcp_only()).unwrap();
    assert_eq!(changes.len(), 4);
    assert!(changes.iter().all(|c| c.action == Action::Remove));
    fx.apply(changes);
    assert!(fx.dir().join("opencode.json").exists());
    assert!(!fx.dir().join("plugins/mmhmem-read-routing.js").exists());
    assert!(fx.plan(&mcp_only()).unwrap().is_empty());
}

#[test]
fn detect_by_program_by_folder_and_not_at_all() {
    let fx = fx();
    let none = fx.agent.detect(&fx.home.env);
    assert!(!none.is_found());
    fx.put("anything.json", "{}");
    match fx.agent.detect(&fx.home.env) {
        Detection::Found { version, paths } => {
            assert_eq!(version, None);
            assert_eq!(paths, [fx.dir()]);
        }
        other => panic!("{other:?}"),
    }
    let fresh = self::fx();
    let program = fresh.home.install_program("opencode");
    doubles_cli::opencode(&fresh.home.runner, &fresh.home.env);
    match fresh.agent.detect(&fresh.home.env) {
        Detection::Found { version, paths } => {
            assert_eq!(version.as_deref(), Some("2.2.5"));
            assert_eq!(paths, [program]);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_failing_version_call_does_not_stop_detection() {
    let fx = fx();
    fx.home.install_program("opencode");
    match fx.agent.detect(&fx.home.env) {
        Detection::Found { version, .. } => assert_eq!(version, None),
        other => panic!("{other:?}"),
    }
}
