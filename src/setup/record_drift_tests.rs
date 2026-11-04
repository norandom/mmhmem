//! Tests for the drift and recovery half of `setup::record` (task 2.4):
//! entry-level hashes, unrelated edits, missing files and entries, the
//! recovery scan after a crash at each journal step, and the no-secrets
//! guarantee.

use std::path::{Path, PathBuf};

use serde_json::Value;

use super::tests::{json_target, sample_record};
use super::*;

const JSON_V1: &str = "{\n  // user comment\n  \"theme\": \"dark\",\n  \"mcpServers\": {\n    \"mmhmem\": { \"command\": \"mmhmem\", \"args\": [\"stdio\"] }\n  }\n}\n";
const TOML_V1: &str =
    "model = \"o3\"\n\n[mcp_servers.mmhmem]\ncommand = \"mmhmem\"\nargs = [\"stdio\"]\n";

pub(super) fn present(obs: Observation) -> Sha256Hex {
    match obs {
        Observation::Present(h) => h,
        other => panic!("expected a present entry, got {other:?}"),
    }
}

/// A record whose single `done` entry is `target` as it is on disk now.
pub(super) fn record_for(scope: Scope, target: &Target) -> InstallRecord {
    let mut record = InstallRecord::new();
    let after = present(observe(target, &Probe::default()));
    record.begin_change(
        scope,
        PendingChange {
            target: target.clone(),
            before: None,
            after: Some(after),
            backup: None,
            missing_parents: Some(0),
        },
    );
    record.finish_change(scope, target).unwrap();
    record
}

pub(super) fn only_drift(record: &InstallRecord) -> Drift {
    let reports = detect_drift(record, &Probe::default());
    assert_eq!(reports.len(), 1);
    reports.into_iter().next().unwrap().drift
}

fn toml_target(file: &Path) -> Target {
    Target::TomlTable {
        file: file.to_path_buf(),
        key: key_path(&["mcp_servers", "mmhmem"]).unwrap(),
        managed_keys: None,
    }
}

#[test]
fn json_unrelated_edit_is_unchanged_and_entry_edit_is_changed() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("settings.json");
    std::fs::write(&file, JSON_V1).unwrap();
    let target = json_target(&file);
    let record = record_for(Scope::Agent(AgentId::ClaudeDesktop), &target);
    let unrelated = JSON_V1
        .replace("\"dark\"", "\"light\", \"extra\": [1, 2]")
        .replace(
            "\"command\": \"mmhmem\", \"args\": [\"stdio\"]",
            "\"args\":[\"stdio\"],\"command\":\"mmhmem\"",
        )
        .replace('\n', "\r\n");
    std::fs::write(&file, unrelated).unwrap();
    assert_eq!(only_drift(&record), Drift::Unchanged);
    std::fs::write(&file, JSON_V1.replace("\"stdio\"", "\"serve\"")).unwrap();
    let Drift::Changed { expected, actual } = only_drift(&record) else {
        panic!("expected drift");
    };
    assert_eq!(Some(expected), record.done()[0].1.sha256.clone());
    assert_ne!(actual, record.done()[0].1.sha256.clone().unwrap());
}

#[test]
fn toml_unrelated_edit_is_unchanged_and_entry_edit_is_changed() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("config.toml");
    std::fs::write(&file, TOML_V1).unwrap();
    let target = toml_target(&file);
    let record = record_for(Scope::Agent(AgentId::Codex), &target);
    let unrelated = "# mine\nmodel = \"gpt-5\"\n[mcp_servers.mmhmem]\nargs = [ \"stdio\" ] # c\ncommand = \"mmhmem\"\n\n[mcp_servers.other]\ncommand = \"x\"\n";
    std::fs::write(&file, unrelated).unwrap();
    assert_eq!(only_drift(&record), Drift::Unchanged);
    let inline =
        "model = \"o3\"\nmcp_servers.mmhmem = { command = \"mmhmem\", args = [\"stdio\"] }\n";
    std::fs::write(&file, inline).unwrap();
    assert_eq!(only_drift(&record), Drift::Unchanged, "layout is not drift");
    std::fs::write(&file, TOML_V1.replace("[\"stdio\"]", "[\"stdio\", \"-v\"]")).unwrap();
    assert!(matches!(only_drift(&record), Drift::Changed { .. }));
}

