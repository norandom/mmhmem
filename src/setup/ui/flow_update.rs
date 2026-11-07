//! "Update to vX.Y.Z" (task 7.1, design.md §8; Requirements 7.1 and 7.2).
//!
//! 1. Without a trusted release key nothing is downloaded and nothing
//!    changes ([`text::UPDATE_NO_KEY`]): `self_update` would skip the
//!    signature check otherwise (research.md §11).
//! 2. [`release::update_binary`] downloads, verifies and puts the new
//!    program at its permanent place (in place, through `self_replace`, when
//!    that is the running program, which covers the Windows file-in-use
//!    case). On any failure the program is unchanged and the reason is
//!    shown.
//! 3. The record gets the new program's hash and the release tag at once
//!    (the program is already replaced), then every agent is planned again
//!    with the recorded choices and the Review screen shows what changes.
//!    Confirming applies and checks as usual; an empty review says
//!    "Everything is current".
//!
//! **Restart into the new program.** The running program still carries the
//! previous release's files (skills, commands, hooks), so after a
//! successful update it does not plan the agents itself: [`Flow::begin`]
//! returns [`Next::Restart`], the screens close, and the caller starts the
//! new program at its permanent place as `setup <same internal arguments>
//! --restarted <tag>`, with the same terminal (stdin, stdout and stderr are
//! inherited), and exits with its exit code ([`Flow::restart_now`]). The
//! new program plans the recorded choices with its own files
//! ([`Flow::resume`]) and asks only if needed (owner decision 2026-11-06,
//! Requirement 7.5): when that plan changes nothing it only checks the
//! tools ([`Resumed::Check`], [`Flow::apply_update`]); otherwise it shows
//! the review ([`Resumed::Review`]). `mmhmem update` restarts as
//! `update …` instead of `setup …` ([`Command`]).
//!
//! Loop guard: a run started with `--restarted` never restarts again (its
//! [`Restart::exec`] is `None`), and no restart happens when the release
//! updated to is this program's own release. Without an `exec` (tests and
//! the restarted run) the agents are planned in this process as before.
//!
//! The release source, the trusted keys and the platform come from
//! [`Deps::releases`] (GitHub and the built-in keys for real runs; a
//! `--release-dir` folder or test keys otherwise).
//!
//! [`Deps::releases`]: super::Deps::releases

use std::path::PathBuf;
use std::sync::Arc;

use super::home::is_newer;
use super::{Confirmed, Flow, Next, ReviewView, text};
use crate::setup::download_line;
use crate::setup::record::{BinaryInfo, InstallRecord, ReleaseTag, Target};
use crate::setup::release::{self, LocalRelease, ReleaseSource};
use crate::setup::release_keys::trusted_release_keys;

/// Where updates come from and the keys that must have signed them.
#[derive(Debug, Clone)]
pub struct Releases {
    /// The release server.
    pub source: ReleaseSource,
    /// The trusted release keys (empty: updates are refused).
    pub keys: Vec<[u8; 32]>,
    /// This computer's platform (`None`: there are no releases for it).
    pub target: Option<release::Target>,
    /// Keeps a `--release-dir` folder served while the flow runs.
    pub local: Option<Arc<LocalRelease>>,
}

impl Releases {
    /// GitHub releases checked with the keys built into this program.
    pub fn github() -> Self {
        Self {
            source: ReleaseSource::github(),
            keys: trusted_release_keys(),
            target: release::Target::current().ok(),
            local: None,
        }
    }

    /// A local release folder (`--release-dir`), with the built-in keys.
    pub fn local(server: LocalRelease) -> Self {
        Self {
            source: server.source().clone(),
            local: Some(Arc::new(server)),
            ..Self::github()
        }
    }
}

/// Starts a program with arguments, inheriting the terminal, and returns
/// its exit code.
pub type Exec = Arc<dyn Fn(&Relaunch) -> std::io::Result<i32> + Send + Sync>;

