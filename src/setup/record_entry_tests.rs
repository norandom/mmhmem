//! Tests for managed-key entries and shared-array elements in
//! `setup::record` (task 2.4 follow-up): user keys inside mmhmem's entry and
//! the user's own hook groups are not drift.

use std::path::Path;

use super::drift_tests::{only_drift, present, record_for};
use super::*;

const OPENCODE: &str = "{\n  \"mcp\": {\n    \"servers\": {\n      \"mmhmem\": { \"type\": \"local\", \"command\": [\"mmhmem\", \"stdio\"] }\n    }\n  }\n}\n";

fn opencode_target(file: &Path) -> Target {
    Target::JsonEntry {
        file: file.to_path_buf(),
        pointer: key_path(&["mcp", "servers", "mmhmem"]).unwrap(),
        managed_keys: Some(key_path(&["type", "command"]).unwrap()),
    }
}

#[test]
fn json_user_key_inside_entry_is_unchanged_and_managed_key_is_changed() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("opencode.json");
    std::fs::write(&file, OPENCODE).unwrap();
    let record = record_for(Scope::Agent(AgentId::OpenCode), &opencode_target(&file));
    let user = OPENCODE.replace(
        "\"type\": \"local\",",
        "\"type\": \"local\", \"enabled\": false,",
    );
    std::fs::write(&file, user).unwrap();
    assert_eq!(only_drift(&record), Drift::Unchanged);
    std::fs::write(&file, OPENCODE.replace("\"stdio\"", "\"serve\"")).unwrap();
    assert!(matches!(only_drift(&record), Drift::Changed { .. }));
    std::fs::write(&file, OPENCODE.replace("\"type\": \"local\", ", "")).unwrap();
    assert!(
        matches!(only_drift(&record), Drift::Changed { .. }),
        "a managed key that disappears is a change"
    );
}

#[test]
fn json_entry_without_managed_keys_hashes_the_whole_value() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("opencode.json");
    std::fs::write(&file, OPENCODE).unwrap();
    let Target::JsonEntry {
        file: f, pointer, ..
    } = opencode_target(&file)
    else {
        unreachable!()
    };
    let whole = Target::JsonEntry {
        file: f,
        pointer,
        managed_keys: None,
    };
    let record = record_for(Scope::Agent(AgentId::OpenCode), &whole);
    let user = OPENCODE.replace(
        "\"type\": \"local\",",
        "\"type\": \"local\", \"enabled\": false,",
    );
    std::fs::write(&file, user).unwrap();
    assert!(matches!(only_drift(&record), Drift::Changed { .. }));
}

#[test]
fn toml_user_key_inside_table_is_unchanged_and_managed_key_is_changed() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("config.toml");
    let v1 = "[mcp_servers.mmhmem]\ncommand = \"mmhmem\"\nargs = [\"stdio\"]\n";
    std::fs::write(&file, v1).unwrap();
    let target = Target::TomlTable {
        file: file.clone(),
        key: key_path(&["mcp_servers", "mmhmem"]).unwrap(),
        managed_keys: Some(key_path(&["command", "args"]).unwrap()),
    };
    let record = record_for(Scope::Agent(AgentId::Codex), &target);
    std::fs::write(&file, format!("{v1}tool_timeout_sec = 60\n")).unwrap();
    assert_eq!(only_drift(&record), Drift::Unchanged);
    std::fs::write(&file, v1.replace("\"mmhmem\"", "\"other\"")).unwrap();
    assert!(matches!(only_drift(&record), Drift::Changed { .. }));
}

#[test]
fn an_entry_left_with_only_user_keys_is_missing() {
    let json = opencode_target(Path::new("/h/opencode.json"));
    let user_only = "{\"mcp\": {\"servers\": {\"mmhmem\": {\"enabled\": false}}}}";
    assert_eq!(
        observe_in(&json, user_only.as_bytes()),
        Observation::Missing
    );
    let empty = "{\"mcp\": {\"servers\": {\"mmhmem\": {}}}}";
    assert_eq!(observe_in(&json, empty.as_bytes()), Observation::Missing);
    let one_left = "{\"mcp\": {\"servers\": {\"mmhmem\": {\"type\": \"local\", \"x\": 1}}}}";
    assert!(matches!(
        observe_in(&json, one_left.as_bytes()),
        Observation::Present(_)
    ));
    let toml = Target::TomlTable {
        file: Path::new("/h/config.toml").to_path_buf(),
        key: key_path(&["mcp_servers", "mmhmem"]).unwrap(),
        managed_keys: Some(key_path(&["command", "args"]).unwrap()),
    };
    let user_only = "[mcp_servers.mmhmem]\ntool_timeout_sec = 60\n";
    assert_eq!(
        observe_in(&toml, user_only.as_bytes()),
        Observation::Missing
    );
    let whole = Target::JsonEntry {
        file: Path::new("/h/opencode.json").to_path_buf(),
        pointer: key_path(&["mcp", "servers", "mmhmem"]).unwrap(),
        managed_keys: None,
    };
    assert!(matches!(
        observe_in(&whole, empty.as_bytes()),
        Observation::Present(_)
    ));
}

