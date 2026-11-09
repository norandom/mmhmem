//! Antigravity (antigravity-native-setup, design.md "Antigravity agent";
//! reliable-installers task 3.5).
//!
//! - **Detection** (task 3.1): the `agy` CLI on PATH or Antigravity's own
//!   data folders under the Gemini home; the version is the first line of
//!   `agy --version`.
//! - **MCP** (task 3.2; Requirements 1.1–1.3 and 5.3): the leaf keys
//!   `command` (the installed binary) and `args` (`["stdio"]`) of
//!   `mcpServers.mmhmem` in the global server file
//!   `~/.gemini/config/mcp_config.json`, which the CLI and the IDE both read.
//!   A missing file is created with only that entry. Other servers, comments,
//!   formatting and keys the user added inside mmhmem's entry are kept.
//!   Removal takes out only `command` and `args` and then the parent objects
//!   this install created, exactly as for Claude Desktop; an entry left with
//!   only user keys counts as removed.
//! - **Read hook** (task 3.3; Requirements 2.1, 2.3 and 2.5): the whole
//!   named hook [`HOOK_NAME`] in the global hook file
//!   `~/.gemini/config/hooks.json` (`antigravity_hook.rs`). When the binary's
//!   path cannot be an Antigravity command, the hook is left out, the server
//!   entry is still planned and [`Agent::feature_problems`] says why.
//!   Removal takes out that one key.
//! - **Per feature:** each recorded entry belongs to one feature; a feature
//!   that is no longer selected loses only its own entries.
//! - **Snippet era:** older installs saved an import snippet next to the
//!   binary ([`SNIPPET_FILE`]) and recorded
//!   [`PendingUserStep::ImportAntigravitySnippet`]. Nothing writes the
//!   snippet any more; a recorded snippet file is still removed. The check
//!   and the pending step use the snippet until tasks 3.4, 3.5 and 4.1
//!   replace them.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::{
    Agent, AgentId, AgentProblem, AgentRecord, CheckReport, Detection, Env, Feature, FileChain,
    Invocation, PendingUserStep, Selection,
};
use crate::setup::check::Registration;
use crate::setup::check::agent::Steps;
use crate::setup::edit::json as edit;
use crate::setup::plan::{Action, Change};
use crate::setup::record::{KeySegment, ManagedEntry, Scope, Target, key_path};
use crate::setup::ui::text::changes::{self as say, What};

#[path = "antigravity_hook.rs"]
mod hook;
pub use hook::{HOOK_MATCHER, HOOK_NAME, HOOK_TIMEOUT, hook_command};

/// What the user does to take mmhmem out again.
pub const REMOVE_INSTRUCTION: &str =
    "Remove the mmhmem server in Antigravity's settings (MCP servers).";

/// The import snippet older installs saved next to the binary (removal only).
pub const SNIPPET_FILE: &str = "antigravity-mcp.json";

const ID: AgentId = AgentId::Antigravity;
const SCOPE: Scope = Scope::Agent(ID);

/// The features in menu order.
const FEATURES: [Feature; 2] = [Feature::Mcp, Feature::ReadHook];

/// The Antigravity CLI program (`agy --version` prints `1.5.3`).
const PROGRAM: &str = "agy";

/// Antigravity's CLI, app and IDE data folders under the Gemini home.
const DATA_FOLDERS: [&str; 3] = ["antigravity-cli", "antigravity", "antigravity-ide"];

const ENTRY: [&str; 2] = ["mcpServers", "mmhmem"];
const COMMAND: [&str; 3] = ["mcpServers", "mmhmem", "command"];
const ARGS: [&str; 3] = ["mcpServers", "mmhmem", "args"];

/// The Antigravity integration.
#[derive(Debug, Clone)]
pub struct Antigravity {
    /// The installed mmhmem binary Antigravity starts.
    pub binary: PathBuf,
}

impl Antigravity {
    /// The Antigravity integration for the installed `binary`.
    pub fn new(binary: impl Into<PathBuf>) -> Self {
        Self {
            binary: binary.into(),
        }
    }

    /// The JSON older installs had the user import in Antigravity.
    pub fn snippet_value(&self) -> Value {
        json!({"mcpServers": {"mmhmem": {
            "command": self.binary.to_string_lossy(),
            "args": ["stdio"],
        }}})
    }

    /// [`Antigravity::snippet_value`] as pretty-printed text to show.
    pub fn snippet(&self) -> String {
        serde_json::to_string_pretty(&self.snippet_value()).expect("a JSON value serializes")
    }

    /// The file older installs saved the snippet to, next to the binary.
    pub fn snippet_file(&self) -> PathBuf {
        self.binary
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(SNIPPET_FILE)
    }

    /// The step only the user can do after install.
    pub fn pending_step() -> PendingUserStep {
        PendingUserStep::ImportAntigravitySnippet
    }

