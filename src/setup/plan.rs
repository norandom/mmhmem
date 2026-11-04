//! Plans, apply and per-agent rollback (task 2.5,
//! `.kiro/specs/reliable-installers/design.md` §3 "Atomic writes" and §4;
//! Requirements 3.2, 3.3, 3.5, 5.2 and 8.5).
//!
//! A [`Plan`] is built once, shown on the review screen and then applied
//! unchanged: [`apply`] writes exactly each [`Change::after`], nothing is
//! recomputed after the user confirms (8.5). Submodules:
//!
//! - `plan_check.rs`: the pre-write check of one agent's changes (stale
//!   files, re-parse of the new content, plan coherence).
//! - `plan_apply.rs`: the journaled apply and the rollback.
//!
//! **One change per entry, chained per file.** A [`Change`] carries one
//! record [`Target`] plus the *whole-file* bytes before and after it. When
//! several entries live in one file (an MCP entry and a hook group in one
//! `settings.json`), the planner emits one change per entry in order, and
//! each later change's `before` is exactly the previous change's `after`.
//! Every change is therefore reviewed with its own diff, written as exactly
//! the reviewed bytes, and journaled as its own record entry. The pre-write
//! check refuses a chain that does not follow on, and a later change that
//! would alter an earlier change's entry, so the record always matches the
//! final file.
//!
//! **Per change, in order (5.2):** check the file still holds `before`;
//! back up the file into the run's backup folder; save the record with the
//! entry `applying` (its `after` hash computed from the planned bytes); write
//! the file atomically (owner-only when [`Change::owner_only`], or delete it
//! when `after` is `None`); save the record with the entry `done`.
//!
//! **Per agent (3.2, 3.5):** all of an agent's changes are checked before the
//! first write. A stale file or invalid new content skips the whole agent
//! with nothing written. A failure during the writes restores that agent's
//! files from this run's backups in reverse order, puts its record entries
//! back as they were, and the other agents continue.
//!
//! Nothing here prints or logs file contents; [`Change`]'s `Debug` shows only
//! lengths and messages name files, never contents (Requirement 9).

use std::fmt;
use std::path::{Path, PathBuf};

use crate::setup::record::{BackupPath, ManagedEntry, Scope, Sha256Hex, Target, UtcTimestamp};
use crate::setup::user_path::PathValue;

#[path = "plan_check.rs"]
mod check;
#[path = "plan_apply.rs"]
mod engine;

pub use engine::{apply, apply_with, rollback_agent};

/// The diff line shown instead of the contents of an owner-only file or a
/// change with [`Change::hide_diff`].
pub const HIDDEN_DIFF: &str = "contents hidden (contains a key)";
/// The diff line shown for content that is not UTF-8 text.
pub const BINARY_DIFF: &str = "contents not shown (not text)";
/// The diff line shown when a change leaves the file bytes as they are.
pub const NO_DIFF: &str = "no change to the file contents";

/// What a change does to its entry (for the review screen).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// mmhmem's entry or file is new.
    Add,
    /// mmhmem's entry or file is changed (or adopted unchanged).
    Update,
    /// mmhmem's entry or file is taken out.
    Remove,
}

/// Everything one run will change, in apply order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    /// When the run started: names the backup folder, one per run.
    pub run: UtcTimestamp,
    /// The changes, grouped by agent in the order agents first appear.
    pub changes: Vec<Change>,
}

/// One planned change of one record entry, with whole-file bytes.
#[derive(Clone, PartialEq, Eq)]
pub struct Change {
    /// Whose entry (an agent, or install-wide).
    pub scope: Scope,
    /// The record entry this change adds, updates or removes.
    pub target: Target,
    /// The file written; must be the file `target` names (for a
    /// [`Target::PathEntry`]: the folder).
    pub file: PathBuf,
    /// What the change does.
    pub action: Action,
    /// The whole file as read at plan time (`None`: the file did not exist).
    /// For a [`Target::PathEntry`]: the user's `PATH` value
    /// ([`PathValue::encode`]).
    pub before: Option<Vec<u8>>,
    /// The exact whole file to write (`None`: delete the file; only for a
    /// [`Target::File`] or [`Target::Binary`] mmhmem owns).
    pub after: Option<Vec<u8>>,
    /// Write with [`crate::setup::fsx::write_owner_only`] (key files). Its
    /// contents are never shown or logged.
    pub owner_only: bool,
    /// Never show the contents in the diff, but keep the file's permissions
    /// (a shared file holding account data or other tools' secrets).
    pub hide_diff: bool,
    /// Parent keys the change creates (JSON/TOML), stored in the record so
    /// removal prunes only those.
    pub missing_parents: Option<u32>,
    /// One plain-language line for the review screen.
    pub summary: String,
}

