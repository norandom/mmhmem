//! Tests of the Antigravity integration: detection (antigravity-native-setup
//! task 3.1) and the MCP entry in Antigravity's global server file (task 3.2).
//! Temporary homes only.

use super::*;
use crate::setup::agents::doubles::{Reply, TestHome};
use crate::setup::plan::{self, Plan};
use crate::setup::record::{
    AgentRecord, Drift, InstallRecord, ManagedEntry, Os, Probe, RecordStore, UtcTimestamp,
    detect_drift, key_path,
};

/// `agy --version` as captured from agy 1.5.3 (task 1.1).
const AGY_VERSION: &str =
    include_str!("../../../tests/fixtures/setup/agents/antigravity/agy-version.txt");

/// A server file with other servers, comments and odd formatting (golden).
const OTHER_SERVERS: &str =
    include_str!("../../../tests/fixtures/setup/agents/antigravity/mcp_config-other-servers.json");

/// The file `agy mcp add` wrote (task 1.1): mmhmem with `"disabled": false`.
const AGY_WROTE: &str = include_str!(
    "../../../tests/fixtures/setup/agents/antigravity/mcp_config-after-agy-mcp-add.json"
);

/// The CLI, app and IDE data folders under the Gemini home, in that order.
const DATA_FOLDERS: [&str; 3] = ["antigravity-cli", "antigravity", "antigravity-ide"];

const ID: AgentId = AgentId::Antigravity;

