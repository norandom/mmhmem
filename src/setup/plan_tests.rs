//! Tests for `setup::plan` (task 2.5): applied bytes equal planned bytes,
//! per-agent rollback, the stale and validity checks, owner-only changes,
//! whole-file deletion, two entries in one file and the journal order.
//! Temporary folders only.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::engine::{begin, write_change};
use super::*;
use crate::setup::edit;
use crate::setup::record::{
    AgentId, Drift, ElementMatcher, HookHarness, InstallRecord, Observation, PendingChange, Probe,
    RecordStore, Recovery, SectionId, detect_drift, key_path, observe_in, recovery_scan,
};

pub(super) const CODEX: Scope = Scope::Agent(AgentId::Codex);
pub(super) const CLAUDE: Scope = Scope::Agent(AgentId::ClaudeCode);
pub(super) const OPENCODE: Scope = Scope::Agent(AgentId::OpenCode);

pub(super) const TOML_BEFORE: &str = "# my settings\nmodel = \"o3\"\n";
pub(super) const TOML_AFTER: &str = "# my settings\nmodel = \"o3\"\n\n[mcp_servers.mmhmem]\ncommand = \"mmhmem\"\nargs = [\"stdio\"]\n";

pub(super) struct Env {
    _dir: tempfile::TempDir,
    pub(super) root: PathBuf,
    pub(super) store: RecordStore,
}

pub(super) fn env() -> Env {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("home");
    std::fs::create_dir_all(&root).unwrap();
    let store = RecordStore::new(dir.path().join("state"));
    Env {
        _dir: dir,
        root,
        store,
    }
}

pub(super) fn run() -> UtcTimestamp {
    UtcTimestamp::try_from("2026-11-04T09:30:00Z".to_owned()).unwrap()
}

pub(super) fn plan(changes: Vec<Change>) -> Plan {
    Plan {
        run: run(),
        changes,
    }
}

pub(super) fn read(path: &Path) -> Option<Vec<u8>> {
    std::fs::read(path).ok()
}

pub(super) fn change(
    scope: Scope,
    target: Target,
    before: Option<&[u8]>,
    after: Option<&[u8]>,
) -> Change {
    let action = match (before, after) {
        (_, None) => Action::Remove,
        (None, Some(_)) => Action::Add,
        (Some(_), Some(_)) => Action::Update,
    };
    Change {
        scope,
        file: target_file(&target).unwrap().to_path_buf(),
        target,
        action,
        before: before.map(<[u8]>::to_vec),
        after: after.map(<[u8]>::to_vec),
        owner_only: false,
        hide_diff: false,
        missing_parents: None,
        summary: "test change".to_owned(),
    }
}

pub(super) fn file_target(file: &Path) -> Target {
    Target::File {
        file: file.to_path_buf(),
    }
}

pub(super) fn toml_target(file: &Path) -> Target {
    Target::TomlTable {
        file: file.to_path_buf(),
        key: key_path(&["mcp_servers", "mmhmem"]).unwrap(),
        managed_keys: None,
    }
}

pub(super) fn json_target(file: &Path, keys: &[&str]) -> Target {
    Target::JsonEntry {
        file: file.to_path_buf(),
        pointer: key_path(keys).unwrap(),
        managed_keys: None,
    }
}

pub(super) fn hook_target(file: &Path) -> Target {
    Target::JsonArrayElement {
        file: file.to_path_buf(),
        pointer: key_path(&["hooks", "PreToolUse"]).unwrap(),
        matcher: ElementMatcher::MmhmemHook(HookHarness::Claude),
    }
}

/// A JSON entry change computed with `edit::json::set`, as an agent would.
pub(super) fn json_set(
    scope: Scope,
    file: &Path,
    before: Option<&str>,
    keys: &[&str],
    value: &Value,
) -> Change {
    let input = before.unwrap_or("{}\n");
    let after = edit::json::set(input, keys, value).unwrap();
    let mut c = change(
        scope,
        json_target(file, keys),
        before.map(str::as_bytes),
        Some(after.as_bytes()),
    );
    c.action = Action::Add;
    c.missing_parents =
        Some(u32::try_from(edit::json::missing_parents(input, keys).unwrap()).unwrap());
    c
}

pub(super) fn present(target: &Target, bytes: &[u8]) -> Sha256Hex {
    match observe_in(target, bytes) {
        Observation::Present(h) => h,
        other => panic!("expected an entry, got {other:?}"),
    }
}

