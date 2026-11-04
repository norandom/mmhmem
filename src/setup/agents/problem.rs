//! Agent planning problems and the per-file change chain
//! (reliable-installers task 3.1, `.kiro/specs/reliable-installers/design.md`
//! §4 and §5; Requirements 3.1 and 3.4).
//!
//! An [`AgentProblem`] skips one agent and names files and fixed reasons,
//! never file contents. [`FileChain`] gives planners the "chained per file"
//! rule of `plan.rs`: every change's `before` is the previous change's
//! `after` for the same file, or the bytes on disk for the first one.

use std::collections::BTreeMap;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use crate::setup::edit::json::JsonEditError;
use crate::setup::edit::markdown::{AMBIGUOUS_MARKERS, AmbiguousMarkers};
use crate::setup::edit::toml::TomlEditError;
use crate::setup::plan::Change;

/// Why one agent cannot be planned. The other agents go on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentProblem {
    /// A file could not be read.
    Unreadable {
        /// The file.
        file: PathBuf,
        /// The OS error kind (`InvalidData`: not UTF-8 text).
        kind: io::ErrorKind,
    },
    /// A JSON/JSONC file does not parse or cannot take the edit.
    Json {
        /// The file.
        file: PathBuf,
        /// What is wrong (never contains file text).
        error: JsonEditError,
    },
    /// A TOML file does not parse or cannot take the edit.
    Toml {
        /// The file.
        file: PathBuf,
        /// What is wrong (never contains file text).
        error: TomlEditError,
    },
    /// mmhmem's Markdown section cannot be found unambiguously.
    Markdown {
        /// The file.
        file: PathBuf,
        /// Fixed reason, e.g. [`AMBIGUOUS_MARKERS`] (never file text).
        reason: &'static str,
    },
    /// mmhmem's entry is in more than one file the agent loads (OpenCode's
    /// `opencode.json` and `opencode.jsonc`); the user keeps one.
    DuplicateEntry {
        /// The files holding an entry.
        files: Vec<PathBuf>,
    },
    /// More than one profile was found (Claude Desktop); the user picks one.
    SeveralProfiles {
        /// The profile folders.
        profiles: Vec<PathBuf>,
    },
}

/// `a, b, c` for a list of paths.
fn list(paths: &[PathBuf]) -> String {
    paths
        .iter()
        .map(|p| p.display().to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

impl fmt::Display for AgentProblem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unreadable { file, kind } if *kind == io::ErrorKind::InvalidData => {
                write!(f, "{} is not UTF-8 text", file.display())
            }
            Self::Unreadable { file, kind } => {
                write!(f, "{} could not be read ({kind})", file.display())
            }
            Self::Json { file, error } => write!(f, "{}: {error}", file.display()),
            Self::Toml { file, error } => write!(f, "{}: {error}", file.display()),
            Self::Markdown { file, reason } => write!(f, "{}: {reason}", file.display()),
            Self::DuplicateEntry { files } => write!(
                f,
                "mmhmem is set up in more than one file: {}; keep only one",
                list(files)
            ),
            Self::SeveralProfiles { profiles } => write!(
                f,
                "more than one profile was found: {}; choose one",
                list(profiles)
            ),
        }
    }
}

impl std::error::Error for AgentProblem {}

impl AgentProblem {
    /// The problem for ambiguous section markers in `file`.
    pub fn markdown(file: &Path) -> impl Fn(AmbiguousMarkers) -> Self + use<> {
        let file = file.to_path_buf();
        move |_| Self::Markdown {
            file: file.clone(),
            reason: AMBIGUOUS_MARKERS,
        }
    }
}

/// Reads a whole file; `Ok(None)` when it does not exist.
pub fn read_optional(file: &Path) -> Result<Option<Vec<u8>>, AgentProblem> {
    match std::fs::read(file) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(AgentProblem::Unreadable {
            file: file.to_path_buf(),
            kind: e.kind(),
        }),
    }
}

/// One agent's planned changes, chained per file (see `plan.rs`).
///
/// Ask [`FileChain::bytes`] or [`FileChain::text`] for a file's current
/// content (the last planned `after`, else the disk), build the change from
/// it, then [`FileChain::push`] it.
#[derive(Debug, Default)]
pub struct FileChain {
    current: BTreeMap<PathBuf, Option<Vec<u8>>>,
    changes: Vec<Change>,
}

impl FileChain {
    /// An empty chain.
    pub fn new() -> Self {
        Self::default()
    }

    /// The file's bytes as the next change will see them (`None`: absent).
    pub fn bytes(&mut self, file: &Path) -> Result<Option<Vec<u8>>, AgentProblem> {
        if let Some(bytes) = self.current.get(file) {
            return Ok(bytes.clone());
        }
        let bytes = read_optional(file)?;
        self.current.insert(file.to_path_buf(), bytes.clone());
        Ok(bytes)
    }

    /// As [`FileChain::bytes`], decoded as UTF-8 text.
    pub fn text(&mut self, file: &Path) -> Result<Option<String>, AgentProblem> {
        self.bytes(file)?
            .map(|bytes| {
                String::from_utf8(bytes).map_err(|_| AgentProblem::Unreadable {
                    file: file.to_path_buf(),
                    kind: io::ErrorKind::InvalidData,
                })
            })
            .transpose()
    }

    /// Adds a change; its `after` becomes the file's current content. The
    /// change's `before` must be what [`FileChain::bytes`] returned.
    pub fn push(&mut self, change: Change) {
        debug_assert!(
            self.current
                .get(&change.file)
                .is_none_or(|now| *now == change.before),
            "a change must follow on from the previous change of its file"
        );
        self.current
            .insert(change.file.clone(), change.after.clone());
        self.changes.push(change);
    }

    /// The changes so far, in order.
    pub fn changes(&self) -> &[Change] {
        &self.changes
    }

    /// The changes, in order.
    pub fn into_changes(self) -> Vec<Change> {
        self.changes
    }
}
