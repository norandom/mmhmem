//! Claude Code integration (reliable-installers task 3.2,
//! `.kiro/specs/reliable-installers/design.md` §5; Requirements 3.1, 3.4 and
//! 7.3).
//!
//! What is planned, per feature (paths as the old installers used them, see
//! `adopt_table.rs`):
//!
//! - **MCP** (always planned when any Claude Code feature is selected): the
//!   leaf keys `type`, `command` and `args` of `mcpServers.mmhmem` in
//!   `~/.claude.json`, edited as file bytes with `edit::json` (decision for
//!   task 3.2: reviewable, journaled and rollback-safe, unlike
//!   `claude mcp add`). The file is rewritten by Claude Code often and can be
//!   large, so it is planned against its current bytes, never reformatted,
//!   and stays strict JSON. It can hold account data and other servers'
//!   secrets, so its diff is hidden; its permissions are kept as they are.
//! - **Skill**: `~/.claude/skills/unified-memory/SKILL.md`.
//! - **Recap command**: `~/.claude/commands/mmhmem-recap.md`.
//! - **Instructions**: the `<!-- mmhmem:routing:… -->` section in
//!   `~/.claude/rules/mmhmem.md`, in the exact form `manage_routing.ps1`
//!   writes (`edit::markdown`).
//! - **Read hook**: one handler in `<CLAUDE_CONFIG_DIR or ~/.claude>/settings.json`
//!   `hooks.PreToolUse`, the shape `manage_hooks.ps1` writes (lines 201-205
//!   and 230-232): `{type: "command", command: <binary>, args: ["hook",
//!   "claude"], timeout: 10}`, new in its own group with matcher
//!   `Read|Bash|PowerShell`. mmhmem owns the handler, never the group: one
//!   already in a user's group is updated or removed there alone
//!   (`hook_entry.rs`).
//!
//! The installed binary path comes from the caller ([`ClaudeCode::new`]).
//!
//! **Removal** works from the record's entries only. Keys the user added
//! inside `mcpServers.mmhmem` are kept, and so is the `mmhmem` object while it
//! still holds such a key; an object left empty is removed, then as many
//! parents as the record says mmhmem created. A file that no longer parses is
//! skipped by [`Agent::plan_remove`]: its entry stays in the record (drift
//! reports it as unreadable) so a later run can remove it once it is fixed.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::{
    Agent, AgentId, AgentProblem, AgentRecord, CheckReport, Detection, Env, Feature, FileChain,
    Invocation, Selection, hook_entry,
};
use crate::setup::assets::{CLAUDE_RECAP_COMMAND, ROUTING_INSTRUCTIONS, UNIFIED_MEMORY_SKILL};
use crate::setup::edit::json::{self as edit_json, JsonEditError};
use crate::setup::edit::markdown::{self, routing};
use crate::setup::plan::{Action, Change};
use crate::setup::record::{
    ElementMatcher, EntryState, HookHarness, KeySegment, Observation, Os, Scope, Sha256Hex, Target,
    key_path, observe_in,
};
use crate::setup::ui::text::changes::{self as say, What};

#[path = "claude_code_remove.rs"]
mod remove;
#[path = "claude_code_check.rs"]
mod verify;
use remove::removal;

/// The features in menu order.
const FEATURES: [Feature; 5] = [
    Feature::Mcp,
    Feature::Skill,
    Feature::RecapCommand,
    Feature::Instructions,
    Feature::ReadHook,
];
const SCOPE: Scope = Scope::Agent(AgentId::ClaudeCode);
/// `mcpServers.mmhmem`.
const MCP_POINTER: [&str; 2] = ["mcpServers", "mmhmem"];
/// The keys mmhmem writes inside its MCP entry.
const MCP_KEYS: [&str; 3] = ["type", "command", "args"];
/// `hooks.PreToolUse`.
const HOOK_POINTER: [&str; 2] = ["hooks", "PreToolUse"];
/// The tools the hook sees (`manage_hooks.ps1:203`).
pub const HOOK_MATCHER: &str = "Read|Bash|PowerShell";
/// The hook's time limit in seconds (`manage_hooks.ps1:205`).
pub const HOOK_TIMEOUT: u64 = 10;

/// Claude Code. `binary` is the installed mmhmem binary the MCP entry and
/// the hook start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaudeCode {
    binary: PathBuf,
}

impl ClaudeCode {
    /// Claude Code wired to the installed binary at `binary`.
    pub fn new(binary: impl Into<PathBuf>) -> Self {
        Self {
            binary: binary.into(),
        }
    }

    fn binary_text(&self) -> String {
        self.binary.to_string_lossy().into_owned()
    }
}

/// Where Claude Code's files are.
struct Paths {
    claude_json: PathBuf,
    skill: PathBuf,
    command: PathBuf,
    rules: PathBuf,
    settings: PathBuf,
}