    /// What the user does to remove mmhmem again.
    pub fn removal_instruction() -> &'static str {
        REMOVE_INSTRUCTION
    }

    fn gemini_dir(env: &Env) -> PathBuf {
        env.home().join(".gemini")
    }

    /// Antigravity's global MCP server file, read by the CLI and the IDE.
    pub fn mcp_file(env: &Env) -> PathBuf {
        Self::gemini_dir(env).join("config").join("mcp_config.json")
    }

    /// Antigravity's global hook file.
    pub fn hooks_file(env: &Env) -> PathBuf {
        Self::gemini_dir(env).join("config").join("hooks.json")
    }

    /// The server file Antigravity no longer reads (stale entries only).
    pub fn stale_file(env: &Env) -> PathBuf {
        Self::gemini_dir(env)
            .join("antigravity")
            .join("mcp_config.json")
    }

    /// The data folders that exist, in [`DATA_FOLDERS`] order. The Gemini
    /// home alone proves nothing: the Gemini CLI and other tools create it.
    fn data_dirs(env: &Env) -> Vec<PathBuf> {
        let gemini = Self::gemini_dir(env);
        DATA_FOLDERS
            .iter()
            .map(|folder| gemini.join(folder))
            .filter(|dir| dir.is_dir())
            .collect()
    }

    /// Plans `command` and `args` of mmhmem's server entry; nothing when
    /// both already hold the wanted values and the entry is recorded.
    fn mcp_change(
        &self,
        chain: &mut FileChain,
        env: &Env,
        rec: Option<&AgentRecord>,
    ) -> Result<(), AgentProblem> {
        let file = Self::mcp_file(env);
        let target = mcp_target(&file);
        let recorded = rec.and_then(|r| r.managed.iter().find(|e| e.target == target));
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
            return Ok(());
        }
        let existed = edit::get(input, &ENTRY).map_err(bad)?.is_some();
        let parents = edit::missing_parents(input, &COMMAND).map_err(bad)?;
        let after = edit::set(input, &COMMAND, &command).map_err(bad)?;
        let after = edit::set(&after, &ARGS, &args).map_err(bad)?;
        let missing = recorded
            .and_then(|e| e.missing_parents)
            .unwrap_or(u32::try_from(parents).unwrap_or(0));
        chain.push(Change {
            scope: SCOPE,
            target,
            file: file.clone(),
            action: if existed { Action::Update } else { Action::Add },
            before,
            after: Some(after.into_bytes()),
            owner_only: false,
            hide_diff: false,
            missing_parents: Some(missing),
            summary: say::add(What::Server, AgentId::Antigravity.label(), &file),
        });
        Ok(())
    }

    /// Plans taking one recorded entry out: mmhmem's keys of the server
    /// entry (and the parents this install created), mmhmem's named hook or
    /// an old snippet file. Nothing for a missing file or a target that is
    /// not mmhmem's.
    fn remove_entry(
        &self,
        chain: &mut FileChain,
        entry: &ManagedEntry,
    ) -> Result<(), AgentProblem> {
        let (file, after) = match &entry.target {
            Target::File { file } if *file == self.snippet_file() => {
                if chain.bytes(file)?.is_none() {
                    return Ok(());
                }
                (file, None)
            }
            Target::JsonEntry { file, pointer, .. } if feature_of(&entry.target).is_some() => {
                let Some(text) = chain.text(file)? else {
                    return Ok(());
                };
                let left = if hook::is_hook(pointer) {
                    hook::without_hook(&text)
                } else {
                    without_server(&text, entry.missing_parents)
                };
                let left = left.map_err(|error| AgentProblem::Json {
                    file: file.clone(),
                    error,
                })?;
                (file, Some(left.into_bytes()))
            }
            _ => return Ok(()),
        };
        let before = chain.bytes(file)?;
        chain.push(Change {
            scope: SCOPE,
            target: entry.target.clone(),
            file: file.clone(),
            action: Action::Remove,
            before,
            after,
            owner_only: false,
            hide_diff: false,
            missing_parents: None,
            summary: say::remove(AgentId::Antigravity.label(), file),
        });
        Ok(())
    }
}

/// mmhmem's server entry in `file`; only `command` and `args` are its own.
fn mcp_target(file: &Path) -> Target {
    Target::JsonEntry {
        file: file.to_path_buf(),
        pointer: key_path(&ENTRY).expect("static keys"),
        managed_keys: Some(key_path(&["command", "args"]).expect("static keys")),
    }
}

fn is_entry(pointer: &[KeySegment]) -> bool {
    pointer.iter().map(KeySegment::as_str).eq(ENTRY)
}

