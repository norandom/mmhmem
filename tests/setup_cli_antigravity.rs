//! `mmhmem setup` for Antigravity, from a first install to the removal
//! (antigravity-native-setup task 5.1; Requirements 1.1, 1.2, 2.1, 2.5,
//! 4.1, 4.3 and 5.1–5.4).
//!
//! These tests drive the setup flow that `mmhmem setup` runs, on a
//! temporary home: the install and the re-run go through the run without a
//! terminal (`fallback::run_default`, which the install script starts with
//! `--bootstrap`), the removal through the menu's own calls
//! (`Flow::begin(Remove)`, the review, `Flow::confirm`). The binary's own
//! `setup` is not started: on Windows it would find the real user's home.
//!
//! Antigravity is found by its CLI data folder and an `agy` program on the
//! home's `PATH`. `agy` is a double: `agy --version` prints the captured
//! version, and `agy mcp list` prints the captured listing while
//! Antigravity's global server file names mmhmem (the captured empty
//! listing otherwise), so the `agent_command` step passes only when the file
//! Antigravity reads has the entry. The installed program is a copy of the
//! real `mmhmem` binary: the check starts the command that server file names
//! (with a private store added to its environment) and runs the real
//! `mmhmem hook antigravity` sample. Nothing outside the temporary folder is
//! read or written, so this runs on Linux, macOS and Windows alike.
//!
//! The rollback after a failed check (Requirement 4.4) is in
//! `tests/setup_verify.rs`: a wrong command (registration step) and a
//! program that does not answer (handshake step) both restore the original
//! bytes of both files.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use mmhmem::setup::agents::{
    CommandOutput, CommandRunner, Env, Invocation, RunError, ScanEnv, SystemRunner,
};
use mmhmem::setup::check::agent::{McpProbe, ServerProbe};
use mmhmem::setup::check::{McpCheck, Registration};
use mmhmem::setup::record::{Os, RecordStore, ReleaseTag, Sha256Hex, UtcTimestamp};
use mmhmem::setup::ui::fallback::run_default;
use mmhmem::setup::ui::flow::{Confirmed, Deps, Flow, HomeAction, Newest, Next, Releases, Restart};
use mmhmem::setup::user_path::NoUserPath;
use serde_json::{Value, json};

const BIN: &str = env!("CARGO_BIN_EXE_mmhmem");
const AGY_VERSION: &str = include_str!("fixtures/setup/agents/antigravity/agy-version.txt");
const AGY_LISTED: &str = include_str!("fixtures/setup/agents/antigravity/agy-mcp-list-mmhmem.txt");
const AGY_EMPTY: &str = include_str!("fixtures/setup/agents/antigravity/agy-mcp-list-empty.txt");
/// The user's own servers and hooks (comments, tabs, odd formatting).
const OTHER_SERVERS: &str =
    include_str!("fixtures/setup/agents/antigravity/mcp_config-other-servers.json");
const OTHER_HOOKS: &str = include_str!("fixtures/setup/agents/antigravity/hooks-other-hooks.json");
/// The owner's `~/.gemini/antigravity/mcp_config.json`: the pasted
/// snippet-era entry between two other servers.
const STALE: &str = include_str!("fixtures/setup/agents/antigravity/stale-mcp_config.json");

const SERVER_FILE: &str = ".gemini/config/mcp_config.json";
const HOOKS_FILE: &str = ".gemini/config/hooks.json";
const STALE_FILE: &str = ".gemini/antigravity/mcp_config.json";

/// What the results and the review must never tell the user to do for
/// Antigravity (Requirements 1.4 and 6.2).
const MANUAL_WORDS: [&str; 3] = ["snippet", "paste", "import"];

