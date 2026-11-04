//! Record rebuild and adoption of old installs (task 2.6,
//! `.kiro/specs/reliable-installers/design.md` §3 "Rebuild"; Requirements
//! 5.4 and 5.6).
//!
//! When the install record is missing or unreadable, [`scan`] looks in every
//! place the old PowerShell and Bash installers wrote to (the static table in
//! `adopt_table.rs`, taken from `deploy/*`) and reports what it found per
//! agent. [`adoption_plan`] turns that into a [`Plan`] the caller shows on
//! the review screen: each artefact becomes an `Update` change whose bytes
//! are unchanged (applying it only journals, backs up and records it), except
//! the old Codex `# BEGIN mmhmem` block, which becomes a plain
//! `[mcp_servers.mmhmem]` table by removing exactly the two marker lines.
//!
//! **Nothing is applied here.** Scanning and planning only read; the caller
//! applies the plan with [`crate::setup::plan::apply`] after the user
//! confirms (Requirement 5.6). Submodules:
//!
//! - `adopt_table.rs`: paths and targets the old installers used.
//! - `adopt_scan.rs`: reads and checks the files.
//! - `adopt_plan.rs`: the plan and its plain-language summaries.
//!
//! **Whole agents, not single files.** If any file of an agent is not valid
//! JSON/TOML (or its mmhmem markers do not match), that agent is listed as
//! "cannot adopt: <file> is not valid <format>" and gets no change; other
//! agents are adopted. Problems name files and fixed reasons, never contents.

use std::collections::BTreeSet;
use std::fmt;
use std::path::PathBuf;

use crate::setup::record::{AgentId, Feature, Os, Target};

#[path = "adopt_plan.rs"]
mod plan_build;
#[path = "adopt_scan.rs"]
mod scan_files;
#[path = "adopt_table.rs"]
mod table;

pub use plan_build::{adoption_plan, agent_label};

/// The outside inputs of a scan, injectable so tests never read the real
/// user folders.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanEnv {
    /// Which OS rules apply.
    pub os: Os,
    /// The user's home folder.
    pub home: PathBuf,
    /// `%APPDATA%` (Windows).
    pub appdata: Option<PathBuf>,
    /// `%LOCALAPPDATA%` (Windows): MSIX profiles and the binary folder.
    pub local_app_data: Option<PathBuf>,
    /// `XDG_CONFIG_HOME`: where the old hook helper put the OpenCode plugin.
    pub xdg_config_home: Option<PathBuf>,
    /// `CLAUDE_CONFIG_DIR`: where the old hook helper found `settings.json`.
    pub claude_config_dir: Option<PathBuf>,
    /// `CODEX_HOME`: where the old hook helper found `hooks.json`.
    pub codex_home: Option<PathBuf>,
    /// The folders on the user's `PATH` (`None`: not inspected).
    pub user_path: Option<Vec<PathBuf>>,
}

impl ScanEnv {
    /// An environment with only a home folder (all overrides unset).
    pub fn new(os: Os, home: impl Into<PathBuf>) -> Self {
        Self {
            os,
            home: home.into(),
            appdata: None,
            local_app_data: None,
            xdg_config_home: None,
            claude_config_dir: None,
            codex_home: None,
            user_path: None,
        }
    }

    /// The real environment of this process. `user_path` is the process
    /// `PATH`, an approximation of the user's.
    pub fn from_process() -> Option<Self> {
        let var = |name: &str| {
            std::env::var_os(name)
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
        };
        let home = etcetera::home_dir().ok()?;
        Some(Self {
            os: Os::current(),
            home,
            appdata: var("APPDATA"),
            local_app_data: var("LOCALAPPDATA"),
            xdg_config_home: var("XDG_CONFIG_HOME"),
            claude_config_dir: var("CLAUDE_CONFIG_DIR"),
            codex_home: var("CODEX_HOME"),
            user_path: std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()),
        })
    }
}

/// What kind of thing an artefact is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtefactKind {
    /// mmhmem's MCP server entry in an agent's settings.
    McpEntry,
    /// A `SKILL.md` file.
    Skill,
    /// The `mmhmem-recap` command or prompt file.
    RecapCommand,
    /// The delimited routing section in an instructions file.
    RoutingSection,
    /// mmhmem's hook group in a hooks file.
    ReadHook,
    /// The OpenCode read-routing plugin file.
    ReadPlugin,
}

impl ArtefactKind {
    /// The user-selectable feature this kind belongs to.
    pub fn feature(self) -> Feature {
        match self {
            Self::McpEntry => Feature::Mcp,
            Self::Skill => Feature::Skill,
            Self::RecapCommand => Feature::RecapCommand,
            Self::RoutingSection => Feature::Instructions,
            Self::ReadHook | Self::ReadPlugin => Feature::ReadHook,
        }
    }
}

/// Whether the artefact is recognisably from the old installers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// It carries the old installers' signature (the Codex marker block, the
    /// routing section's prefix line, the plugin header).
    OldInstaller,
    /// An mmhmem entry or file that does not say who made it.
    Unknown,
}

