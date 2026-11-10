//! Tests for `setup::record` (task 2.4): schema round trip, atomic save,
//! unreadable records, per-OS location and the journaling state machine.

use std::path::{Path, PathBuf};

use super::*;

pub(super) fn sha(seed: &str) -> Sha256Hex {
    Sha256Hex::of(seed.as_bytes())
}

pub(super) fn json_target(file: &Path) -> Target {
    Target::JsonEntry {
        file: file.to_path_buf(),
        pointer: key_path(&["mcpServers", "mmhmem"]).unwrap(),
        managed_keys: None,
    }
}

fn done(target: Target, seed: &str) -> ManagedEntry {
    ManagedEntry {
        target,
        sha256: Some(sha(seed)),
        before_sha256: None,
        was_managed: false,
        backup: None,
        missing_parents: None,
        created_file: false,
        state: EntryState::Done,
    }
}

/// An agent record that uses every agent field and entry kind.
fn sample_agent(root: &Path) -> AgentRecord {
    let mut toml = done(
        Target::TomlTable {
            file: root.join("config.toml"),
            key: key_path(&["mcp_servers", "mmhmem"]).unwrap(),
            managed_keys: None,
        },
        "toml",
    );
    toml.backup =
        Some(BackupPath::try_from("backups/2026-11-04T09-30-00Z/config.toml".to_owned()).unwrap());
    toml.missing_parents = Some(1);
    let mut agent = AgentRecord::default();
    agent.features.insert(Feature::Mcp, true);
    agent.features.insert(Feature::ReadHook, false);
    agent.managed = vec![
        toml,
        done(json_target(&root.join("settings.json")), "json"),
        done(
            Target::JsonArrayElement {
                file: root.join("settings.json"),
                pointer: key_path(&["hooks", "PreToolUse"]).unwrap(),
                matcher: ElementMatcher::MmhmemHook(HookHarness::Claude),
            },
            "hook",
        ),
        done(
            Target::JsonEntry {
                file: root.join("opencode.json"),
                pointer: key_path(&["mcp", "servers", "mmhmem"]).unwrap(),
                managed_keys: Some(key_path(&["type", "command"]).unwrap()),
            },
            "opencode",
        ),
        done(
            Target::File {
                file: root.join("SKILL.md"),
            },
            "file",
        ),
        done(
            Target::MarkdownSection {
                file: root.join("CLAUDE.md"),
                section: SectionId::try_from("routing".to_owned()).unwrap(),
            },
            "md",
        ),
    ];
    agent.pending_user_steps = vec![PendingUserStep::ApproveHookInCodex];
    agent.last_check = Some(LastCheck {
        at: UtcTimestamp::try_from("2026-11-04T09:31:12Z".to_owned()).unwrap(),
        result: CheckResult::Pass,
        details: vec![CheckDetail {
            step: CheckStep::Handshake,
            outcome: StepOutcome::Pass,
        }],
    });
    agent
}

/// A record that uses every field and every entry kind.
pub(super) fn sample_record(root: &Path) -> InstallRecord {
    let bin = root.join("bin").join("mmhmem.exe");
    let mut record = InstallRecord::new();
    record.release = Some(ReleaseTag::try_from("v0.5.0".to_owned()).unwrap());
    record.binary = Some(BinaryInfo {
        path: bin.clone(),
        sha256: sha("bin"),
    });
    record.path_entry = Some(PathEntryInfo {
        added: true,
        value: root.join("bin"),
        created_value: false,
        trailing_separator: false,
    });
    record.worker_configured = true;
    record.managed = vec![
        done(Target::Binary { path: bin }, "bin"),
        done(
            Target::PathEntry {
                value: root.join("bin"),
            },
            "path",
        ),
    ];
    record.agents.insert(AgentId::Codex, sample_agent(root));
    record
}

#[test]
fn save_load_round_trip_keeps_every_field() {
    let dir = tempfile::tempdir().unwrap();
    let store = RecordStore::new(dir.path());
    let record = sample_record(dir.path());
    store.save(&record).unwrap();
    assert_eq!(store.load().unwrap(), Some(record));
    let text = std::fs::read_to_string(store.record_path()).unwrap();
    assert!(text.contains("\"kind\": \"toml_table\""));
    assert!(text.contains("\"state\": \"done\""));
}

#[test]
fn load_of_missing_record_is_none() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(
        RecordStore::new(dir.path().join("nope")).load().unwrap(),
        None
    );
}