impl Paths {
    fn of(env: &Env) -> Self {
        let dir = env.home().join(".claude");
        Self {
            claude_json: env.home().join(".claude.json"),
            skill: dir.join("skills").join("unified-memory").join("SKILL.md"),
            command: dir.join("commands").join("mmhmem-recap.md"),
            rules: dir.join("rules").join("mmhmem.md"),
            settings: env.claude_config_dir().join("settings.json"),
        }
    }
}

fn keys(parts: &[&str]) -> Vec<KeySegment> {
    key_path(parts).expect("static key names are valid")
}

fn mcp_target(paths: &Paths) -> Target {
    Target::JsonEntry {
        file: paths.claude_json.clone(),
        pointer: keys(&MCP_POINTER),
        managed_keys: Some(keys(&MCP_KEYS)),
    }
}

fn hook_target(paths: &Paths) -> Target {
    Target::JsonArrayElement {
        file: paths.settings.clone(),
        pointer: keys(&HOOK_POINTER),
        matcher: ElementMatcher::MmhmemHook(HookHarness::Claude),
    }
}

/// The hook group `manage_hooks.ps1` writes for Claude Code.
pub fn hook_group(binary: &str) -> Value {
    json!({
        "matcher": HOOK_MATCHER,
        "hooks": [{
            "type": "command",
            "command": binary,
            "args": ["hook", HookHarness::Claude.as_str()],
            "timeout": HOOK_TIMEOUT,
        }],
    })
}

fn json_problem(file: &Path) -> impl Fn(JsonEditError) -> AgentProblem + use<> {
    let file = file.to_path_buf();
    move |error| AgentProblem::Json {
        file: file.clone(),
        error,
    }
}

fn hash_in(target: &Target, bytes: Option<&[u8]>) -> Option<Sha256Hex> {
    match observe_in(target, bytes?) {
        Observation::Present(hash) => Some(hash),
        _ => None,
    }
}

/// One install change before it is compared with the disk and the record.
struct Proposal {
    target: Target,
    file: PathBuf,
    after: Vec<u8>,
    missing_parents: Option<u32>,
    summary: String,
}

/// Hidden diff: `~/.claude.json` can hold account data and other servers'
/// secrets. It keeps its own permissions (written with `write_atomic`).
fn hides_contents(file: &Path) -> bool {
    file.file_name().is_some_and(|n| n == ".claude.json")
}

/// Adds `p` to the chain unless it changes nothing the record already has.
fn propose(
    chain: &mut FileChain,
    rec: Option<&AgentRecord>,
    p: Proposal,
) -> Result<(), AgentProblem> {
    let before = chain.bytes(&p.file)?;
    let present = hash_in(&p.target, before.as_deref()).is_some();
    let recorded = rec.and_then(|r| r.managed.iter().find(|e| e.target == p.target));
    let same_bytes = before.as_deref() == Some(p.after.as_slice());
    let after_hash = hash_in(&p.target, Some(&p.after));
    if same_bytes && recorded.is_some_and(|e| e.state == EntryState::Done && e.sha256 == after_hash)
    {
        return Ok(());
    }
    let missing_parents = match recorded {
        Some(e) if present => e.missing_parents.or(p.missing_parents),
        _ => p.missing_parents,
    };
    chain.push(Change {
        scope: SCOPE,
        owner_only: false,
        hide_diff: hides_contents(&p.file),
        target: p.target,
        file: p.file,
        action: if present { Action::Update } else { Action::Add },
        before,
        after: Some(p.after),
        missing_parents,
        summary: p.summary,
    });
    Ok(())
}

impl ClaudeCode {
    fn plan_mcp(&self, chain: &mut FileChain, paths: &Paths) -> Result<Proposal, AgentProblem> {
        let file = &paths.claude_json;
        let err = json_problem(file);
        let mut text = chain.text(file)?.unwrap_or_default();
        let missing = edit_json::missing_parents(&text, &MCP_POINTER).map_err(&err)?;
        let values = [json!("stdio"), json!(self.binary_text()), json!(["stdio"])];
        for (key, value) in MCP_KEYS.iter().zip(values) {
            let path = [MCP_POINTER[0], MCP_POINTER[1], key];
            text = edit_json::set(&text, &path, &value).map_err(&err)?;
        }
        Ok(Proposal {
            target: mcp_target(paths),
            file: file.clone(),
            after: text.into_bytes(),
            missing_parents: Some(missing as u32),
            summary: say::add(What::Server, AgentId::ClaudeCode.label(), file),
        })
    }

    fn plan_hook(&self, chain: &mut FileChain, paths: &Paths) -> Result<Proposal, AgentProblem> {
        let file = &paths.settings;
        let err = json_problem(file);
        let text = chain.text(file)?.unwrap_or_default();
        let want = hook_group(&self.binary_text());
        let matcher = ElementMatcher::MmhmemHook(HookHarness::Claude);
        // Exec form: no shell reads it, so the OS does not matter.
        let os = Os::current();
        let planned =
            hook_entry::install(&text, &HOOK_POINTER, matcher, &want, os).map_err(&err)?;
        Ok(Proposal {
            target: hook_target(paths),
            file: file.clone(),
            after: planned.after.into_bytes(),
            missing_parents: Some(planned.missing as u32),
            summary: say::add(What::ReadHook, AgentId::ClaudeCode.label(), file),
        })
    }