/// The variables that keep a started `mmhmem` on the private store and off
/// any worker model (a removed variable would fall back to the machine's
/// `.mmhmem.json`; see `tests/local_stdio.rs`).
fn private_env(data: &Path) -> [(&'static str, String); 5] {
    [
        ("AGENT_MEM_DATA_DIR", data.to_string_lossy().into_owned()),
        ("MMHMEM_NO_UPDATE_CHECK", "1".to_owned()),
        ("WORKER_API_URL", String::new()),
        ("WORKER_MODEL", String::new()),
        ("WORKER_API_KEY", String::new()),
    ]
}

/// `agy` as captured (see the module docs); every other program, the hook
/// sample, really runs, on the private store.
struct Agy {
    home: PathBuf,
    data: PathBuf,
}

impl Agy {
    fn prints(stdout: &str) -> Result<CommandOutput, RunError> {
        Ok(CommandOutput {
            code: Some(0),
            stdout: stdout.to_owned(),
            stderr: String::new(),
        })
    }

    /// Lists mmhmem only while Antigravity's global server file names it.
    fn mcp_list(&self) -> Result<CommandOutput, RunError> {
        let file = rel_path(&self.home, SERVER_FILE);
        let named = std::fs::read(file)
            .is_ok_and(|bytes| Registration::from_claude_json(&bytes, "mmhmem").is_ok());
        Self::prints(if named { AGY_LISTED } else { AGY_EMPTY })
    }
}

impl CommandRunner for Agy {
    fn run(&self, invocation: &Invocation) -> Result<CommandOutput, RunError> {
        if invocation.program_name() != "agy" {
            let mut real = invocation.clone();
            for (key, value) in private_env(&self.data) {
                real = real.env(key, value);
            }
            return SystemRunner.run(&real);
        }
        match invocation.args.as_slice() {
            [flag] if flag == "--version" => Self::prints(AGY_VERSION),
            [mcp, list] if mcp == "mcp" && list == "list" => self.mcp_list(),
            other => panic!("setup ran an unexpected agy command: {other:?}"),
        }
    }
}

/// The real MCP check of the registration read from Antigravity's file,
/// with the private store added to its environment.
struct PrivateStore(PathBuf);

impl McpProbe for PrivateStore {
    fn probe(&self, registration: &Registration) -> McpCheck {
        let mut registration = registration.clone();
        for (key, value) in private_env(&self.0) {
            registration = registration.with_env(key, value);
        }
        ServerProbe.probe(&registration)
    }
}

fn rel_path(root: &Path, rel: &str) -> PathBuf {
    rel.split('/').fold(root.to_path_buf(), |p, s| p.join(s))
}

/// A temporary folder with the home, the `PATH` folder holding `agy`, the
/// private store, the install record folder and the installed program.
struct Home {
    dir: tempfile::TempDir,
    runs: Arc<AtomicU32>,
}

impl Home {
    /// A home where Antigravity is installed: its CLI data folder and `agy`
    /// on `PATH`. No other agent is found.
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let home = Self {
            dir,
            runs: Arc::new(AtomicU32::new(0)),
        };
        for folder in [home.home(), home.bin(), home.data()] {
            std::fs::create_dir_all(folder).unwrap();
        }
        std::fs::create_dir_all(home.path(".gemini/antigravity-cli")).unwrap();
        let agy = home
            .bin()
            .join(if cfg!(windows) { "agy.exe" } else { "agy" });
        std::fs::write(&agy, b"").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&agy, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        home
    }

    fn home(&self) -> PathBuf {
        self.dir.path().join("home")
    }

    fn bin(&self) -> PathBuf {
        self.dir.path().join("bin")
    }

    fn data(&self) -> PathBuf {
        self.dir.path().join("data")
    }

    fn path(&self, rel: &str) -> PathBuf {
        rel_path(&self.home(), rel)
    }

    fn write(&self, rel: &str, text: &str) -> PathBuf {
        let file = self.path(rel);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, text).unwrap();
        file
    }

    fn read(&self, rel: &str) -> String {
        std::fs::read_to_string(self.path(rel)).unwrap()
    }

    /// The installed program the agents launch (setup copies `BIN` there).
    fn binary(&self) -> PathBuf {
        let name = if cfg!(windows) {
            "mmhmem.exe"
        } else {
            "mmhmem"
        };
        self.dir.path().join("programs").join(name)
    }

    fn store(&self) -> RecordStore {
        RecordStore::new(self.dir.path().join("state"))
    }

    fn deps(&self) -> Deps {
        let mut dirs = ScanEnv::new(Os::current(), self.home());
        dirs.appdata = Some(self.path("AppData/Roaming"));
        dirs.local_app_data = Some(self.path("AppData/Local"));
        dirs.user_path = Some(vec![self.bin()]);
        let agy = Agy {
            home: self.home(),
            data: self.data(),
        };
        let env = Env::new(dirs, Arc::new(agy))
            .with_worker_configured(false)
            .with_mcp_probe(Arc::new(PrivateStore(self.data())));
        let version = format!("v{}", env!("CARGO_PKG_VERSION"));
        let runs = self.runs.clone();
        Deps {
            install_from: Some(PathBuf::from(BIN)),
            paths: Arc::new(NoUserPath),
            releases: Releases {
                keys: Vec::new(),
                ..Releases::github()
            },
            // One second per run, so every run has its own backup folder.
            now: Arc::new(move || {
                let n = runs.fetch_add(1, Ordering::SeqCst);
                let at = format!("2026-11-10T10:{:02}:{:02}Z", n / 60, n % 60);
                UtcTimestamp::try_from(at).unwrap()
            }),
            memories: None,
            restart: Restart::default(),
            in_use: Arc::new(|_| false),
            models: Arc::new(|_, _| None),
            ..Deps::new(
                env,
                self.store(),
                self.binary(),
                ReleaseTag::try_from(version).unwrap(),
            )
        }
    }

    /// The flow as `mmhmem setup` starts it, offline.
    fn flow(&self) -> Flow {
        let mut flow = Flow::load(self.deps());
        flow.set_newest(Newest::Unknown);
        flow
    }

    /// `mmhmem setup` without a terminal: its exit code and what it printed.
    fn setup(&self) -> (i32, String) {
        let mut out = Vec::new();
        let code = run_default(&mut self.flow(), &mut out);
        (code, String::from_utf8(out).unwrap())
    }

    /// `install.json`, parsed, and its text.
    fn record(&self) -> (Value, String) {
        let text = std::fs::read_to_string(self.store().record_path()).unwrap();
        (serde_json::from_str(&text).unwrap(), text)
    }

    /// The program as Antigravity's files name it.
    fn program(&self) -> String {
        let program = self.binary().to_string_lossy().into_owned();
        // So the hook command is the path itself (no 8.3 short form).
        assert!(!program.contains(char::is_whitespace), "{program}");
        program
    }
}

