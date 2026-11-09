//! Tests of Antigravity's read-routing hook (antigravity-native-setup task
//! 3.3): the whole named hook in `~/.gemini/config/hooks.json`, its unquoted
//! command, and removal per feature. Temporary homes only.

use super::tests::{agent_record, at, drift_is_clean, run_apply, server_file, setup, wanted_entry};
use super::*;
use crate::setup::agents::doubles::TestHome;
use crate::setup::record::{InstallRecord, Os, key_path};

/// A hook file with two other named hooks, comments and odd formatting.
const OTHER_HOOKS: &str =
    include_str!("../../../tests/fixtures/setup/agents/antigravity/hooks-other-hooks.json");

const ID: AgentId = AgentId::Antigravity;

fn want(features: &[Feature]) -> Selection {
    Selection::new().with(ID, features)
}

fn want_both() -> Selection {
    want(&[Feature::Mcp, Feature::ReadHook])
}

/// `~/.gemini/config/hooks.json`, spelled out.
fn hook_file(home: &TestHome) -> PathBuf {
    home.home()
        .join(".gemini")
        .join("config")
        .join("hooks.json")
}

fn seed_hooks(home: &TestHome, text: &str) -> PathBuf {
    home.write(&[".gemini", "config", "hooks.json"], text)
}

/// The hook entry contract (design.md) for `agent`'s binary.
fn wanted_hook(agent: &Antigravity) -> Value {
    let command = format!("{} hook antigravity", agent.binary.display());
    json!({"PreToolUse": [{
        "matcher": "^read_file$",
        "hooks": [{"type": "command", "command": command, "timeout": 10}],
    }]})
}

fn read(file: &Path) -> String {
    std::fs::read_to_string(file).expect("read file")
}

/// Plans and applies `features`; returns the record.
fn install(home: &TestHome, agent: &Antigravity, features: &[Feature]) -> InstallRecord {
    let changes = agent.plan(&home.env, &want(features), None).unwrap();
    let mut record = InstallRecord::new();
    run_apply(home, changes, &mut record);
    record
}

#[test]
fn the_hook_is_the_contract_value_and_a_feature() {
    let (_home, agent) = setup();
    assert_eq!(HOOK_NAME, "mmhmem-read-routing");
    assert_eq!(HOOK_MATCHER, "^read_file$");
    assert_eq!(HOOK_TIMEOUT, 10);
    assert_eq!(agent.features(), [Feature::Mcp, Feature::ReadHook]);
    assert_eq!(agent.hook_entry(Os::Windows), Ok(wanted_hook(&agent)));
}

#[test]
fn a_whitespace_free_path_is_the_first_token_unquoted() {
    let unix = Path::new("/opt/mmhmem/bin/mmhmem");
    let windows = Path::new(r"C:\Users\me\AppData\Local\Programs\mmhmem\mmhmem.exe");
    for os in [Os::Linux, Os::MacOs, Os::Windows] {
        for binary in [unix, windows] {
            let command = hook_command(binary, os).unwrap();
            assert_eq!(command, format!("{} hook antigravity", binary.display()));
            assert!(!command.contains(['"', '\'']), "{command}");
        }
    }
}

#[test]
fn a_path_with_whitespace_off_windows_cannot_be_a_hook_command() {
    for os in [Os::Linux, Os::MacOs] {
        for binary in ["/home/a b/mmhmem", "/opt/x\ty/mmhmem", "/srv/\u{a0}/mmhmem"] {
            let problem = hook_command(Path::new(binary), os).unwrap_err();
            let file = PathBuf::from(binary);
            assert_eq!(problem, AgentProblem::HookCommandUnrepresentable { file });
        }
    }
    let problem = hook_command(Path::new("/home/a b/mmhmem"), Os::Linux).unwrap_err();
    let text = problem.to_string();
    for part in ["read hook", "Antigravity", "spaces", "/home/a b/mmhmem"] {
        assert!(text.contains(part), "{text}");
    }
    // A tab is shown escaped, never printed.
    let tab = hook_command(Path::new("/opt/x\ty/mmhmem"), Os::Linux).unwrap_err();
    assert!(tab.to_string().contains(r"/opt/x\ty/mmhmem"), "{tab}");
}