/// The feature a recorded entry belongs to (the snippet-era import file was
/// the server registration); `None` for a target that is not mmhmem's.
fn feature_of(target: &Target) -> Option<Feature> {
    match target {
        Target::File { .. } => Some(Feature::Mcp),
        Target::JsonEntry { pointer, .. } if is_entry(pointer) => Some(Feature::Mcp),
        Target::JsonEntry { pointer, .. } if hook::is_hook(pointer) => Some(Feature::ReadHook),
        _ => None,
    }
}

/// The text without mmhmem's `command` and `args`, and without the
/// `parents` objects this install created for them.
fn without_server(text: &str, parents: Option<u32>) -> Result<String, edit::JsonEditError> {
    let parents = parents.unwrap_or(0) as usize;
    let text = edit::remove(text, &COMMAND)?;
    edit::remove_and_prune(&text, &ARGS, parents)
}

impl Agent for Antigravity {
    fn id(&self) -> AgentId {
        AgentId::Antigravity
    }

    /// Found when `agy` is on PATH or a data folder exists; the program
    /// comes first in `paths`, and only the program gives a version.
    fn detect(&self, env: &Env) -> Detection {
        let program = env.find_program(PROGRAM);
        let mut paths: Vec<PathBuf> = program.iter().cloned().collect();
        paths.extend(Self::data_dirs(env));
        if paths.is_empty() {
            tracing::debug!(
                "{PROGRAM} is not on PATH and none of {} exist in {}",
                DATA_FOLDERS.join(", "),
                Self::gemini_dir(env).display()
            );
            return Detection::NotFound {
                reason: say::not_installed(AgentId::Antigravity.label()),
            };
        }
        let version = program.and_then(|p| version(env, &p));
        Detection::Found { version, paths }
    }

    fn features(&self) -> &'static [Feature] {
        &FEATURES
    }

    /// The server entry in [`Antigravity::mcp_file`] and the named hook in
    /// [`Antigravity::hooks_file`]; what the record lists for a feature no
    /// longer wanted comes back out. A hook command that cannot be written
    /// leaves only the hook out ([`Agent::feature_problems`]).
    fn plan(
        &self,
        env: &Env,
        want: &Selection,
        rec: Option<&AgentRecord>,
    ) -> Result<Vec<Change>, AgentProblem> {
        let mut chain = FileChain::new();
        for entry in rec.map(|r| r.managed.as_slice()).unwrap_or_default() {
            if feature_of(&entry.target).is_some_and(|f| !want.wants(ID, f)) {
                self.remove_entry(&mut chain, entry)?;
            }
        }
        if want.wants(ID, Feature::Mcp) {
            self.mcp_change(&mut chain, env, rec)?;
        }
        if want.wants(ID, Feature::ReadHook) {
            match self.hook_entry(env.os()) {
                Ok(value) => Self::hook_change(&mut chain, env, rec, &value)?,
                Err(problem) => tracing::warn!("{problem}"),
            }
        }
        Ok(chain.into_changes())
    }

    /// The read hook when its command cannot be written.
    fn feature_problems(&self, env: &Env, want: &Selection) -> Vec<AgentProblem> {
        if !want.wants(ID, Feature::ReadHook) {
            return Vec::new();
        }
        self.hook_entry(env.os()).err().into_iter().collect()
    }

    /// Starts exactly the command the snippet names (the same `command` and
    /// `args` as the server entry); task 3.5 reads the server file instead.
    fn check(&self, env: &Env, rec: &AgentRecord) -> CheckReport {
        let snippet = self.snippet();
        let registration = Registration::from_claude_json(snippet.as_bytes(), "mmhmem");
        Steps::start(env, AgentId::Antigravity, registration).finish(rec)
    }

    /// Takes mmhmem's keys out of the server entry, mmhmem's named hook and
    /// an old snippet file out; everything else in Antigravity's files stays
    /// as it was.
    fn plan_remove(&self, _env: &Env, rec: &AgentRecord) -> Vec<Change> {
        let mut chain = FileChain::new();
        for entry in &rec.managed {
            let _ = self.remove_entry(&mut chain, entry);
        }
        chain.into_changes()
    }

    /// The user imports [`Antigravity::snippet`] themselves (until task 4.1).
    fn pending_steps(&self, want: &Selection) -> Vec<PendingUserStep> {
        if want.wants(AgentId::Antigravity, Feature::Mcp) {
            vec![Self::pending_step()]
        } else {
            Vec::new()
        }
    }
}

/// The first line of `agy --version` (`1.5.3`); `None` when the call fails
/// or prints nothing.
fn version(env: &Env, program: &Path) -> Option<String> {
    let out = env.run(&Invocation::new(program, &["--version"])).ok()?;
    if !out.success() {
        return None;
    }
    let line = out.stdout.lines().next()?.trim();
    (!line.is_empty()).then(|| line.to_owned())
}

#[cfg(test)]
#[path = "antigravity_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "antigravity_hook_tests.rs"]
mod hook_tests;