impl fmt::Debug for Change {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let len = |b: &Option<Vec<u8>>| b.as_ref().map(Vec::len);
        f.debug_struct("Change")
            .field("scope", &self.scope)
            .field("target", &self.target)
            .field("file", &self.file)
            .field("action", &self.action)
            .field("before_len", &len(&self.before))
            .field("after_len", &len(&self.after))
            .field("owner_only", &self.owner_only)
            .field("hide_diff", &self.hide_diff)
            .field("missing_parents", &self.missing_parents)
            .field("summary", &self.summary)
            .finish()
    }
}

impl Change {
    /// A unified diff of `before` → `after` for the review screen. Owner-only
    /// and hidden-diff files give [`HIDDEN_DIFF`], non-text content
    /// [`BINARY_DIFF`] and identical bytes [`NO_DIFF`].
    pub fn diff(&self) -> String {
        if self.owner_only || self.hide_diff {
            return HIDDEN_DIFF.to_owned();
        }
        if self.before == self.after {
            return NO_DIFF.to_owned();
        }
        match self.target {
            Target::PathEntry { .. } => return path_diff(&self.before, &self.after),
            Target::Binary { .. } => return BINARY_DIFF.to_owned(),
            _ => {}
        }
        let (Ok(old), Ok(new)) = (text_of(&self.before), text_of(&self.after)) else {
            return BINARY_DIFF.to_owned();
        };
        let name = self.file.display().to_string();
        let old_name = if self.before.is_some() {
            name.as_str()
        } else {
            "(new file)"
        };
        let new_name = if self.after.is_some() {
            name.as_str()
        } else {
            "(deleted)"
        };
        similar::TextDiff::from_lines(old, new)
            .unified_diff()
            .header(old_name, new_name)
            .to_string()
    }
}

/// The `PATH` folders before and after, one per line.
fn path_diff(before: &Option<Vec<u8>>, after: &Option<Vec<u8>>) -> String {
    let lines = |b: &Option<Vec<u8>>| -> String {
        b.as_deref()
            .and_then(|b| PathValue::decode(b).ok())
            .map(|v| v.entries())
            .unwrap_or_default()
            .iter()
            .map(|p| format!("{}\n", p.display()))
            .collect()
    };
    let (old, new) = (lines(before), lines(after));
    similar::TextDiff::from_lines(&old, &new)
        .unified_diff()
        .header("PATH", "PATH")
        .to_string()
}

/// The bytes as text (`""` when absent).
fn text_of(bytes: &Option<Vec<u8>>) -> Result<&str, std::str::Utf8Error> {
    bytes.as_deref().map_or(Ok(""), std::str::from_utf8)
}

impl Plan {
    /// An empty plan for the run started at `run`.
    pub fn new(run: UtcTimestamp) -> Self {
        Self {
            run,
            changes: Vec::new(),
        }
    }

    /// The scopes in the plan, in the order they first appear.
    pub fn scopes(&self) -> Vec<Scope> {
        let mut scopes = Vec::new();
        for change in &self.changes {
            if !scopes.contains(&change.scope) {
                scopes.push(change.scope);
            }
        }
        scopes
    }

    /// The changes of `scope`, in plan order.
    pub fn changes_for(&self, scope: Scope) -> impl Iterator<Item = &Change> {
        self.changes.iter().filter(move |c| c.scope == scope)
    }
}