#[test]
fn a_windows_path_with_no_short_form_cannot_be_a_hook_command() {
    // The OS has no short form for a folder that does not exist (and none
    // at all off Windows), so the path keeps its space.
    let dir = tempfile::tempdir().expect("temp dir");
    let binary = dir.path().join("no such folder").join("mmhmem.exe");
    assert_eq!(
        hook_command(&binary, Os::Windows),
        Err(AgentProblem::HookCommandUnrepresentable {
            file: binary.clone()
        })
    );
}

/// The first token of `command` names the same file as `long`, has no
/// whitespace and is followed by `hook antigravity`.
#[cfg(windows)]
fn assert_short_form_of(command: &str, long: &Path) {
    let (program, args) = command.split_once(' ').expect("two parts");
    assert_eq!(args, "hook antigravity", "{command}");
    assert!(!program.contains(char::is_whitespace), "{command}");
    assert!(!command.contains('"'), "{command}");
    assert_ne!(Path::new(program), long);
    let same = |p: &Path| std::fs::canonicalize(p).expect("canonical path");
    assert_eq!(same(Path::new(program)), same(long), "{command}");
}

#[cfg(windows)]
#[test]
fn a_windows_path_with_a_space_becomes_its_short_form() {
    // A real temp folder with a space. A volume without 8.3 names (a Dev
    // Drive, or D: on the owner's machine) returns the long path unchanged:
    // then the hook cannot be written.
    let dir = tempfile::Builder::new()
        .prefix("mmhmem space ")
        .tempdir()
        .expect("temp dir with a space");
    let binary = dir.path().join("mmhmem.exe");
    std::fs::write(&binary, b"").expect("binary");
    let short = hook::short_path(&binary).expect("the OS answers for a real file");
    match hook_command(&binary, Os::Windows) {
        Ok(command) => assert_short_form_of(&command, &binary),
        Err(problem) => {
            assert!(short.contains(' '), "{short}: no 8.3 name on this volume");
            assert_eq!(
                problem,
                AgentProblem::HookCommandUnrepresentable { file: binary }
            );
        }
    }
    // The system volume keeps 8.3 names: `C:\Program Files` → `C:\PROGRA~1`.
    let program_files = PathBuf::from(std::env::var_os("ProgramFiles").expect("ProgramFiles"));
    assert!(program_files.to_string_lossy().contains(' '));
    let command = hook_command(&program_files, Os::Windows).unwrap();
    assert_short_form_of(&command, &program_files);
}

/// `%ProgramFiles%` and the OS's whitespace-free 8.3 form of it (the system
/// volume keeps 8.3 names). Read only: nothing is written on that volume.
#[cfg(windows)]
fn program_files() -> (PathBuf, String) {
    let long = PathBuf::from(std::env::var_os("ProgramFiles").expect("ProgramFiles"));
    assert!(long.to_string_lossy().contains(' '), "{}", long.display());
    let short = hook::os_short_path(&long).expect("the OS answers for an existing folder");
    assert!(!short.contains(char::is_whitespace), "{short}");
    (long, short)
}

/// An existing folder in `program_files` with a long name and no space.
#[cfg(windows)]
fn long_named_folder(program_files: &Path) -> PathBuf {
    let preferred = program_files.join("WindowsPowerShell");
    if preferred.is_dir() {
        return preferred;
    }
    let long_name = |p: &PathBuf| {
        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or_default();
        p.is_dir() && name.len() > 8 && !name.contains(char::is_whitespace)
    };
    std::fs::read_dir(program_files)
        .expect("list Program Files")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(long_name)
        .expect("a long-named folder in Program Files")
}

#[cfg(windows)]
#[test]
fn a_binary_not_copied_yet_under_a_spaced_folder_gets_the_short_form() {
    // A first install: the program is copied only when the plan is applied.
    let (long, short) = program_files();
    let binary = long.join("mmhmem-not-there").join("mmhmem.exe");
    assert!(!binary.exists());
    assert_eq!(
        hook_command(&binary, Os::Windows),
        Ok(format!(
            "{short}\\mmhmem-not-there\\mmhmem.exe hook antigravity"
        ))
    );
}

