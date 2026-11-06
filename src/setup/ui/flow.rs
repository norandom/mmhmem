//! The setup controller shared by the full-screen menu (`app.rs`) and the
//! numbered fallback (task 6.3): reliable-installers task 6.2,
//! `.kiro/specs/reliable-installers/design.md` §7; Requirements 4.1–4.7,
//! 1.5, 5.2–5.6 and 6.5–6.7.
//!
//! [`Flow::load`] reads everything the home screen shows, without writing:
//! the install record (or, when it is missing or unreadable, the adoption
//! scan of an earlier install), the recovery of entries an interrupted run
//! left `applying` (resolved in memory, saved with the next confirmed
//! change), drift of managed entries, the binary's hash, live detection of
//! every agent and whether the recorded choices still plan no change. The
//! newest release is not looked up here: the caller asks for it off the
//! UI thread and hands the answer to [`Flow::set_newest`], so an offline
//! machine never blocks the menu.
//!
//! The screens drive the flow in this order: [`Flow::home`] and
//! [`Flow::begin`]; the agent choices ([`Flow::agent_choices`]) and the
//! features ([`Flow::feature_rows`]); [`Flow::make_plan`] and
//! [`Flow::review`] with [`Flow::diff`]; and [`Flow::confirm`], the only
//! call that writes, which applies the plan, runs the functional check,
//! stores the record and fills [`Flow::progress`]. Quitting before
//! `confirm` changes nothing ([`text::CANCELLED`], exit code 2).
//!
//! Submodules: `flow_home.rs` (home model, actions), `flow_select.rs`
//! (agents and features), `flow_review.rs` (plan and review),
//! `flow_install.rs` (the program and `PATH` changes), `flow_update.rs`
//! (the update and the restart into the new program), `flow_remove.rs`
//! (the removal) and `flow_apply.rs` (apply, check, progress lines).
//!
//! Everything outside comes in through [`Deps`]: the agents' [`Env`], the
//! record folder, the binary the agents launch and the program to install
//! there, the user's `PATH` store, the release source and keys, this
//! build's release tag, the clock, the memory store a removal may delete
//! and how to start the new program after an update, so tests run on
//! temporary homes with the agent doubles, a fake `PATH`, local releases
//! and no real process.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::text;
use crate::setup::adopt::{self, Scan};
use crate::setup::agents::{
    AgentId, Detection, Env, Registry, Selection, antigravity, claude_code, claude_desktop, codex,
    opencode,
};
use crate::setup::plan::target_file;
use crate::setup::record::{
    Drift, InstallRecord, Probe, RecordStore, Recovery, ReleaseTag, Target, UtcTimestamp,
    detect_drift, recovery_scan,
};
use crate::setup::release::{BinaryStatus, status_hash};
use crate::setup::user_path::{UserPathStore, process_store};

#[path = "flow_apply.rs"]
mod finish;
#[path = "flow_home.rs"]
mod home;
#[path = "flow_install.rs"]
mod install;
#[path = "flow_remove.rs"]
mod remove;
#[path = "flow_review.rs"]
mod review;
#[path = "flow_select.rs"]
mod select;
#[path = "flow_update.rs"]
mod update;

pub use finish::{Confirmed, Progress};
pub use home::{AgentRow, AgentState, HomeAction, HomeView};
pub use remove::Memories;
pub use review::{FileGroup, MemoriesChoice, ReviewItem, ReviewView};
pub use select::{Choice, FeatureRow};
pub use update::{Command, Exec, Relaunch, Releases, Restart, Resumed, spawn};

/// The exit code when the user cancels or quits before confirming.
pub const EXIT_CANCELLED: i32 = 2;

/// The clock that names a run (and its backup folder).
pub type Clock = Arc<dyn Fn() -> UtcTimestamp + Send + Sync>;

/// Whether a program other than this setup is running the mmhmem program
/// at the given path (an AI tool's `mmhmem stdio`, another window).
pub type InUse = Arc<dyn Fn(&Path) -> bool + Send + Sync>;

/// Everything the flow needs from outside.
#[derive(Clone)]
pub struct Deps {
    /// The agents' environment (home, `PATH`, CLI runner, MCP probe).
    pub env: Env,
    /// Where the install record and backups live.
    pub store: RecordStore,
    /// The installed mmhmem program the agents are set up to launch: always
    /// its permanent place, never a temporary copy.
    pub binary: PathBuf,
    /// The program to copy to [`Deps::binary`] (the bootstrap's temporary
    /// copy); `None` keeps what is there.
    pub install_from: Option<PathBuf>,
    /// The user's `PATH` value (the registry on Windows; none elsewhere).
    pub paths: Arc<dyn UserPathStore>,
    /// Where updates come from and the keys they must be signed with.
    pub releases: Releases,
    /// This build's release tag.
    pub version: ReleaseTag,
    /// The run clock.
    pub now: Clock,
    /// The memory store a removal deletes when the user asks for it
    /// (`None`: unknown, the option is not offered).
    pub memories: Option<Memories>,
    /// Starting the new program after an update (task 7.1 follow-up).
    pub restart: Restart,
    /// Whether another program still runs the installed program (a
    /// removal keeps it then, see `flow_remove.rs`).
    pub in_use: InUse,
}