/// A record holding `target` as `done` with its hash in `bytes`.
pub(super) fn managed(record: &mut InstallRecord, scope: Scope, target: &Target, bytes: &[u8]) {
    record.begin_change(
        scope,
        PendingChange {
            target: target.clone(),
            before: None,
            after: Some(present(target, bytes)),
            backup: None,
            missing_parents: Some(1),
        },
    );
    record.finish_change(scope, target).unwrap();
}

/// Record and disk agree, nothing is `applying`, and the saved record is the
/// one in memory.
pub(super) fn assert_record_matches_disk(e: &Env, record: &InstallRecord) {
    assert!(record.applying().is_empty(), "leftover applying entries");
    for report in detect_drift(record, &Probe::default()) {
        assert_eq!(report.drift, Drift::Unchanged, "{:?}", report.target);
    }
    assert_eq!(e.store.load().unwrap().as_ref(), Some(record));
}

pub(super) fn outcome(report: &ApplyReport, scope: Scope) -> &Outcome {
    &report.agent(scope).unwrap().outcome
}

#[test]
fn applied_bytes_equal_planned_bytes_and_record_matches_disk() {
    let e = env();
    let settings = e.root.join(".claude/settings.json");
    let config = e.root.join(".codex/config.toml");
    let skill = e.root.join(".codex/skills/unified-memory/SKILL.md");
    let notes = e.root.join(".claude/CLAUDE.md");
    let settings_before = "{\n  \"theme\": \"dark\"\n}\n";
    std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
    std::fs::create_dir_all(config.parent().unwrap()).unwrap();
    std::fs::write(&settings, settings_before).unwrap();
    std::fs::write(&config, TOML_BEFORE).unwrap();
    std::fs::write(&notes, "# Mine\r\n").unwrap();
    let section = Target::MarkdownSection {
        file: notes.clone(),
        section: SectionId::try_from("routing".to_owned()).unwrap(),
    };
    let notes_after = "# Mine\r\n\r\n<!-- mmhmem:routing:start -->\r\nUse mmhmem.\r\n<!-- mmhmem:routing:end -->\r\n";
    let mcp = json!({ "command": "mmhmem", "args": ["stdio"] });
    let changes = vec![
        json_set(
            CLAUDE,
            &settings,
            Some(settings_before),
            &["mcpServers", "mmhmem"],
            &mcp,
        ),
        change(
            CODEX,
            toml_target(&config),
            Some(TOML_BEFORE.as_bytes()),
            Some(TOML_AFTER.as_bytes()),
        ),
        change(CODEX, file_target(&skill), None, Some(b"# skill\n")),
        change(
            CLAUDE,
            section,
            Some(b"# Mine\r\n"),
            Some(notes_after.as_bytes()),
        ),
    ];
    let plan = plan(changes);
    let mut record = InstallRecord::new();
    let report = apply(&plan, &e.store, &mut record);

    assert_eq!(report.agents.len(), 2);
    assert_eq!(outcome(&report, CLAUDE), &Outcome::Applied);
    assert_eq!(outcome(&report, CODEX), &Outcome::Applied);
    for c in &plan.changes {
        assert_eq!(read(&c.file), c.after, "{}", c.file.display());
        let entry = record.find(c.scope, &c.target).unwrap();
        assert_eq!(entry.state, crate::setup::record::EntryState::Done);
        let after = c.after.as_deref().unwrap();
        assert_eq!(entry.sha256, Some(present(&c.target, after)));
    }
    assert_record_matches_disk(&e, &record);
    let claude = report.agent(CLAUDE).unwrap();
    let backup = claude.backups()[0].resolve(e.store.dir());
    assert_eq!(read(&backup).unwrap(), settings_before.as_bytes());
    assert_eq!(claude.files(), vec![settings.as_path(), notes.as_path()]);
}