/// `text` parsed as JSON with comments (Antigravity's files may have them).
fn jsonc(text: &str) -> Value {
    mmhmem::setup::edit::json::get(text, &[])
        .unwrap()
        .expect("a JSON document")
}

/// mmhmem's server entry as setup writes it (Requirement 1.1).
fn server_entry(home: &Home) -> Value {
    json!({"command": home.program(), "args": ["stdio"]})
}

/// mmhmem's named hook as setup writes it (Requirement 2.1).
fn hook_entry(home: &Home) -> Value {
    json!({"PreToolUse": [{
        "matcher": "^read_file$",
        "hooks": [{
            "type": "command",
            "command": format!("{} hook antigravity", home.program()),
            "timeout": 10,
        }],
    }]})
}

/// The record's entry for one managed JSON entry: kind, file, pointer and
/// state.
fn managed_entry(file: &Path, pointer: &[&str]) -> Value {
    json!({"kind": "json_entry", "file": file, "pointer": pointer, "state": "done"})
}

/// The kind, file, pointer and state of each entry Antigravity's record
/// manages, in order.
fn managed(record: &Value) -> Vec<Value> {
    let entries = record["agents"]["antigravity"]["managed"]
        .as_array()
        .unwrap();
    entries
        .iter()
        .map(|e| {
            json!({"kind": e["kind"], "file": e["file"], "pointer": e["pointer"],
                "state": e["state"]})
        })
        .collect()
}