/// antigravity-native-setup Requirement 5.4: the snippet-era import step
/// still loads from old records but is never written back.
#[test]
fn a_legacy_pending_step_loads_but_is_dropped_on_save() {
    let dir = tempfile::tempdir().unwrap();
    let store = RecordStore::new(dir.path());
    let mut record = sample_record(dir.path());
    let codex = record.agents.get_mut(&AgentId::Codex).unwrap();
    codex.pending_user_steps = vec![
        PendingUserStep::ImportAntigravitySnippet,
        PendingUserStep::ApproveHookInCodex,
    ];
    let old: PendingUserStep = serde_json::from_str("\"import_antigravity_snippet\"").unwrap();
    assert_eq!(old, PendingUserStep::ImportAntigravitySnippet);
    assert!(old.is_legacy());
    assert!(!PendingUserStep::ApproveHookInCodex.is_legacy());
    store.save(&record).unwrap();
    let text = std::fs::read_to_string(store.record_path()).unwrap();
    assert!(!text.contains("import_antigravity_snippet"), "{text}");
    let loaded = store.load().unwrap().unwrap();
    assert_eq!(
        loaded.agents[&AgentId::Codex].pending_user_steps,
        [PendingUserStep::ApproveHookInCodex]
    );
}

#[test]
fn opencode_agent_id_serializes_as_opencode() {
    let text = serde_json::to_string(&AgentId::OpenCode).unwrap();
    assert_eq!(text, "\"opencode\"");
}

#[test]
fn failed_save_onto_a_directory_reports_save_error() {
    let dir = tempfile::tempdir().unwrap();
    let store = RecordStore::new(dir.path());
    std::fs::create_dir(store.record_path()).unwrap();
    let err = store.save(&InstallRecord::new()).unwrap_err();
    assert!(matches!(err, RecordError::Save { .. }), "{err}");
    assert!(store.record_path().is_dir());
    let leftovers: Vec<_> = std::fs::read_dir(dir.path()).unwrap().collect();
    assert_eq!(leftovers.len(), 1, "no temporary file may be left behind");
}

#[cfg(windows)]
fn hold_exclusively(path: &Path) -> std::fs::File {
    use std::os::windows::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(path)
        .unwrap()
}

#[cfg(unix)]
fn set_folder_mode(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
}

#[test]
fn failed_save_leaves_the_old_record_intact() {
    let dir = tempfile::tempdir().unwrap();
    let store = RecordStore::new(dir.path());
    let old = sample_record(dir.path());
    store.save(&old).unwrap();
    let before = std::fs::read(store.record_path()).unwrap();
    #[cfg(windows)]
    let guard = hold_exclusively(&store.record_path());
    #[cfg(unix)]
    set_folder_mode(dir.path(), 0o500);
    let result = store.save(&InstallRecord::new());
    #[cfg(windows)]
    drop(guard);
    #[cfg(unix)]
    set_folder_mode(dir.path(), 0o700);
    if result.is_ok() {
        // Running as root on Unix ignores folder permissions; nothing to prove.
        return;
    }
    assert!(matches!(result, Err(RecordError::Save { .. })));
    assert_eq!(std::fs::read(store.record_path()).unwrap(), before);
    assert_eq!(store.load().unwrap(), Some(old));
}

fn unreadable_kind(dir: &Path, content: &[u8]) -> (UnreadableKind, String) {
    let store = RecordStore::new(dir);
    std::fs::write(store.record_path(), content).unwrap();
    match store.load() {
        Err(err @ RecordError::Unreadable { kind, .. }) => (kind, err.to_string()),
        other => panic!("expected an unreadable record, got {other:?}"),
    }
}

#[test]
fn unreadable_records_have_distinct_kinds_and_never_echo_contents() {
    let dir = tempfile::tempdir().unwrap();
    let secret = "sk-live-NEVER-PRINT-ME";
    let cases: Vec<(String, UnreadableKind)> = vec![
        (
            format!("{{\"schema\": 1, \"release\": \"{secret}"),
            UnreadableKind::NotJson {
                line: 1,
                column: 48,
            },
        ),
        (
            format!("{{\"schema\": 2, \"x\": \"{secret}\"}}"),
            UnreadableKind::UnknownSchema { found: Some(2) },
        ),
        (
            format!("{{\"release\": \"{secret}\"}}"),
            UnreadableKind::UnknownSchema { found: None },
        ),
        (
            format!("{{\"schema\": 1, \"release\": \"{secret}\"}}"),
            UnreadableKind::InvalidShape,
        ),
    ];
    for (content, want) in cases {
        let (kind, message) = unreadable_kind(dir.path(), content.as_bytes());
        assert_eq!(kind, want, "{content}");
        assert!(!message.contains(secret), "{message}");
    }
}