const OURS: &str = r#"{ "matcher": "Read", "hooks": [ { "type": "command", "command": "C:\\bin\\mmhmem.exe", "args": ["hook", "claude"], "timeout": 5 } ] }"#;

fn settings(user_a: &str, ours: &[&str]) -> String {
    let user_b =
        r#"{ "matcher": "Bash", "hooks": [ { "type": "command", "command": "lint.sh" } ] }"#;
    let mut groups = vec![user_a.to_owned()];
    groups.extend(ours.iter().map(|s| (*s).to_owned()));
    groups.push(user_b.to_owned());
    format!(
        "{{\n  \"theme\": \"dark\",\n  \"hooks\": {{\n    \"PreToolUse\": [\n      {}\n    ]\n  }}\n}}\n",
        groups.join(",\n      ")
    )
}

const USER_A: &str =
    r#"{ "matcher": "Edit", "hooks": [ { "type": "command", "command": "fmt.sh" } ] }"#;

fn hook_target(file: &Path) -> Target {
    Target::JsonArrayElement {
        file: file.to_path_buf(),
        pointer: key_path(&["hooks", "PreToolUse"]).unwrap(),
        matcher: ElementMatcher::MmhmemHook(HookHarness::Claude),
    }
}

#[test]
fn hook_group_drift_ignores_user_groups() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("settings.json");
    std::fs::write(&file, settings(USER_A, &[OURS])).unwrap();
    let record = record_for(Scope::Agent(AgentId::ClaudeCode), &hook_target(&file));
    let edited_user = USER_A.replace("fmt.sh", "prettier --write");
    std::fs::write(
        &file,
        settings(&edited_user, &[OURS]).replace("dark", "light"),
    )
    .unwrap();
    assert_eq!(only_drift(&record), Drift::Unchanged);
    std::fs::write(
        &file,
        settings(USER_A, &[&OURS.replace("\"timeout\": 5", "\"timeout\": 9")]),
    )
    .unwrap();
    assert!(matches!(only_drift(&record), Drift::Changed { .. }));
    std::fs::write(&file, settings(USER_A, &[])).unwrap();
    assert_eq!(only_drift(&record), Drift::Missing);
    // A duplicate is a change outside setup (an install collapses it), not
    // an unreadable file: the plan's check must still accept the file.
    std::fs::write(&file, settings(USER_A, &[OURS, OURS])).unwrap();
    assert!(matches!(only_drift(&record), Drift::Changed { .. }));
    std::fs::write(&file, "{\"theme\": \"dark\"}").unwrap();
    assert_eq!(only_drift(&record), Drift::Missing);
}

#[test]
fn hook_matcher_recognises_both_handler_forms_only() {
    let m = |harness, handler: &str| {
        let group: serde_json::Value =
            serde_json::from_str(&format!("{{\"hooks\": [{handler}]}}")).unwrap();
        ElementMatcher::MmhmemHook(harness).matches(&group)
    };
    use HookHarness::{Claude, Codex};
    assert!(m(
        Claude,
        r#"{"command": "/usr/local/bin/mmhmem", "args": ["hook", "claude"]}"#
    ));
    assert!(!m(
        Codex,
        r#"{"command": "/usr/local/bin/mmhmem", "args": ["hook", "claude"]}"#
    ));
    assert!(!m(
        Claude,
        r#"{"command": "/bin/notmmhmem", "args": ["hook", "claude"]}"#
    ));
    assert!(!m(
        Claude,
        r#"{"command": "/bin/mmhmem", "args": ["hook", "claude", "-x"]}"#
    ));
    assert!(m(
        Codex,
        r#"{"command": "& 'C:\\Program Files\\mmhmem\\mmhmem.exe' hook codex"}"#
    ));
    assert!(m(Codex, r#"{"command": "\"/opt/m m/mmhmem\" hook codex"}"#));
    assert!(m(Codex, r#"{"command": "/opt/mmhmem hook codex"}"#));
    assert!(!m(
        Codex,
        r#"{"command": "/opt/mmhmem hook codex; rm -rf /"}"#
    ));
    assert!(!m(Codex, r#"{"command": "& 'C:\\mmhmem.exe hook codex"}"#));
    assert!(!m(Claude, r#"{"type": "command"}"#));
}

#[test]
fn array_element_after_hash_comes_from_planned_bytes() {
    let target = hook_target(Path::new("/h/settings.json"));
    let planned = settings(USER_A, &[OURS]);
    let element: serde_json::Value = serde_json::from_str(OURS).unwrap();
    let want = Sha256Hex::of(normalize_json(&element).as_bytes());
    assert_eq!(present(observe_in(&target, planned.as_bytes())), want);
    let text = serde_json::to_string(&target).unwrap();
    assert!(
        text.contains("\"matcher\":{\"mmhmem_hook\":\"claude\"}"),
        "{text}"
    );
}