/// Antigravity's last check passed, with its own CLI and, when `hook`, the
/// hook sample passing too (Requirements 4.1–4.3).
fn assert_checked(record: &Value, hook: bool) {
    let last = &record["agents"]["antigravity"]["last_check"];
    assert_eq!(last["result"], "pass", "{last}");
    let details = last["details"].as_array().unwrap();
    let mut steps = vec![
        "registration",
        "handshake",
        "tools_list",
        "status_tool",
        "agent_command",
    ];
    if hook {
        steps.push("hook_sample");
    }
    for step in steps {
        let pass = json!({"step": step, "outcome": "pass"});
        assert!(details.contains(&pass), "{step}: {last}");
    }
}

/// The output tells the user nothing to do by hand for Antigravity.
fn assert_no_manual_step(output: &str) {
    let lower = output.to_lowercase();
    for word in MANUAL_WORDS {
        assert!(!lower.contains(word), "{word}: {output}");
    }
}

#[test]
fn a_clean_install_writes_both_entries_passes_the_check_and_records_both() {
    let home = Home::new();
    let (code, output) = home.setup();
    assert_eq!(code, 0, "{output}");
    // Antigravity reads these two global files (Requirements 1.1 and 2.1).
    let servers = jsonc(&home.read(SERVER_FILE));
    assert_eq!(
        servers,
        json!({"mcpServers": {"mmhmem": server_entry(&home)}})
    );
    let hooks = jsonc(&home.read(HOOKS_FILE));
    assert_eq!(hooks, json!({"mmhmem-read-routing": hook_entry(&home)}));
    // The check read them back, `agy mcp list` named mmhmem and the hook
    // answered the sample read (Requirements 4.1–4.3).
    assert!(
        output.contains("✔ Antigravity — mmhmem connected, 10 tools"),
        "{output}"
    );
    assert!(output.contains("Antigravity: start a new session."));
    assert_no_manual_step(&output);
    let (record, _) = home.record();
    let agents: Vec<&String> = record["agents"].as_object().unwrap().keys().collect();
    assert_eq!(agents, ["antigravity"], "only Antigravity is installed");
    let antigravity = &record["agents"]["antigravity"];
    assert_eq!(
        antigravity["features"],
        json!({"mcp": true, "read_hook": true})
    );
    assert_eq!(
        managed(&record),
        [
            managed_entry(&home.path(SERVER_FILE), &["mcpServers", "mmhmem"]),
            managed_entry(&home.path(HOOKS_FILE), &["mmhmem-read-routing"]),
        ]
    );
    assert_eq!(antigravity["pending_user_steps"], json!([]));
    assert_checked(&record, true);
    // A second run has nothing to change.
    let (code, again) = home.setup();
    assert_eq!(code, 0, "{again}");
    assert!(again.contains("Everything is current"), "{again}");
}

/// The snippet file 0.5.1 saved next to the program, and its record entry.
fn snippet(home: &Home) -> (PathBuf, Value) {
    let file = home.binary().with_file_name("antigravity-mcp.json");
    let value = json!({"mcpServers": {"mmhmem": server_entry(home)}});
    let text = serde_json::to_string_pretty(&value).unwrap();
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, &text).unwrap();
    let entry = json!({"kind": "file", "file": file,
        "sha256": Sha256Hex::of(text.as_bytes()).as_str(), "backup": null,
        "created_file": true, "state": "done"});
    (file, entry)
}

