//! The install record (task 2.4, `.kiro/specs/reliable-installers/design.md`
//! §3; Requirements 5.1, 5.2, 5.3, 5.5 and 5.7).
//!
//! One JSON file per user (`install.json`, schema 1) lists the installed
//! release, the binary, the `PATH` entry, and per agent the selected features,
//! every file or config entry mmhmem manages, pending user steps and the last
//! functional check. Submodules:
//!
//! - `record_types.rs`: validated value types (hashes, tags, timestamps, key
//!   names, section ids, backup paths).
//! - `record_store.rs`: per-OS location, atomic save, load and its errors.
//! - `record_drift.rs`: entry normalization, drift detection and the
//!   recovery scan for interrupted changes.
//!
//! **Journaling (5.2).** Each change runs in this order: write the backup;
//! [`InstallRecord::begin_change`] + save (entry `applying`); write the
//! target; [`InstallRecord::finish_change`] + save (entry `done`). After an
//! interruption [`recovery_scan`] compares every `applying` entry with the
//! disk so the next run can finish or undo it.
//!
//! **No secrets (5.7).** No field holds free-form text: values are only
//! [`Sha256Hex`] hashes, [`ReleaseTag`]s, [`UtcTimestamp`]s, config key
//! names, section ids, paths, booleans, counts and closed enums. A config
//! entry (which may contain a worker key) is stored only as the hash of its
//! normalized value; worker settings appear only as `worker_configured`.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[path = "record_drift.rs"]
mod drift;
#[path = "record_store.rs"]
mod store;
#[path = "record_types.rs"]
mod types;

pub use drift::{
    Drift, DriftReport, Observation, Probe, Recovery, RecoveryReport, detect_drift, normalize_json,
    normalize_text, observe, observe_in, path_entry_hash, recovery_scan,
};
pub use store::{
    BACKUPS_DIR, LocationError, LocationInput, Os, RECORD_FILE, RecordError, RecordStore,
    UnreadableKind, parse_record,
};
pub use types::{
    BackupPath, ElementMatcher, HookHarness, InvalidValue, KeySegment, ReleaseTag, SectionId,
    Sha256Hex, UtcTimestamp, key_path,
};

/// The only schema version this build reads and writes.
pub const SCHEMA_VERSION: u32 = 1;

/// The whole install record (`install.json`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstallRecord {
    /// Always [`SCHEMA_VERSION`].
    pub schema: u32,
    /// The installed release, once a binary is installed.
    pub release: Option<ReleaseTag>,
    /// The installed binary.
    pub binary: Option<BinaryInfo>,
    /// The `PATH` entry for the binary's folder.
    pub path_entry: Option<PathEntryInfo>,
    /// Whether worker settings are configured (never the settings themselves).
    #[serde(default)]
    pub worker_configured: bool,
    /// Install-wide managed entries (kinds `binary` and `path_entry`).
    #[serde(default)]
    pub managed: Vec<ManagedEntry>,
    /// Per-agent state.
    #[serde(default)]
    pub agents: BTreeMap<AgentId, AgentRecord>,
}

/// The installed binary's path and hash.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BinaryInfo {
    /// Absolute path of the binary.
    pub path: PathBuf,
    /// SHA-256 of the binary file.
    pub sha256: Sha256Hex,
}

/// The `PATH` entry for the binary's folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PathEntryInfo {
    /// True when mmhmem added the entry (so removal takes it out again).
    pub added: bool,
    /// The folder on `PATH`.
    pub value: PathBuf,
}

/// The agents the installer integrates with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentId {
    /// Claude Code.
    ClaudeCode,
    /// Claude Desktop.
    ClaudeDesktop,
    /// Codex.
    Codex,
    /// OpenCode.
    #[serde(rename = "opencode")]
    OpenCode,
    /// Antigravity (detect and show a snippet only).
    Antigravity,
}

/// A feature the user can switch on per agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Feature {
    /// MCP server registration.
    Mcp,
    /// The `unified-memory` skill.
    Skill,
    /// Routing instructions.
    Instructions,
    /// The recap command.
    RecapCommand,
    /// The read-routing hook or plugin.
    ReadHook,
}

/// A step only the user can do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PendingUserStep {
    /// Approve the mmhmem hook in Codex (`/hooks`).
    ApproveHookInCodex,
    /// Restart Claude Desktop.
    RestartClaudeDesktop,
    /// Paste the import snippet into Antigravity.
    ImportAntigravitySnippet,
}

/// The overall result of a functional check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckResult {
    /// Everything passed.
    Pass,
    /// Something failed.
    Fail,
    /// Passed, but user steps are pending.
    UserStepsPending,
}

