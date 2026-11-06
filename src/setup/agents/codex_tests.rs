//! Tests of the Codex integration (reliable-installers task 3.4): install,
//! re-plan, the old marker block, a duplicate table, removal, user keys, the
//! hook and invalid files. Temporary homes only.

use std::path::{Path, PathBuf};

use super::*;
use crate::setup::agents::doubles::{Reply, TestHome};
use crate::setup::edit::toml::{MARKER_BEGIN_PS1, MARKER_BEGIN_SH, MARKER_END};
use crate::setup::plan::{Outcome, Plan, apply};
use crate::setup::record::{Drift, InstallRecord, Probe, RecordStore, UtcTimestamp, detect_drift};

const HOOKS_SEED: &str = "{\n  \"hooks\": {\n    \"PreToolUse\": [\n      {\n        \"matcher\": \"Write\",\n        \"hooks\": [{ \"type\": \"command\", \"command\": \"echo hi\" }]\n      }\n    ]\n  }\n}\n";
const CONFIG_SEED: &str = "# my codex\r\nmodel = \"o3\"\r\n\r\n[profiles.fast]\r\nmodel = \"o4-mini\" # quick\r\n\r\n[mcp_servers.other]\r\ncommand = \"npx\"\r\nargs = [\"-y\", \"other\"]\r\n";

struct Fx {
    home: TestHome,
    store: RecordStore,
    record: InstallRecord,
    codex: Codex,
}

fn fx() -> Fx {
    let home = TestHome::new(Os::Linux);
    let bin = home.bin().join("mmhmem");
    let store = RecordStore::new(home.dir.path().join("state"));
    Fx {
        home,
        store,
        record: InstallRecord::new(),
        codex: Codex::new(bin),
    }
}

fn all() -> Selection {
    Selection::new().with(ID, &FEATURES)
}

fn only(features: &[Feature]) -> Selection {
    Selection::new().with(ID, features)
}

impl Fx {
    fn file(&self, rel: &[&str]) -> PathBuf {
        rel.iter()
            .fold(self.home.home().to_path_buf(), |p, s| p.join(s))
    }

    fn config(&self) -> PathBuf {
        self.file(&[".codex", "config.toml"])
    }

    fn hooks(&self) -> PathBuf {
        self.file(&[".codex", "hooks.json"])
    }

    fn plan(&self, want: &Selection) -> Result<Vec<Change>, AgentProblem> {
        self.codex
            .plan(&self.home.env, want, self.record.agents.get(&ID))
    }

    fn run(&mut self, changes: Vec<Change>) {
        let run = UtcTimestamp::try_from("2026-11-04T09:30:00Z".to_owned()).unwrap();
        let report = apply(&Plan { run, changes }, &self.store, &mut self.record);
        for agent in &report.agents {
            assert_eq!(agent.outcome, Outcome::Applied);
        }
    }

    fn install(&mut self, want: &Selection) {
        let changes = self.plan(want).expect("plan");
        self.run(changes);
    }

    fn remove_all(&mut self) {
        let rec = self.record.agents.get(&ID).cloned().unwrap_or_default();
        let changes = self.codex.plan_remove(&self.home.env, &rec);
        self.run(changes);
    }

    fn assert_no_drift(&self) {
        let reports = detect_drift(&self.record, &Probe::default());
        assert!(!reports.is_empty());
        for r in reports {
            assert_eq!(r.drift, Drift::Unchanged, "{:?}", r.target);
        }
    }
}

fn text(file: &Path) -> String {
    std::fs::read_to_string(file).expect("file")
}

fn seed(file: &Path, content: &str) {
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(file, content).unwrap();
}

fn marked(begin: &str) -> String {
    format!(
        "model = \"o3\"\n\n{begin}\n[mcp_servers.mmhmem]\ncommand = \"/old/mmhmem\"\nargs = [\"stdio\"]\n{MARKER_END}\n"
    )
}