#[test]
fn failure_mid_agent_restores_its_files_and_keeps_other_agents() {
    let e = env();
    let config = e.root.join("config.toml");
    let v2 = TOML_AFTER.replace("[\"stdio\"]", "[\"stdio\", \"--v2\"]");
    std::fs::write(&config, TOML_AFTER).unwrap();
    let mut record = InstallRecord::new();
    managed(
        &mut record,
        CODEX,
        &toml_target(&config),
        TOML_AFTER.as_bytes(),
    );
    e.store.save(&record).unwrap();
    let prior = record.find(CODEX, &toml_target(&config)).cloned();
    // The second Codex file sits below a regular file, so it cannot be written.
    let blocker = e.root.join("blocker");
    std::fs::write(&blocker, b"not a folder").unwrap();
    let skill = blocker.join("SKILL.md");
    let settings = e.root.join("settings.json");
    let mcp = json!({ "command": "mmhmem" });
    let plan = plan(vec![
        change(
            CODEX,
            toml_target(&config),
            Some(TOML_AFTER.as_bytes()),
            Some(v2.as_bytes()),
        ),
        change(CODEX, file_target(&skill), None, Some(b"# skill\n")),
        json_set(CLAUDE, &settings, None, &["mcpServers", "mmhmem"], &mcp),
    ]);
    let report = apply(&plan, &e.store, &mut record);

    assert!(
        matches!(outcome(&report, CODEX), Outcome::RolledBack { .. }),
        "{report:?}"
    );
    assert_eq!(
        read(&config).unwrap(),
        TOML_AFTER.as_bytes(),
        "restored byte-identically"
    );
    assert_eq!(read(&blocker).unwrap(), b"not a folder");
    assert_eq!(record.find(CODEX, &toml_target(&config)).cloned(), prior);
    assert!(record.find(CODEX, &file_target(&skill)).is_none());
    assert_eq!(outcome(&report, CLAUDE), &Outcome::Applied);
    assert_eq!(read(&settings), plan.changes[2].after);
    assert_record_matches_disk(&e, &record);
}

#[test]
fn failed_new_agent_leaves_no_files_and_no_record() {
    let e = env();
    let first = e.root.join("opencode/commands/recap.md");
    let blocker = e.root.join("blocker");
    std::fs::write(&blocker, b"x").unwrap();
    let plan = plan(vec![
        change(OPENCODE, file_target(&first), None, Some(b"recap\n")),
        change(
            OPENCODE,
            file_target(&blocker.join("p.js")),
            None,
            Some(b"//\n"),
        ),
    ]);
    let mut record = InstallRecord::new();
    let report = apply(&plan, &e.store, &mut record);
    assert!(matches!(
        outcome(&report, OPENCODE),
        Outcome::RolledBack { .. }
    ));
    assert!(!first.exists(), "the new file is removed again");
    assert_eq!(record, InstallRecord::new());
    assert_record_matches_disk(&e, &record);
}

#[test]
fn owner_only_change_is_private_and_its_contents_never_shown() {
    let e = env();
    let key = e.root.join("worker.toml");
    let secret = b"api_key = \"sk-test-SECRET\"\n";
    let mut c = change(Scope::Install, file_target(&key), None, Some(secret));
    c.owner_only = true;
    assert_eq!(c.diff(), HIDDEN_DIFF);
    assert!(!format!("{c:?}").contains("SECRET"));
    let mut broken = c.clone();
    broken.after = Some(b"api_key = \"sk-test-SECRET\n".to_vec());
    let mut record = InstallRecord::new();
    let report = apply(&plan(vec![broken]), &e.store, &mut record);
    let Outcome::Skipped(skip) = outcome(&report, Scope::Install) else {
        panic!("{report:?}")
    };
    assert!(!skip.to_string().contains("SECRET") && !format!("{report:?}").contains("SECRET"));
    assert!(!key.exists());

    let report = apply(&plan(vec![c.clone()]), &e.store, &mut record);
    assert_eq!(outcome(&report, Scope::Install), &Outcome::Applied);
    assert_eq!(read(&key).unwrap(), secret);
    #[cfg(windows)]
    crate::setup::fsx::tests::windows::assert_current_user_only(&key);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&key).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    assert_record_matches_disk(&e, &record);
}

#[test]
fn deleting_a_managed_file_backs_it_up_and_rollback_restores_it() {
    let e = env();
    let skill = e.root.join("skills/unified-memory/SKILL.md");
    let command = e.root.join("commands/recap.md");
    let original = b"old skill\r\n".as_slice();
    std::fs::create_dir_all(skill.parent().unwrap()).unwrap();
    std::fs::write(&skill, original).unwrap();
    let mut record = InstallRecord::new();
    managed(&mut record, CODEX, &file_target(&skill), original);
    e.store.save(&record).unwrap();
    let before_run = record.clone();
    let plan = plan(vec![
        change(CODEX, file_target(&skill), Some(original), None),
        change(CODEX, file_target(&command), None, Some(b"recap\n")),
    ]);
    let mut report = apply(&plan, &e.store, &mut record);
    assert_eq!(outcome(&report, CODEX), &Outcome::Applied);
    assert!(!skill.exists());
    assert!(record.find(CODEX, &file_target(&skill)).is_none());
    let backups = report.agent(CODEX).unwrap().backups();
    assert_eq!(backups.len(), 1);
    assert_eq!(read(&backups[0].resolve(e.store.dir())).unwrap(), original);
    assert_record_matches_disk(&e, &record);

    rollback_agent(
        &mut report,
        CODEX,
        &e.store,
        &mut record,
        "the check failed",
    )
    .unwrap();
    assert_eq!(read(&skill).unwrap(), original);
    assert!(!command.exists());
    assert_eq!(record, before_run);
    assert_record_matches_disk(&e, &record);
    let reason = "the check failed".to_owned();
    assert_eq!(outcome(&report, CODEX), &Outcome::RolledBack { reason });
    let again = rollback_agent(&mut report, CODEX, &e.store, &mut record, "again");
    assert_eq!(again, Err(RollbackError::NotApplied));
}