impl Deps {
    /// Dependencies of a real run: the system clock, this process's `PATH`
    /// store and GitHub releases; nothing to copy.
    pub fn new(env: Env, store: RecordStore, binary: PathBuf, version: ReleaseTag) -> Self {
        Self {
            env,
            store,
            binary,
            install_from: None,
            paths: process_store(),
            releases: Releases::github(),
            version,
            now: Arc::new(UtcTimestamp::now),
            memories: None,
            restart: Restart::default(),
            in_use: Arc::new(crate::setup::running::used_by_other_process),
        }
    }
}

/// The newest published release, as far as the menu knows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Newest {
    /// The lookup has not answered yet.
    Checking,
    /// The newest stable release.
    Known(ReleaseTag),
    /// The lookup failed (offline, rate limit); the menu goes on without it.
    Unknown,
}

/// Where the screens go after [`Flow::begin`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Next {
    /// Choose the agents.
    Agents,
    /// Review a plan.
    Review,
    /// Results are ready.
    Progress,
    /// The program was updated: leave the screens and start the new
    /// program ([`Flow::relaunch`]), which plans the agents again with its
    /// own files and asks only if that changes anything ([`Flow::resume`]).
    Restart,
    /// Stay on the home screen and show this line.
    Note(&'static str),
}

/// What [`Flow::load`] found.
struct Loaded {
    record: Option<InstallRecord>,
    notes: Vec<String>,
    scan: Option<Scan>,
    detections: Vec<(AgentId, Detection)>,
    profiles: Vec<PathBuf>,
    current: bool,
}

/// The setup controller (see the module docs).
pub struct Flow {
    deps: Deps,
    loaded: Loaded,
    newest: Newest,
    selection: Selection,
    profile: Option<PathBuf>,
    worker: bool,
    /// A worker model is already set up (decided as `mmhmem hook` does).
    worker_ready: bool,
    planned: Option<review::Planned>,
    progress: Option<Progress>,
    /// A line above the next review (after an update).
    intro: Option<String>,
    /// The removal also deletes the saved memories.
    delete_memories: bool,
    /// The release the program was updated to, when the new program must
    /// be started.
    relaunch: Option<ReleaseTag>,
}

impl Flow {
    /// Reads the state for the home screen. Writes nothing.
    pub fn load(deps: Deps) -> Self {
        let mut notes = Vec::new();
        let mut record = deps.store.load().unwrap_or_else(|_| {
            notes.push(text::RECORD_UNREADABLE.to_owned());
            None
        });
        if let Some(rec) = record.as_mut() {
            let dirs = install::user_path_dirs(&deps);
            notes.extend(recover(rec, deps.env.home(), &dirs));
            notes.extend(drift_notes(rec, deps.env.home(), &dirs));
            notes.extend(binary_note(rec));
        }
        let scan = record
            .is_none()
            .then(|| adopt::scan(&deps.env.dirs))
            .filter(|s| !s.agents.is_empty());
        if let Some(scan) = &scan {
            notes.extend(adoption_notes(scan, deps.env.home()));
        }
        let detections = registry(&deps.binary, None)
            .agents()
            .map(|a| (a.id(), a.detect(&deps.env)))
            .collect();
        let profiles = claude_desktop::profiles(&deps.env.dirs)
            .into_iter()
            .map(|p| p.config_dir)
            .collect();
        let loaded = Loaded {
            record,
            notes,
            scan,
            detections,
            profiles,
            current: false,
        };
        let worker_ready = deps.env.worker_configured();
        let mut flow = Self {
            deps,
            loaded,
            newest: Newest::Checking,
            selection: Selection::new(),
            profile: None,
            worker: false,
            worker_ready,
            planned: None,
            progress: None,
            intro: None,
            delete_memories: false,
            relaunch: None,
        };
        flow.loaded.current = flow.is_current();
        flow
    }

    /// Hands over the answer of the newest-release lookup.
    pub fn set_newest(&mut self, newest: Newest) {
        self.newest = newest;
    }

    /// Where releases are looked up and downloaded.
    pub fn release_source(&self) -> crate::setup::release::ReleaseSource {
        self.deps.releases.source.clone()
    }

    /// The newest release as far as known.
    pub fn newest(&self) -> &Newest {
        &self.newest
    }

    /// The install record as loaded (after in-memory recovery) or as stored
    /// by the last confirm or check.
    pub fn record(&self) -> Option<&InstallRecord> {
        self.loaded.record.as_ref()
    }

