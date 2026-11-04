//! The static table of artefacts the old installers left behind (task 2.6),
//! with the paths they wrote. Every path below was taken from
//! `deploy/install_local.ps1`, `deploy/install_local.sh`,
//! `deploy/manage_hooks.{ps1,py}` and `deploy/manage_routing.{ps1,py}`; the
//! line references are in the doc comment of each function.
//!
//! The folders follow the same overrides the agents use (`CLAUDE_CONFIG_DIR`,
//! `CODEX_HOME`, `XDG_CONFIG_HOME`; see [`ScanEnv`]), and the managed keys of
//! each entry are the agent's, so an adopted entry and a planned one are the
//! same record [`Target`]. Nothing here touches the disk except the Claude
//! Desktop profile discovery shared with the agent
//! ([`claude_desktop::profiles`]), which lists folders.

use std::path::{Path, PathBuf};

use super::{ArtefactKind, ScanEnv};
use crate::setup::agents::claude_desktop;
use crate::setup::record::{
    AgentId, ElementMatcher, HookHarness, KeySegment, Os, SectionId, Target, key_path,
};

/// Marker the old OpenCode plugin file starts with (`manage_hooks.ps1:486`,
/// `manage_hooks.py:973`).
pub(super) const PLUGIN_SIGNATURE: &[u8] = b"// mmhmem read-routing bridge:";

/// One place an old installer may have written to.
pub(super) struct Spec {
    pub(super) agent: AgentId,
    pub(super) kind: ArtefactKind,
    pub(super) target: Target,
    /// A file that must start with these bytes to count as mmhmem's.
    pub(super) signature: Option<&'static [u8]>,
}

fn join(base: &Path, parts: &[&str]) -> PathBuf {
    parts.iter().fold(base.to_path_buf(), |p, s| p.join(s))
}

fn keys(parts: &[&str]) -> Vec<KeySegment> {
    key_path(parts).expect("static key names are valid")
}

fn spec(agent: AgentId, kind: ArtefactKind, target: Target) -> Spec {
    Spec {
        agent,
        kind,
        target,
        signature: None,
    }
}

fn file(agent: AgentId, kind: ArtefactKind, path: PathBuf) -> Spec {
    spec(agent, kind, Target::File { file: path })
}

fn json_entry(agent: AgentId, file: PathBuf, pointer: &[&str], managed: &[&str]) -> Spec {
    spec(
        agent,
        ArtefactKind::McpEntry,
        Target::JsonEntry {
            file,
            pointer: keys(pointer),
            managed_keys: Some(keys(managed)),
        },
    )
}

fn routing(agent: AgentId, file: PathBuf) -> Spec {
    let section = SectionId::try_from("routing".to_owned()).expect("static section id");
    spec(
        agent,
        ArtefactKind::RoutingSection,
        Target::MarkdownSection { file, section },
    )
}

fn hook(agent: AgentId, file: PathBuf, harness: HookHarness) -> Spec {
    spec(
        agent,
        ArtefactKind::ReadHook,
        Target::JsonArrayElement {
            file,
            pointer: keys(&["hooks", "PreToolUse"]),
            matcher: ElementMatcher::MmhmemHook(harness),
        },
    )
}

/// Claude Code: `claude mcp add --scope user` writes `mcpServers.mmhmem` into
/// `~/.claude.json` (ps1:532-534, sh:735-737); skill `~/.claude/skills/unified-memory`
/// (ps1:547, sh:724); command `~/.claude/commands/mmhmem-recap.md` (ps1:549,
/// sh:726); routing section in the dedicated `~/.claude/rules/mmhmem.md`
/// (`routing_onboarding.ps1:17`, `routing_onboarding.py:25`; not CLAUDE.md);
/// hook group in `<CLAUDE_CONFIG_DIR or ~/.claude>/settings.json`
/// (`manage_hooks.ps1:227,533`).
fn claude_code(env: &ScanEnv) -> Vec<Spec> {
    let id = AgentId::ClaudeCode;
    let dir = join(&env.home, &[".claude"]);
    let settings_dir = env.claude_config_dir();
    vec![
        json_entry(
            id,
            join(&env.home, &[".claude.json"]),
            &["mcpServers", "mmhmem"],
            &["type", "command", "args"],
        ),
        file(
            id,
            ArtefactKind::Skill,
            join(&dir, &["skills", "unified-memory", "SKILL.md"]),
        ),
        file(
            id,
            ArtefactKind::RecapCommand,
            join(&dir, &["commands", "mmhmem-recap.md"]),
        ),
        routing(id, join(&dir, &["rules", "mmhmem.md"])),
        hook(
            id,
            join(&settings_dir, &["settings.json"]),
            HookHarness::Claude,
        ),
    ]
}

