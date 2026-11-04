//! Where the install record lives and how it is loaded and saved (task 2.4,
//! design.md §3).
//!
//! - Linux: `$XDG_STATE_HOME/mmhmem` (default `~/.local/state/mmhmem`);
//!   macOS: `~/Library/Application Support/mmhmem`; Windows:
//!   `%LOCALAPPDATA%\mmhmem`. The file is `install.json`; backups go in
//!   `backups/<UTC timestamp>/` next to it.
//! - The folder is injectable ([`RecordStore::new`], [`LocationInput`]) so
//!   tests never touch the real user folders.
//! - Saving is atomic (`fsx::write_atomic`): an interrupted or failed save
//!   leaves the previous record intact. The record holds no secrets, so it
//!   is not owner-only; backups are (fsx makes them owner-only).
//! - Load errors carry positions and fixed messages, never file contents.

use std::ffi::OsString;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::{BackupPath, UtcTimestamp};
use super::{InstallRecord, NotApplying, PendingChange, SCHEMA_VERSION, Scope, Target};
use crate::setup::fsx;

/// The record file name inside the record folder.
pub const RECORD_FILE: &str = "install.json";
/// The backups folder name inside the record folder.
pub const BACKUPS_DIR: &str = "backups";

/// The operating system whose location rules apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    /// Linux and other Unix systems (XDG rules).
    Linux,
    /// macOS.
    MacOs,
    /// Windows.
    Windows,
}

impl Os {
    /// The OS this binary runs on.
    pub fn current() -> Self {
        if cfg!(windows) {
            Self::Windows
        } else if cfg!(target_os = "macos") {
            Self::MacOs
        } else {
            Self::Linux
        }
    }
}

/// The inputs of the location rules, injectable for tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocationInput {
    /// Which rules apply.
    pub os: Os,
    /// The user's home folder.
    pub home: Option<PathBuf>,
    /// `XDG_STATE_HOME` (used on Linux only).
    pub xdg_state_home: Option<OsString>,
    /// `LOCALAPPDATA` (used on Windows only).
    pub local_app_data: Option<OsString>,
}

/// The record folder could not be determined.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocationError {
    /// No home folder is known and no override applies.
    NoHome,
}

impl fmt::Display for LocationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoHome => f.write_str("could not find your home folder"),
        }
    }
}

impl std::error::Error for LocationError {}

impl LocationInput {
    /// The real inputs of this process (home folder via `etcetera`).
    pub fn from_process() -> Self {
        Self {
            os: Os::current(),
            home: etcetera::home_dir().ok(),
            xdg_state_home: std::env::var_os("XDG_STATE_HOME"),
            local_app_data: std::env::var_os("LOCALAPPDATA"),
        }
    }

    /// The record folder for these inputs. An override variable that is
    /// empty or not an absolute path for the OS is ignored (XDG spec).
    pub fn resolve(&self) -> Result<PathBuf, LocationError> {
        let home = || self.home.clone().ok_or(LocationError::NoHome);
        let base = match self.os {
            Os::Linux => match usable(self.os, self.xdg_state_home.as_ref()) {
                Some(dir) => dir,
                None => home()?.join(".local").join("state"),
            },
            Os::MacOs => home()?.join("Library").join("Application Support"),
            Os::Windows => match usable(self.os, self.local_app_data.as_ref()) {
                Some(dir) => dir,
                None => home()?.join("AppData").join("Local"),
            },
        };
        Ok(base.join("mmhmem"))
    }
}

/// `value` as a folder when it is an absolute path under the rules of `os`.
fn usable(os: Os, value: Option<&OsString>) -> Option<PathBuf> {
    let text = value?.to_str()?;
    let b = text.as_bytes();
    let absolute = match os {
        Os::Windows => {
            text.starts_with("\\\\")
                || (b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':')
                    && matches!(b[2], b'\\' | b'/')
        }
        Os::Linux | Os::MacOs => text.starts_with('/'),
    };
    absolute.then(|| PathBuf::from(text))
}

/// Why a stored record cannot be used. Never contains file contents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnreadableKind {
    /// The file exists but could not be read.
    Io(io::ErrorKind),
    /// The file is not valid JSON.
    NotJson {
        /// 1-based line.
        line: usize,
        /// 1-based column.
        column: usize,
    },
    /// The file has no or another schema version.
    UnknownSchema {
        /// The schema number found, if it is a number.
        found: Option<u64>,
    },
    /// The JSON does not have the record's shape or breaks its invariants.
    InvalidShape,
}

impl fmt::Display for UnreadableKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(kind) => write!(f, "it could not be read ({kind})"),
            Self::NotJson { line, column } => {
                write!(f, "it is not valid JSON (line {line}, column {column})")
            }
            Self::UnknownSchema { found: Some(n) } => {
                write!(
                    f,
                    "it has schema version {n}, this installer reads {SCHEMA_VERSION}"
                )
            }
            Self::UnknownSchema { found: None } => f.write_str("it has no schema version"),
            Self::InvalidShape => f.write_str("it does not have the expected fields"),
        }
    }
}

