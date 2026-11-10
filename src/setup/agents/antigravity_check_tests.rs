//! Tests of the snippet-era migration and the stale-location cleanup
//! (antigravity-native-setup task 3.4; Requirements 2.3, 5.1, 5.2 and 5.4).
//! Temporary homes only.

use super::check::{is_snippet, stale_entry};
use super::hook::hook_target;
use super::tests::{agent_record, at, drift_is_clean, run_apply, server_file, setup};
use super::*;
use crate::setup::agents::doubles::TestHome;
use crate::setup::record::{
    EntryState, InstallRecord, Os, RecordStore, Sha256Hex, key_path, parse_record,
};

/// The owner's stale server file: the pasted snippet between two servers.
const STALE: &str =
    include_str!("../../../tests/fixtures/setup/agents/antigravity/stale-mcp_config.json");

/// The owner's snippet-era record (research.md "Existing install on the
/// owner's machine"): nothing managed and the import step pending.
const OWNER_RECORD: &str = r#"{
  "schema": 1,
  "release": "v0.5.0",
  "binary": null,
  "path_entry": null,
  "agents": {
    "antigravity": {
      "features": {"mcp": true},
      "managed": [],
      "pending_user_steps": ["import_antigravity_snippet"],
      "last_check": {
        "at": "2026-11-06T10:00:00Z",
        "result": "user_steps_pending",
        "details": [
          {"step": "registration", "outcome": "pass"},
          {"step": "handshake", "outcome": "pass"},
          {"step": "tools_list", "outcome": "pass"},
          {"step": "status_tool", "outcome": "pass"}
        ]
      }
    }
  }
}"#;

const ID: AgentId = AgentId::Antigravity;

fn want(features: &[Feature]) -> Selection {
    Selection::new().with(ID, features)
}

fn want_both() -> Selection {
    want(&[Feature::Mcp, Feature::ReadHook])
}

/// `~/.gemini/config/hooks.json`, spelled out.
fn hooks_file(home: &TestHome) -> PathBuf {
    home.home()
        .join(".gemini")
        .join("config")
        .join("hooks.json")
}

/// `~/.gemini/antigravity/mcp_config.json`, spelled out.
fn stale_path(home: &TestHome) -> PathBuf {
    home.home()
        .join(".gemini")
        .join("antigravity")
        .join("mcp_config.json")
}

fn seed_stale(home: &TestHome, text: &str) -> PathBuf {
    home.write(&[".gemini", "antigravity", "mcp_config.json"], text)
}

fn read(file: &Path) -> String {
    std::fs::read_to_string(file).expect("read file")
}

/// Each change's file and action, in plan order.
fn planned(changes: &[Change]) -> Vec<(PathBuf, Action)> {
    changes.iter().map(|c| (c.file.clone(), c.action)).collect()
}

/// A record that manages the snippet file `file` (which may be gone).
fn snippet_record(file: &Path, text: &str) -> AgentRecord {
    let mut rec = AgentRecord::default();
    rec.features.insert(Feature::Mcp, true);
    rec.managed.push(ManagedEntry {
        target: Target::File {
            file: file.to_path_buf(),
        },
        sha256: Some(Sha256Hex::of(text.as_bytes())),
        before_sha256: None,
        was_managed: false,
        backup: None,
        missing_parents: None,
        created_file: true,
        state: EntryState::Done,
    });
    rec
}

/// Saves the snippet next to `agent`'s binary, as installs from 0.5.1 on
/// did, and returns it with a record that manages it.
fn snippet_era(agent: &Antigravity) -> (PathBuf, AgentRecord) {
    let file = agent.snippet_file();
    let text = agent.snippet();
    std::fs::write(&file, &text).expect("snippet file");
    let rec = snippet_record(&file, &text);
    (file, rec)
}

#[test]
fn a_snippet_era_record_plans_both_entries_and_the_snippet_removal() {
    let (home, agent) = setup();
    let (snippet, rec) = snippet_era(&agent);
    let changes = agent.plan(&home.env, &want_both(), Some(&rec)).unwrap();
    assert_eq!(
        planned(&changes),
        [
            (snippet.clone(), Action::Remove),
            (server_file(&home), Action::Add),
            (hooks_file(&home), Action::Add),
        ]
    );
    assert_eq!(changes[0].target, rec.managed[0].target);
    assert_eq!(changes[0].after, None, "the whole file goes");
    let mut record = InstallRecord::new();
    record.agents.insert(ID, rec);
    run_apply(&home, changes, &mut record);
    assert!(!snippet.exists());
    let targets: Vec<Target> = agent_record(&record)
        .managed
        .into_iter()
        .map(|e| e.target)
        .collect();
    let hooks = hook_target(&hooks_file(&home));
    assert_eq!(targets, [mcp_target(&server_file(&home)), hooks]);
    drift_is_clean(&record);
    let rec = agent_record(&record);
    let again = agent.plan(&home.env, &want_both(), Some(&rec)).unwrap();
    assert!(again.is_empty(), "{:?}", planned(&again));
}