/// A snippet-era install record (research.md "Existing install on the
/// owner's machine"): shared memory on, the import step pending and
/// `managed` as given (the owner's has nothing).
fn snippet_era_record(managed: Vec<Value>) -> Value {
    let pass = |step: &str| json!({"step": step, "outcome": "pass"});
    json!({
        "schema": 1, "release": "v0.5.0", "binary": null, "path_entry": null,
        "agents": {"antigravity": {
            "features": {"mcp": true},
            "managed": managed,
            "pending_user_steps": ["import_antigravity_snippet"],
            "last_check": {
                "at": "2026-11-06T10:00:00Z",
                "result": "user_steps_pending",
                "details": [pass("registration"), pass("handshake"),
                    pass("tools_list"), pass("status_tool")]
            }
        }}
    })
}

/// `text` without the lines of its `"mmhmem": {…}` object (from the line
/// naming it to the first line closing an object).
fn without_mmhmem_block(text: &str) -> String {
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let start = lines.iter().position(|l| l.contains("\"mmhmem\"")).unwrap();
    let end = (start..lines.len())
        .find(|&i| lines[i].trim_start().starts_with('}'))
        .unwrap();
    let mut kept = lines[..start].to_vec();
    kept.extend(&lines[end + 1..]);
    kept.concat()
}

/// Seeds a snippet-era install: the stale server file Antigravity no longer
/// reads and the old record; with `recorded` (0.5.1 on) also the snippet
/// file it manages. The stale file and the snippet file.
fn seed_snippet_era(home: &Home, recorded: bool) -> (PathBuf, Option<PathBuf>) {
    let stale = home.write(STALE_FILE, STALE);
    let snippet = recorded.then(|| snippet(home));
    let managed_before = snippet.iter().map(|(_, e)| e.clone()).collect();
    let old = serde_json::to_string_pretty(&snippet_era_record(managed_before)).unwrap();
    let record_file = home.store().record_path();
    std::fs::create_dir_all(record_file.parent().unwrap()).unwrap();
    std::fs::write(&record_file, old).unwrap();
    (stale, snippet.map(|(file, _)| file))
}

/// The re-run's files: the review listed every change (Requirements 5.1
/// and 5.2), the new registration is where Antigravity reads it (1.1), only
/// mmhmem's entry left the stale file (its two other servers are
/// byte-identical) and the snippet file is gone.
fn assert_files_forward(
    home: &Home,
    name: &str,
    output: &str,
    stale: &Path,
    snippet: Option<&Path>,
) {
    for change in [
        "Let Antigravity use mmhmem (adds mmhmem to Antigravity's server list in ",
        "Antigravity no longer reads this file; remove the mmhmem entry (",
    ] {
        assert!(output.contains(change), "{name}: {change}: {output}");
    }
    assert!(output.contains("✔ Antigravity — mmhmem connected, 10 tools"));
    assert!(output.contains("Antigravity: start a new session."));
    let servers = jsonc(&home.read(SERVER_FILE));
    let entry = &servers["mcpServers"]["mmhmem"];
    assert_eq!(entry, &server_entry(home), "{name}");
    let left = std::fs::read_to_string(stale).unwrap();
    assert_eq!(left, without_mmhmem_block(STALE), "{name}");
    if let Some(file) = snippet {
        let removal = format!(
            "Remove mmhmem from Antigravity (changes {})",
            file.display()
        );
        assert!(output.contains(&removal), "{name}: {output}");
        assert!(!file.exists(), "{name}: the snippet file is gone");
    }
}

/// The re-run's record (Requirement 5.4): the new entry, no old step. The
/// re-run keeps the recorded choice (shared memory only), so no hook is
/// added and the hook is recorded as off.
fn assert_record_forward(home: &Home, name: &str) {
    assert!(!home.path(HOOKS_FILE).exists(), "{name}");
    let (record, text) = home.record();
    let old_step = "import_antigravity_snippet";
    assert!(!text.contains(old_step), "{name}: {text}");
    let antigravity = &record["agents"]["antigravity"];
    let features = json!({"mcp": true, "read_hook": false});
    assert_eq!(antigravity["features"], features, "{name}");
    assert_eq!(antigravity["pending_user_steps"], json!([]), "{name}");
    let server = managed_entry(&home.path(SERVER_FILE), &["mcpServers", "mmhmem"]);
    assert_eq!(managed(&record), [server], "{name}");
    assert_checked(&record, false);
}