/// One step of the functional check (design.md §6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckStep {
    /// Read back the registration.
    Registration,
    /// Start the server and run `initialize`.
    Handshake,
    /// `tools/list`.
    ToolsList,
    /// The `status` tool.
    StatusTool,
    /// The agent's own check command.
    AgentCommand,
    /// The hook sample.
    HookSample,
    /// The agent's config load.
    ConfigLoad,
}

/// The outcome of one check step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepOutcome {
    /// The step passed.
    Pass,
    /// The step failed.
    Fail,
    /// The step did not run.
    Skipped,
}

/// One step's outcome in [`LastCheck::details`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckDetail {
    /// Which step.
    pub step: CheckStep,
    /// How it went.
    pub outcome: StepOutcome,
}

/// The last functional check of an agent (Requirement 5.5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LastCheck {
    /// When it ran.
    pub at: UtcTimestamp,
    /// The overall result.
    pub result: CheckResult,
    /// Per-step outcomes.
    #[serde(default)]
    pub details: Vec<CheckDetail>,
}

/// Everything recorded for one agent.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentRecord {
    /// Selected features.
    #[serde(default)]
    pub features: BTreeMap<Feature, bool>,
    /// Files and entries mmhmem manages for this agent.
    #[serde(default)]
    pub managed: Vec<ManagedEntry>,
    /// Steps only the user can do.
    #[serde(default)]
    pub pending_user_steps: Vec<PendingUserStep>,
    /// The last functional check.
    #[serde(default)]
    pub last_check: Option<LastCheck>,
}

/// What a managed entry points at. Equal targets mean the same entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Target {
    /// A whole file mmhmem owns (a skill, command or plugin file).
    File {
        /// Absolute path.
        file: PathBuf,
    },
    /// One entry in a shared JSON/JSONC file, by object keys.
    JsonEntry {
        /// Absolute path.
        file: PathBuf,
        /// Object keys from the root, e.g. `["mcpServers", "mmhmem"]`.
        pointer: Vec<KeySegment>,
        /// When set, only these top-level keys of the entry are hashed, so
        /// keys the user adds inside mmhmem's entry are not drift.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        managed_keys: Option<Vec<KeySegment>>,
    },
    /// One table in a shared TOML file, by key path.
    TomlTable {
        /// Absolute path.
        file: PathBuf,
        /// Keys from the root, e.g. `["mcp_servers", "mmhmem"]`.
        key: Vec<KeySegment>,
        /// As for [`Target::JsonEntry`].
        #[serde(default, skip_serializing_if = "Option::is_none")]
        managed_keys: Option<Vec<KeySegment>>,
    },
    /// mmhmem's element of a shared JSON array (e.g. its hook group in
    /// `hooks.PreToolUse`), found by `matcher`.
    JsonArrayElement {
        /// Absolute path.
        file: PathBuf,
        /// Object keys from the root to the array.
        pointer: Vec<KeySegment>,
        /// How mmhmem's element is recognised.
        matcher: ElementMatcher,
    },
    /// A delimited section in a shared Markdown file.
    MarkdownSection {
        /// Absolute path.
        file: PathBuf,
        /// The section id in its markers.
        section: SectionId,
    },
    /// The binary's folder on the user's `PATH`.
    PathEntry {
        /// The folder.
        value: PathBuf,
    },
    /// The installed binary.
    Binary {
        /// Absolute path.
        path: PathBuf,
    },
}

/// Journal state of a managed entry (Requirement 5.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryState {
    /// Saved before the target write; the write may not have happened.
    Applying,
    /// The target was written and matches `sha256`.
    Done,
}

/// One file or entry mmhmem added or changed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManagedEntry {
    /// What it points at.
    #[serde(flatten)]
    pub target: Target,
    /// Hash of the entry's normalized value after the change. `None` only
    /// while a removal is `applying` (the entry will be gone afterwards).
    pub sha256: Option<Sha256Hex>,
    /// While `applying`: the hash on disk before the change (`None` when the
    /// entry did not exist), so recovery can tell "not written" from drift.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before_sha256: Option<Sha256Hex>,
    /// While `applying`: whether the record managed the entry before.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub was_managed: bool,
    /// Backup path relative to the record folder, or `None`.
    pub backup: Option<BackupPath>,
    /// For JSON/TOML entries: how many parent keys mmhmem created, so removal
    /// prunes only those.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub missing_parents: Option<u32>,
    /// Journal state.
    pub state: EntryState,
}

/// Whose entry list a change belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// Install-wide entries ([`InstallRecord::managed`]).
    Install,
    /// One agent's entries.
    Agent(AgentId),
}

/// A change about to be applied, for [`InstallRecord::begin_change`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingChange {
    /// The entry.
    pub target: Target,
    /// Hash on disk before the change (`None`: absent).
    pub before: Option<Sha256Hex>,
    /// Hash after the change (`None`: the change removes the entry).
    pub after: Option<Sha256Hex>,
    /// The backup taken before the change.
    pub backup: Option<BackupPath>,
    /// Parent keys the change creates (JSON/TOML).
    pub missing_parents: Option<u32>,
}