#[test]
fn a_snippet_next_to_a_moved_binary_is_still_removed() {
    let (home, old) = setup();
    let (snippet, mut rec) = snippet_era(&old);
    let moved = Antigravity::new(home.bin().join("new").join("mmhmem.exe"));
    assert_ne!(moved.snippet_file(), snippet);
    assert!(is_snippet(&snippet));
    for selection in [want(&[Feature::Mcp]), want(&[Feature::ReadHook]), want(&[])] {
        let changes = moved.plan(&home.env, &selection, Some(&rec)).unwrap();
        let removals: Vec<&Change> = changes.iter().filter(|c| c.file == snippet).collect();
        assert_eq!(removals.len(), 1, "{selection:?}");
        assert_eq!(removals[0].action, Action::Remove, "{selection:?}");
    }
    let removes = moved.plan_remove(&home.env, &rec);
    assert_eq!(planned(&removes), [(snippet.clone(), Action::Remove)]);

    // A snippet already deleted by hand: nothing to remove.
    std::fs::remove_file(&snippet).unwrap();
    assert!(moved.plan_remove(&home.env, &rec).is_empty());

    // Another whole file in the record is not the snippet: never touched.
    let other = home.bin().join("mcp_config.json");
    std::fs::write(&other, "{}").unwrap();
    assert!(!is_snippet(&other));
    rec.managed[0].target = Target::File {
        file: other.clone(),
    };
    assert!(moved.plan_remove(&home.env, &rec).is_empty());
    let changes = moved.plan(&home.env, &want(&[]), Some(&rec)).unwrap();
    assert!(changes.is_empty(), "{:?}", planned(&changes));
}

#[test]
fn an_old_record_with_the_import_step_is_read_and_saved_without_it() {
    let (home, agent) = setup();
    let mut record = parse_record(OWNER_RECORD.as_bytes()).expect("old records still load");
    let rec = agent_record(&record);
    assert_eq!(
        rec.pending_user_steps,
        [PendingUserStep::ImportAntigravitySnippet]
    );
    for selection in [want_both(), want(&[Feature::Mcp]), want(&[])] {
        assert!(agent.pending_steps(&selection).is_empty(), "{selection:?}");
    }
    let changes = agent.plan(&home.env, &want_both(), Some(&rec)).unwrap();
    assert_eq!(
        planned(&changes),
        [
            (server_file(&home), Action::Add),
            (hooks_file(&home), Action::Add),
        ]
    );
    // The apply saves the record (journal): the old step is not written.
    run_apply(&home, changes, &mut record);
    let store = RecordStore::new(home.dir.path().join("state"));
    let saved = read(&store.record_path());
    assert!(!saved.contains("import_antigravity_snippet"), "{saved}");
    let loaded = store.load().unwrap().expect("a saved record");
    let rec = agent_record(&loaded);
    assert!(rec.pending_user_steps.is_empty());
    assert_eq!(rec.managed.len(), 2);
}

/// `text` without the lines of the `"mmhmem": {…}` object (it starts on the
/// line naming it and ends on the first line closing an object).
fn without_mmhmem_block(text: &str) -> String {
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let start = lines
        .iter()
        .position(|l| l.contains("\"mmhmem\""))
        .expect("an mmhmem entry");
    let end = (start..lines.len())
        .find(|&i| lines[i].trim_start().starts_with('}'))
        .expect("its closing brace");
    let mut kept = lines[..start].to_vec();
    kept.extend(&lines[end + 1..]);
    kept.concat()
}

/// mmhmem's whole entry in `stale`, as the cleanup removes it.
fn stale_target(stale: &Path) -> Target {
    Target::JsonEntry {
        file: stale.to_path_buf(),
        pointer: key_path(&["mcpServers", "mmhmem"]).unwrap(),
        managed_keys: None,
    }
}