#[test]
fn toml_types_are_not_confused() {
    let parse = |text: &str| {
        normalize_json(
            &crate::setup::edit::toml::get_item_json(text, &["t"])
                .unwrap()
                .unwrap(),
        )
    };
    assert_ne!(parse("[t]\na = 1\n"), parse("[t]\na = 1.0\n"));
    assert_ne!(
        parse("[t]\na = \"1979-05-27\"\n"),
        parse("[t]\na = 1979-05-27\n")
    );
    assert_eq!(parse("[t]\nb = 2\na = 1\n"), "{\"a\":1,\"b\":2}");
}

#[test]
fn missing_file_and_missing_entry_are_reported() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("config.toml");
    std::fs::write(&file, TOML_V1).unwrap();
    let record = record_for(Scope::Agent(AgentId::Codex), &toml_target(&file));
    std::fs::write(&file, "model = \"o3\"\n").unwrap();
    assert_eq!(only_drift(&record), Drift::Missing);
    std::fs::remove_file(&file).unwrap();
    assert_eq!(only_drift(&record), Drift::Missing);
    let json = dir.path().join("s.json");
    std::fs::write(&json, JSON_V1).unwrap();
    let record = record_for(Scope::Install, &json_target(&json));
    std::fs::write(&json, "{\"mcpServers\": {}}").unwrap();
    assert_eq!(only_drift(&record), Drift::Missing);
}

#[test]
fn broken_shared_file_is_unreadable_without_contents() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("config.toml");
    std::fs::write(&file, TOML_V1).unwrap();
    let record = record_for(Scope::Agent(AgentId::Codex), &toml_target(&file));
    std::fs::write(&file, "api_key = \"sk-SECRET\n[[[").unwrap();
    let Drift::Unreadable { reason } = only_drift(&record) else {
        panic!("expected unreadable");
    };
    assert!(!reason.contains("sk-SECRET"), "{reason}");
}

#[test]
fn markdown_section_and_whole_file_ignore_crlf_and_outside_edits() {
    let dir = tempfile::tempdir().unwrap();
    let md = dir.path().join("CLAUDE.md");
    let section = "<!-- mmhmem:routing:start -->\nUse smart_read.\n<!-- mmhmem:routing:end -->";
    std::fs::write(&md, format!("# Mine\n\n{section}\n")).unwrap();
    let target = Target::MarkdownSection {
        file: md.clone(),
        section: SectionId::try_from("routing".to_owned()).unwrap(),
    };
    let record = record_for(Scope::Agent(AgentId::ClaudeCode), &target);
    let edited = format!(
        "# Mine, edited\r\n\r\n{}\r\ntail\r\n",
        section.replace('\n', "\r\n")
    );
    std::fs::write(&md, edited).unwrap();
    assert_eq!(only_drift(&record), Drift::Unchanged);
    std::fs::write(&md, section.replace("smart_read", "cat")).unwrap();
    assert!(matches!(only_drift(&record), Drift::Changed { .. }));
    std::fs::write(&md, format!("{section}\n{section}\n")).unwrap();
    assert!(matches!(only_drift(&record), Drift::Unreadable { .. }));
    std::fs::write(&md, "<!-- mmhmem:routing:start -->\nopen\n").unwrap();
    assert!(matches!(only_drift(&record), Drift::Unreadable { .. }));
    let skill = dir.path().join("SKILL.md");
    std::fs::write(&skill, "a\nb\n").unwrap();
    let record = record_for(
        Scope::Install,
        &Target::File {
            file: skill.clone(),
        },
    );
    std::fs::write(&skill, "a\r\nb\r\n").unwrap();
    assert_eq!(only_drift(&record), Drift::Unchanged);
}