/// Install record errors. Messages hold paths and fixed text only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordError {
    /// The record exists but cannot be used; the installer rebuilds it from
    /// disk (task 2.6, Requirement 5.6).
    Unreadable {
        /// The record file.
        path: PathBuf,
        /// What is wrong.
        kind: UnreadableKind,
    },
    /// The record could not be saved; the previous record is intact.
    Save {
        /// The record file.
        path: PathBuf,
        /// The underlying failure (OS error text, no contents).
        reason: String,
    },
    /// A backup could not be written.
    Backup {
        /// The file being backed up.
        source: PathBuf,
        /// The underlying failure (OS error text, no contents).
        reason: String,
    },
    /// A journaling call named an entry that is not `applying`.
    NotApplying,
}

impl fmt::Display for RecordError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unreadable { path, kind } => {
                write!(
                    f,
                    "the install record {} is unusable: {kind}",
                    path.display()
                )
            }
            Self::Save { path, reason } => {
                write!(
                    f,
                    "could not save the install record {}: {reason}",
                    path.display()
                )
            }
            Self::Backup { source, reason } => {
                write!(f, "could not back up {}: {reason}", source.display())
            }
            Self::NotApplying => NotApplying.fmt(f),
        }
    }
}

impl std::error::Error for RecordError {}

/// Parse record bytes, checking the schema before the shape.
pub fn parse_record(bytes: &[u8]) -> Result<InstallRecord, UnreadableKind> {
    let value: Value = serde_json::from_slice(bytes).map_err(|e| UnreadableKind::NotJson {
        line: e.line(),
        column: e.column(),
    })?;
    let found = value.get("schema").and_then(Value::as_u64);
    if found != Some(u64::from(SCHEMA_VERSION)) {
        return Err(UnreadableKind::UnknownSchema { found });
    }
    let record: InstallRecord =
        serde_json::from_value(value).map_err(|_| UnreadableKind::InvalidShape)?;
    if record.is_consistent() {
        Ok(record)
    } else {
        Err(UnreadableKind::InvalidShape)
    }
}

/// The record folder: `install.json` plus `backups/`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordStore {
    dir: PathBuf,
}

impl RecordStore {
    /// A store in `dir` (tests pass a temporary folder).
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// The store at the per-OS location for `input`.
    pub fn locate(input: &LocationInput) -> Result<Self, LocationError> {
        input.resolve().map(Self::new)
    }

    /// The record folder.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The record file.
    pub fn record_path(&self) -> PathBuf {
        self.dir.join(RECORD_FILE)
    }

    /// The backup folder for a run that started at `at`.
    pub fn backup_dir(&self, at: &UtcTimestamp) -> PathBuf {
        self.dir.join(BACKUPS_DIR).join(at.folder_name())
    }

    /// Load the record: `Ok(None)` when there is none yet.
    pub fn load(&self) -> Result<Option<InstallRecord>, RecordError> {
        let path = self.record_path();
        let bytes = match fs_err::read(&path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => {
                let kind = UnreadableKind::Io(e.kind());
                return Err(RecordError::Unreadable { path, kind });
            }
        };
        parse_record(&bytes)
            .map(Some)
            .map_err(|kind| RecordError::Unreadable { path, kind })
    }

    /// Save the record atomically.
    pub fn save(&self, record: &InstallRecord) -> Result<(), RecordError> {
        let path = self.record_path();
        let mut bytes = serde_json::to_vec_pretty(record).map_err(|_| RecordError::Save {
            path: path.clone(),
            reason: "a path in the record is not valid text".to_owned(),
        })?;
        bytes.push(b'\n');
        fsx::write_atomic(&path, &bytes).map_err(|e| RecordError::Save {
            path,
            reason: format!("{e:#}"),
        })
    }

    /// Journal step 1: back up `source` into the run's backup folder.
    /// `Ok(None)` when `source` does not exist.
    pub fn backup(
        &self,
        source: &Path,
        at: &UtcTimestamp,
    ) -> Result<Option<BackupPath>, RecordError> {
        let fail = |reason: String| RecordError::Backup {
            source: source.to_path_buf(),
            reason,
        };
        let Some(abs) =
            fsx::backup(source, &self.backup_dir(at)).map_err(|e| fail(format!("{e:#}")))?
        else {
            return Ok(None);
        };
        BackupPath::from_absolute(&self.dir, &abs)
            .map(Some)
            .map_err(|e| fail(e.to_string()))
    }

    /// Journal step 2: mark the change `applying` and save the record.
    pub fn journal_begin(
        &self,
        record: &mut InstallRecord,
        scope: Scope,
        change: PendingChange,
    ) -> Result<(), RecordError> {
        record.begin_change(scope, change);
        self.save(record)
    }

    /// Journal step 4: mark the change `done` and save the record.
    pub fn journal_finish(
        &self,
        record: &mut InstallRecord,
        scope: Scope,
        target: &Target,
    ) -> Result<(), RecordError> {
        record
            .finish_change(scope, target)
            .map_err(|_| RecordError::NotApplying)?;
        self.save(record)
    }
}