#[test]
fn running_setup_again_brings_a_snippet_era_install_forward() {
    for (name, recorded) in [("owner", false), ("0.5.1", true)] {
        let home = Home::new();
        let (stale, snippet) = seed_snippet_era(&home, recorded);
        let (code, output) = home.setup();
        assert_eq!(code, 0, "{name}: {output}");
        assert_files_forward(&home, name, &output, &stale, snippet.as_deref());
        assert_record_forward(&home, name);
    }
}

/// Installs over the user's own servers and hooks; the two files.
fn install_over_user_files(home: &Home) -> (PathBuf, PathBuf) {
    let servers = home.write(SERVER_FILE, OTHER_SERVERS);
    let hooks = home.write(HOOKS_FILE, OTHER_HOOKS);
    let (code, output) = home.setup();
    assert_eq!(code, 0, "{output}");
    let installed = jsonc(&home.read(SERVER_FILE));
    assert_eq!(installed["mcpServers"]["mmhmem"], server_entry(home));
    assert_eq!(installed["mcpServers"]["files"]["command"], "npx");
    let installed = jsonc(&home.read(HOOKS_FILE));
    assert_eq!(installed["mmhmem-read-routing"], hook_entry(home));
    assert_checked(&home.record().0, true);
    (servers, hooks)
}

/// "Remove mmhmem" from the menu: the review lists the removal from both
/// files, then it is confirmed. The results.
fn remove(home: &Home, files: [&Path; 2]) -> mmhmem::setup::ui::flow::Progress {
    let mut flow = home.flow();
    assert_eq!(flow.begin(&HomeAction::Remove), Next::Review);
    let review: Vec<String> = flow
        .review()
        .groups
        .iter()
        .flat_map(|g| g.items.iter().map(|i| i.summary.clone()))
        .collect();
    for file in files {
        let removal = format!(
            "Remove mmhmem from Antigravity (changes {})",
            file.display()
        );
        let shown = removal.replace(&home.home().display().to_string(), "~");
        assert!(review.contains(&shown), "{shown}: {review:#?}");
    }
    assert_eq!(flow.confirm(), Confirmed::Done);
    flow.progress().unwrap().clone()
}

#[test]
fn removing_mmhmem_takes_out_both_entries_and_leaves_the_rest_byte_identical() {
    let home = Home::new();
    let (servers, hooks) = install_over_user_files(&home);
    let progress = remove(&home, [&servers, &hooks]);
    assert_eq!(progress.code, 0, "{progress:#?}");
    // Both of Antigravity's entries came out and nothing is left to do for
    // it: no next step, and the removal is complete (Requirement 5.3).
    let lines = progress.all_lines();
    assert!(lines.contains(&"Antigravity: 2 change(s) made.".to_owned()));
    assert_eq!(
        lines.last().map(String::as_str),
        Some("mmhmem was removed.")
    );
    assert!(progress.next_steps.is_empty(), "{progress:#?}");
    let antigravity: Vec<&String> = lines.iter().filter(|l| l.contains("Antigravity")).collect();
    assert_eq!(antigravity, ["Antigravity: 2 change(s) made."]);
    assert_no_manual_step(&lines.join("\n"));
    // Everything else in both files is byte for byte as the user had it.
    assert_eq!(std::fs::read_to_string(&servers).unwrap(), OTHER_SERVERS);
    assert_eq!(std::fs::read_to_string(&hooks).unwrap(), OTHER_HOOKS);
    // The record, and so every Antigravity entry in it, and the program are
    // gone.
    assert_eq!(home.store().load().unwrap(), None);
    assert!(!home.binary().exists());
}
