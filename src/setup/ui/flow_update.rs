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
//! The release source, the trusted keys and the platform come from
//! [`Deps::releases`] (GitHub and the built-in keys for real runs; a
//! `--release-dir` folder or test keys otherwise).
//!
//! Limitation: the agents are planned with the assets of the running
//! program, which is still the previous release until setup starts again.
//!
//! [`Deps::releases`]: super::Deps::releases

use std::sync::Arc;

use super::{Flow, Next, text};
use crate::setup::record::{BinaryInfo, InstallRecord, ReleaseTag, Target};
use crate::setup::release::{self, LocalRelease, ReleaseSource};
use crate::setup::release_keys::RELEASE_PUBLIC_KEYS;

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
            keys: RELEASE_PUBLIC_KEYS.to_vec(),
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

impl Flow {
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
        self.deps.version = tag.clone();
        // The new program is in place; nothing else may overwrite it.
        self.deps.install_from = None;
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
