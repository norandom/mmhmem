//! Codex integration (reliable-installers task 3.4,
//! `.kiro/specs/reliable-installers/design.md` §5 "Codex"; Requirements 3.1,
//! 3.4, 6.6 and 7.3).
//!
//! Plans, in `<CODEX_HOME or ~/.codex>`:
//!
//! - `config.toml`: `[mcp_servers.mmhmem]` with `command` (the installed
//!   binary) and `args = ["stdio"]`, through `edit::toml::set_table`, which
//!   keeps every other key of the table (`tool_timeout_sec`, `env`, ...). The
//!   record hashes only `command` and `args`. The old installer's
//!   `# BEGIN mmhmem` block is converted in the same change.
//! - `hooks.json`: one `PreToolUse` group (matcher `^(Bash|shell)$`, command
//!   `<binary> hook codex`, timeout 10 s: the shape of `manage_hooks.ps1` and
//!   `manage_hooks.py`) as one element of the shared array.
//! - `prompts/mmhmem-recap.md`, and in `~/.agents/skills` the
//!   `unified-memory` and `mmhmem-recap` skills.
//!
//! A hook that Codex does not know yet is skipped until the user approves it
//! in `/hooks`: [`Codex::pending_steps`] names that step. `codex mcp list
//! --json` prints env values unmasked, so only the check (`codex_check.rs`)
//! runs it, parsing it in memory.
//!
//! **Removal** takes out only what the record lists. Only `command` and
//! `args` leave mmhmem's table; keys the user added stay byte-identical, and
//! a table left empty goes whole. After removing the last hook group of a
//! `hooks.json` mmhmem created, the file is left as `{}`.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::{
    Agent, AgentId, AgentProblem, AgentRecord, CheckReport, Detection, Env, Feature, FileChain,
    Invocation, PendingUserStep, Selection,
};

#[path = "codex_check.rs"]
mod verify;
use crate::setup::assets::{self, Asset};
use crate::setup::edit::json::{self as ejson, JsonEditError};
use crate::setup::edit::toml::{self as etoml, McpServerEntry, TomlEditError};
use crate::setup::plan::{Action, Change};
use crate::setup::record::{
    ElementMatcher, HookHarness, KeySegment, ManagedEntry, Os, Scope, Target, key_path,
};
use crate::setup::ui::text::changes::{self as say, What};

const ID: AgentId = AgentId::Codex;
const SCOPE: Scope = Scope::Agent(ID);
const TABLE: [&str; 2] = ["mcp_servers", "mmhmem"];
const MANAGED_KEYS: [&str; 2] = ["command", "args"];
const HOOK_PATH: [&str; 2] = ["hooks", "PreToolUse"];
const HOOK_MATCHER: &str = "^(Bash|shell)$";
const HOOK_TIMEOUT: u64 = 10;
const OWN_HOOK: ElementMatcher = ElementMatcher::MmhmemHook(HookHarness::Codex);
const FEATURES: [Feature; 4] = [
    Feature::Mcp,
    Feature::Skill,
    Feature::RecapCommand,
    Feature::ReadHook,
];

/// The Codex integration.
#[derive(Debug, Clone)]
pub struct Codex {
    /// The installed `mmhmem` binary (registered and used by the hook).
    pub binary: PathBuf,
}

/// Where Codex's files live for one [`Env`].
struct Paths {
    config: PathBuf,
    hooks: PathBuf,
    prompt: PathBuf,
    skills: [(PathBuf, Asset); 2],
}

impl Paths {
    fn new(env: &Env) -> Self {
        let codex = env.codex_home();
        let skills = env.home().join(".agents").join("skills");
        let skill = |name: &str| skills.join(name).join("SKILL.md");
        Self {
            config: codex.join("config.toml"),
            hooks: codex.join("hooks.json"),
            prompt: codex.join("prompts").join("mmhmem-recap.md"),
            skills: [
                (skill("unified-memory"), assets::UNIFIED_MEMORY_SKILL),
                (skill("mmhmem-recap"), assets::CODEX_RECAP_SKILL),
            ],
        }
    }

