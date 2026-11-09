//! Tests of the Antigravity integration (reliable-installers task 3.5).

use super::*;
use crate::setup::agents::doubles::{Reply, TestHome};
use crate::setup::record::{AgentRecord, Os};

/// `agy --version` as captured from agy 1.5.3 (task 1.1).
const AGY_VERSION: &str =
    include_str!("../../../tests/fixtures/setup/agents/antigravity/agy-version.txt");

/// The CLI, app and IDE data folders under the Gemini home, in that order.
const DATA_FOLDERS: [&str; 3] = ["antigravity-cli", "antigravity", "antigravity-ide"];

fn setup() -> (TestHome, Antigravity) {
    let home = TestHome::new(Os::Windows);
    let agent = Antigravity::new(home.bin().join("mmhmem.exe"));
    (home, agent)
}

/// Creates `~/.gemini/<folder>` and returns it.
fn data_folder(home: &TestHome, folder: &str) -> PathBuf {
    let dir = home.home().join(".gemini").join(folder);
    std::fs::create_dir_all(&dir).expect("data folder");
    dir
}

#[test]
fn detects_agy_with_its_version_and_lists_the_program_first() {
    let (home, agent) = setup();
    let folders: Vec<PathBuf> = DATA_FOLDERS.iter().map(|f| data_folder(&home, f)).collect();
    let program = home.install_program("agy");
    home.runner
        .on("agy", &["--version"], vec![Reply::ok(AGY_VERSION)]);
    let mut paths = vec![program];
    paths.extend(folders);
    assert_eq!(
        agent.detect(&home.env),
        Detection::Found {
            version: Some("1.5.3".to_owned()),
            paths
        }
    );
    assert_eq!(home.runner.args_of("agy"), [["--version"]]);
}

#[test]
fn agy_whose_version_call_fails_is_still_found() {
    let (home, agent) = setup();
    let program = home.install_program("agy");
    let found = Detection::Found {
        version: None,
        paths: vec![program],
    };
    // Unscripted: running `agy --version` fails to start.
    assert_eq!(agent.detect(&home.env), found);
    home.runner
        .on("agy", &["--version"], vec![Reply::exit(1, "", "boom")]);
    assert_eq!(agent.detect(&home.env), found);
    // Exit 0 with nothing printed: no version to show.
    home.runner
        .on("agy", &["--version"], vec![Reply::ok(" \n")]);
    assert_eq!(agent.detect(&home.env), found);
}

#[test]
fn each_data_folder_alone_is_found_without_a_version() {
    for folder in DATA_FOLDERS {
        let (home, agent) = setup();
        let dir = data_folder(&home, folder);
        assert_eq!(
            agent.detect(&home.env),
            Detection::Found {
                version: None,
                paths: vec![dir]
            },
            "{folder}"
        );
        assert!(home.runner.calls().is_empty(), "{folder}");
    }
}

#[test]
fn the_bare_gemini_home_is_not_antigravity() {
    let (home, agent) = setup();
    assert!(!agent.detect(&home.env).is_found());
    // What the Gemini CLI (or another tool) leaves behind.
    home.write(&[".gemini", "settings.json"], "{}");
    home.write(&[".gemini", "GEMINI.md"], "# notes");
    // A file, not a folder, with a data folder's name proves nothing either.
    home.write(&[".gemini", "antigravity"], "");
    assert!(matches!(
        agent.detect(&home.env),
        Detection::NotFound { reason } if reason == "Antigravity is not installed"
    ));
}

#[test]
fn the_snippet_names_the_binary_and_stdio() {
    let (_home, agent) = setup();
    let value: Value = serde_json::from_str(&agent.snippet()).unwrap();
    let entry = &value["mcpServers"]["mmhmem"];
    assert_eq!(entry["command"], agent.binary.to_string_lossy().as_ref());
    assert_eq!(entry["args"], json!(["stdio"]));
    assert!(agent.snippet().contains("mmhmem"));
}

#[test]
fn the_snippet_file_sits_next_to_the_binary() {
    let (_home, agent) = setup();
    let file = agent.snippet_file();
    assert_eq!(
        file.file_name().and_then(|n| n.to_str()),
        Some(SNIPPET_FILE)
    );
    assert_eq!(file.parent(), agent.binary.parent());
}

#[test]
fn plan_saves_the_snippet_and_removal_takes_it_back() {
    let (home, agent) = setup();
    home.write(&[".gemini", "settings.json"], "{}");
    let want = Selection::new().with(AgentId::Antigravity, &[Feature::Mcp]);

    // Not selected: nothing planned.
    let none = Selection::new();
    assert!(agent.plan(&home.env, &none, None).unwrap().is_empty());

    // Selected: one whole-file change with the snippet as its bytes.
    let changes = agent.plan(&home.env, &want, None).unwrap();
    assert_eq!(changes.len(), 1);
    let change = &changes[0];
    assert_eq!(change.action, Action::Add);
    assert_eq!(change.file, agent.snippet_file());
    assert!(change.before.is_none());
    assert_eq!(change.after.as_deref(), Some(agent.snippet().as_bytes()));

    // Idempotent: the recorded file with the same bytes plans nothing.
    let file = agent.snippet_file();
    std::fs::write(&file, agent.snippet()).expect("snippet file");
    let mut rec = AgentRecord::default();
    rec.managed.push(recorded_file(&agent));
    assert!(agent.plan(&home.env, &want, Some(&rec)).unwrap().is_empty());

    // Removal takes the file back out.
    let removes = agent.plan_remove(&home.env, &rec);
    assert_eq!(removes.len(), 1);
    assert_eq!(removes[0].action, Action::Remove);
    assert_eq!(removes[0].after, None);

    assert_eq!(
        Antigravity::pending_step(),
        PendingUserStep::ImportAntigravitySnippet
    );
    assert!(Antigravity::removal_instruction().contains("Antigravity"));
}

/// A record entry saying the snippet file is managed and written.
fn recorded_file(agent: &Antigravity) -> crate::setup::record::ManagedEntry {
    crate::setup::record::ManagedEntry {
        target: Target::File {
            file: agent.snippet_file(),
        },
        sha256: Some(crate::setup::record::Sha256Hex::of(
            agent.snippet().as_bytes(),
        )),
        before_sha256: None,
        was_managed: false,
        backup: None,
        missing_parents: None,
        created_file: true,
        state: crate::setup::record::EntryState::Done,
    }
}