#[cfg(windows)]
#[test]
fn the_part_after_the_last_space_keeps_its_long_names() {
    // Whether the folders after the last space exist or not, they keep
    // their long names: the command is the same before and after the
    // program is copied.
    let (long, short) = program_files();
    let dir = long_named_folder(&long);
    let name = dir.file_name().and_then(|n| n.to_str()).expect("name");
    assert_eq!(
        hook_command(&dir, Os::Windows),
        Ok(format!("{short}\\{name} hook antigravity"))
    );
    let binary = dir.join("mmhmem-not-there.exe");
    assert!(!binary.exists());
    assert_eq!(
        hook_command(&binary, Os::Windows),
        Ok(format!(
            "{short}\\{name}\\mmhmem-not-there.exe hook antigravity"
        ))
    );
}

#[cfg(windows)]
#[test]
fn a_first_install_under_a_spaced_folder_plans_the_hook() {
    let (long, short) = program_files();
    let home = TestHome::new(Os::Windows);
    let agent = Antigravity::new(long.join("mmhmem-not-there").join("mmhmem.exe"));
    assert!(agent.feature_problems(&home.env, &want_both()).is_empty());
    let changes = agent.plan(&home.env, &want_both(), None).unwrap();
    assert_eq!(changes.len(), 2, "the server entry and the hook");
    assert_eq!(changes[1].file, hook_file(&home));
    let after = String::from_utf8(changes[1].after.clone().unwrap()).unwrap();
    let hook = at(&after, &[HOOK_NAME]).expect("the hook");
    let command = format!("{short}\\mmhmem-not-there\\mmhmem.exe hook antigravity");
    assert_eq!(hook["PreToolUse"][0]["hooks"][0]["command"], json!(command));
    // Applied in the temporary home only; the re-plan changes nothing.
    let mut record = InstallRecord::new();
    run_apply(&home, changes, &mut record);
    replans_to_nothing(&home, &agent, &record);
}

#[test]
fn a_hook_that_cannot_be_written_still_plans_the_server_entry() {
    let home = TestHome::new(Os::Linux);
    let agent = Antigravity::new(home.bin().join("my tools").join("mmhmem"));
    let changes = agent.plan(&home.env, &want_both(), None).unwrap();
    assert_eq!(changes.len(), 1, "only the server entry");
    assert_eq!(changes[0].file, server_file(&home));
    assert!(changes.iter().all(|c| c.file != hook_file(&home)));
    let problem = AgentProblem::HookCommandUnrepresentable {
        file: agent.binary.clone(),
    };
    assert_eq!(agent.feature_problems(&home.env, &want_both()), [problem]);
    // Not selected: nothing to report.
    let mcp = want(&[Feature::Mcp]);
    assert!(agent.feature_problems(&home.env, &mcp).is_empty());
    // MCP takes the command as a JSON string: a space is fine there.
    let mut record = InstallRecord::new();
    run_apply(&home, changes, &mut record);
    let installed = read(&server_file(&home));
    let entry = at(&installed, &["mcpServers", "mmhmem"]);
    assert_eq!(entry, Some(wanted_entry(&agent)));
    assert!(!hook_file(&home).exists());
    // A whitespace-free path has nothing to report.
    let (home, agent) = setup();
    assert!(agent.feature_problems(&home.env, &want_both()).is_empty());
}

#[test]
fn a_missing_hook_file_is_created_with_only_the_mmhmem_hook() {
    let (home, agent) = setup();
    let changes = agent.plan(&home.env, &want_both(), None).unwrap();
    assert_eq!(changes.len(), 2, "the server entry and the hook");
    assert_eq!(changes[0].file, server_file(&home));
    let change = &changes[1];
    let file = hook_file(&home);
    assert_eq!(change.file, file);
    assert_eq!(
        change.target,
        Target::JsonEntry {
            file: file.clone(),
            pointer: key_path(&["mmhmem-read-routing"]).unwrap(),
            managed_keys: None,
        }
    );
    assert_eq!(change.action, Action::Add);
    assert!(change.before.is_none());
    assert_eq!(change.missing_parents, Some(0));
    let summary = &change.summary;
    assert!(
        summary.contains("Antigravity") && summary.contains("hooks.json"),
        "{summary}"
    );
    let after = String::from_utf8(change.after.clone().unwrap()).unwrap();
    let whole: Value = serde_json::from_str(&after).expect("plain JSON");
    assert_eq!(whole, json!({ HOOK_NAME: wanted_hook(&agent) }));
    // Two-space indents, nothing but mmhmem's hook, no quotes in the command.
    let command = format!("{} hook antigravity", agent.binary.display());
    let command = serde_json::to_string(&command).unwrap();
    let expected = format!(
        "{{\n  \"mmhmem-read-routing\": {{\n    \"PreToolUse\": [\n      {{\n        \
         \"hooks\": [\n          {{\n            \"command\": {command},\n            \
         \"timeout\": 10,\n            \"type\": \"command\"\n          }}\n        ],\n        \
         \"matcher\": \"^read_file$\"\n      }}\n    ]\n  }}\n}}\n"
    );
    assert_eq!(after, expected);
}

