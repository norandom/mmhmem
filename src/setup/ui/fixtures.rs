//! Test fixtures of the menu (task 6.2): a temporary home with the agent
//! doubles, flow dependencies with a fixed clock, records in given states,
//! a digest of the whole temporary folder and `TestBackend` rendering.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use serde_json::{Value, json};

use super::app::{App, Key};
use super::flow::{
    Confirmed, Deps, Exec, Flow, HomeAction, Memories, Newest, Next, Relaunch, Releases, Restart,
};
use crate::setup::agents::doubles::{Reply, TestHome};
use crate::setup::agents::doubles_cli::{self, CLAUDE_MCP_GET, CODEX_MCP_LIST_JSON};
use crate::setup::agents::{AgentId, AgentRecord, Feature, PendingUserStep};
use crate::setup::record::{
    CheckResult, InstallRecord, LastCheck, Os, RecordStore, ReleaseTag, Sha256Hex, UtcTimestamp,
};
use crate::setup::release::fixtures::FakeRelease;
use crate::setup::release::{LocalRelease, SUMS_NAME, SUMS_SIG_NAME};
use crate::setup::user_path::NoUserPath;

/// The program path in the captured CLI fixtures.
const FIXTURE_BIN: &str = r"C:\Users\user\AppData\Local\Programs\mmhmem\mmhmem.exe";

pub(crate) fn tag(text: &str) -> ReleaseTag {
    ReleaseTag::try_from(text.to_owned()).expect("tag")
}

pub(crate) fn at(text: &str) -> UtcTimestamp {
    UtcTimestamp::try_from(text.to_owned()).expect("time")
}

/// A temporary home and the flow's dependencies on it.
pub(crate) struct Fx {
    pub(crate) home: TestHome,
    pub(crate) deps: Deps,
}

/// A fresh Linux-rules home with no agent; the record folder, the
/// installed program and the memory store live under the home; restarting
/// after an update is off. No release key is trusted, whatever this build
/// embeds, so an update is refused before anything is downloaded. No worker
/// model is set up, so the read-routing hook (on by default) stays inactive
/// and its sample read, which needs a real program, is not run.
pub(crate) fn fx() -> Fx {
    let mut home = TestHome::new(Os::Linux);
    home.env = home.env.clone().with_worker_configured(false);
    let state = home.home().join(".local").join("state").join("mmhmem");
    let binary = home.home().join(".local").join("bin").join("mmhmem");
    let memories = home.home().join(".local").join("share").join("mmhmem");
    let deps = Deps {
        env: home.env.clone(),
        store: RecordStore::new(state),
        binary,
        install_from: None,
        paths: Arc::new(NoUserPath),
        releases: Releases {
            keys: Vec::new(),
            ..Releases::github()
        },
        version: tag("v0.4.1"),
        now: Arc::new(|| at("2026-11-04T10:00:00Z")),
        memories: Some(Memories {
            dir: memories.clone(),
            log_file: memories.join("audit.jsonl"),
        }),
        restart: Restart::default(),
        in_use: Arc::new(|_| false),
    };
    Fx { home, deps }
}

/// A home where Claude Code, Codex and OpenCode are installed and their
/// CLIs answer like the real ones for this home's program.
pub(crate) fn with_clis() -> Fx {
    let fx = fx();
    for program in ["claude", "codex", "opencode"] {
        fx.home.install_program(program);
    }
    fx.script_checks();
    fx
}

impl Fx {
    /// Scripts the CLIs, with check answers naming this home's program.
    pub(crate) fn script_checks(&self) {
        let runner = &self.home.runner;
        doubles_cli::all(runner, &self.home.env);
        let bin = self.deps.binary.to_string_lossy().into_owned();
        let get = CLAUDE_MCP_GET.replace(FIXTURE_BIN, &bin);
        runner.on("claude", &["mcp", "get", "mmhmem"], vec![Reply::ok(&get)]);
        let mut list: Value = serde_json::from_str(CODEX_MCP_LIST_JSON).expect("fixture");
        list[0]["transport"]["command"] = json!(bin);
        let list = list.to_string();
        runner.on("codex", &["mcp", "list", "--json"], vec![Reply::ok(&list)]);
    }

    /// The flow as `mmhmem setup` starts it, with the release answer given.
    pub(crate) fn flow(&self, newest: Newest) -> Flow {
        let mut flow = Flow::load(self.deps.clone());
        flow.set_newest(newest);
        flow
    }

    /// The menu on its home screen.
    pub(crate) fn app(&self, newest: Newest) -> App {
        App::new(self.flow(newest))
    }

