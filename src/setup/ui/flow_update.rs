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
//! new program goes straight to the review of the recorded choices with
//! its own files ([`Flow::resume`]).
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

use super::{Flow, Next, text};
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

/// How the new program is started after an update.
#[derive(Clone, Default)]
pub struct Restart {
    /// Starts it (`None`: never restart; the run was restarted already).
    pub exec: Option<Exec>,
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
    /// `setup …` with the internal arguments and `--restarted <tag>`.
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
        let mut args = vec!["setup".to_owned()];
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
    /// this program's files for the review. `false` (and nothing done) in
    /// any other run.
    pub fn resume(&mut self) -> bool {
        let Some(tag) = self.deps.restart.resumed.take() else {
            return false;
        };
        self.intro = Some(text::updated(tag.as_str()));
        self.selection = self.recorded_selection();
        self.make_plan();
        true
    }

    /// Updates the program to `tag`, then plans the recorded choices again.
    pub(super) fn update_to(&mut self, tag: &ReleaseTag) -> Next {
        let releases = self.deps.releases.clone();
        if releases.keys.is_empty() {
            return Next::Note(text::UPDATE_NO_KEY);
        }
        let Some(target) = releases.target else {
            return Next::Note(text::UPDATE_NO_PLATFORM);
        };
        let installed = match release::update_binary(
            &releases.source,
            tag,
            target,
            &releases.keys,
            &self.deps.binary,
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