#[test]
fn record_with_a_done_entry_without_hash_is_invalid() {
    let dir = tempfile::tempdir().unwrap();
    let mut record = sample_record(dir.path());
    record.managed[0].sha256 = None;
    let bytes = serde_json::to_vec(&record).unwrap();
    assert_eq!(parse_record(&bytes), Err(UnreadableKind::InvalidShape));
}

#[test]
fn record_that_is_a_directory_is_an_io_error() {
    let dir = tempfile::tempdir().unwrap();
    let store = RecordStore::new(dir.path());
    std::fs::create_dir(store.record_path()).unwrap();
    let err = store.load().unwrap_err();
    assert!(
        matches!(
            err,
            RecordError::Unreadable {
                kind: UnreadableKind::Io(_),
                ..
            }
        ),
        "{err:?}"
    );
}

fn input(os: Os, home: Option<&str>, xdg: Option<&str>, local: Option<&str>) -> LocationInput {
    LocationInput {
        os,
        home: home.map(PathBuf::from),
        xdg_state_home: xdg.map(Into::into),
        local_app_data: local.map(Into::into),
    }
}

#[test]
fn location_follows_each_os_rule() {
    let linux = |xdg| input(Os::Linux, Some("/home/u"), xdg, Some("C:\\ignored")).resolve();
    let home_state = PathBuf::from("/home/u")
        .join(".local")
        .join("state")
        .join("mmhmem");
    assert_eq!(linux(None).unwrap(), home_state);
    assert_eq!(linux(Some("")).unwrap(), home_state);
    assert_eq!(linux(Some("relative/state")).unwrap(), home_state);
    assert_eq!(
        linux(Some("/xdg/state")).unwrap(),
        PathBuf::from("/xdg/state").join("mmhmem")
    );
    let mac = input(Os::MacOs, Some("/Users/u"), Some("/xdg"), None).resolve();
    assert_eq!(
        mac.unwrap(),
        PathBuf::from("/Users/u")
            .join("Library")
            .join("Application Support")
            .join("mmhmem")
    );
    let win = |local| input(Os::Windows, Some("C:\\Users\\u"), Some("/xdg"), local).resolve();
    assert_eq!(
        win(Some("D:\\Local")).unwrap(),
        PathBuf::from("D:\\Local").join("mmhmem")
    );
    assert_eq!(
        win(Some("not-absolute")).unwrap(),
        PathBuf::from("C:\\Users\\u")
            .join("AppData")
            .join("Local")
            .join("mmhmem")
    );
}

#[test]
fn location_without_home_or_override_fails() {
    let err = input(Os::Linux, None, None, None).resolve().unwrap_err();
    assert_eq!(err, LocationError::NoHome);
    assert!(input(Os::Linux, None, Some("/s"), None).resolve().is_ok());
    assert!(
        input(Os::Windows, None, None, Some("C:\\L"))
            .resolve()
            .is_ok()
    );
    let store = RecordStore::locate(&input(Os::MacOs, Some("/Users/u"), None, None)).unwrap();
    assert!(store.record_path().ends_with(RECORD_FILE));
}

#[test]
fn backup_dir_uses_a_portable_timestamp_folder() {
    let store = RecordStore::new("/state");
    let at = UtcTimestamp::try_from("2026-11-04T09:30:00Z".to_owned()).unwrap();
    assert_eq!(
        store.backup_dir(&at),
        PathBuf::from("/state")
            .join("backups")
            .join("2026-11-04T09-30-00Z")
    );
    let now = UtcTimestamp::now();
    assert!(UtcTimestamp::try_from(now.as_str().to_owned()).is_ok());
}

fn change(target: Target, before: Option<&str>, after: Option<&str>) -> PendingChange {
    PendingChange {
        target,
        before: before.map(sha),
        after: after.map(sha),
        backup: None,
        created_file: false,
        missing_parents: Some(0),
    }
}

#[test]
fn journal_marks_applying_then_done() {
    let target = json_target(Path::new("/h/settings.json"));
    let scope = Scope::Agent(AgentId::ClaudeCode);
    let mut record = InstallRecord::new();
    record.begin_change(scope, change(target.clone(), None, Some("v1")));
    assert_eq!(record.applying().len(), 1);
    assert_eq!(
        record.find(scope, &target).unwrap().state,
        EntryState::Applying
    );
    record.finish_change(scope, &target).unwrap();
    let entry = record.find(scope, &target).unwrap();
    assert_eq!(
        (entry.state, entry.sha256.clone()),
        (EntryState::Done, Some(sha("v1")))
    );
    assert_eq!(entry.before_sha256, None);
    assert!(record.applying().is_empty());
    assert_eq!(record.finish_change(scope, &target), Err(NotApplying));
}