#[test]
fn binary_is_hashed_raw_and_path_entry_needs_the_path() {
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("mmhmem.exe");
    std::fs::write(&bin, b"MZ\r\n\0").unwrap();
    let target = Target::Binary { path: bin.clone() };
    assert_eq!(
        present(observe(&target, &Probe::default())),
        Sha256Hex::of(b"MZ\r\n\0")
    );
    let folder = dir.path().join("bin");
    let entry = Target::PathEntry {
        value: folder.clone(),
    };
    assert!(matches!(
        observe(&entry, &Probe::default()),
        Observation::Unreadable(_)
    ));
    let with = [
        PathBuf::from("/usr/bin"),
        PathBuf::from(format!("{}/", folder.display())),
    ];
    let probe = Probe {
        user_path: Some(&with),
    };
    assert_eq!(present(observe(&entry, &probe)), path_entry_hash(&folder));
    let without = [PathBuf::from("/usr/bin")];
    let probe = Probe {
        user_path: Some(&without),
    };
    assert_eq!(observe(&entry, &probe), Observation::Missing);
}

/// Disk at v1 with a `done` record; then a run that changes the entry to v2.
struct Crash {
    _dir: tempfile::TempDir,
    store: RecordStore,
    file: PathBuf,
    target: Target,
    v1: Sha256Hex,
    v2: Sha256Hex,
    v2_text: String,
}

const SCOPE: Scope = Scope::Agent(AgentId::OpenCode);

fn crash_setup() -> Crash {
    let dir = tempfile::tempdir().unwrap();
    let store = RecordStore::new(dir.path().join("state"));
    let file = dir.path().join("opencode.json");
    std::fs::write(&file, JSON_V1).unwrap();
    let target = json_target(&file);
    store.save(&record_for(SCOPE, &target)).unwrap();
    let v2_text = JSON_V1.replace("\"stdio\"", "\"stdio\", \"--v2\"");
    let v1 = present(observe_in(&target, JSON_V1.as_bytes()));
    let v2 = present(observe_in(&target, v2_text.as_bytes()));
    Crash {
        _dir: dir,
        store,
        file,
        target,
        v1,
        v2,
        v2_text,
    }
}

fn begin(c: &Crash) {
    let mut record = c.store.load().unwrap().unwrap();
    let at = UtcTimestamp::now();
    let backup = c.store.backup(&c.file, &at).unwrap();
    let change = PendingChange {
        target: c.target.clone(),
        before: Some(c.v1.clone()),
        after: Some(c.v2.clone()),
        backup,
        missing_parents: Some(0),
    };
    c.store.journal_begin(&mut record, SCOPE, change).unwrap();
}

fn scan(c: &Crash) -> (InstallRecord, Vec<RecoveryReport>) {
    let record = c.store.load().unwrap().unwrap();
    let reports = recovery_scan(&record, &Probe::default());
    (record, reports)
}

#[test]
fn crash_after_backup_before_record_save_leaves_a_clean_record() {
    let c = crash_setup();
    let at = UtcTimestamp::now();
    let backup = c.store.backup(&c.file, &at).unwrap().unwrap();
    let (record, reports) = scan(&c);
    assert!(reports.is_empty());
    assert_eq!(only_drift(&record), Drift::Unchanged);
    assert_eq!(
        std::fs::read(backup.resolve(c.store.dir())).unwrap(),
        JSON_V1.as_bytes()
    );
}

#[test]
fn crash_after_applying_saved_before_target_write_is_not_applied() {
    let c = crash_setup();
    begin(&c);
    let (mut record, reports) = scan(&c);
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].recovery, Recovery::NotApplied);
    assert_eq!(reports[0].target, c.target);
    record.undo_change(SCOPE, &c.target).unwrap();
    assert_eq!(only_drift(&record), Drift::Unchanged);
}

#[test]
fn crash_after_target_write_before_done_is_applied() {
    let c = crash_setup();
    begin(&c);
    crate::setup::fsx::write_atomic(&c.file, c.v2_text.as_bytes()).unwrap();
    let (mut record, reports) = scan(&c);
    assert_eq!(reports[0].recovery, Recovery::Applied);
    c.store
        .journal_finish(&mut record, SCOPE, &c.target)
        .unwrap();
    let record = c.store.load().unwrap().unwrap();
    assert!(record.applying().is_empty());
    assert_eq!(only_drift(&record), Drift::Unchanged);
    let entry = record.find(SCOPE, &c.target).unwrap();
    assert!(entry.backup.is_some());
}