/// Re-planning both unchanged, recorded entries changes nothing.
fn replans_to_nothing(home: &TestHome, agent: &Antigravity, record: &InstallRecord) {
    drift_is_clean(record);
    let rec = agent_record(record);
    let changes = agent.plan(&home.env, &want_both(), Some(&rec)).unwrap();
    assert!(changes.is_empty(), "{changes:?}");
}

#[test]
fn a_created_hook_file_is_recorded_and_removal_leaves_an_empty_object() {
    let (home, agent) = setup();
    let mut record = install(&home, &agent, &[Feature::Mcp, Feature::ReadHook]);
    let rec = agent_record(&record);
    assert_eq!(rec.managed.len(), 2);
    let hook = &rec.managed[1];
    assert_eq!(hook.target, hook::hook_target(&hook_file(&home)));
    assert_eq!(hook.missing_parents, Some(0));
    assert!(hook.created_file);
    replans_to_nothing(&home, &agent, &record);

    run_apply(&home, agent.plan_remove(&home.env, &rec), &mut record);
    let left: Value = serde_json::from_str(&read(&hook_file(&home))).unwrap();
    assert_eq!(left, json!({}));
    assert!(agent_record(&record).managed.is_empty());
}

#[test]
fn an_empty_hook_file_takes_the_hook_and_gives_it_back() {
    // The owner's machine: `~/.gemini/config/hooks.json` is `{}`.
    for seeded in ["{}", "{}\n", "{}\r\n"] {
        let (home, agent) = setup();
        let file = seed_hooks(&home, seeded);
        let changes = agent.plan(&home.env, &want(&[Feature::ReadHook]), None);
        let changes = changes.unwrap();
        assert_eq!(changes.len(), 1, "{seeded:?}");
        assert_eq!(changes[0].action, Action::Add, "{seeded:?}");
        assert_eq!(changes[0].before.as_deref(), Some(seeded.as_bytes()));
        let mut record = InstallRecord::new();
        run_apply(&home, changes, &mut record);
        let installed = read(&file);
        assert_eq!(at(&installed, &[HOOK_NAME]), Some(wanted_hook(&agent)));
        assert!(!agent_record(&record).managed[0].created_file);

        run_apply(
            &home,
            agent.plan_remove(&home.env, &agent_record(&record)),
            &mut record,
        );
        assert_eq!(read(&file), seeded, "byte-identical");
    }
}