#[test]
fn journal_removal_drops_the_entry_when_done() {
    let target = Target::File {
        file: PathBuf::from("/h/SKILL.md"),
    };
    let mut record = InstallRecord::new();
    record.begin_change(Scope::Install, change(target.clone(), None, Some("v1")));
    record.finish_change(Scope::Install, &target).unwrap();
    record.begin_change(Scope::Install, change(target.clone(), Some("v1"), None));
    assert_eq!(
        record.entries(Scope::Install).len(),
        1,
        "same target is replaced"
    );
    record.finish_change(Scope::Install, &target).unwrap();
    assert!(record.entries(Scope::Install).is_empty());
}

#[test]
fn journal_undo_restores_the_previous_state() {
    let target = json_target(Path::new("/h/settings.json"));
    let scope = Scope::Agent(AgentId::Codex);
    let mut record = InstallRecord::new();
    record.begin_change(scope, change(target.clone(), None, Some("v1")));
    record.undo_change(scope, &target).unwrap();
    assert!(record.entries(scope).is_empty(), "a new entry is forgotten");
    record.begin_change(scope, change(target.clone(), None, Some("v1")));
    record.finish_change(scope, &target).unwrap();
    record.begin_change(scope, change(target.clone(), Some("v1"), Some("v2")));
    record.undo_change(scope, &target).unwrap();
    let entry = record.find(scope, &target).unwrap();
    assert_eq!(
        (entry.state, entry.sha256.clone()),
        (EntryState::Done, Some(sha("v1")))
    );
    assert!(!entry.was_managed);
}

#[test]
fn store_journal_helpers_save_each_step() {
    let dir = tempfile::tempdir().unwrap();
    let store = RecordStore::new(dir.path());
    let target = json_target(&dir.path().join("settings.json"));
    let scope = Scope::Agent(AgentId::ClaudeDesktop);
    let mut record = InstallRecord::new();
    store
        .journal_begin(&mut record, scope, change(target.clone(), None, Some("v1")))
        .unwrap();
    assert_eq!(store.load().unwrap().unwrap().applying().len(), 1);
    store.journal_finish(&mut record, scope, &target).unwrap();
    assert_eq!(store.load().unwrap().unwrap(), record);
    let err = store
        .journal_finish(&mut record, scope, &target)
        .unwrap_err();
    assert_eq!(err, RecordError::NotApplying);
}

#[test]
fn value_types_reject_free_text() {
    assert!(Sha256Hex::try_from("sk-abc".to_owned()).is_err());
    assert!(Sha256Hex::try_from("A".repeat(64)).is_err());
    assert!(ReleaseTag::try_from("api key here".to_owned()).is_err());
    assert!(ReleaseTag::try_from("v0.5.0-rc.1".to_owned()).is_ok());
    assert!(UtcTimestamp::try_from("yesterday".to_owned()).is_err());
    assert!(SectionId::try_from("Routing".to_owned()).is_err());
    assert!(KeySegment::try_from("a\nb").is_err());
    assert!(BackupPath::try_from("../x".to_owned()).is_err());
    assert!(BackupPath::try_from("backups/../x".to_owned()).is_err());
    assert!(BackupPath::try_from("backups/C:/x".to_owned()).is_err());
    let err = Sha256Hex::try_from("sk-secret-value".to_owned()).unwrap_err();
    assert!(!err.to_string().contains("sk-secret"));
}

#[test]
fn backup_paths_round_trip_through_the_record_folder() {
    let dir = tempfile::tempdir().unwrap();
    let store = RecordStore::new(dir.path());
    let source = dir.path().join("config.toml");
    std::fs::write(&source, b"x = 1\n").unwrap();
    let at = UtcTimestamp::try_from("2026-11-04T09:30:00Z".to_owned()).unwrap();
    let rel = store.backup(&source, &at).unwrap().unwrap();
    assert_eq!(rel.as_str(), "backups/2026-11-04T09-30-00Z/config.toml");
    assert_eq!(std::fs::read(rel.resolve(store.dir())).unwrap(), b"x = 1\n");
    assert_eq!(store.backup(&dir.path().join("absent"), &at).unwrap(), None);
    let outside = BackupPath::from_absolute(store.dir(), Path::new("/elsewhere/x"));
    assert!(outside.is_err());
}