#[test]
fn crash_then_outside_edit_is_diverged() {
    let c = crash_setup();
    begin(&c);
    std::fs::write(&c.file, "{\"mcpServers\": {\"mmhmem\": 1}}").unwrap();
    let (_, reports) = scan(&c);
    assert!(matches!(
        reports[0].recovery,
        Recovery::Diverged { actual: Some(_) }
    ));
    std::fs::remove_file(&c.file).unwrap();
    let (_, reports) = scan(&c);
    assert_eq!(reports[0].recovery, Recovery::Diverged { actual: None });
}

#[test]
fn crash_during_new_entry_or_removal_is_classified() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("SKILL.md");
    let target = Target::File { file: file.clone() };
    let mut record = InstallRecord::new();
    let pending = |before: Option<Sha256Hex>, after: Option<Sha256Hex>| PendingChange {
        target: target.clone(),
        before,
        after,
        backup: None,
        missing_parents: None,
    };
    let skill = present(observe_in(&target, b"skill\n"));
    record.begin_change(Scope::Install, pending(None, Some(skill.clone())));
    let first = recovery_scan(&record, &Probe::default());
    assert_eq!(first[0].recovery, Recovery::NotApplied);
    std::fs::write(&file, "skill\n").unwrap();
    record.finish_change(Scope::Install, &target).unwrap();
    record.begin_change(Scope::Install, pending(Some(skill), None));
    std::fs::remove_file(&file).unwrap();
    let removal = recovery_scan(&record, &Probe::default());
    assert_eq!(removal[0].recovery, Recovery::Applied);
}

fn is_allowed_string(s: &str, root: &str) -> bool {
    Sha256Hex::try_from(s.to_owned()).is_ok()
        || ReleaseTag::try_from(s.to_owned()).is_ok()
        || UtcTimestamp::try_from(s.to_owned()).is_ok()
        || BackupPath::try_from(s.to_owned()).is_ok()
        || s.starts_with(root)
        || [
            "mcpServers",
            "mmhmem",
            "mcp_servers",
            "routing",
            "PreToolUse",
        ]
        .contains(&s)
        || s.bytes().all(|b| b.is_ascii_lowercase() || b == b'_')
}

fn assert_allowed_shapes(value: &Value, root: &str, path: &str) {
    match value {
        Value::String(s) => assert!(is_allowed_string(s, root), "{path} holds free text"),
        Value::Array(items) => items
            .iter()
            .for_each(|v| assert_allowed_shapes(v, root, &format!("{path}[]"))),
        Value::Object(map) => map.iter().for_each(|(k, v)| {
            let key_ok = k
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
            assert!(key_ok, "unexpected key {k} at {path}");
            assert_allowed_shapes(v, root, &format!("{path}.{k}"));
        }),
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

#[test]
fn serialized_record_holds_only_hashes_paths_tags_and_enums() {
    let dir = tempfile::tempdir().unwrap();
    let value = serde_json::to_value(sample_record(dir.path())).unwrap();
    let root = dir.path().to_string_lossy().into_owned();
    assert_allowed_shapes(&value, &root, "$");
}

#[test]
fn secret_inside_a_json_entry_is_stored_only_as_a_hash() {
    let dir = tempfile::tempdir().unwrap();
    let store = RecordStore::new(dir.path().join("state"));
    let file = dir.path().join("claude_desktop_config.json");
    let secret = "sk-test-0123456789abcdef";
    let doc = format!(
        "{{\"mcpServers\": {{\"mmhmem\": {{\"command\": \"mmhmem\", \"env\": {{\"api_key\": \"{secret}\"}}}}}}}}"
    );
    std::fs::write(&file, &doc).unwrap();
    let target = json_target(&file);
    let record = record_for(Scope::Agent(AgentId::ClaudeDesktop), &target);
    store.save(&record).unwrap();
    let text = std::fs::read_to_string(store.record_path()).unwrap();
    assert!(!text.contains(secret) && !text.contains("api_key") && !text.contains("sk-"));
    let canonical = "{\"command\":\"mmhmem\",\"env\":{\"api_key\":\"sk-test-0123456789abcdef\"}}";
    let hash = Sha256Hex::of(canonical.as_bytes());
    assert_eq!(record.done()[0].1.sha256, Some(hash.clone()));
    assert!(text.contains(hash.as_str()));
}
