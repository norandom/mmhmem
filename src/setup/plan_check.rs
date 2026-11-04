//! The pre-write check of one agent's changes (task 2.5, design.md §4
//! "Validation before writing"; Requirements 3.2 and 8.5).
//!
//! Before the first write for an agent, every change of that agent is
//! checked in plan order:
//!
//! - **Shape:** the change's file is its target's file, a `PATH` entry is not
//!   a file change, and only a whole file mmhmem owns may be deleted.
//! - **Stale guard:** the first change of each file must find exactly its
//!   `before` bytes on disk; a later change of the same file must start from
//!   the previous change's `after`.
//! - **Re-parse:** the new content is parsed with the file's own parser
//!   (JSON/JSONC via `edit::json`, TOML via `edit::toml`; Markdown and other
//!   files are not parsed), and a strict JSON file must stay strict JSON.
//! - **Entries:** the target's hash before and after comes from the planned
//!   bytes, the action matches it (a removal leaves no entry, anything else
//!   leaves one), and a later change in the same file leaves every earlier
//!   change's entry exactly as planned.
//!
//! Reasons are fixed text and positions, never contents.

use std::io;
use std::path::{Path, PathBuf};

use super::{Action, Change, Skip, target_file};
use crate::setup::edit;
use crate::setup::record::{Observation, Sha256Hex, Target, observe_in};

/// The parser a file's new content is checked with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FileKind {
    Json,
    Toml,
    Text,
}

/// Entry hashes of a change: (before, after), `None` for an absent entry.
pub(super) type Hashes = (Option<Sha256Hex>, Option<Sha256Hex>);

/// What the check knows about one file while walking the agent's changes.
struct FileState {
    file: PathBuf,
    bytes: Option<Vec<u8>>,
    entries: Vec<(Target, Option<Sha256Hex>)>,
}

/// Check all of one agent's changes before anything is written.
pub(super) fn check_agent(changes: &[&Change]) -> Result<(), Skip> {
    let mut files: Vec<FileState> = Vec::new();
    for change in changes {
        let invalid = |reason: &str| Skip::Invalid {
            file: change.file.clone(),
            reason: reason.to_owned(),
        };
        check_shape(change).map_err(invalid)?;
        let index = match files.iter().position(|s| s.file == change.file) {
            Some(i) if files[i].bytes != change.before => {
                return Err(invalid(
                    "the planned changes for this file do not follow each other",
                ));
            }
            Some(i) => i,
            None => {
                if read_current(&change.file)? != change.before {
                    return Err(Skip::Stale {
                        file: change.file.clone(),
                    });
                }
                files.push(FileState {
                    file: change.file.clone(),
                    bytes: change.before.clone(),
                    entries: Vec::new(),
                });
                files.len() - 1
            }
        };
        validate_content(change).map_err(|r| invalid(&r))?;
        let (_, after) = hashes(change).map_err(|r| invalid(&r))?;
        let state = &mut files[index];
        for (target, planned) in &state.entries {
            if &entry_hash(target, change.after.as_deref()).map_err(|r| invalid(&r))? != planned {
                return Err(invalid(
                    "a later change would undo an earlier one in this file",
                ));
            }
        }
        state.bytes = change.after.clone();
        state.entries.push((change.target.clone(), after));
    }
    Ok(())
}

/// The file's bytes now (`None` when it does not exist).
pub(super) fn read_current(file: &Path) -> Result<Option<Vec<u8>>, Skip> {
    match std::fs::read(file) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e)
            if matches!(
                e.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
            ) =>
        {
            Ok(None)
        }
        Err(e) => Err(Skip::Unreadable {
            file: file.to_path_buf(),
            reason: e.kind().to_string(),
        }),
    }
}

/// The entry hashes before and after `change`, from the planned bytes, with
/// the action checked against them.
pub(super) fn hashes(change: &Change) -> Result<Hashes, String> {
    let before = entry_hash(&change.target, change.before.as_deref())?;
    let after = entry_hash(&change.target, change.after.as_deref())?;
    match (change.action, &after) {
        (Action::Remove, Some(_)) => Err("the removal leaves mmhmem's entry in place".to_owned()),
        (Action::Add | Action::Update, None) => {
            Err("the change would leave no mmhmem entry but is not a removal".to_owned())
        }
        _ => Ok((before, after)),
    }
}

/// The hash of `target` in `bytes` (`None`: no file or no entry).
fn entry_hash(target: &Target, bytes: Option<&[u8]>) -> Result<Option<Sha256Hex>, String> {
    let Some(bytes) = bytes else {
        return Ok(None);
    };
    match observe_in(target, bytes) {
        Observation::Present(hash) => Ok(Some(hash)),
        Observation::Missing => Ok(None),
        Observation::Unreadable(reason) => Err(reason),
    }
}

fn check_shape(change: &Change) -> Result<(), &'static str> {
    match target_file(&change.target) {
        None => return Err("a PATH entry is not a file change"),
        Some(file) if file != change.file => {
            return Err("the change names a different file than its entry");
        }
        Some(_) => {}
    }
    let whole_file = matches!(change.target, Target::File { .. } | Target::Binary { .. });
    if change.after.is_none() && !whole_file {
        return Err("only a whole file mmhmem owns can be deleted");
    }
    Ok(())
}

fn kind(change: &Change) -> FileKind {
    match &change.target {
        Target::JsonEntry { .. } | Target::JsonArrayElement { .. } => FileKind::Json,
        Target::TomlTable { .. } => FileKind::Toml,
        Target::MarkdownSection { .. } | Target::PathEntry { .. } => FileKind::Text,
        Target::File { file } | Target::Binary { path: file } => {
            match file.extension().and_then(|e| e.to_str()) {
                Some(e) if e.eq_ignore_ascii_case("json") || e.eq_ignore_ascii_case("jsonc") => {
                    FileKind::Json
                }
                Some(e) if e.eq_ignore_ascii_case("toml") => FileKind::Toml,
                _ => FileKind::Text,
            }
        }
    }
}

/// Re-parse the new content with the file's own parser (Requirement 3.2).
fn validate_content(change: &Change) -> Result<(), String> {
    let Some(after) = change.after.as_deref() else {
        return Ok(());
    };
    let kind = kind(change);
    if kind == FileKind::Text {
        return Ok(());
    }
    let text = std::str::from_utf8(after).map_err(|_| "the file is not UTF-8 text".to_owned())?;
    if kind == FileKind::Toml {
        return edit::toml::validate(text).map_err(|e| e.to_string());
    }
    let dialect = edit::json::dialect(text).map_err(|e| e.to_string())?;
    let was_strict = change
        .before
        .as_deref()
        .and_then(|b| std::str::from_utf8(b).ok())
        .is_some_and(|b| edit::json::dialect(b) == Ok(edit::json::Dialect::Json));
    if was_strict && dialect != edit::json::Dialect::Json {
        return Err("the file would no longer be plain JSON".to_owned());
    }
    Ok(())
}