#[test]
fn install_applies_with_no_drift_and_a_replan_has_zero_changes() {
    let mut fx = fx();
    fx.install(&all());
    fx.assert_no_drift();
    let rec = fx.record.agents.get(&ID).unwrap();
    assert_eq!(rec.managed.len(), 5);
    let config = text(&fx.config());
    assert!(config.contains("[mcp_servers.mmhmem]"));
    assert!(config.contains("args = [\"stdio\"]"));
    assert!(!config.contains("env"));
    assert!(
        fx.file(&[".agents", "skills", "unified-memory", "SKILL.md"])
            .is_file()
    );
    assert!(
        fx.file(&[".agents", "skills", "mmhmem-recap", "SKILL.md"])
            .is_file()
    );
    assert!(fx.file(&[".codex", "prompts", "mmhmem-recap.md"]).is_file());
    assert!(text(&fx.hooks()).contains("hook codex"));
    assert_eq!(fx.plan(&all()).unwrap(), Vec::new());
}

#[test]
fn replan_with_a_moved_binary_updates_only_command_and_hook() {
    let mut fx = fx();
    fx.install(&only(&[Feature::Mcp, Feature::ReadHook]));
    fx.codex = Codex::new(fx.home.bin().join("moved").join("mmhmem"));
    let changes = fx.plan(&only(&[Feature::Mcp, Feature::ReadHook])).unwrap();
    assert_eq!(changes.len(), 2);
    fx.run(changes);
    fx.assert_no_drift();
    assert!(text(&fx.config()).contains("moved"));
    assert_eq!(text(&fx.hooks()).matches("hook codex").count(), 1);
}

#[test]
fn old_marker_block_is_converted_and_then_tracked_for_both_variants() {
    for begin in [MARKER_BEGIN_PS1, MARKER_BEGIN_SH] {
        let mut fx = fx();
        seed(&fx.config(), &marked(begin));
        let changes = fx.plan(&only(&[Feature::Mcp])).unwrap();
        assert_eq!(changes.len(), 1, "one chained change in config.toml");
        fx.run(changes);
        let config = text(&fx.config());
        assert!(!config.contains("BEGIN mmhmem") && !config.contains("END mmhmem"));
        assert_eq!(config.matches("[mcp_servers.mmhmem]").count(), 1);
        assert!(config.starts_with("model = \"o3\"\n\n[mcp_servers.mmhmem]"));
        fx.assert_no_drift();
        assert_eq!(fx.plan(&only(&[Feature::Mcp])).unwrap(), Vec::new());
    }
}

#[test]
fn duplicate_table_skips_only_codex_and_touches_nothing() {
    let fx = fx();
    let dup = "[mcp_servers.mmhmem]\ncommand = \"a\"\n\n[mcp_servers.mmhmem]\ncommand = \"b\"\n";
    seed(&fx.config(), dup);
    let err = fx.plan(&all()).unwrap_err();
    assert!(matches!(
        err,
        AgentProblem::Toml {
            error: TomlEditError::DuplicateTable {
                is_mmhmem: true,
                ..
            },
            ..
        }
    ));
    assert_eq!(text(&fx.config()), dup);
    assert!(!fx.hooks().exists());
}

#[test]
fn removal_restores_a_seeded_crlf_config_byte_for_byte() {
    let mut fx = fx();
    seed(&fx.config(), CONFIG_SEED);
    fx.install(&only(&[Feature::Mcp]));
    assert!(text(&fx.config()).contains("[mcp_servers.other]"));
    assert_ne!(text(&fx.config()), CONFIG_SEED);
    fx.remove_all();
    assert_eq!(text(&fx.config()), CONFIG_SEED);
    assert!(
        fx.record
            .agents
            .get(&ID)
            .is_none_or(|r| r.managed.is_empty())
    );
}

