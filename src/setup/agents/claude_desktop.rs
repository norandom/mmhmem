//! Claude Desktop integration (reliable-installers task 3.3,
//! `.kiro/specs/reliable-installers/design.md` §5; Requirements 3, 6.6 and
//! 7.3).
//!
//! Claude Desktop has no command line, so mmhmem edits its files:
//!
//! - **MCP (required):** the leaf keys `command` (the installed binary,
//!   given by the caller) and `args` (`["stdio"]`) of `mcpServers.mmhmem` in
//!   `claude_desktop_config.json`. Other servers and keys the user added
//!   inside mmhmem's entry are kept; a missing file is created as plain JSON.
//! - **Skill:** `unified-memory/SKILL.md` in the profile's skills folder.
//! - **Pending step:** [`PENDING_STEPS`] (Desktop must be restarted); the
//!   caller adds it to the agent's record.
//!
//! **Profiles.** Windows has `%APPDATA%\Claude` and every MSIX
//! `%LOCALAPPDATA%\Packages\Claude_*\LocalCache\Roaming\Claude`; macOS has
//! `~/Library/Application Support/Claude`, Linux `~/.config/Claude`. With
//! several profiles [`Agent::plan`] gives [`AgentProblem::SeveralProfiles`]
//! until the user picks one ([`ClaudeDesktop::with_profile`]); a profile
//! already in the install record is kept for updates.
//!
//! Removal takes out only `command` and `args` and then the parent objects
//! this install created, so a config that had other servers returns to its
//! earlier bytes and one mmhmem created is left as `{}`. If the user added
//! keys inside `mcpServers.mmhmem`, they stay byte-identical in the object;
//! without mmhmem's keys the entry counts as removed and leaves the record.

use std::path::{Path, PathBuf};

use serde_json::json;

use super::{
    Agent, AgentId, AgentProblem, AgentRecord, CheckReport, Detection, Env, Feature, FileChain,
    PendingUserStep, ScanEnv, Selection,
};
use crate::setup::assets::UNIFIED_MEMORY_SKILL;
use crate::setup::check::agent::{Steps, read_registration};
use crate::setup::check::{Registration, RegistrationError};
use crate::setup::edit::json as edit;
use crate::setup::plan::{Action, Change};
use crate::setup::record::{KeySegment, ManagedEntry, Os, Scope, Target, key_path};
use crate::setup::ui::text::changes::{self as say, What};

/// The config file name inside a profile folder.
pub const CONFIG_FILE: &str = "claude_desktop_config.json";
/// The steps only the user can do after an install.
pub const PENDING_STEPS: [PendingUserStep; 1] = [PendingUserStep::RestartClaudeDesktop];

const SCOPE: Scope = Scope::Agent(AgentId::ClaudeDesktop);
const FEATURES: &[Feature] = &[Feature::Mcp, Feature::Skill];
const ENTRY: [&str; 2] = ["mcpServers", "mmhmem"];
const COMMAND: [&str; 3] = ["mcpServers", "mmhmem", "command"];
const ARGS: [&str; 3] = ["mcpServers", "mmhmem", "args"];

/// One Claude Desktop profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profile {
    /// The folder holding `claude_desktop_config.json`.
    pub config_dir: PathBuf,
    /// The folder the profile loads skills from.
    pub skill_dir: PathBuf,
}

impl Profile {
    fn in_config_dir(dir: PathBuf) -> Self {
        Self {
            skill_dir: dir.join("skills"),
            config_dir: dir,
        }
    }

    fn config(&self) -> PathBuf {
        self.config_dir.join(CONFIG_FILE)
    }

    fn skill(&self) -> PathBuf {
        self.skill_dir.join("unified-memory").join("SKILL.md")
    }
}

/// The Claude Desktop profile folders that exist. The one discovery rule,
/// shared with the adoption scan (`adopt_table.rs`), as the old installers
/// used it (`install_local.ps1:290-303`, `install_local.sh:438-441,786-790`).
pub fn profiles(dirs: &ScanEnv) -> Vec<Profile> {
    let join =
        |base: &Path, parts: &[&str]| parts.iter().fold(base.to_path_buf(), |p, s| p.join(s));
    if dirs.os == Os::Windows {
        let mut all = msix_profiles(dirs.local_app_data.as_deref());
        all.extend(dirs.appdata.iter().map(|a| a.join("Claude")));
        return all
            .into_iter()
            .filter(|d| d.is_dir())
            .map(Profile::in_config_dir)
            .collect();
    }
    let home = &dirs.home;
    let mut found = Vec::new();
    let mac = join(home, &["Library", "Application Support", "Claude"]);
    if mac.is_dir() {
        found.push(Profile {
            config_dir: mac,
            skill_dir: join(home, &["Library", "Claude", "skills"]),
        });
    }
    let linux = join(home, &[".config", "Claude"]);
    if linux.is_dir() {
        found.push(Profile::in_config_dir(linux));
    }
    found
}