    /// The feature a recorded target belongs to.
    fn feature_of(&self, target: &Target) -> Option<Feature> {
        match target {
            Target::TomlTable { .. } => Some(Feature::Mcp),
            Target::JsonArrayElement { .. } => Some(Feature::ReadHook),
            Target::File { file } if *file == self.prompt => Some(Feature::RecapCommand),
            Target::File { file } if self.skills.iter().any(|(f, _)| f == file) => {
                Some(Feature::Skill)
            }
            _ => None,
        }
    }
}

fn keys(parts: &[&str]) -> Vec<KeySegment> {
    key_path(parts).expect("static key names are valid")
}

fn mcp_target(file: &Path) -> Target {
    Target::TomlTable {
        file: file.to_path_buf(),
        key: keys(&TABLE),
        managed_keys: Some(keys(&MANAGED_KEYS)),
    }
}

fn hook_target(file: &Path) -> Target {
    Target::JsonArrayElement {
        file: file.to_path_buf(),
        pointer: keys(&HOOK_PATH),
        matcher: OWN_HOOK,
    }
}

fn toml_problem(file: &Path) -> impl Fn(TomlEditError) -> AgentProblem + '_ {
    |error| AgentProblem::Toml {
        file: file.to_path_buf(),
        error,
    }
}

fn json_problem(file: &Path) -> impl Fn(JsonEditError) -> AgentProblem + '_ {
    |error| AgentProblem::Json {
        file: file.to_path_buf(),
        error,
    }
}

/// The hook command: PowerShell form on Windows, quoted shell form elsewhere.
fn hook_command(binary: &Path, os: Os) -> String {
    let bin = binary.to_string_lossy();
    if os == Os::Windows {
        return format!("& '{}' hook codex", bin.replace('\'', "''"));
    }
    let plain = |c: char| c.is_ascii_alphanumeric() || "_@%+=:,./-".contains(c);
    if !bin.is_empty() && bin.chars().all(plain) {
        format!("{bin} hook codex")
    } else {
        format!("'{}' hook codex", bin.replace('\'', "'\"'\"'"))
    }
}

fn change(
    target: Target,
    file: &Path,
    before: Option<Vec<u8>>,
    after: Option<Vec<u8>>,
    summary: String,
) -> Change {
    let action = match (&before, &after) {
        (_, None) => Action::Remove,
        (None, _) => Action::Add,
        (Some(_), Some(_)) => Action::Update,
    };
    Change {
        scope: SCOPE,
        target,
        file: file.to_path_buf(),
        action,
        before,
        after,
        owner_only: false,
        hide_diff: false,
        missing_parents: None,
        summary,
    }
}

/// Adds `change` unless it changes nothing and the record already tracks it.
fn push_unless_noop(chain: &mut FileChain, rec: Option<&AgentRecord>, change: Change) {
    let tracked = rec.is_some_and(|r| r.managed.iter().any(|e| e.target == change.target));
    if !(tracked && change.before == change.after) {
        chain.push(change);
    }
}

fn pointer_of(pointer: &[KeySegment]) -> Vec<&str> {
    pointer.iter().map(KeySegment::as_str).collect()
}

impl Codex {
    /// A Codex integration for the installed `binary`.
    pub fn new(binary: impl Into<PathBuf>) -> Self {
        Self {
            binary: binary.into(),
        }
    }

    /// The steps only the user can do for `want`: approving a new hook.
    pub fn pending_steps(want: &Selection) -> Vec<PendingUserStep> {
        if want.wants(ID, Feature::ReadHook) {
            vec![PendingUserStep::ApproveHookInCodex]
        } else {
            Vec::new()
        }
    }

    fn hook_group(&self, os: Os) -> Value {
        json!({
            "matcher": HOOK_MATCHER,
            "hooks": [{
                "type": "command",
                "command": hook_command(&self.binary, os),
                "timeout": HOOK_TIMEOUT,
            }],
        })
    }

