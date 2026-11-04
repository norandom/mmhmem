//! The install-wide part of a plan (task 7.1, design.md §1 and §3;
//! Requirements 2.1, 2.6, 5.1 and 7.1): the program at its permanent place
//! and its folder on the user's `PATH`.
//!
//! **The program.** The agents always launch [`Deps::binary`], the
//! permanent place (Windows `%LOCALAPPDATA%\Programs\mmhmem\mmhmem.exe`,
//! elsewhere `~/.local/bin/mmhmem`), never the temporary copy the bootstrap
//! started. When [`Deps::install_from`] names that copy, the plan starts
//! with a reviewed [`Target::Binary`] change that copies it there (no diff:
//! the review shows its SHA-256). Identical bytes plan nothing, so a second
//! run of the same release has no change.
//!
//! **`PATH`.** Where the user's `PATH` can be changed (Windows,
//! [`UserPathStore::supported`]) and the record has not settled it yet, the
//! folder is added once as a reviewed [`Target::PathEntry`] change. On Linux
//! and macOS nothing is edited: the record says `added: false` and, when the
//! folder is not on `PATH`, the results tell the user how to add it.
//!
//! [`Deps::binary`]: super::Deps::binary
//! [`Deps::install_from`]: super::Deps::install_from
//! [`UserPathStore::supported`]: crate::setup::user_path::UserPathStore::supported

use std::path::PathBuf;

use super::{Deps, Flow, text};
use crate::setup::plan::{Action, Change};
use crate::setup::record::{
    BinaryInfo, EntryState, InstallRecord, PathEntryInfo, Scope, Sha256Hex, Target,
};
use crate::setup::user_path::{PathValue, on_path, path_hint, same_folder};

impl Flow {
    /// The install-wide changes, in apply order: the program, then `PATH`.
    pub(super) fn install_changes(&self) -> Vec<Change> {
        let mut changes = Vec::new();
        changes.extend(self.binary_change());
        changes.extend(self.path_change());
        changes
    }

    /// Copy the running program to its permanent place, when asked to and
    /// the bytes differ.
    fn binary_change(&self) -> Option<Change> {
        let from = self.deps.install_from.as_ref()?;
        let after = std::fs::read(from).ok()?;
        let path = self.deps.binary.clone();
        let before = std::fs::read(&path).ok();
        if before.as_ref() == Some(&after) {
            return None;
        }
        let sha = Sha256Hex::of(&after);
        let update = before.is_some();
        Some(Change {
            scope: Scope::Install,
            target: Target::Binary { path: path.clone() },
            file: path,
            action: if update { Action::Update } else { Action::Add },
            before,
            after: Some(after),
            owner_only: false,
            hide_diff: false,
            missing_parents: None,
            summary: text::change_binary(update, sha.as_str()),
        })
    }

    /// Add the program's folder to the user's `PATH` (Windows), once.
    fn path_change(&self) -> Option<Change> {
        let paths = &self.deps.paths;
        let settled = self
            .loaded
            .record
            .as_ref()
            .is_some_and(|r| r.path_entry.is_some());
        if !paths.supported() || settled {
            return None;
        }
        let folder = self.deps.binary.parent()?.to_path_buf();
        let current = paths.read().ok()?;
        if current.as_ref().is_some_and(|v| v.contains(&folder)) {
            return None;
        }
        let after = current
            .as_ref()
            .map_or_else(|| PathValue::new_with(&folder), |v| v.with_folder(&folder));
        let shown = self.tidy(&folder.display().to_string());
        Some(Change {
            scope: Scope::Install,
            target: Target::PathEntry {
                value: folder.clone(),
            },
            file: folder,
            action: Action::Add,
            before: current.map(|v| v.encode()),
            after: Some(after.encode()),
            owner_only: false,
            hide_diff: false,
            missing_parents: None,
            summary: text::change_path(&shown),
        })
    }

    /// Records the program and the `PATH` entry after a confirm.
    pub(super) fn finish_install(&self, record: &mut InstallRecord) {
        let binary = &self.deps.binary;
        let copied = record.managed.iter().find_map(|e| match &e.target {
            Target::Binary { path } if path == binary && e.state == EntryState::Done => {
                e.sha256.clone()
            }
            _ => None,
        });
        if let Some(sha256) = copied {
            record.binary = Some(BinaryInfo {
                path: binary.clone(),
                sha256,
            });
        } else if record.binary.is_none()
            && let Ok(bytes) = std::fs::read(binary)
        {
            record.binary = Some(BinaryInfo {
                path: binary.clone(),
                sha256: Sha256Hex::of(&bytes),
            });
        }
        if record.path_entry.is_none() {
            record.path_entry = self.path_entry_of(record);
        }
    }

    /// `added: true` when this install put the folder on `PATH`; `false`
    /// when it was there already or setup does not change `PATH` here;
    /// `None` (offered again next time) when the change did not happen.
    fn path_entry_of(&self, record: &InstallRecord) -> Option<PathEntryInfo> {
        let folder = self.deps.binary.parent()?.to_path_buf();
        let added = record.managed.iter().any(|e| {
            e.state == EntryState::Done
                && matches!(&e.target, Target::PathEntry { value } if same_folder(value, &folder))
        });
        let known =
            added || !self.deps.paths.supported() || on_path(&user_path_dirs(&self.deps), &folder);
        known.then_some(PathEntryInfo {
            added,
            value: folder,
        })
    }

    /// Linux and macOS: how to put the program's folder on `PATH`, when it
    /// is not there.
    pub(super) fn path_next_step(&self) -> Option<String> {
        if self.deps.paths.supported() {
            return None;
        }
        let folder = self.deps.binary.parent()?;
        path_hint(self.deps.env.path_dirs(), folder, self.deps.env.home())
    }
}

/// The folders on the user's `PATH` for drift and recovery: the stored
/// user value where setup can change it (Windows), else the process `PATH`.
pub(super) fn user_path_dirs(deps: &Deps) -> Vec<PathBuf> {
    if deps.paths.supported() {
        return deps
            .paths
            .read()
            .ok()
            .flatten()
            .map(|v| v.entries())
            .unwrap_or_default();
    }
    deps.env.path_dirs().to_vec()
}
