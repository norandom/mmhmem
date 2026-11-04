//! OpenCode (reliable-installers task 3.5, `.kiro/specs/reliable-installers/
//! design.md` §5 and `research.md` §8; Requirements 3.1, 3.4 and 7.3).
//!
//! Plans, as whole-file bytes chained per file:
//!
//! - the MCP entry `mcp.servers.mmhmem` (`type: "local"`, `command: [bin,
//!   "stdio"]`), edited leaf key by leaf key, so keys the user adds inside
//!   the entry (`enabled`, `environment`) stay. **File choice**: OpenCode
//!   loads both `opencode.json` and `opencode.jsonc`, so the file that
//!   already holds an mmhmem entry is edited; with none, `opencode.jsonc`
//!   when it exists, else `opencode.json`. An entry in both is a
//!   [`AgentProblem::DuplicateEntry`] (the user keeps one).
//! - the skill, the recap command (in `commands/`; an existing copy in the
//!   old installer's `command/` folder is kept and tracked instead of being
//!   duplicated), the `AGENTS.md` routing section (the byte-preserving
//!   format of the old `manage_routing.py`) and the optional read-routing
//!   plugin (never touched when it lacks mmhmem's header line).
//!
//! `check` is in `opencode_check.rs` (task 4.2).

use std::io;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::{
    Agent, AgentId, AgentProblem, AgentRecord, CheckReport, Detection, Env, Feature, FileChain,
    Invocation, Selection, section_left,
};

#[path = "opencode_check.rs"]
mod verify;
use crate::setup::assets;
use crate::setup::edit::json as jedit;
use crate::setup::edit::markdown::{self, routing};
use crate::setup::plan::{Action, Change, target_file};
use crate::setup::record::{ManagedEntry, Scope, Target, key_path, observe_in};
use crate::setup::ui::text::changes::{self as say, What};

const SCOPE: Scope = Scope::Agent(AgentId::OpenCode);
const MCP: [&str; 3] = ["mcp", "servers", "mmhmem"];
const MANAGED: [&str; 2] = ["type", "command"];
const PLUGIN_HEADER: &str = "// mmhmem read-routing bridge:";
const BINARY_PLACEHOLDER: &str = "__MMHMEM_BINARY__";
const FEATURES: &[Feature] = &[
    Feature::Mcp,
    Feature::Skill,
    Feature::RecapCommand,
    Feature::Instructions,
    Feature::ReadHook,
];

/// The OpenCode integration.
#[derive(Debug, Clone)]
pub struct OpenCode {
    /// The installed mmhmem binary the MCP entry and the plugin run.
    pub binary: PathBuf,
}

impl OpenCode {
    /// The OpenCode integration for the installed `binary`.
    pub fn new(binary: impl Into<PathBuf>) -> Self {
        Self {
            binary: binary.into(),
        }
    }

    fn config_dir(env: &Env) -> PathBuf {
        env.config_home().join("opencode")
    }
}

fn bad_text(file: &Path) -> AgentProblem {
    AgentProblem::Unreadable {
        file: file.to_path_buf(),
        kind: io::ErrorKind::InvalidData,
    }
}

fn json_problem(file: &Path) -> impl Fn(jedit::JsonEditError) -> AgentProblem + '_ {
    |error| AgentProblem::Json {
        file: file.to_path_buf(),
        error,
    }
}

/// The feature a recorded entry belongs to.
fn feature_of(target: &Target) -> Option<Feature> {
    match target {
        Target::JsonEntry { .. } => Some(Feature::Mcp),
        Target::MarkdownSection { .. } => Some(Feature::Instructions),
        Target::File { file } => match file.file_name()?.to_str()? {
            "SKILL.md" => Some(Feature::Skill),
            "mmhmem-recap.md" => Some(Feature::RecapCommand),
            "mmhmem-read-routing.js" => Some(Feature::ReadHook),
            _ => None,
        },
        _ => None,
    }
}

fn is_plugin(bytes: &[u8]) -> bool {
    bytes.starts_with(PLUGIN_HEADER.as_bytes())
}

/// One entry to add, update or keep.
struct Draft {
    target: Target,
    before: Option<Vec<u8>>,
    after: Vec<u8>,
    missing_parents: Option<u32>,
    what: What,
}

struct Planner<'a> {
    rec: Option<&'a AgentRecord>,
    dir: PathBuf,
    chain: FileChain,
}

impl<'a> Planner<'a> {
    fn new(env: &Env, rec: Option<&'a AgentRecord>) -> Self {
        Self {
            rec,
            dir: OpenCode::config_dir(env),
            chain: FileChain::new(),
        }
    }