/// The golden file as committed (line endings normalised to LF), with CRLF
/// line endings, and with trailing blanks.
fn golden_variants() -> [(&'static str, String); 3] {
    let lf = OTHER_HOOKS.replace("\r\n", "\n");
    let crlf = lf.replace('\n', "\r\n");
    let blanks = format!("{}\n\n", lf.replace(",\n", ", \t\n"));
    [("lf", lf), ("crlf", crlf), ("trailing blanks", blanks)]
}

#[test]
fn add_then_remove_is_byte_identical_around_other_hooks() {
    for (name, seeded) in golden_variants() {
        let (home, agent) = setup();
        let file = seed_hooks(&home, &seeded);
        let changes = agent.plan(&home.env, &want(&[Feature::ReadHook]), None);
        let changes = changes.unwrap();
        assert_eq!(changes.len(), 1, "{name}");
        assert_eq!(changes[0].action, Action::Add, "{name}");
        assert_eq!(changes[0].before.as_deref(), Some(seeded.as_bytes()));
        let mut record = InstallRecord::new();
        run_apply(&home, changes, &mut record);

        let installed = read(&file);
        assert_eq!(at(&installed, &[HOOK_NAME]), Some(wanted_hook(&agent)));
        for other in ["lint-checker", "audit-commands"] {
            assert_eq!(at(&installed, &[other]), at(&seeded, &[other]), "{name}");
        }
        assert!(installed.contains("// Hooks the user added"), "{name}");
        assert!(installed.contains("/* off for now */"), "{name}");
        drift_is_clean(&record);
        let rec = agent_record(&record);
        let again = agent.plan(&home.env, &want(&[Feature::ReadHook]), Some(&rec));
        assert!(again.unwrap().is_empty(), "{name}");

        let removal = agent.plan_remove(&home.env, &rec);
        assert_eq!(removal.len(), 1, "{name}");
        assert_eq!(removal[0].action, Action::Remove, "{name}");
        run_apply(&home, removal, &mut record);
        assert_eq!(read(&file), seeded, "{name}");
        assert!(agent_record(&record).managed.is_empty(), "{name}");
    }
}

#[test]
fn a_changed_hook_or_a_moved_binary_is_put_back() {
    let (home, agent) = setup();
    let mut record = install(&home, &agent, &[Feature::Mcp, Feature::ReadHook]);
    let moved = Antigravity::new(home.bin().join("new").join("mmhmem.exe"));
    let rec = agent_record(&record);
    let changes = moved.plan(&home.env, &want_both(), Some(&rec)).unwrap();
    assert_eq!(changes.len(), 2);
    assert!(changes.iter().all(|c| c.action == Action::Update));
    run_apply(&home, changes, &mut record);
    let installed = read(&hook_file(&home));
    assert_eq!(at(&installed, &[HOOK_NAME]), Some(wanted_hook(&moved)));

    // The user edited mmhmem's hook: the whole value is mmhmem's.
    let edited = installed.replace("^read_file$", "^view_file$");
    seed_hooks(&home, &edited);
    let rec = agent_record(&record);
    let changes = moved.plan(&home.env, &want_both(), Some(&rec)).unwrap();
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].file, hook_file(&home));
    assert_eq!(changes[0].action, Action::Update);
}

#[test]
fn deselecting_one_feature_removes_only_its_entry() {
    for (keep, gone) in [
        (Feature::Mcp, Feature::ReadHook),
        (Feature::ReadHook, Feature::Mcp),
    ] {
        let (home, agent) = setup();
        let mut record = install(&home, &agent, &[Feature::Mcp, Feature::ReadHook]);
        let (kept_file, gone_file) = match keep {
            Feature::Mcp => (server_file(&home), hook_file(&home)),
            _ => (hook_file(&home), server_file(&home)),
        };
        let kept_bytes = read(&kept_file);
        let rec = agent_record(&record);
        let changes = agent.plan(&home.env, &want(&[keep]), Some(&rec)).unwrap();
        assert_eq!(changes.len(), 1, "{gone:?}");
        assert_eq!(changes[0].action, Action::Remove, "{gone:?}");
        assert_eq!(changes[0].file, gone_file, "{gone:?}");
        run_apply(&home, changes, &mut record);
        assert_eq!(read(&kept_file), kept_bytes, "{gone:?}");
        let left: Value = serde_json::from_str(&read(&gone_file)).unwrap();
        assert_eq!(left, json!({}), "{gone:?}");
        let managed = agent_record(&record).managed;
        assert_eq!(managed.len(), 1, "{gone:?}");
        assert_eq!(
            managed[0].target,
            rec.managed[usize::from(keep == Feature::ReadHook)].target
        );
    }
}

#[test]
fn an_invalid_hook_file_is_a_problem_naming_the_file() {
    let (home, agent) = setup();
    for text in ["{ \"secret-token-123\": ", "[1]"] {
        let file = seed_hooks(&home, text);
        let problem = agent.plan(&home.env, &want_both(), None).unwrap_err();
        assert!(matches!(&problem, AgentProblem::Json { file: f, .. } if *f == file));
        let shown = problem.to_string();
        assert!(shown.contains("hooks.json"), "{shown}");
        assert!(!shown.contains("secret-token-123"), "{shown}");
        assert_eq!(read(&file), text, "nothing was written");
    }
    assert!(!server_file(&home).exists());
}