/// Checks the planned removal in `stale` (seeded with `text`) takes out
/// mmhmem's whole entry and nothing else; returns the file after it.
fn assert_stale_removal(name: &str, change: &Change, stale: &Path, text: &str) -> String {
    assert_eq!(change.action, Action::Remove, "{name}");
    assert_eq!(change.target, stale_target(stale), "{name}");
    assert_eq!(change.before.as_deref(), Some(text.as_bytes()), "{name}");
    assert_eq!(change.missing_parents, None, "{name}");
    let summary = format!(
        "Antigravity no longer reads this file; remove the mmhmem entry ({})",
        stale.display()
    );
    assert_eq!(change.summary, summary, "{name}");
    let expected = without_mmhmem_block(text);
    let after = change.after.as_deref().map(String::from_utf8_lossy);
    assert_eq!(after.as_deref(), Some(expected.as_str()), "{name}");
    expected
}

#[test]
fn a_stale_entry_naming_mmhmem_is_the_only_thing_removed_there() {
    let lf = STALE.replace("\r\n", "\n");
    let crlf = lf.replace('\n', "\r\n");
    for (name, text) in [("lf", lf), ("crlf", crlf)] {
        let (home, agent) = setup();
        let stale = seed_stale(&home, &text);
        let changes = agent.plan(&home.env, &want(&[Feature::Mcp]), None).unwrap();
        assert_eq!(changes.len(), 2, "{name}: the server and the stale entry");
        let removals: Vec<&Change> = changes.iter().filter(|c| c.file == stale).collect();
        assert_eq!(removals.len(), 1, "{name}");
        let expected = assert_stale_removal(name, removals[0], &stale, &text);
        assert_eq!(
            stale_entry(&stale, &agent.binary).as_ref(),
            Some(removals[0])
        );

        let mut record = InstallRecord::new();
        run_apply(&home, changes, &mut record);
        assert_eq!(read(&stale), expected, "{name}: the rest byte-identical");
        for server in ["files", "notes"] {
            let path = ["mcpServers", server];
            assert_eq!(at(&expected, &path), at(&text, &path), "{name}");
        }
        let rec = agent_record(&record);
        assert_eq!(rec.managed.len(), 1, "{name}: the removal is not recorded");
        assert_ne!(rec.managed[0].target, stale_target(&stale), "{name}");
        drift_is_clean(&record);
        assert_eq!(stale_entry(&stale, &agent.binary), None, "{name}");
        let again = agent.plan(&home.env, &want(&[Feature::Mcp]), Some(&rec));
        assert!(again.unwrap().is_empty(), "{name}: a re-plan is empty");
    }
}

#[test]
fn a_stale_entry_is_matched_by_the_program_name_in_either_slash_style() {
    let (home, _agent) = setup();
    let stale = stale_path(&home);
    let custom = Antigravity::new(home.bin().join("custom-name.exe"));
    let commands = [
        r"C:\Users\<user>\AppData\Local\Programs\mmhmem\mmhmem.exe".to_owned(),
        "C:/old/place/MMHMEM.EXE".to_owned(),
        "/usr/local/bin/mmhmem".to_owned(),
        "mmhmem".to_owned(),
        // The installed program itself, whatever its name.
        custom.binary.to_string_lossy().into_owned(),
    ];
    for command in commands {
        let text = json!({"mcpServers": {"mmhmem": {"command": command, "args": ["stdio"]}}});
        seed_stale(&home, &serde_json::to_string_pretty(&text).unwrap());
        assert!(stale_entry(&stale, &custom.binary).is_some(), "{command}");
    }
}

#[test]
fn a_stale_mmhmem_entry_naming_another_program_is_left_alone() {
    let (home, agent) = setup();
    let stale = stale_path(&home);
    let commands = [
        json!(r"C:\tools\other\server.exe"),
        json!("/opt/mmhmem/bin/mmhmem-dev"),
        json!("C:\\mmhmem\\"),
        json!("npx"),
        json!(["mmhmem"]),
        json!(null),
    ];
    for command in commands {
        let text = json!({"mcpServers": {"mmhmem": {"command": command, "args": ["stdio"]}}});
        seed_stale(&home, &serde_json::to_string_pretty(&text).unwrap());
        assert_eq!(stale_entry(&stale, &agent.binary), None, "{command}");
        let changes = agent.plan(&home.env, &want(&[Feature::Mcp]), None).unwrap();
        assert!(changes.iter().all(|c| c.file != stale), "{command}");
    }
    // An entry without a command, and a file without mmhmem's entry.
    for text in [
        r#"{"mcpServers": {"mmhmem": {"url": "http://localhost:1/mcp"}}}"#,
        r#"{"mcpServers": {"files": {"command": "mmhmem"}}}"#,
    ] {
        seed_stale(&home, text);
        assert_eq!(stale_entry(&stale, &agent.binary), None, "{text}");
    }
}