fn msix_profiles(local: Option<&Path>) -> Vec<PathBuf> {
    let Some(entries) = local.and_then(|l| std::fs::read_dir(l.join("Packages")).ok()) else {
        return Vec::new();
    };
    let mut dirs: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .filter(|e| e.file_name().to_string_lossy().starts_with("Claude_"))
        .map(|e| e.path().join("LocalCache").join("Roaming").join("Claude"))
        .collect();
    dirs.sort();
    dirs
}

/// Claude Desktop (design.md §5).
#[derive(Debug, Clone)]
pub struct ClaudeDesktop {
    binary: PathBuf,
    profile: Option<PathBuf>,
}

impl ClaudeDesktop {
    /// Desktop launching the installed `binary`, with no profile chosen.
    pub fn new(binary: impl Into<PathBuf>) -> Self {
        Self {
            binary: binary.into(),
            profile: None,
        }
    }

    /// The same, with the profile (its config folder) the user picked.
    pub fn with_profile(mut self, config_dir: impl Into<PathBuf>) -> Self {
        self.profile = Some(config_dir.into());
        self
    }

    /// The profile to write to: the chosen one, else the one in the record,
    /// else the only one found.
    fn resolve(&self, env: &Env, rec: Option<&AgentRecord>) -> Result<Profile, AgentProblem> {
        let found = profiles(&env.dirs);
        let chosen = self
            .profile
            .as_ref()
            .filter(|c| found.iter().any(|p| &p.config_dir == *c));
        if let Some(dir) = chosen
            .cloned()
            .or_else(|| rec.and_then(recorded_config_dir))
        {
            let known = found.iter().find(|p| p.config_dir == dir);
            return Ok(known
                .cloned()
                .unwrap_or_else(|| Profile::in_config_dir(dir)));
        }
        match found.as_slice() {
            [only] => Ok(only.clone()),
            _ => Err(AgentProblem::SeveralProfiles {
                profiles: found.into_iter().map(|p| p.config_dir).collect(),
            }),
        }
    }

    fn mcp_change(
        &self,
        chain: &mut FileChain,
        profile: &Profile,
        rec: Option<&AgentRecord>,
    ) -> Result<Option<Change>, AgentProblem> {
        let file = profile.config();
        let target = mcp_target(&file);
        let recorded = rec.and_then(|r| find(r, &target));
        let before = chain.bytes(&file)?;
        let text = chain.text(&file)?;
        let input = text.as_deref().unwrap_or("{}\n");
        let bad = |error| AgentProblem::Json {
            file: file.clone(),
            error,
        };
        let command = json!(self.binary.to_string_lossy());
        let args = json!(["stdio"]);
        let same = edit::get(input, &COMMAND).map_err(bad)?.as_ref() == Some(&command)
            && edit::get(input, &ARGS).map_err(bad)?.as_ref() == Some(&args);
        if same && recorded.is_some() {
            return Ok(None);
        }
        let existed = edit::get(input, &ENTRY).map_err(bad)?.is_some();
        let parents = edit::missing_parents(input, &COMMAND).map_err(bad)?;
        let after = edit::set(input, &COMMAND, &command).map_err(bad)?;
        let after = edit::set(&after, &ARGS, &args).map_err(bad)?;
        let missing = recorded
            .and_then(|e| e.missing_parents)
            .unwrap_or(u32::try_from(parents).unwrap_or(0));
        Ok(Some(push(
            chain,
            Change {
                scope: SCOPE,
                target,
                file: file.clone(),
                action: if existed { Action::Update } else { Action::Add },
                before,
                after: Some(after.into_bytes()),
                owner_only: false,
                hide_diff: false,
                missing_parents: Some(missing),
                summary: say::add(What::Server, AgentId::ClaudeDesktop.label(), &file),
            },
        )))
    }

    fn skill_change(
        &self,
        chain: &mut FileChain,
        profile: &Profile,
        rec: Option<&AgentRecord>,
    ) -> Result<Option<Change>, AgentProblem> {
        let file = profile.skill();
        let target = Target::File { file: file.clone() };
        let before = chain.bytes(&file)?;
        let wanted = UNIFIED_MEMORY_SKILL.bytes();
        if before.as_deref() == Some(wanted) && rec.and_then(|r| find(r, &target)).is_some() {
            return Ok(None);
        }
        Ok(Some(push(
            chain,
            Change {
                scope: SCOPE,
                target,
                file: file.clone(),
                action: if before.is_some() {
                    Action::Update
                } else {
                    Action::Add
                },
                before,
                after: Some(wanted.to_vec()),
                owner_only: false,
                hide_diff: false,
                missing_parents: None,
                summary: say::add(What::Guide, AgentId::ClaudeDesktop.label(), &file),
            },
        )))
    }
}

fn push(chain: &mut FileChain, change: Change) -> Change {
    chain.push(change.clone());
    change
}

fn mcp_target(file: &Path) -> Target {
    Target::JsonEntry {
        file: file.to_path_buf(),
        pointer: key_path(&ENTRY).expect("static keys"),
        managed_keys: Some(key_path(&["command", "args"]).expect("static keys")),
    }
}