    fn recorded(&self, target: &Target) -> Option<&'a ManagedEntry> {
        self.rec?.managed.iter().find(|e| &e.target == target)
    }

    /// Adds the change for `draft`: nothing when the record already holds an
    /// identical entry, "keep and track" (before == after) when an identical
    /// entry exists but is not recorded yet (adoption).
    fn put(&mut self, draft: Draft) {
        let Draft {
            target,
            before,
            after,
            missing_parents,
            what,
        } = draft;
        let same = before
            .as_ref()
            .is_some_and(|b| observe_in(&target, b) == observe_in(&target, &after));
        if same && self.recorded(&target).is_some() {
            return;
        }
        let file = target_file(&target)
            .expect("an OpenCode entry has a file")
            .to_path_buf();
        let label = AgentId::OpenCode.label();
        let (action, after, summary) = match (&before, same) {
            (None, _) => (Action::Add, after, say::add(what, label, &file)),
            (Some(b), true) => (Action::Update, b.clone(), say::keep(what, label, &file)),
            (Some(_), false) => (Action::Update, after, say::update(what, label, &file)),
        };
        self.chain.push(Change {
            scope: SCOPE,
            target,
            file,
            action,
            before,
            after: Some(after),
            owner_only: false,
            hide_diff: false,
            missing_parents,
            summary,
        });
    }

    /// The config file for the MCP entry (see the module comment).
    fn mcp_file(&mut self) -> Result<PathBuf, AgentProblem> {
        let json = self.dir.join("opencode.json");
        let jsonc = self.dir.join("opencode.jsonc");
        let mut holders = Vec::new();
        let mut existing = Vec::new();
        for file in [&json, &jsonc] {
            let Some(text) = self.chain.text(file)? else {
                continue;
            };
            existing.push(file.clone());
            let entry = jedit::get(&text, &MCP).map_err(json_problem(file))?;
            if entry.is_some() {
                holders.push(file.clone());
            }
        }
        if holders.len() > 1 {
            return Err(AgentProblem::DuplicateEntry { files: holders });
        }
        Ok(holders.pop().unwrap_or_else(|| {
            if existing.contains(&jsonc) {
                jsonc
            } else {
                json
            }
        }))
    }

    fn plan_mcp(&mut self, binary: &Path) -> Result<(), AgentProblem> {
        let file = self.mcp_file()?;
        let before = self.chain.bytes(&file)?;
        let text = match &before {
            Some(bytes) => String::from_utf8(bytes.clone()).map_err(|_| bad_text(&file))?,
            None => "{}\n".to_owned(),
        };
        let target = Target::JsonEntry {
            file: file.clone(),
            pointer: key_path(&MCP).expect("static keys"),
            managed_keys: Some(key_path(&MANAGED).expect("static keys")),
        };
        let leaf = [MCP[0], MCP[1], MCP[2], MANAGED[0]];
        let missing = match self.recorded(&target).and_then(|e| e.missing_parents) {
            Some(n) => n,
            None => jedit::missing_parents(&text, &leaf).map_err(json_problem(&file))? as u32,
        };
        let values = [json!("local"), json!([binary.to_string_lossy(), "stdio"])];
        let mut after = text;
        for (key, value) in MANAGED.iter().zip(&values) {
            let path = [MCP[0], MCP[1], MCP[2], key];
            after = jedit::set(&after, &path, value).map_err(json_problem(&file))?;
        }
        self.put(Draft {
            target,
            before,
            after: after.into_bytes(),
            missing_parents: Some(missing),
            what: What::Server,
        });
        Ok(())
    }

    fn plan_file(&mut self, file: PathBuf, bytes: Vec<u8>, what: What) -> Result<(), AgentProblem> {
        let before = self.chain.bytes(&file)?;
        self.put(Draft {
            target: Target::File { file },
            before,
            after: bytes,
            missing_parents: None,
            what,
        });
        Ok(())
    }

    /// `commands/` unless the old installer's `command/` copy exists (or is
    /// the one recorded): that one is kept and tracked, not duplicated.
    fn recap_file(&self) -> PathBuf {
        let new = self.dir.join("commands").join("mmhmem-recap.md");
        let old = self.dir.join("command").join("mmhmem-recap.md");
        let held =
            |p: &PathBuf| p.exists() || self.recorded(&Target::File { file: p.clone() }).is_some();
        if held(&new) || !held(&old) { new } else { old }
    }

    fn plan_plugin(&mut self, binary: &Path) -> Result<(), AgentProblem> {
        let file = self.dir.join("plugins").join("mmhmem-read-routing.js");
        let recorded = self.recorded(&Target::File { file: file.clone() });
        if !self.chain.bytes(&file)?.is_none_or(|b| is_plugin(&b)) && recorded.is_none() {
            return Ok(());
        }
        let quoted = Value::String(binary.to_string_lossy().into_owned()).to_string();
        let template = assets::OPENCODE_READ_PLUGIN.text;
        let text = template.replace(BINARY_PLACEHOLDER, &quoted);
        self.plan_file(file, text.into_bytes(), What::ReadPlugin)
    }

    fn plan_section(&mut self) -> Result<(), AgentProblem> {
        let file = self.dir.join("AGENTS.md");
        let before = self.chain.bytes(&file)?;
        let text = match &before {
            Some(b) => std::str::from_utf8(b).map_err(|_| bad_text(&file))?,
            None => "",
        };
        let canonical = assets::ROUTING_INSTRUCTIONS.text;
        let after = markdown::install_section(text, &routing(), canonical)
            .map_err(AgentProblem::markdown(&file))?;
        self.put(Draft {
            target: Target::MarkdownSection {
                file,
                section: routing(),
            },
            before,
            after: after.into_bytes(),
            missing_parents: None,
            what: What::Instructions,
        });
        Ok(())
    }

    /// Takes out one recorded entry, never anything else.
    fn remove(&mut self, entry: &ManagedEntry) -> Result<(), AgentProblem> {
        let Some(file) = target_file(&entry.target) else {
            return Ok(());
        };
        let Some(before) = self.chain.bytes(file)? else {
            return Ok(());
        };
        let text = std::str::from_utf8(&before).map_err(|_| bad_text(file))?;
        let after = match &entry.target {
            Target::JsonEntry { .. } => Some(remove_entry(text, entry, file)?.into_bytes()),
            Target::MarkdownSection { .. } => section_left(
                markdown::remove_section(text, &routing()).map_err(AgentProblem::markdown(file))?,
                entry,
            ),
            Target::File { .. } if feature_of(&entry.target) == Some(Feature::ReadHook) => {
                if !is_plugin(&before) {
                    return Ok(());
                }
                None
            }
            Target::File { .. } => None,
            _ => return Ok(()),
        };
        if after.as_ref() == Some(&before) {
            return Ok(());
        }
        self.chain.push(Change {
            scope: SCOPE,
            target: entry.target.clone(),
            file: file.to_path_buf(),
            action: Action::Remove,
            before: Some(before),
            after,
            owner_only: false,
            hide_diff: false,
            missing_parents: entry.missing_parents,
            summary: say::remove(AgentId::OpenCode.label(), file),
        });
        Ok(())
    }
}