#[test]
fn a_missing_unreadable_or_invalid_stale_file_is_ignored() {
    let (home, agent) = setup();
    let stale = stale_path(&home);
    assert_eq!(stale_entry(&stale, &agent.binary), None, "missing");
    for text in [
        "{ \"mcpServers\": ",
        "[1]",
        "",
        r#"{"mcpServers": ["mmhmem"]}"#,
    ] {
        seed_stale(&home, text);
        assert_eq!(stale_entry(&stale, &agent.binary), None, "{text:?}");
        let changes = agent.plan(&home.env, &want(&[Feature::Mcp]), None);
        let changes = changes.expect("never a problem for the agent");
        assert!(changes.iter().all(|c| c.file != stale), "{text:?}");
    }
    std::fs::write(&stale, [0xff, 0xfe, b'{']).unwrap();
    assert_eq!(stale_entry(&stale, &agent.binary), None, "not UTF-8");
    std::fs::remove_file(&stale).unwrap();
    std::fs::create_dir_all(&stale).unwrap();
    assert_eq!(stale_entry(&stale, &agent.binary), None, "a folder");
    assert!(agent.plan(&home.env, &want(&[Feature::Mcp]), None).is_ok());
}

/// The changes of `changes` to the stale file, as (file, action).
fn stale_changes(changes: &[Change], stale: &Path) -> Vec<(PathBuf, Action)> {
    let mut all = planned(changes);
    all.retain(|(file, _)| file == stale);
    all
}

#[test]
fn the_stale_entry_goes_whatever_is_selected_and_on_removal() {
    let (home, agent) = setup();
    let changes = agent.plan(&home.env, &want(&[Feature::Mcp]), None).unwrap();
    let mut record = InstallRecord::new();
    run_apply(&home, changes, &mut record);
    let rec = agent_record(&record);
    let stale = seed_stale(&home, STALE);
    let once = [(stale.clone(), Action::Remove)];
    let selections = [want(&[Feature::Mcp]), want(&[Feature::ReadHook]), want(&[])];
    for selection in selections {
        let changes = agent.plan(&home.env, &selection, Some(&rec)).unwrap();
        assert_eq!(stale_changes(&changes, &stale), once, "{selection:?}");
    }
    let hook_only = agent.plan(&home.env, &want(&[Feature::ReadHook]), None);
    assert_eq!(stale_changes(&hook_only.unwrap(), &stale), once);
    let removal = agent.plan_remove(&home.env, &rec);
    assert_eq!(stale_changes(&removal, &stale), once);
    let removal = agent.plan_remove(&home.env, &AgentRecord::default());
    assert_eq!(planned(&removal), once);
}

#[test]
fn a_recorded_hook_whose_command_can_no_longer_be_written_is_taken_out() {
    let home = TestHome::new(Os::Linux);
    let agent = Antigravity::new(home.bin().join("mmhmem"));
    let changes = agent.plan(&home.env, &want_both(), None).unwrap();
    let mut record = InstallRecord::new();
    run_apply(&home, changes, &mut record);
    let hooks = hooks_file(&home);
    assert!(at(&read(&hooks), &[HOOK_NAME]).is_some());

    // The program moved to a folder with a space: no hook command can name
    // it, and the recorded hook would start a program that is gone.
    let moved = Antigravity::new(home.bin().join("my tools").join("mmhmem"));
    let rec = agent_record(&record);
    let changes = moved.plan(&home.env, &want_both(), Some(&rec)).unwrap();
    assert_eq!(
        planned(&changes),
        [
            (hooks.clone(), Action::Remove),
            (server_file(&home), Action::Update),
        ]
    );
    let problem = AgentProblem::HookCommandUnrepresentable {
        file: moved.binary.clone(),
    };
    assert_eq!(moved.feature_problems(&home.env, &want_both()), [problem]);
    run_apply(&home, changes, &mut record);
    assert_eq!(at(&read(&hooks), &[HOOK_NAME]), None);
    let rec = agent_record(&record);
    let targets: Vec<&Target> = rec.managed.iter().map(|e| &e.target).collect();
    assert_eq!(targets, [&mcp_target(&server_file(&home))]);
    drift_is_clean(&record);
    let again = moved.plan(&home.env, &want_both(), Some(&rec)).unwrap();
    assert!(again.is_empty(), "{:?}", planned(&again));
}
