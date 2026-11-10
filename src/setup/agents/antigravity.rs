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
//! - **Earlier installs** (task 3.4; Requirements 2.3, 5.1, 5.2 and 5.4):
//!   a recorded import snippet ([`SNIPPET_FILE`]) is removed in every plan,
//!   and mmhmem's entry in the server file Antigravity no longer reads
//!   ([`Antigravity::stale_file`]) is removed in every plan, whatever is
//!   selected, and on removal (`antigravity_check.rs`). A recorded hook
//!   whose command can no longer be written comes out too: it would start a
//!   program that is gone and so block every Antigravity tool. No pending
//!   step is emitted; the snippet-era
//!   [`PendingUserStep::ImportAntigravitySnippet`] still loads from old
//!   records and is never saved again.
//! - **Check** (task 3.5; Requirements 2.6 and 4.1–4.3): from the server
//!   file's bytes, `agy mcp list` when `agy` is on PATH, and the hook sample
//!   whenever the hook is selected and recorded (`antigravity_check.rs`).

use std::path::{Path, PathBuf};

#[cfg(test)]
use serde_json::Value;
use serde_json::json;

use super::{
    Agent, AgentId, AgentProblem, AgentRecord, CheckReport, Detection, Env, Feature, FileChain,
    Invocation, PendingUserStep, Selection,
};
use crate::setup::edit::json as edit;
use crate::setup::plan::{Action, Change};
use crate::setup::record::{KeySegment, ManagedEntry, Scope, Target, key_path};
use crate::setup::ui::text::changes::{self as say, What};

#[path = "antigravity_check.rs"]
mod check;
#[path = "antigravity_hook.rs"]
mod hook;
pub use hook::{HOOK_MATCHER, HOOK_NAME, HOOK_TIMEOUT, hook_command};

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

    /// The JSON older installs (0.5.1) saved for the user to import; tests
    /// recreate such an install with it. Setup never writes it any more.
    #[cfg(test)]
    pub fn snippet(&self) -> String {
        let value = json!({"mcpServers": {"mmhmem": {
            "command": self.binary.to_string_lossy(),
            "args": ["stdio"],
        }}});
        serde_json::to_string_pretty(&value).expect("a JSON value serializes")
    }

    /// The file older installs saved the snippet to, next to the binary.
    #[cfg(test)]
    pub fn snippet_file(&self) -> PathBuf {
        self.binary
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(SNIPPET_FILE)
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
    /// an old snippet file, wherever it is. Nothing for a missing file or a
    /// target that is not mmhmem's.
    fn remove_entry(
        &self,
        chain: &mut FileChain,
        entry: &ManagedEntry,
    ) -> Result<(), AgentProblem> {
        let (file, after) = match &entry.target {
            Target::File { file } if check::is_snippet(file) => {
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

    /// Plans the removal of mmhmem's entry in [`Antigravity::stale_file`]
    /// when there is one ([`check::stale_entry`]).
    fn stale_change(&self, chain: &mut FileChain, env: &Env) {
        if let Some(change) = check::stale_entry(&Self::stale_file(env), &self.binary) {
            chain.push(change);
        }
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

/// The feature a recorded JSON entry belongs to; `None` for a target that
/// is neither of mmhmem's two entries.
fn feature_of(target: &Target) -> Option<Feature> {
    match target {
        Target::JsonEntry { pointer, .. } if is_entry(pointer) => Some(Feature::Mcp),
        Target::JsonEntry { pointer, .. } if hook::is_hook(pointer) => Some(Feature::ReadHook),
        _ => None,
    }
}

/// Whether a recorded entry comes out in a plan for `want`: an old snippet
/// file always (Requirement 5.1), the server entry when its feature is no
/// longer wanted, and the hook unless it is wanted and its command can be
/// written (`hook_ok`; Requirement 2.3). Other targets are not mmhmem's.
fn comes_out(target: &Target, want: &Selection, hook_ok: bool) -> bool {
    match (target, feature_of(target)) {
        (Target::File { file }, _) => check::is_snippet(file),
        (_, Some(Feature::ReadHook)) => !hook_ok,
        (_, Some(feature)) => !want.wants(ID, feature),
        (_, None) => false,
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

    /// The server entry in [`Antigravity::mcp_file`], the removal of a stale
    /// entry in [`Antigravity::stale_file`] whatever is selected, and the
    /// named hook in [`Antigravity::hooks_file`]; recorded entries that come
    /// out ([`comes_out`]) go first. A hook command that cannot be written
    /// leaves only the hook out ([`Agent::feature_problems`]) and takes a
    /// recorded hook out.
    fn plan(
        &self,
        env: &Env,
        want: &Selection,
        rec: Option<&AgentRecord>,
    ) -> Result<Vec<Change>, AgentProblem> {
        let mut chain = FileChain::new();
        let hook = want
            .wants(ID, Feature::ReadHook)
            .then(|| self.hook_entry(env.os()));
        let hook_ok = matches!(hook, Some(Ok(_)));
        for entry in rec.map(|r| r.managed.as_slice()).unwrap_or_default() {
            if comes_out(&entry.target, want, hook_ok) {
                self.remove_entry(&mut chain, entry)?;
            }
        }
        if want.wants(ID, Feature::Mcp) {
            self.mcp_change(&mut chain, env, rec)?;
        }
        self.stale_change(&mut chain, env);
        match hook {
            Some(Ok(value)) => Self::hook_change(&mut chain, env, rec, &value)?,
            Some(Err(problem)) => tracing::warn!("{problem}"),
            None => {}
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

    /// Starts exactly the command [`Antigravity::mcp_file`] names, confirms
    /// with `agy mcp list` when `agy` is on PATH and runs the hook sample
    /// when the hook is selected and recorded ([`check::check`]).
    fn check(&self, env: &Env, rec: &AgentRecord) -> CheckReport {
        check::check(self, env, rec)
    }

    /// Takes mmhmem's keys out of the server entry, mmhmem's named hook, an
    /// old snippet file and a stale entry out; everything else in
    /// Antigravity's files stays as it was.
    fn plan_remove(&self, env: &Env, rec: &AgentRecord) -> Vec<Change> {
        let mut chain = FileChain::new();
        for entry in &rec.managed {
            let _ = self.remove_entry(&mut chain, entry);
        }
        self.stale_change(&mut chain, env);
        chain.into_changes()
    }

    /// None: a new Antigravity session is the generic next step, and the
    /// snippet-era import step is not emitted again (Requirement 5.4).
    fn pending_steps(&self, _want: &Selection) -> Vec<PendingUserStep> {
        Vec::new()
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

#[cfg(test)]
#[path = "antigravity_check_tests.rs"]
mod check_tests;

#[cfg(test)]
#[path = "antigravity_functional_tests.rs"]
mod functional_tests;