/// One old-install artefact found on disk.
#[derive(Clone, PartialEq, Eq)]
pub struct Artefact {
    /// What it is.
    pub kind: ArtefactKind,
    /// Where it lives, as the record will name it.
    pub target: Target,
    /// The file it lives in.
    pub file: PathBuf,
    /// Who made it, as far as the file says.
    pub origin: Origin,
    /// The whole file as read by the scan.
    pub before: Vec<u8>,
    /// The file as adopted: equal to `before` except for the Codex marker
    /// block, whose two marker lines are removed.
    pub after: Vec<u8>,
}

impl Artefact {
    /// Whether adopting changes the file (the old Codex marker block).
    pub fn converts_marker_block(&self) -> bool {
        self.before != self.after
    }
}

impl fmt::Debug for Artefact {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Artefact")
            .field("kind", &self.kind)
            .field("target", &self.target)
            .field("origin", &self.origin)
            .field("before_len", &self.before.len())
            .field("after_len", &self.after.len())
            .finish()
    }
}

/// The file format named in a [`Problem`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// JSON or JSONC.
    Json,
    /// TOML.
    Toml,
}

/// Why a file stops its agent from being adopted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProblemKind {
    /// The file does not parse.
    NotValid(Format),
    /// The file exists but could not be read (OS error kind).
    Unreadable(String),
    /// Anything else, as fixed text without contents.
    Unexpected(String),
}

/// A reason an agent cannot be adopted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    /// The file concerned.
    pub file: PathBuf,
    /// What is wrong with it.
    pub kind: ProblemKind,
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let file = self.file.display();
        match &self.kind {
            ProblemKind::NotValid(Format::Json) => {
                write!(f, "cannot adopt: {file} is not valid JSON")
            }
            ProblemKind::NotValid(Format::Toml) => {
                write!(f, "cannot adopt: {file} is not valid TOML")
            }
            ProblemKind::Unreadable(why) => {
                write!(f, "cannot adopt: {file} could not be read ({why})")
            }
            ProblemKind::Unexpected(why) => write!(f, "cannot adopt: {file} {why}"),
        }
    }
}

impl std::error::Error for Problem {}

/// What was found for one agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentScan {
    /// The agent.
    pub agent: AgentId,
    /// Artefacts found (also listed when the agent cannot be adopted).
    pub artefacts: Vec<Artefact>,
    /// Why the agent cannot be adopted; empty when it can.
    pub problems: Vec<Problem>,
    /// Same-named files that do not look like mmhmem's; left alone.
    pub foreign: Vec<PathBuf>,
}

impl AgentScan {
    /// Whether the agent's artefacts can be adopted.
    pub fn can_adopt(&self) -> bool {
        self.problems.is_empty()
    }

    /// The features the found artefacts correspond to.
    pub fn features(&self) -> BTreeSet<Feature> {
        self.artefacts.iter().map(|a| a.kind.feature()).collect()
    }

    /// One line per problem, led by the agent's name.
    pub fn problem_lines(&self) -> Vec<String> {
        let label = agent_label(self.agent);
        self.problems
            .iter()
            .map(|p| format!("{label}: {p}"))
            .collect()
    }
}

/// Everything the old installers left, found by [`scan`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scan {
    /// Agents with artefacts or problems, in a fixed order.
    pub agents: Vec<AgentScan>,
    /// The installed binary, when it exists at the old installers' location.
    /// Not part of the adoption plan: the install and update steps record
    /// the binary (with its hash) themselves.
    pub binary: Option<PathBuf>,
    /// The binary's folder when it is on the user's `PATH`. Whether the old
    /// installer added it cannot be told, so adopt it as not added by mmhmem.
    pub path_entry: Option<PathBuf>,
    /// Helper files the old installers left that nothing needs now.
    pub leftovers: Vec<PathBuf>,
}

impl Scan {
    /// True when the old installers left nothing at all.
    pub fn is_empty(&self) -> bool {
        self.agents.is_empty()
            && self.binary.is_none()
            && self.path_entry.is_none()
            && self.leftovers.is_empty()
    }

    /// The scan of `agent`, when anything was found for it.
    pub fn agent(&self, agent: AgentId) -> Option<&AgentScan> {
        self.agents.iter().find(|a| a.agent == agent)
    }
}

/// Find every artefact the old installers left, under `env`. Only reads;
/// invalid files become [`Problem`]s, never errors.
pub fn scan(env: &ScanEnv) -> Scan {
    let binary = Some(table::binary(env)).filter(|p| p.is_file());
    let path_entry = binary
        .as_ref()
        .and_then(|b| b.parent())
        .filter(|dir| on_path(env, dir))
        .map(PathBuf::from);
    Scan {
        agents: scan_files::scan_agents(table::specs(env)),
        binary,
        path_entry,
        leftovers: table::leftovers(env),
    }
}

fn on_path(env: &ScanEnv, dir: &std::path::Path) -> bool {
    let norm = |p: &std::path::Path| -> Vec<String> {
        p.components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .map(|c| {
                if env.os == Os::Windows {
                    c.to_lowercase()
                } else {
                    c
                }
            })
            .collect()
    };
    env.user_path
        .as_ref()
        .is_some_and(|path| path.iter().any(|p| norm(p) == norm(dir)))
}

#[cfg(test)]
#[path = "adopt_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "adopt_fixtures.rs"]
mod fixtures;