    fn plan_mcp(&self, p: &Paths, chain: &mut FileChain, rec: Option<&AgentRecord>) -> Planned {
        let file = &p.config;
        let before = chain.bytes(file)?;
        let mut text = chain.text(file)?.unwrap_or_default();
        let problem = toml_problem(file);
        let existed = etoml::get_table(&text, &TABLE).map_err(&problem)?.is_some();
        if etoml::has_marker_block(&text) {
            text = etoml::convert_marker_block(&text, &TABLE).map_err(&problem)?;
        }
        let entry = McpServerEntry {
            command: self.binary.to_string_lossy().into_owned(),
            args: vec!["stdio".to_owned()],
            ..McpServerEntry::default()
        };
        let after = etoml::set_table(&text, &TABLE, &entry).map_err(&problem)?;
        let mut c = change(
            mcp_target(file),
            file,
            before,
            Some(after.into_bytes()),
            say::add(What::Server, ID.label(), file),
        );
        c.action = if existed { Action::Update } else { Action::Add };
        push_unless_noop(chain, rec, c);
        Ok(())
    }

    fn plan_hook(
        &self,
        env: &Env,
        p: &Paths,
        chain: &mut FileChain,
        rec: Option<&AgentRecord>,
    ) -> Planned {
        let file = &p.hooks;
        let before = chain.bytes(file)?;
        let mut text = chain.text(file)?.unwrap_or_else(|| "{}\n".to_owned());
        let problem = json_problem(file);
        let found =
            ejson::array_find(&text, &HOOK_PATH, |v| OWN_HOOK.matches(v)).map_err(&problem)?;
        let want = self.hook_group(env.os());
        let target = hook_target(file);
        let prior = rec
            .and_then(|r| r.managed.iter().find(|e| e.target == target))
            .and_then(|e| e.missing_parents);
        let mut missing = prior.unwrap_or(0);
        let current = ejson::get(&text, &HOOK_PATH).map_err(&problem)?;
        let up_to_date = found.len() == 1
            && current
                .as_ref()
                .and_then(Value::as_array)
                .is_some_and(|a| a[found[0]] == want);
        if !up_to_date {
            for index in found.iter().rev() {
                text = ejson::array_remove(&text, &HOOK_PATH, *index).map_err(&problem)?;
            }
            if found.is_empty() {
                missing = ejson::missing_keys(&text, &HOOK_PATH).map_err(&problem)? as u32;
            }
            text = ejson::array_append(&text, &HOOK_PATH, &want).map_err(&problem)?;
        }
        let mut c = change(
            target,
            file,
            before,
            Some(text.into_bytes()),
            say::add(What::ReadHook, ID.label(), file),
        );
        c.missing_parents = Some(missing);
        if found.is_empty() {
            c.action = Action::Add;
        }
        push_unless_noop(chain, rec, c);
        Ok(())
    }

    fn plan_file(
        chain: &mut FileChain,
        rec: Option<&AgentRecord>,
        file: &Path,
        asset: Asset,
        what: What,
    ) -> Planned {
        let before = chain.bytes(file)?;
        let target = Target::File {
            file: file.to_path_buf(),
        };
        let summary = say::add(what, ID.label(), file);
        let c = change(target, file, before, Some(asset.bytes().to_vec()), summary);
        push_unless_noop(chain, rec, c);
        Ok(())
    }

    /// The change that takes out one recorded entry; `None` when its file is
    /// gone or the entry is not Codex's kind.
    fn removal(
        chain: &mut FileChain,
        entry: &ManagedEntry,
    ) -> Result<Option<Change>, AgentProblem> {
        let summary = |file: &Path| say::remove(ID.label(), file);
        match &entry.target {
            Target::File { file } => {
                let before = chain.bytes(file)?;
                Ok(Some(change(
                    entry.target.clone(),
                    file,
                    before,
                    None,
                    summary(file),
                )))
            }
            Target::TomlTable { file, .. } => {
                let Some(text) = chain.text(file)? else {
                    return Ok(None);
                };
                let after =
                    etoml::remove_keys(&text, &TABLE, &MANAGED_KEYS).map_err(toml_problem(file))?;
                let before = Some(text.into_bytes());
                let c = change(
                    entry.target.clone(),
                    file,
                    before,
                    Some(after.into_bytes()),
                    summary(file),
                );
                Ok(Some(Change {
                    action: Action::Remove,
                    ..c
                }))
            }
            Target::JsonArrayElement {
                file,
                pointer,
                matcher,
            } => {
                let Some(text) = chain.text(file)? else {
                    return Ok(None);
                };
                let problem = json_problem(file);
                let path = pointer_of(pointer);
                let found =
                    ejson::array_find(&text, &path, |v| matcher.matches(v)).map_err(&problem)?;
                let levels = entry.missing_parents.unwrap_or(0) as usize;
                let mut after = text.clone();
                for index in found.iter().rev() {
                    after = ejson::array_remove_and_prune(&after, &path, *index, levels)
                        .map_err(&problem)?;
                }
                let c = change(
                    entry.target.clone(),
                    file,
                    Some(text.into_bytes()),
                    Some(after.into_bytes()),
                    summary(file),
                );
                Ok(Some(Change {
                    action: Action::Remove,
                    ..c
                }))
            }
            _ => Ok(None),
        }
    }