pub(super) fn hook_group() -> Value {
    json!({ "matcher": "Read", "hooks": [{ "type": "command", "command": "/opt/mmhmem/mmhmem", "args": ["hook", "claude"] }] })
}

pub(super) const SETTINGS: &str = "{\n  // mine\n  \"hooks\": {\n    \"PreToolUse\": [\n      { \"matcher\": \"Bash\", \"hooks\": [{ \"type\": \"command\", \"command\": \"lint\" }] }\n    ]\n  }\n}\n";

/// An MCP entry and then mmhmem's hook group in the same settings file.
pub(super) fn two_changes(file: &Path) -> (Change, Change) {
    let mcp = json!({ "command": "mmhmem", "args": ["stdio"] });
    let first = json_set(
        CLAUDE,
        file,
        Some(SETTINGS),
        &["mcpServers", "mmhmem"],
        &mcp,
    );
    let mid = String::from_utf8(first.after.clone().unwrap()).unwrap();
    let path = ["hooks", "PreToolUse"];
    let after = edit::json::array_append(&mid, &path, &hook_group()).unwrap();
    let mut second = change(
        CLAUDE,
        hook_target(file),
        Some(mid.as_bytes()),
        Some(after.as_bytes()),
    );
    second.action = Action::Add;
    (first, second)
}

#[test]
fn two_changes_in_one_file_apply_and_are_both_recorded() {
    let e = env();
    let settings = e.root.join("settings.json");
    std::fs::write(&settings, SETTINGS).unwrap();
    let (first, second) = two_changes(&settings);
    let added: Vec<String> = second
        .diff()
        .lines()
        .filter(|l| l.starts_with('+') && !l.starts_with("+++"))
        .map(str::to_owned)
        .collect();
    assert!(added.iter().any(|l| l.contains("mmhmem")), "{added:?}");
    assert!(
        added.iter().all(|l| !l.contains("mcpServers")),
        "only the hook is new"
    );
    let plan = plan(vec![first.clone(), second.clone()]);
    let mut record = InstallRecord::new();
    let report = apply(&plan, &e.store, &mut record);
    assert_eq!(outcome(&report, CLAUDE), &Outcome::Applied);
    assert_eq!(read(&settings), second.after);
    let final_bytes = second.after.as_deref().unwrap();
    for target in [&first.target, &second.target] {
        let entry = record.find(CLAUDE, target).unwrap();
        assert_eq!(entry.sha256, Some(present(target, final_bytes)));
    }
    assert_eq!(report.agent(CLAUDE).unwrap().backups().len(), 2);
    assert_record_matches_disk(&e, &record);
}

#[test]
fn crash_after_journal_begin_before_write_is_recoverable() {
    let e = env();
    let config = e.root.join("config.toml");
    std::fs::write(&config, TOML_BEFORE).unwrap();
    let c = change(
        CODEX,
        toml_target(&config),
        Some(TOML_BEFORE.as_bytes()),
        Some(TOML_AFTER.as_bytes()),
    );
    let mut record = InstallRecord::new();
    let applied =
        begin(&c, &run(), &e.store, &mut record).unwrap_or_else(|f| panic!("{}", f.reason));
    // "Crash" here: the backup and the applying record are on disk.
    let saved = e.store.load().unwrap().unwrap();
    let scan = recovery_scan(&saved, &Probe::default());
    assert_eq!(scan.len(), 1);
    assert_eq!(scan[0].recovery, Recovery::NotApplied);
    let backup = saved
        .find(CODEX, &c.target)
        .unwrap()
        .backup
        .clone()
        .unwrap();
    assert_eq!(
        read(&backup.resolve(e.store.dir())).unwrap(),
        TOML_BEFORE.as_bytes()
    );
    assert_eq!(applied.backup, Some(backup));
    // A crash after the write but before `done` is classified as applied.
    write_change(&c).unwrap();
    let scan = recovery_scan(&e.store.load().unwrap().unwrap(), &Probe::default());
    assert_eq!(scan[0].recovery, Recovery::Applied);
}