/// Codex: `[mcp_servers.mmhmem]` (plain, or inside the `# BEGIN mmhmem` block)
/// in `config.toml` (ps1:314-316,354-358, sh:451-455); skills
/// `~/.agents/skills/unified-memory` and `~/.agents/skills/mmhmem-recap`
/// (ps1:608-609, sh:819-820); prompt `prompts/mmhmem-recap.md` (ps1:610,
/// sh:821); hook in `hooks.json` (`manage_hooks.ps1:225,533`). The old
/// installers used `~/.codex`; like the agent, `CODEX_HOME` is honoured
/// for all three.
fn codex(env: &ScanEnv) -> Vec<Spec> {
    let id = AgentId::Codex;
    let dir = env.codex_home();
    let skills = join(&env.home, &[".agents", "skills"]);
    let table = Target::TomlTable {
        file: join(&dir, &["config.toml"]),
        key: keys(&["mcp_servers", "mmhmem"]),
        managed_keys: Some(keys(&["command", "args"])),
    };
    vec![
        spec(id, ArtefactKind::McpEntry, table),
        file(
            id,
            ArtefactKind::Skill,
            join(&skills, &["unified-memory", "SKILL.md"]),
        ),
        file(
            id,
            ArtefactKind::Skill,
            join(&skills, &["mmhmem-recap", "SKILL.md"]),
        ),
        file(
            id,
            ArtefactKind::RecapCommand,
            join(&dir, &["prompts", "mmhmem-recap.md"]),
        ),
        hook(id, join(&dir, &["hooks.json"]), HookHarness::Codex),
    ]
}

/// OpenCode, all in `<XDG_CONFIG_HOME or ~/.config>/opencode` like the
/// agent: `mcp.servers.mmhmem` in `opencode.json` (ps1:288,560, sh:434,767);
/// `opencode.jsonc` is also scanned because OpenCode loads both (research
/// 8); skill (ps1:563, sh:771); command in `command/` (ps1:564, sh:772) and,
/// for current OpenCode, `commands/`; routing section in `AGENTS.md`
/// (`routing_onboarding.ps1:33`); plugin `plugins/mmhmem-read-routing.js`
/// (`manage_hooks.ps1:476-478`).
fn opencode(env: &ScanEnv) -> Vec<Spec> {
    let id = AgentId::OpenCode;
    let dir = env.config_home().join("opencode");
    let mcp = ["mcp", "servers", "mmhmem"];
    let mut plugin = file(
        id,
        ArtefactKind::ReadPlugin,
        join(&dir, &["plugins", "mmhmem-read-routing.js"]),
    );
    plugin.signature = Some(PLUGIN_SIGNATURE);
    vec![
        json_entry(id, dir.join("opencode.json"), &mcp, &["type", "command"]),
        json_entry(id, dir.join("opencode.jsonc"), &mcp, &["type", "command"]),
        file(
            id,
            ArtefactKind::Skill,
            join(&dir, &["skills", "unified-memory", "SKILL.md"]),
        ),
        file(
            id,
            ArtefactKind::RecapCommand,
            join(&dir, &["command", "mmhmem-recap.md"]),
        ),
        file(
            id,
            ArtefactKind::RecapCommand,
            join(&dir, &["commands", "mmhmem-recap.md"]),
        ),
        routing(id, dir.join("AGENTS.md")),
        plugin,
    ]
}

/// Claude Desktop: `mcpServers.mmhmem` in `claude_desktop_config.json`
/// (ps1:577, sh:785) and the skill beside it (ps1:580, sh:789-790).
fn claude_desktop(env: &ScanEnv) -> Vec<Spec> {
    let id = AgentId::ClaudeDesktop;
    let mut specs = Vec::new();
    for profile in claude_desktop::profiles(env) {
        specs.push(json_entry(
            id,
            profile.config_dir.join(claude_desktop::CONFIG_FILE),
            &["mcpServers", "mmhmem"],
            &["command", "args"],
        ));
        specs.push(file(
            id,
            ArtefactKind::Skill,
            join(&profile.skill_dir, &["unified-memory", "SKILL.md"]),
        ));
    }
    specs
}

/// Every place an old installer may have written, in agent order.
pub(super) fn specs(env: &ScanEnv) -> Vec<Spec> {
    let mut all = claude_code(env);
    all.extend(claude_desktop(env));
    all.extend(codex(env));
    all.extend(opencode(env));
    all
}

/// The folder the old installers copied the binary into
/// (ps1:94 `%LOCALAPPDATA%\Programs\mmhmem`, else `~/.local/bin`; sh:138).
pub(super) fn bin_dir(env: &ScanEnv) -> PathBuf {
    match (&env.os, &env.local_app_data) {
        (Os::Windows, Some(local)) => join(local, &["Programs", "mmhmem"]),
        _ => join(&env.home, &[".local", "bin"]),
    }
}

/// The installed binary (ps1:92-95, sh:288).
pub(super) fn binary(env: &ScanEnv) -> PathBuf {
    let name = if env.os == Os::Windows {
        "mmhmem.exe"
    } else {
        "mmhmem"
    };
    bin_dir(env).join(name)
}

/// Helper files the old installers left that nothing needs any more: the
/// hook helper copy (ps1:98 `manage_hooks.ps1`, sh:137
/// `mmhmem-manage-hooks.py`), the plugin template copy
/// (`hooks_onboarding.ps1:66-69`), the previous binary (sh:854
/// `mmhmem.old`) and the Codex config backup (ps1:349, sh:492
/// `config.toml.mmhmem.bak`). Only reported, never adopted.
pub(super) fn leftovers(env: &ScanEnv) -> Vec<PathBuf> {
    let bin = bin_dir(env);
    let candidates = [
        bin.join("manage_hooks.ps1"),
        join(&bin, &["opencode", "mmhmem-read-routing.js"]),
        bin.join("mmhmem-manage-hooks.py"),
        bin.join("mmhmem.old"),
        join(&env.home, &[".codex", "config.toml.mmhmem.bak"]),
    ];
    candidates.into_iter().filter(|p| p.is_file()).collect()
}