    fn plan_removals(
        p: &Paths,
        chain: &mut FileChain,
        want: &Selection,
        rec: &AgentRecord,
    ) -> Planned {
        for entry in &rec.managed {
            let dropped = p
                .feature_of(&entry.target)
                .is_some_and(|f| !want.wants(ID, f));
            if dropped && let Some(c) = Self::removal(chain, entry)? {
                chain.push(c);
            }
        }
        Ok(())
    }
}

type Planned = Result<(), AgentProblem>;

impl Agent for Codex {
    fn id(&self) -> AgentId {
        ID
    }

    fn detect(&self, env: &Env) -> Detection {
        let program = env.find_program("codex");
        let home = env.codex_home();
        let mut paths: Vec<PathBuf> = program.iter().cloned().collect();
        if home.exists() {
            paths.push(home.clone());
        }
        if paths.is_empty() {
            tracing::debug!("codex is not on PATH and {} does not exist", home.display());
            return Detection::NotFound {
                reason: say::not_installed(ID.label()),
            };
        }
        let version = program
            .and_then(|p| env.run(&Invocation::new(p, &["--version"])).ok())
            .filter(|out| out.success())
            .and_then(|out| out.stdout.lines().next().map(str::trim).map(str::to_owned))
            .map(|line| line.strip_prefix("codex-cli ").unwrap_or(&line).to_owned())
            .filter(|v| !v.is_empty());
        Detection::Found { version, paths }
    }

    fn features(&self) -> &'static [Feature] {
        &FEATURES
    }

    fn plan(
        &self,
        env: &Env,
        want: &Selection,
        rec: Option<&AgentRecord>,
    ) -> Result<Vec<Change>, AgentProblem> {
        let p = Paths::new(env);
        let mut chain = FileChain::new();
        if want.wants(ID, Feature::Mcp) {
            self.plan_mcp(&p, &mut chain, rec)?;
        }
        if want.wants(ID, Feature::ReadHook) {
            self.plan_hook(env, &p, &mut chain, rec)?;
        }
        if want.wants(ID, Feature::Skill) {
            for (file, asset) in &p.skills {
                Self::plan_file(&mut chain, rec, file, *asset, What::Guide)?;
            }
        }
        if want.wants(ID, Feature::RecapCommand) {
            let asset = assets::CODEX_RECAP_PROMPT;
            Self::plan_file(&mut chain, rec, &p.prompt, asset, What::Recap)?;
        }
        if let Some(rec) = rec {
            Self::plan_removals(&p, &mut chain, want, rec)?;
        }
        Ok(chain.into_changes())
    }

    fn check(&self, env: &Env, rec: &AgentRecord) -> CheckReport {
        verify::check(self, env, rec)
    }

    fn plan_remove(&self, _env: &Env, rec: &AgentRecord) -> Vec<Change> {
        let mut chain = FileChain::new();
        for entry in &rec.managed {
            if let Ok(Some(c)) = Self::removal(&mut chain, entry) {
                chain.push(c);
            }
        }
        chain.into_changes()
    }

    fn pending_steps(&self, want: &Selection) -> Vec<PendingUserStep> {
        Self::pending_steps(want)
    }
}

#[cfg(test)]
#[path = "codex_tests.rs"]
mod tests;