#[test]
fn removal_takes_out_hook_and_files_and_keeps_user_hooks_identical() {
    let mut fx = fx();
    seed(&fx.hooks(), HOOKS_SEED);
    fx.install(&all());
    assert_eq!(text(&fx.hooks()).matches("\"matcher\"").count(), 2);
    fx.remove_all();
    assert_eq!(text(&fx.hooks()), HOOKS_SEED);
    assert!(
        !fx.file(&[".agents", "skills", "unified-memory", "SKILL.md"])
            .exists()
    );
    assert!(!fx.file(&[".codex", "prompts", "mmhmem-recap.md"]).exists());
    assert!(!text(&fx.config()).contains("mmhmem"));
}

#[test]
fn hook_created_in_a_new_file_is_pruned_on_removal() {
    let mut fx = fx();
    fx.install(&only(&[Feature::ReadHook]));
    let rec = fx.record.agents.get(&ID).unwrap();
    assert_eq!(rec.managed[0].missing_parents, Some(2));
    fx.remove_all();
    let left: serde_json::Value = serde_json::from_str(&text(&fx.hooks())).unwrap();
    assert_eq!(left, serde_json::json!({}));
}

#[test]
fn deselecting_the_hook_or_the_skills_removes_only_them() {
    let mut fx = fx();
    seed(&fx.hooks(), HOOKS_SEED);
    fx.install(&all());
    let want = only(&[Feature::Mcp, Feature::RecapCommand]);
    let changes = fx.plan(&want).unwrap();
    assert_eq!(changes.len(), 3, "hook and two skills");
    assert!(changes.iter().all(|c| c.action == Action::Remove));
    fx.run(changes);
    assert_eq!(text(&fx.hooks()), HOOKS_SEED);
    assert!(text(&fx.config()).contains("[mcp_servers.mmhmem]"));
    fx.assert_no_drift();
    assert_eq!(fx.plan(&want).unwrap(), Vec::new());
}

#[test]
fn user_keys_inside_our_table_survive_replan_and_are_not_drift() {
    let mut fx = fx();
    fx.install(&only(&[Feature::Mcp]));
    let with_user = text(&fx.config()).replace(
        "args = [\"stdio\"]\n",
        "args = [\"stdio\"]\ntool_timeout_sec = 60\n",
    );
    assert!(with_user.contains("tool_timeout_sec"));
    std::fs::write(fx.config(), &with_user).unwrap();
    fx.assert_no_drift();
    assert_eq!(fx.plan(&only(&[Feature::Mcp])).unwrap(), Vec::new());
    fx.codex = Codex::new(fx.home.bin().join("moved").join("mmhmem"));
    fx.install(&only(&[Feature::Mcp]));
    let config = text(&fx.config());
    assert!(config.contains("tool_timeout_sec = 60") && config.contains("moved"));
}

#[test]
fn removal_with_user_keys_removes_our_command_and_keeps_other_content() {
    let mut fx = fx();
    seed(&fx.config(), CONFIG_SEED);
    fx.install(&only(&[Feature::Mcp]));
    let edited = text(&fx.config()).replace(
        "args = [\"stdio\"]\r\n",
        "args = [\"stdio\"]\r\ntool_timeout_sec = 60\r\n",
    );
    std::fs::write(fx.config(), &edited).unwrap();
    let ours = |l: &&str| {
        (l.starts_with("command = ") && !l.contains("npx")) || *l == "args = [\"stdio\"]"
    };
    let expected: String = edited
        .split_inclusive("\r\n")
        .filter(|l| !ours(&l.trim_end()))
        .collect();
    fx.remove_all();
    let config = text(&fx.config());
    assert_eq!(config, expected, "only mmhmem's keys leave the table");
    assert!(config.contains("[mcp_servers.mmhmem]\r\ntool_timeout_sec = 60\r\n"));
    assert!(config.starts_with(CONFIG_SEED));
    let rec = fx.record.agents.get(&ID).cloned().unwrap_or_default();
    assert!(
        !rec.managed
            .iter()
            .any(|e| matches!(e.target, Target::TomlTable { .. }))
    );
}