fn find<'a>(rec: &'a AgentRecord, target: &Target) -> Option<&'a ManagedEntry> {
    rec.managed.iter().find(|e| &e.target == target)
}

fn is_entry(pointer: &[KeySegment]) -> bool {
    pointer.iter().map(KeySegment::as_str).eq(ENTRY)
}

/// The config folder of the recorded MCP entry.
fn recorded_config_dir(rec: &AgentRecord) -> Option<PathBuf> {
    rec.managed.iter().find_map(|e| match &e.target {
        Target::JsonEntry { file, pointer, .. } if is_entry(pointer) => {
            file.parent().map(Path::to_path_buf)
        }
        _ => None,
    })
}

/// The change that takes one recorded entry out (`None`: nothing to do).
fn remove_entry(
    chain: &mut FileChain,
    entry: &ManagedEntry,
) -> Result<Option<Change>, AgentProblem> {
    let file = match &entry.target {
        Target::File { file } => file,
        Target::JsonEntry { file, pointer, .. } if is_entry(pointer) => file,
        _ => return Ok(None),
    };
    let bad = |error| AgentProblem::Json {
        file: file.clone(),
        error,
    };
    let before = chain.bytes(file)?;
    let after = match (&entry.target, chain.text(file)?) {
        (_, None) => return Ok(None),
        (Target::File { .. }, Some(_)) => None,
        (_, Some(text)) => {
            let parents = entry.missing_parents.unwrap_or(0) as usize;
            let text = edit::remove(&text, &COMMAND).map_err(bad)?;
            let text = edit::remove_and_prune(&text, &ARGS, parents).map_err(bad)?;
            Some(text.into_bytes())
        }
    };
    Ok(Some(push(
        chain,
        Change {
            scope: SCOPE,
            target: entry.target.clone(),
            file: file.clone(),
            action: Action::Remove,
            before,
            after,
            owner_only: false,
            hide_diff: false,
            missing_parents: None,
            summary: say::remove(AgentId::ClaudeDesktop.label(), file),
        },
    )))
}

impl Agent for ClaudeDesktop {
    fn id(&self) -> AgentId {
        AgentId::ClaudeDesktop
    }

    fn detect(&self, env: &Env) -> Detection {
        let found = profiles(&env.dirs);
        if found.is_empty() {
            return Detection::NotFound {
                reason: "no Claude Desktop folder was found".to_owned(),
            };
        }
        Detection::Found {
            version: None,
            paths: found.into_iter().map(|p| p.config_dir).collect(),
        }
    }

    fn features(&self) -> &'static [Feature] {
        FEATURES
    }

    fn plan(
        &self,
        env: &Env,
        want: &Selection,
        rec: Option<&AgentRecord>,
    ) -> Result<Vec<Change>, AgentProblem> {
        let mut chain = FileChain::new();
        for entry in rec.map(|r| r.managed.as_slice()).unwrap_or_default() {
            let feature = match entry.target {
                Target::File { .. } => Feature::Skill,
                _ => Feature::Mcp,
            };
            if !want.wants(AgentId::ClaudeDesktop, feature) {
                remove_entry(&mut chain, entry)?;
            }
        }
        let mcp = want.wants(AgentId::ClaudeDesktop, Feature::Mcp);
        let skill = want.wants(AgentId::ClaudeDesktop, Feature::Skill);
        if mcp || skill {
            let profile = self.resolve(env, rec)?;
            if mcp {
                self.mcp_change(&mut chain, &profile, rec)?;
            }
            if skill {
                self.skill_change(&mut chain, &profile, rec)?;
            }
        }
        Ok(chain.into_changes())
    }

    /// The functional check only (Desktop has no CLI): the registration is
    /// read from the recorded profile's `claude_desktop_config.json` and
    /// started; the restart stays a pending user step.
    fn check(&self, env: &Env, rec: &AgentRecord) -> CheckReport {
        let registration = self
            .resolve(env, Some(rec))
            .map_err(|_| RegistrationError::Missing)
            .and_then(|profile| {
                read_registration(&profile.config(), |bytes| {
                    Registration::from_claude_json(bytes, "mmhmem")
                })
            });
        Steps::start(env, AgentId::ClaudeDesktop, registration).finish(rec)
    }

    fn plan_remove(&self, _env: &Env, rec: &AgentRecord) -> Vec<Change> {
        let mut chain = FileChain::new();
        for entry in &rec.managed {
            let _ = remove_entry(&mut chain, entry);
        }
        chain.into_changes()
    }

    /// Desktop must be restarted after any install ([`PENDING_STEPS`]).
    fn pending_steps(&self, want: &Selection) -> Vec<PendingUserStep> {
        if want.features(AgentId::ClaudeDesktop).is_empty() {
            Vec::new()
        } else {
            PENDING_STEPS.to_vec()
        }
    }
}

#[cfg(test)]
#[path = "claude_desktop_tests.rs"]
mod tests;