/// The command the new program is started with after an update.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Command {
    /// `mmhmem setup` (the menu).
    #[default]
    Setup,
    /// `mmhmem update` (plain questions, `ui/update.rs`).
    Update,
}

impl Command {
    /// The command's name on the command line.
    pub fn name(self) -> &'static str {
        match self {
            Self::Setup => "setup",
            Self::Update => "update",
        }
    }
}

/// What the run started after an update does first ([`Flow::resume`]);
/// both hold the release updated to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resumed {
    /// The recorded choices change files: show the review.
    Review(ReleaseTag),
    /// Nothing needs to change: only check the tools
    /// ([`Flow::apply_update`]).
    Check(ReleaseTag),
}

/// How the new program is started after an update.
#[derive(Clone, Default)]
pub struct Restart {
    /// Starts it (`None`: never restart; the run was restarted already).
    pub exec: Option<Exec>,
    /// `setup` or `update`.
    pub command: Command,
    /// The internal arguments to pass again (`--release-dir <dir>`).
    pub args: Vec<String>,
    /// In the restarted run: the release the previous run updated to.
    pub resumed: Option<ReleaseTag>,
}

/// The new program and its arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Relaunch {
    /// The program at its permanent place.
    pub program: PathBuf,
    /// `setup …` or `update …` with the internal arguments and
    /// `--restarted <tag>`.
    pub args: Vec<String>,
}

/// The real [`Exec`]: runs the program with this process's stdin, stdout
/// and stderr and waits for it (a missing exit code, killed by a signal,
/// counts as 1).
pub fn spawn(relaunch: &Relaunch) -> std::io::Result<i32> {
    let status = std::process::Command::new(&relaunch.program)
        .args(&relaunch.args)
        .status()?;
    Ok(status.code().unwrap_or(1))
}

impl Flow {
    /// The new program to start, when an update asked for it.
    pub fn relaunch(&self) -> Option<Relaunch> {
        let tag = self.relaunch.as_ref()?;
        let mut args = vec![self.deps.restart.command.name().to_owned()];
        args.extend(self.deps.restart.args.iter().cloned());
        args.extend(["--restarted".to_owned(), tag.as_str().to_owned()]);
        Some(Relaunch {
            program: self.deps.binary.clone(),
            args,
        })
    }

    /// Starts the new program after an update and waits for it: `None`
    /// when no restart is due, else its exit code, or why it could not
    /// start.
    pub fn restart_now(&self) -> Option<Result<i32, String>> {
        let relaunch = self.relaunch()?;
        let exec = self.deps.restart.exec.as_ref()?;
        Some(exec(&relaunch).map_err(|e| e.kind().to_string()))
    }

    /// In the run started after an update: plans the recorded choices with
    /// this program's files. [`Resumed::Review`] when that changes
    /// anything (the review intro says the program was updated),
    /// [`Resumed::Check`] when not; `None` (and nothing done) in any other
    /// run.
    pub fn resume(&mut self) -> Option<Resumed> {
        let tag = self.deps.restart.resumed.take()?;
        self.plan_recorded();
        if self.planned.as_ref().is_some_and(|p| p.is_empty()) {
            return Some(Resumed::Check(tag));
        }
        self.intro = Some(text::updated(tag.as_str()));
        Some(Resumed::Review(tag))
    }

    /// Plans the recorded choices (after an update, or with the program the
    /// install script brought). Reads only.
    pub fn plan_recorded(&mut self) {
        self.selection = self.recorded_selection();
        self.make_plan();
    }

    /// The review of an update without the program copy the user already
    /// agreed to: empty (no change, no skipped tool) when nothing needs to
    /// be asked.
    pub fn update_review(&self) -> ReviewView {
        let mut view = self.review();
        let program: Vec<usize> = self.planned.as_ref().map_or_else(Vec::new, |p| {
            p.plan
                .changes
                .iter()
                .enumerate()
                .filter(|(_, c)| matches!(c.target, Target::Binary { .. }))
                .map(|(i, _)| i)
                .collect()
        });
        for group in &mut view.groups {
            group.items.retain(|i| !program.contains(&i.index));
        }
        view.groups.retain(|g| !g.items.is_empty());
        view
    }