pub(super) fn setup() -> (TestHome, Antigravity) {
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

fn want_mcp() -> Selection {
    Selection::new().with(ID, &[Feature::Mcp])
}

/// `~/.gemini/config/mcp_config.json`, spelled out.
pub(super) fn server_file(home: &TestHome) -> PathBuf {
    home.home()
        .join(".gemini")
        .join("config")
        .join("mcp_config.json")
}

fn seed(home: &TestHome, text: &str) -> PathBuf {
    home.write(&[".gemini", "config", "mcp_config.json"], text)
}

pub(super) fn run_apply(home: &TestHome, changes: Vec<Change>, record: &mut InstallRecord) {
    let run = UtcTimestamp::try_from("2026-11-09T09:30:00Z".to_owned()).unwrap();
    let store = RecordStore::new(home.dir.path().join("state"));
    let report = plan::apply(&Plan { run, changes }, &store, record);
    let outcome = format!("{:?}", report.agent(SCOPE).map(|a| &a.outcome));
    assert!(outcome.contains("Applied"), "{outcome}");
}

pub(super) fn drift_is_clean(record: &InstallRecord) {
    assert!(record.applying().is_empty());
    for report in detect_drift(record, &Probe::default()) {
        assert_eq!(report.drift, Drift::Unchanged, "{:?}", report.target);
    }
}

/// Plans and applies the MCP feature; returns the record.
fn install(home: &TestHome, agent: &Antigravity) -> InstallRecord {
    let changes = agent.plan(&home.env, &want_mcp(), None).unwrap();
    let mut record = InstallRecord::new();
    run_apply(home, changes, &mut record);
    record
}

pub(super) fn agent_record(record: &InstallRecord) -> AgentRecord {
    record.agents.get(&ID).cloned().unwrap_or_default()
}

/// The value at `path` in a JSON or JSONC text.
pub(super) fn at(text: &str, path: &[&str]) -> Option<Value> {
    crate::setup::edit::json::get(text, path).unwrap()
}

pub(super) fn wanted_entry(agent: &Antigravity) -> Value {
    json!({"command": agent.binary.to_string_lossy(), "args": ["stdio"]})
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
    // A non-zero exit hides even a version-looking line (the success guard).
    home.runner.on(
        "agy",
        &["--version"],
        vec![Reply::exit(1, "1.5.3\n", "boom")],
    );
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
    // The server file setup itself creates does not count.
    seed(&home, "{}");
    assert!(matches!(
        agent.detect(&home.env),
        Detection::NotFound { reason } if reason == "Antigravity is not installed"
    ));
}

#[test]
fn the_global_files_sit_under_the_gemini_home() {
    let (home, _agent) = setup();
    let gemini = home.home().join(".gemini");
    assert_eq!(Antigravity::mcp_file(&home.env), server_file(&home));
    assert_eq!(
        Antigravity::hooks_file(&home.env),
        gemini.join("config").join("hooks.json")
    );
    assert_eq!(
        Antigravity::stale_file(&home.env),
        gemini.join("antigravity").join("mcp_config.json")
    );
}

#[test]
fn the_snippet_names_the_binary_and_stdio() {
    let (_home, agent) = setup();
    let value: Value = serde_json::from_str(&agent.snippet()).unwrap();
    assert_eq!(value["mcpServers"]["mmhmem"], wanted_entry(&agent));
    let file = agent.snippet_file();
    assert_eq!(
        file.file_name().and_then(|n| n.to_str()),
        Some(SNIPPET_FILE)
    );
    assert_eq!(file.parent(), agent.binary.parent());
}

#[test]
fn a_missing_server_file_is_planned_with_only_the_mmhmem_entry() {
    let (home, agent) = setup();
    // Not selected and nothing recorded: nothing planned.
    let none = agent.plan(&home.env, &Selection::new(), None).unwrap();
    assert!(none.is_empty());

    let changes = agent.plan(&home.env, &want_mcp(), None).unwrap();
    assert_eq!(changes.len(), 1, "one change, and no snippet file");
    let change = &changes[0];
    let file = server_file(&home);
    assert_eq!(change.file, file);
    assert_eq!(
        change.target,
        Target::JsonEntry {
            file: file.clone(),
            pointer: key_path(&["mcpServers", "mmhmem"]).unwrap(),
            managed_keys: Some(key_path(&["command", "args"]).unwrap()),
        }
    );
    assert_eq!(change.action, Action::Add);
    assert!(change.before.is_none());
    assert_eq!(change.missing_parents, Some(2));
    let summary = &change.summary;
    assert!(
        summary.contains("Antigravity") && summary.contains("mcp_config.json"),
        "{summary}"
    );
    // Plain JSON with two-space indents and nothing but mmhmem's entry.
    let command = serde_json::to_string(&agent.binary.to_string_lossy()).unwrap();
    let expected = format!(
        "{{\n  \"mcpServers\": {{\n    \"mmhmem\": {{\n      \"command\": {command},\n      \"args\": [\"stdio\"]\n    }}\n  }}\n}}\n"
    );
    assert_eq!(change.after.as_deref(), Some(expected.as_bytes()));
}

/// Re-planning the unchanged, recorded entry changes nothing.
fn replans_to_nothing(home: &TestHome, agent: &Antigravity, record: &InstallRecord) {
    drift_is_clean(record);
    let rec = agent_record(record);
    let changes = agent.plan(&home.env, &want_mcp(), Some(&rec)).unwrap();
    assert!(changes.is_empty(), "{} changes", changes.len());
}

#[test]
fn a_created_server_file_is_recorded_and_removal_leaves_an_empty_object() {
    let (home, agent) = setup();
    let mut record = install(&home, &agent);
    assert!(server_file(&home).is_file());
    assert!(!agent.snippet_file().exists(), "Req 1.4: no snippet file");
    let rec = agent_record(&record);
    assert_eq!(rec.managed.len(), 1);
    assert_eq!(rec.managed[0].missing_parents, Some(2));
    assert!(rec.managed[0].created_file);
    replans_to_nothing(&home, &agent, &record);

    // Removal prunes the parents this install created; the file stays
    // (it is Antigravity's own settings file).
    run_apply(&home, agent.plan_remove(&home.env, &rec), &mut record);
    let left: Value = serde_json::from_slice(&std::fs::read(server_file(&home)).unwrap()).unwrap();
    assert_eq!(left, json!({}));
    assert!(agent_record(&record).managed.is_empty());
}

/// The golden file as committed (line endings normalised to LF), with CRLF
/// line endings, and with trailing blanks.
fn golden_variants() -> [(&'static str, String); 3] {
    let lf = OTHER_SERVERS.replace("\r\n", "\n");
    let crlf = lf.replace('\n', "\r\n");
    let blanks = format!("{}\n\n", lf.replace(",\n", ", \t\n"));
    [("lf", lf), ("crlf", crlf), ("trailing blanks", blanks)]
}

/// Installs over `seeded` and checks that only mmhmem's entry was added.
fn install_over(name: &str, seeded: &str) -> (TestHome, Antigravity, InstallRecord) {
    let (home, agent) = setup();
    let file = seed(&home, seeded);
    let changes = agent.plan(&home.env, &want_mcp(), None).unwrap();
    assert_eq!(changes.len(), 1, "{name}");
    assert_eq!(changes[0].action, Action::Add, "{name}");
    let before = changes[0].before.as_deref();
    assert_eq!(before, Some(seeded.as_bytes()), "{name}");
    // `mcpServers` exists; only the `mmhmem` object is created.
    assert_eq!(changes[0].missing_parents, Some(1), "{name}");
    let mut record = InstallRecord::new();
    run_apply(&home, changes, &mut record);

    let installed = std::fs::read_to_string(&file).unwrap();
    let entry = at(&installed, &["mcpServers", "mmhmem"]);
    assert_eq!(entry, Some(wanted_entry(&agent)), "{name}");
    for server in ["files", "github"] {
        let path = ["mcpServers", server];
        assert_eq!(at(&installed, &path), at(seeded, &path), "{name}");
    }
    assert!(installed.contains("// Servers the user added"), "{name}");
    assert!(installed.contains("/* off for now */"), "{name}");
    (home, agent, record)
}

#[test]
fn add_then_remove_is_byte_identical_around_other_servers() {
    for (name, seeded) in golden_variants() {
        let (home, agent, mut record) = install_over(name, &seeded);
        replans_to_nothing(&home, &agent, &record);
        let removal = agent.plan_remove(&home.env, &agent_record(&record));
        assert_eq!(removal.len(), 1, "{name}");
        assert_eq!(removal[0].action, Action::Remove, "{name}");
        run_apply(&home, removal, &mut record);
        let left = std::fs::read_to_string(server_file(&home)).unwrap();
        assert_eq!(left, seeded, "{name}");
        assert!(agent_record(&record).managed.is_empty(), "{name}");
    }
}

#[test]
fn user_keys_inside_the_entry_are_kept_byte_identical() {
    let (home, agent) = setup();
    // What `agy mcp add` wrote: another binary path and `"disabled": false`.
    let file = seed(&home, AGY_WROTE);
    let disabled_line = AGY_WROTE
        .lines()
        .find(|l| l.contains("\"disabled\""))
        .unwrap()
        .to_owned();
    let changes = agent.plan(&home.env, &want_mcp(), None).unwrap();
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].action, Action::Update);
    assert_eq!(changes[0].missing_parents, Some(0));
    let mut record = InstallRecord::new();
    run_apply(&home, changes, &mut record);
    let installed = std::fs::read_to_string(&file).unwrap();
    let mut entry = wanted_entry(&agent);
    entry["disabled"] = json!(false);
    assert_eq!(at(&installed, &["mcpServers", "mmhmem"]), Some(entry));
    assert!(installed.lines().any(|l| l == disabled_line));
    drift_is_clean(&record);

    let rec = agent_record(&record);
    run_apply(&home, agent.plan_remove(&home.env, &rec), &mut record);
    let left = std::fs::read_to_string(&file).unwrap();
    assert_eq!(
        at(&left, &["mcpServers", "mmhmem"]),
        Some(json!({"disabled": false}))
    );
    assert!(left.lines().any(|l| l == disabled_line), "byte-identical");
    assert!(agent_record(&record).managed.is_empty(), "left the record");
}

