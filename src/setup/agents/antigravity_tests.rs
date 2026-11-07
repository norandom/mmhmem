//! Tests of the Antigravity integration (reliable-installers task 3.5).

use super::*;
use crate::setup::agents::doubles::TestHome;
use crate::setup::record::{AgentRecord, Os};

fn setup() -> (TestHome, Antigravity) {
    let home = TestHome::new(Os::Windows);
    let agent = Antigravity::new(home.bin().join("mmhmem.exe"));
    (home, agent)
}

#[test]
fn detect_by_program_by_gemini_folder_and_not_at_all() {
    let (home, agent) = setup();
    assert!(!agent.detect(&home.env).is_found());
    home.write(&[".gemini", "settings.json"], "{}");
    assert!(agent.detect(&home.env).is_found());
    let (other, agent) = setup();
    let program = other.install_program("antigravity");
    match agent.detect(&other.env) {
        Detection::Found { paths, .. } => assert_eq!(paths, [program]),
        found => panic!("{found:?}"),
    }
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