    /// Finishes an update: with nothing planned only the check runs (and
    /// is stored); otherwise the plan is applied and checked
    /// ([`Flow::confirm`]). The results are in [`Flow::progress`].
    pub fn apply_update(&mut self) {
        if self.planned.as_ref().is_none_or(|p| p.is_empty()) {
            self.planned = None;
            self.run_check();
            return;
        }
        while self.confirm() == Confirmed::Review {}
    }

    /// The install script brought a newer release than the recorded one:
    /// `(recorded, brought)`. `None` in every other run.
    pub fn bootstrap_offer(&self) -> Option<(ReleaseTag, ReleaseTag)> {
        self.deps.install_from.as_ref()?;
        if !self.is_installed() {
            return None;
        }
        let recorded = self.loaded.record.as_ref()?.release.clone()?;
        is_newer(&self.deps.version, &recorded).then(|| (recorded, self.deps.version.clone()))
    }

    /// Why no update can be downloaded at all (no trusted release key, no
    /// release for this platform); `None` when it may be tried.
    pub fn update_refusal(&self) -> Option<&'static str> {
        if self.deps.releases.keys.is_empty() {
            Some(text::UPDATE_NO_KEY)
        } else if self.deps.releases.target.is_none() {
            Some(text::UPDATE_NO_PLATFORM)
        } else {
            None
        }
    }

    /// The last note (why an update failed).
    pub fn last_note(&self) -> Option<&str> {
        self.loaded.notes.last().map(String::as_str)
    }

    /// Updates the program to `tag`, then plans the recorded choices again.
    /// `progress` (the download-progress hook) is handed to the download.
    pub(super) fn update_to(
        &mut self,
        tag: &ReleaseTag,
        progress: Option<download_line::ProgressHook>,
    ) -> Next {
        if let Some(note) = self.update_refusal() {
            return Next::Note(note);
        }
        let releases = self.deps.releases.clone();
        let Some(target) = releases.target else {
            return Next::Note(text::UPDATE_NO_PLATFORM);
        };
        let installed = match release::update_binary_with_progress(
            &releases.source,
            tag,
            target,
            &releases.keys,
            &self.deps.binary,
            progress,
        ) {
            Ok(installed) => installed,
            Err(e) => {
                let reason = self.tidy(&e.to_string());
                self.loaded.notes.push(reason);
                return Next::Note(text::UPDATE_FAILED);
            }
        };
        let mut record = self.loaded.record.clone().unwrap_or_default();
        record_update(&mut record, &installed);
        if let Err(e) = self.deps.store.save(&record) {
            self.loaded.notes.push(self.tidy(&e.to_string()));
        }
        self.loaded
            .notes
            .retain(|n| n != text::BINARY_CHANGED && n != text::BINARY_MISSING);
        self.loaded.record = Some(record);
        let same_release = *tag == self.deps.version;
        self.deps.version = tag.clone();
        // The new program is in place; nothing else may overwrite it.
        self.deps.install_from = None;
        if self.deps.restart.exec.is_some() && !same_release {
            self.relaunch = Some(tag.clone());
            return Next::Restart;
        }
        self.intro = Some(text::updated(tag.as_str()));
        self.selection = self.recorded_selection();
        self.make_plan();
        Next::Review
    }
}

/// The new program's hash and release in the record; a managed program
/// entry follows, so it is not reported as changed outside setup.
fn record_update(record: &mut InstallRecord, installed: &release::InstalledBinary) {
    record.release = Some(installed.tag.clone());
    record.binary = Some(BinaryInfo {
        path: installed.path.clone(),
        sha256: installed.sha256.clone(),
    });
    let target = Target::Binary {
        path: installed.path.clone(),
    };
    for entry in record.managed.iter_mut().filter(|e| e.target == target) {
        entry.sha256 = Some(installed.sha256.clone());
    }
}