#[test]
fn plan_remove_without_a_record_changes_nothing() {
    let fx = fx();
    seed(&fx.config(), &marked(MARKER_BEGIN_SH));
    let rec = AgentRecord::default();
    assert!(fx.codex.plan_remove(&fx.home.env, &rec).is_empty());
}

#[test]
fn invalid_files_skip_codex_without_echoing_their_content() {
    let fx = fx();
    seed(&fx.config(), "token = \"sk-secret-1\"\n[broken\n");
    let err = fx.plan(&only(&[Feature::Mcp])).unwrap_err();
    assert!(matches!(err, AgentProblem::Toml { .. }));
    assert!(!err.to_string().contains("sk-secret"));
    assert!(!format!("{err:?}").contains("sk-secret"));
    std::fs::remove_file(fx.config()).unwrap();
    seed(&fx.hooks(), "{ \"token\": \"sk-secret-2\", ");
    let err = fx.plan(&only(&[Feature::ReadHook])).unwrap_err();
    assert!(matches!(err, AgentProblem::Json { .. }));
    assert!(!err.to_string().contains("sk-secret"));
    assert!(!format!("{err:?}").contains("sk-secret"));
}

#[test]
fn hook_shape_matches_the_shipped_entry_and_the_record_matcher() {
    let bin = Path::new("/opt/my tools/mmhmem");
    assert_eq!(
        hook_command(bin, Os::Linux).unwrap(),
        "'/opt/my tools/mmhmem' hook codex"
    );
    assert_eq!(
        hook_command(Path::new("/usr/bin/mmhmem"), Os::Linux).unwrap(),
        "/usr/bin/mmhmem hook codex"
    );
    let win = Path::new("C:\\Users\\me\\bin\\mmhmem.exe");
    assert_eq!(
        hook_command(win, Os::Windows).unwrap(),
        "& 'C:\\Users\\me\\bin\\mmhmem.exe' hook codex"
    );
    for os in [Os::Linux, Os::Windows] {
        let codex = Codex::new(if os == Os::Windows { win } else { bin });
        let group = codex.hook_group(os).unwrap();
        assert_eq!(group["matcher"], "^(Bash|shell)$");
        assert_eq!(group["hooks"][0]["type"], "command");
        assert_eq!(group["hooks"][0]["timeout"], 10);
        assert!(OWN_HOOK.matches(&group));
    }
}

#[test]
fn hook_selected_exposes_the_approval_step() {
    assert_eq!(
        Codex::pending_steps(&all()),
        vec![PendingUserStep::ApproveHookInCodex]
    );
    assert!(Codex::pending_steps(&only(&[Feature::Mcp])).is_empty());
}

#[test]
fn detect_uses_path_or_codex_home_and_tolerates_a_failing_version() {
    let fx = fx();
    assert!(!fx.codex.detect(&fx.home.env).is_found());
    std::fs::create_dir_all(fx.home.home().join(".codex")).unwrap();
    assert_eq!(
        fx.codex.detect(&fx.home.env),
        Detection::Found {
            version: None,
            paths: vec![fx.home.home().join(".codex")]
        }
    );
    let program = fx.home.install_program("codex");
    fx.home
        .runner
        .on("codex", &["--version"], vec![Reply::exit(1, "", "boom")]);
    let found = fx.codex.detect(&fx.home.env);
    assert!(matches!(&found, Detection::Found { version: None, paths } if paths[0] == program));
    fx.home.runner.on(
        "codex",
        &["--version"],
        vec![Reply::ok("codex-cli 0.166.0\n")],
    );
    assert!(matches!(
        fx.codex.detect(&fx.home.env),
        Detection::Found { version: Some(v), .. } if v == "0.166.0"
    ));
    assert_eq!(
        fx.home.runner.args_of("codex").last().unwrap(),
        &["--version"]
    );
}