    /// The choices being edited.
    pub fn selection(&self) -> &Selection {
        &self.selection
    }

    /// Whether the user asked for a worker model (set up in a later step;
    /// nothing is planned for it here).
    pub fn worker_wanted(&self) -> bool {
        self.worker
    }

    /// The results of the last confirm or check.
    pub fn progress(&self) -> Option<&Progress> {
        self.progress.as_ref()
    }

    /// The exit code: the results' code, or [`EXIT_CANCELLED`] when nothing
    /// was confirmed.
    pub fn exit_code(&self) -> i32 {
        self.progress.as_ref().map_or(EXIT_CANCELLED, |p| p.code)
    }

    fn registry(&self) -> Registry {
        registry(&self.deps.binary, self.profile.as_deref())
    }

    fn detection(&self, id: AgentId) -> Option<&Detection> {
        self.loaded
            .detections
            .iter()
            .find(|(a, _)| *a == id)
            .map(|(_, d)| d)
    }

    fn tidy(&self, text: &str) -> String {
        tidy(self.deps.env.home(), text)
    }
}

/// The built-in agents launching `binary`, with the Claude Desktop profile
/// the user picked.
fn registry(binary: &Path, profile: Option<&Path>) -> Registry {
    let mut desktop = claude_desktop::ClaudeDesktop::new(binary);
    if let Some(dir) = profile {
        desktop = desktop.with_profile(dir);
    }
    Registry::new(vec![
        Box::new(claude_code::ClaudeCode::new(binary)),
        Box::new(desktop),
        Box::new(codex::Codex::new(binary)),
        Box::new(opencode::OpenCode::new(binary)),
        Box::new(antigravity::Antigravity::new(binary)),
    ])
    .unwrap_or_else(|_| Registry::builtin(binary))
}

/// `text` with the home folder shown as `~`.
pub(crate) fn tidy(home: &Path, text: &str) -> String {
    let home = home.display().to_string();
    if home.is_empty() {
        text.to_owned()
    } else {
        text.replace(&home, "~")
    }
}

/// The file a target lives in, for the user.
pub(super) fn shown_file(home: &Path, target: &Target) -> String {
    target_file(target).map_or_else(
        || text::PATH_SETTING.to_owned(),
        |f| tidy(home, &f.display().to_string()),
    )
}

/// Resolves the `applying` entries of an interrupted run in memory: written
/// ones are finished, unwritten ones undone, the rest reported.
fn recover(record: &mut InstallRecord, home: &Path, dirs: &[PathBuf]) -> Vec<String> {
    let probe = Probe {
        user_path: Some(dirs),
    };
    let mut notes = Vec::new();
    let mut resolved = false;
    for report in recovery_scan(record, &probe) {
        let done = match report.recovery {
            Recovery::Applied => record.finish_change(report.scope, &report.target),
            Recovery::NotApplied => record.undo_change(report.scope, &report.target),
            Recovery::Diverged { .. } | Recovery::Unreadable { .. } => {
                let file = shown_file(home, &report.target);
                notes.push(text::recovery_unclear(&file));
                continue;
            }
        };
        resolved |= done.is_ok();
    }
    if resolved {
        notes.insert(0, text::RECOVERED.to_owned());
    }
    notes
}

/// One line per managed entry changed or deleted outside setup.
fn drift_notes(record: &InstallRecord, home: &Path, dirs: &[PathBuf]) -> Vec<String> {
    let probe = Probe {
        user_path: Some(dirs),
    };
    detect_drift(record, &probe)
        .into_iter()
        .filter_map(|report| {
            let file = shown_file(home, &report.target);
            match report.drift {
                Drift::Unchanged => None,
                Drift::Changed { .. } => Some(text::drift_changed(&file)),
                Drift::Missing => Some(text::drift_missing(&file)),
                Drift::Unreadable { reason } => Some(text::drift_unreadable(&file, &reason)),
            }
        })
        .collect()
}

/// Whether the installed program still matches the record (2.6).
fn binary_note(record: &InstallRecord) -> Option<String> {
    let binary = record.binary.as_ref()?;
    match status_hash(&binary.path, &binary.sha256).ok()? {
        BinaryStatus::Match => None,
        BinaryStatus::Mismatch { .. } => Some(text::BINARY_CHANGED.to_owned()),
        BinaryStatus::Missing => Some(text::BINARY_MISSING.to_owned()),
    }
}

/// What the adoption scan found, per agent.
fn adoption_notes(scan: &Scan, home: &Path) -> Vec<String> {
    let mut notes = Vec::new();
    for agent in &scan.agents {
        if agent.can_adopt() && !agent.artefacts.is_empty() {
            notes.push(text::adopt_found(agent.agent.label()));
        }
        notes.extend(agent.problem_lines().iter().map(|l| tidy(home, l)));
    }
    notes
}

#[cfg(test)]
#[path = "flow_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "flow_install_tests.rs"]
mod install_tests;