/// Removes the leaf keys mmhmem manages, then the parents it created.
fn remove_entry(text: &str, entry: &ManagedEntry, file: &Path) -> Result<String, AgentProblem> {
    let Target::JsonEntry {
        pointer,
        managed_keys,
        ..
    } = &entry.target
    else {
        return Ok(text.to_owned());
    };
    let base: Vec<&str> = pointer.iter().map(|k| k.as_str()).collect();
    let keys = managed_keys.iter().flatten().map(|k| k.as_str());
    let mut paths: Vec<Vec<&str>> = keys.map(|k| [base.clone(), vec![k]].concat()).collect();
    if paths.is_empty() {
        paths.push(base);
    }
    let max = entry.missing_parents.unwrap_or(0) as usize;
    let mut out = text.to_owned();
    for path in paths {
        out = jedit::remove_and_prune(&out, &path, max).map_err(json_problem(file))?;
    }
    Ok(out)
}

impl Agent for OpenCode {
    fn id(&self) -> AgentId {
        AgentId::OpenCode
    }

    fn detect(&self, env: &Env) -> Detection {
        let dir = Self::config_dir(env);
        let program = env.find_program("opencode");
        let mut paths: Vec<PathBuf> = program.iter().cloned().collect();
        if dir.is_dir() {
            paths.push(dir.clone());
        }
        if paths.is_empty() {
            tracing::debug!(
                "opencode is not on PATH and {} does not exist",
                dir.display()
            );
            return Detection::NotFound {
                reason: say::not_installed(AgentId::OpenCode.label()),
            };
        }
        let version = program
            .and_then(|p| env.run(&Invocation::new(p, &["--version"])).ok())
            .filter(|out| out.success())
            .and_then(|out| out.stdout.lines().next().map(|l| l.trim().to_owned()))
            .filter(|v| !v.is_empty());
        Detection::Found { version, paths }
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
        let mut p = Planner::new(env, rec);
        let on = |f| want.wants(AgentId::OpenCode, f);
        if on(Feature::Mcp) {
            p.plan_mcp(&self.binary)?;
        }
        if on(Feature::Skill) {
            let file = p.dir.join("skills").join("unified-memory").join("SKILL.md");
            let bytes = assets::UNIFIED_MEMORY_SKILL.bytes().to_vec();
            p.plan_file(file, bytes, What::Guide)?;
        }
        if on(Feature::RecapCommand) {
            let file = p.recap_file();
            let bytes = assets::OPENCODE_RECAP_COMMAND.bytes().to_vec();
            p.plan_file(file, bytes, What::Recap)?;
        }
        if on(Feature::Instructions) {
            p.plan_section()?;
        }
        if on(Feature::ReadHook) {
            p.plan_plugin(&self.binary)?;
        }
        for entry in rec.into_iter().flat_map(|r| &r.managed) {
            if feature_of(&entry.target).is_some_and(|f| !on(f)) {
                p.remove(entry)?;
            }
        }
        Ok(p.chain.into_changes())
    }

    fn check(&self, env: &Env, rec: &AgentRecord) -> CheckReport {
        verify::check(self, env, rec)
    }

    fn plan_remove(&self, env: &Env, rec: &AgentRecord) -> Vec<Change> {
        let mut p = Planner::new(env, Some(rec));
        for entry in &rec.managed {
            // An entry that cannot be read or parsed is left alone.
            let _ = p.remove(entry);
        }
        p.chain.into_changes()
    }
}

#[cfg(test)]
#[path = "opencode_tests.rs"]
mod tests;
