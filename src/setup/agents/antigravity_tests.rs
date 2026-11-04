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
fn nothing_is_planned_or_removed_and_the_user_step_is_pending() {
    let (home, agent) = setup();
    home.write(&[".gemini", "settings.json"], "{}");
    let want = Selection::new().with(AgentId::Antigravity, &[Feature::Mcp]);
    assert!(agent.plan(&home.env, &want, None).unwrap().is_empty());
    let rec = AgentRecord::default();
    assert!(agent.plan_remove(&home.env, &rec).is_empty());
    assert_eq!(
        Antigravity::pending_step(),
        PendingUserStep::ImportAntigravitySnippet
    );
    assert!(Antigravity::removal_instruction().contains("Antigravity"));
}