/// The file a target lives in (`None` for a `PATH` entry, which is not a
/// file change).
pub fn target_file(target: &Target) -> Option<&Path> {
    match target {
        Target::File { file }
        | Target::JsonEntry { file, .. }
        | Target::TomlTable { file, .. }
        | Target::JsonArrayElement { file, .. }
        | Target::MarkdownSection { file, .. } => Some(file),
        Target::Binary { path } => Some(path),
        Target::PathEntry { .. } => None,
    }
}

/// Why an agent was skipped with nothing written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Skip {
    /// The file no longer holds the bytes shown on the review screen.
    Stale {
        /// The changed file.
        file: PathBuf,
    },
    /// The planned content or the plan itself is not valid.
    Invalid {
        /// The file concerned.
        file: PathBuf,
        /// Fixed reason with positions only, no contents.
        reason: String,
    },
    /// The file could not be read to compare it with the plan.
    Unreadable {
        /// The file concerned.
        file: PathBuf,
        /// The OS error kind, no contents.
        reason: String,
    },
}

impl fmt::Display for Skip {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Stale { file } => write!(
                f,
                "{} changed since you reviewed the changes; nothing was changed",
                file.display()
            ),
            Self::Invalid { file, reason } => write!(
                f,
                "the new content for {} is not valid ({reason}); nothing was changed",
                file.display()
            ),
            Self::Unreadable { file, reason } => write!(
                f,
                "{} could not be read ({reason}); nothing was changed",
                file.display()
            ),
        }
    }
}

/// How one agent's part of the plan ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Every change was written and recorded.
    Applied,
    /// A change failed or the check failed; the agent's files and record
    /// entries were restored.
    RolledBack {
        /// Plain reason (file names and OS errors, no contents).
        reason: String,
    },
    /// Restoring failed for some files; they are named in `problems`.
    RollbackIncomplete {
        /// Why the rollback started.
        reason: String,
        /// One line per file that could not be restored.
        problems: Vec<String>,
    },
    /// Nothing was written for the agent.
    Skipped(Skip),
}

/// One applied change, kept for a later rollback. Holds no file contents.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Applied {
    target: Target,
    file: PathBuf,
    backup: Option<BackupPath>,
    prior: Option<ManagedEntry>,
    before_hash: Option<Sha256Hex>,
    after_hash: Option<Sha256Hex>,
    owner_only: bool,
    /// For a `PATH` entry: the value to write back on rollback (`None`:
    /// there was no value).
    path_restore: Option<Option<PathValue>>,
}

/// The result for one agent (or the install-wide scope).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentReport {
    /// Whose changes.
    pub scope: Scope,
    /// How it ended.
    pub outcome: Outcome,
    applied: Vec<Applied>,
    created_agent: bool,
}

impl AgentReport {
    /// The backups taken for this agent's applied changes, relative to the
    /// record folder (Requirement 3.3: tell the user where they are).
    pub fn backups(&self) -> Vec<&BackupPath> {
        self.applied
            .iter()
            .filter_map(|a| a.backup.as_ref())
            .collect()
    }

    /// The files this agent's applied changes wrote, in apply order.
    pub fn files(&self) -> Vec<&Path> {
        self.applied.iter().map(|a| a.file.as_path()).collect()
    }
}

/// The result of [`apply`], one report per scope in plan order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyReport {
    /// Per-scope results.
    pub agents: Vec<AgentReport>,
}

impl ApplyReport {
    /// The report for `scope`.
    pub fn agent(&self, scope: Scope) -> Option<&AgentReport> {
        self.agents.iter().find(|a| a.scope == scope)
    }
}

/// Why [`rollback_agent`] could not run or finish.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RollbackError {
    /// The report holds no applied changes for this scope.
    NotApplied,
    /// Some files could not be restored (one line per file).
    Incomplete {
        /// One line per file, no contents.
        problems: Vec<String>,
    },
}

impl fmt::Display for RollbackError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotApplied => f.write_str("there are no applied changes to restore"),
            Self::Incomplete { problems } => {
                write!(
                    f,
                    "some files could not be restored: {}",
                    problems.join("; ")
                )
            }
        }
    }
}

impl std::error::Error for RollbackError {}

#[cfg(test)]
#[path = "plan_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "plan_check_tests.rs"]
mod check_tests;