    /// Saves `record` as the install record.
    pub(crate) fn save(&self, record: &InstallRecord) {
        self.deps.store.save(record).expect("save record");
    }

    /// Writes `text` under the home and returns the path.
    pub(crate) fn write(&self, rel: &[&str], text: &str) -> PathBuf {
        self.home.write(rel, text)
    }

    /// A digest of every file and folder in the temporary folder.
    pub(crate) fn digest(&self) -> String {
        let mut entries = Vec::new();
        walk(self.home.dir.path(), self.home.dir.path(), &mut entries);
        entries.sort();
        Sha256Hex::of(entries.join("\n").as_bytes())
            .as_str()
            .to_owned()
    }
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) {
    let Ok(read) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in read.filter_map(Result::ok) {
        let path = entry.path();
        let rel = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .display()
            .to_string();
        if path.is_dir() {
            out.push(format!("dir {rel}"));
            walk(root, &path, out);
        } else {
            let bytes = std::fs::read(&path).unwrap_or_default();
            out.push(format!("file {rel} {}", Sha256Hex::of(&bytes).as_str()));
        }
    }
}

/// A record entry with `features` on, a check result and pending steps.
pub(crate) fn agent_record(
    features: &[Feature],
    result: CheckResult,
    pending: &[PendingUserStep],
) -> AgentRecord {
    AgentRecord {
        features: features.iter().map(|f| (*f, true)).collect(),
        managed: Vec::new(),
        pending_user_steps: pending.to_vec(),
        last_check: Some(LastCheck {
            at: at("2026-11-04T09:31:12Z"),
            result,
            details: Vec::new(),
        }),
        skipped: None,
    }
}

/// A record of release `release` with `agents`.
pub(crate) fn record(release: &str, agents: Vec<(AgentId, AgentRecord)>) -> InstallRecord {
    let mut record = InstallRecord::new();
    record.release = Some(tag(release));
    record.agents = agents.into_iter().collect();
    record
}

/// The menu drawn on an 80×24 `TestBackend`, with the home folder shown as
/// `~` and `/` as the folder separator on every OS.
pub(crate) fn render(app: &App) -> String {
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).expect("terminal");
    terminal.draw(|frame| app.draw(frame)).expect("draw");
    terminal.backend().to_string().replace('\\', "/")
}

/// Feeds `keys` to `app`.
pub(crate) fn press(app: &mut App, keys: &[Key]) {
    let _ = app.keys(keys);
}

/// Installs the default choices on `fx` (asserting it worked).
pub(crate) fn install(fx: &Fx) -> Flow {
    let mut flow = fx.flow(Newest::Unknown);
    assert_eq!(flow.begin(&HomeAction::Install), Next::Agents);
    flow.make_plan();
    assert_eq!(flow.confirm(), Confirmed::Done);
    assert_eq!(flow.exit_code(), 0, "{:?}", flow.progress());
    flow
}

/// A release folder with `fake` as `--release-dir` would hold it.
pub(crate) fn release_dir(fake: &FakeRelease) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("release dir");
    std::fs::write(dir.path().join(fake.archive_name()), &fake.archive).unwrap();
    std::fs::write(dir.path().join(SUMS_NAME), &fake.sums).unwrap();
    std::fs::write(dir.path().join(SUMS_SIG_NAME), &fake.sig).unwrap();
    dir
}

/// An installed home whose program holds `OLD`, updating from `dir` with
/// `keys`.
pub(crate) fn installed_old(dir: &Path, keys: Vec<[u8; 32]>) -> Fx {
    let mut fx = with_clis();
    std::fs::create_dir_all(fx.deps.binary.parent().unwrap()).unwrap();
    std::fs::write(&fx.deps.binary, b"OLD").unwrap();
    install(&fx);
    let server = LocalRelease::serve(dir).expect("serve");
    fx.deps.releases = Releases {
        keys,
        ..Releases::local(server)
    };
    fx
}

/// Release v9.9.9 of this platform, whose program holds `NEW PROGRAM`.
pub(crate) fn new_release() -> FakeRelease {
    let target = crate::setup::release::Target::current().expect("platform");
    FakeRelease::good(target, b"NEW PROGRAM")
}

/// The program starts an [`Exec`] saw.
pub(crate) type Starts = Arc<std::sync::Mutex<Vec<Relaunch>>>;

/// Records every program start instead of starting it; answers `code`.
pub(crate) fn recording_exec(code: i32) -> (Exec, Starts) {
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let log = seen.clone();
    let exec: Exec = Arc::new(move |r: &Relaunch| {
        log.lock().unwrap().push(r.clone());
        Ok(code)
    });
    (exec, seen)
}