/// A journaling call that names an entry the record does not hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NotApplying;

impl std::fmt::Display for NotApplying {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the install record has no change in progress for this entry")
    }
}

impl std::error::Error for NotApplying {}

impl Default for InstallRecord {
    fn default() -> Self {
        Self::new()
    }
}

impl InstallRecord {
    /// An empty schema-1 record.
    pub fn new() -> Self {
        Self {
            schema: SCHEMA_VERSION,
            release: None,
            binary: None,
            path_entry: None,
            worker_configured: false,
            managed: Vec::new(),
            agents: BTreeMap::new(),
        }
    }

    /// The entries of `scope` (empty for an unknown agent).
    pub fn entries(&self, scope: Scope) -> &[ManagedEntry] {
        match scope {
            Scope::Install => &self.managed,
            Scope::Agent(id) => self.agents.get(&id).map_or(&[], |a| a.managed.as_slice()),
        }
    }

    fn entries_mut(&mut self, scope: Scope) -> &mut Vec<ManagedEntry> {
        match scope {
            Scope::Install => &mut self.managed,
            Scope::Agent(id) => &mut self.agents.entry(id).or_default().managed,
        }
    }

    /// The entry for `target` in `scope`.
    pub fn find(&self, scope: Scope, target: &Target) -> Option<&ManagedEntry> {
        self.entries(scope).iter().find(|e| &e.target == target)
    }

    /// Journal step 2: record `change` as `applying`, replacing any entry for
    /// the same target. Save the record before writing the target.
    pub fn begin_change(&mut self, scope: Scope, change: PendingChange) {
        let list = self.entries_mut(scope);
        let old = list.iter().position(|e| e.target == change.target);
        let entry = ManagedEntry {
            target: change.target,
            sha256: change.after,
            before_sha256: change.before,
            was_managed: old.is_some(),
            backup: change.backup,
            missing_parents: change.missing_parents,
            state: EntryState::Applying,
        };
        match old {
            Some(i) => list[i] = entry,
            None => list.push(entry),
        }
    }

    /// Journal step 4: the target was written. The entry becomes `done`, or
    /// is dropped when the change removed it. Save the record afterwards.
    pub fn finish_change(&mut self, scope: Scope, target: &Target) -> Result<(), NotApplying> {
        let list = self.entries_mut(scope);
        let i = applying_index(list, target)?;
        if list[i].sha256.is_none() {
            list.remove(i);
        } else {
            let e = &mut list[i];
            e.state = EntryState::Done;
            e.before_sha256 = None;
            e.was_managed = false;
        }
        Ok(())
    }

    /// Undo an `applying` entry whose target was not written: the record goes
    /// back to describing the disk as it was before the change.
    pub fn undo_change(&mut self, scope: Scope, target: &Target) -> Result<(), NotApplying> {
        let list = self.entries_mut(scope);
        let i = applying_index(list, target)?;
        match (list[i].was_managed, list[i].before_sha256.take()) {
            (true, Some(before)) => {
                let e = &mut list[i];
                e.sha256 = Some(before);
                e.state = EntryState::Done;
                e.was_managed = false;
            }
            _ => {
                list.remove(i);
            }
        }
        Ok(())
    }

    /// Every `applying` entry, in record order (install-wide first).
    pub fn applying(&self) -> Vec<(Scope, &ManagedEntry)> {
        self.in_state(EntryState::Applying)
    }

    /// Every `done` entry, in record order (install-wide first).
    pub fn done(&self) -> Vec<(Scope, &ManagedEntry)> {
        self.in_state(EntryState::Done)
    }

    fn in_state(&self, state: EntryState) -> Vec<(Scope, &ManagedEntry)> {
        let install = self.managed.iter().map(|e| (Scope::Install, e));
        let agents = self
            .agents
            .iter()
            .flat_map(|(id, a)| a.managed.iter().map(move |e| (Scope::Agent(*id), e)));
        install
            .chain(agents)
            .filter(|(_, e)| e.state == state)
            .collect()
    }

    /// Check the invariants a loaded record must hold: schema 1 and every
    /// `done` entry has a hash.
    pub fn is_consistent(&self) -> bool {
        self.schema == SCHEMA_VERSION && self.done().iter().all(|(_, e)| e.sha256.is_some())
    }
}

fn applying_index(list: &[ManagedEntry], target: &Target) -> Result<usize, NotApplying> {
    list.iter()
        .position(|e| &e.target == target && e.state == EntryState::Applying)
        .ok_or(NotApplying)
}

#[cfg(test)]
#[path = "record_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "record_drift_tests.rs"]
mod drift_tests;

#[cfg(test)]
#[path = "record_entry_tests.rs"]
mod entry_tests;