    fn plan_feature(
        &self,
        chain: &mut FileChain,
        paths: &Paths,
        feature: Feature,
    ) -> Result<Proposal, AgentProblem> {
        let owned = |file: &PathBuf, bytes: &[u8], what: What| Proposal {
            target: Target::File { file: file.clone() },
            file: file.clone(),
            after: bytes.to_vec(),
            missing_parents: None,
            summary: say::add(what, AgentId::ClaudeCode.label(), file),
        };
        match feature {
            Feature::Mcp => self.plan_mcp(chain, paths),
            Feature::ReadHook => self.plan_hook(chain, paths),
            Feature::Skill => Ok(owned(
                &paths.skill,
                UNIFIED_MEMORY_SKILL.bytes(),
                What::Guide,
            )),
            Feature::RecapCommand => Ok(owned(
                &paths.command,
                CLAUDE_RECAP_COMMAND.bytes(),
                What::Recap,
            )),
            Feature::Instructions => {
                let file = &paths.rules;
                let text = chain.text(file)?.unwrap_or_default();
                let body = ROUTING_INSTRUCTIONS.text;
                let after = markdown::install_section(&text, &routing(), body)
                    .map_err(AgentProblem::markdown(file))?;
                Ok(Proposal {
                    target: Target::MarkdownSection {
                        file: file.clone(),
                        section: routing(),
                    },
                    file: file.clone(),
                    after: after.into_bytes(),
                    missing_parents: None,
                    summary: say::add(What::Instructions, AgentId::ClaudeCode.label(), file),
                })
            }
        }
    }
}

/// The feature a recorded entry belongs to.
fn feature_of(target: &Target) -> Option<Feature> {
    match target {
        Target::JsonEntry { .. } => Some(Feature::Mcp),
        Target::JsonArrayElement { .. } => Some(Feature::ReadHook),
        Target::MarkdownSection { .. } => Some(Feature::Instructions),
        Target::File { file } if file.file_name().is_some_and(|n| n == "SKILL.md") => {
            Some(Feature::Skill)
        }
        Target::File { .. } => Some(Feature::RecapCommand),
        _ => None,
    }
}

/// The selected features; MCP is required, so it comes with any other.
fn selected(want: &Selection) -> BTreeSet<Feature> {
    let mut features = want.features(AgentId::ClaudeCode);
    if !features.is_empty() {
        features.insert(Feature::Mcp);
    }
    features
}

impl Agent for ClaudeCode {
    fn id(&self) -> AgentId {
        AgentId::ClaudeCode
    }

    fn detect(&self, env: &Env) -> Detection {
        let program = env.find_program("claude");
        let mut paths: Vec<PathBuf> = program.iter().cloned().collect();
        for dir in [env.home().join(".claude"), env.claude_config_dir()] {
            if dir.is_dir() && !paths.contains(&dir) {
                paths.push(dir);
            }
        }
        if paths.is_empty() {
            tracing::debug!(
                "claude is not on PATH and {} does not exist",
                env.home().join(".claude").display()
            );
            let reason = say::not_installed(AgentId::ClaudeCode.label());
            return Detection::NotFound { reason };
        }
        let version = program.and_then(|p| version(env, &p));
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
        let paths = Paths::of(env);
        let selected = selected(want);
        let mut chain = FileChain::new();
        for feature in FEATURES {
            if selected.contains(&feature) {
                let proposal = self.plan_feature(&mut chain, &paths, feature)?;
                propose(&mut chain, rec, proposal)?;
                continue;
            }
            let entries = rec.map_or(&[][..], |r| r.managed.as_slice());
            for entry in entries
                .iter()
                .filter(|e| feature_of(&e.target) == Some(feature))
            {
                if let Some(change) = removal(&mut chain, entry)? {
                    chain.push(change);
                }
            }
        }
        Ok(chain.into_changes())
    }

    fn check(&self, env: &Env, rec: &AgentRecord) -> CheckReport {
        verify::check(self, env, rec)
    }

    fn plan_remove(&self, _env: &Env, rec: &AgentRecord) -> Vec<Change> {
        let mut chain = FileChain::new();
        for entry in &rec.managed {
            // A file that no longer parses is skipped; its entry stays in
            // the record and drift reports it.
            if let Ok(Some(change)) = removal(&mut chain, entry) {
                chain.push(change);
            }
        }
        chain.into_changes()
    }
}

/// `claude --version` → `2.1.302`; `None` when it fails.
fn version(env: &Env, program: &Path) -> Option<String> {
    let out = env.run(&Invocation::new(program, &["--version"])).ok()?;
    if !out.success() {
        return None;
    }
    out.stdout.split_whitespace().next().map(str::to_owned)
}

#[cfg(test)]
#[path = "claude_code_tests.rs"]
mod tests;