#[test]
fn a_moved_binary_updates_and_a_deselected_entry_is_removed() {
    let (home, agent) = setup();
    let mut record = install(&home, &agent);
    let moved = Antigravity::new(home.bin().join("new").join("mmhmem.exe"));
    let rec = agent_record(&record);
    let changes = moved.plan(&home.env, &want_mcp(), Some(&rec)).unwrap();
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].action, Action::Update);
    assert_eq!(changes[0].missing_parents, Some(2), "kept from the record");
    run_apply(&home, changes, &mut record);
    let installed = std::fs::read_to_string(server_file(&home)).unwrap();
    assert_eq!(
        at(&installed, &["mcpServers", "mmhmem"]),
        Some(wanted_entry(&moved))
    );

    // Recorded but no longer selected: the entry is taken out.
    let rec = agent_record(&record);
    let changes = moved
        .plan(&home.env, &Selection::new(), Some(&rec))
        .unwrap();
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].action, Action::Remove);
    run_apply(&home, changes, &mut record);
    let left: Value = serde_json::from_slice(&std::fs::read(server_file(&home)).unwrap()).unwrap();
    assert_eq!(left, json!({}));
}

#[test]
fn an_invalid_server_file_is_a_problem_naming_the_file() {
    let (home, agent) = setup();
    let file = seed(&home, "{ \"secret-token-123\": ");
    let problem = agent.plan(&home.env, &want_mcp(), None).unwrap_err();
    assert!(matches!(&problem, AgentProblem::Json { file: f, .. } if *f == file));
    let text = problem.to_string();
    assert!(text.contains("mcp_config.json"), "{text}");
    assert!(!text.contains("secret-token-123"), "{text}");
    seed(&home, "[1]");
    assert!(matches!(
        agent.plan(&home.env, &want_mcp(), None),
        Err(AgentProblem::Json { .. })
    ));
    // Nothing was written.
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "[1]");
}

#[test]
fn an_old_snippet_record_is_still_removed() {
    let (home, agent) = setup();
    let file = agent.snippet_file();
    std::fs::write(&file, agent.snippet()).expect("snippet file");
    let mut rec = AgentRecord::default();
    rec.managed.push(recorded_snippet(&agent));
    let removes = agent.plan_remove(&home.env, &rec);
    assert_eq!(removes.len(), 1);
    assert_eq!(removes[0].file, file);
    assert_eq!(removes[0].action, Action::Remove);
    assert_eq!(removes[0].after, None);

    // An install over the old record plans the server entry, not the snippet.
    let changes = agent.plan(&home.env, &want_mcp(), Some(&rec)).unwrap();
    assert!(changes.iter().all(|c| c.file != file));
    assert!(changes.iter().any(|c| c.file == server_file(&home)));

    assert_eq!(
        Antigravity::pending_step(),
        PendingUserStep::ImportAntigravitySnippet
    );
    assert!(Antigravity::removal_instruction().contains("Antigravity"));
}

/// A record entry saying the snippet file is managed and written.
fn recorded_snippet(agent: &Antigravity) -> ManagedEntry {
    ManagedEntry {
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
