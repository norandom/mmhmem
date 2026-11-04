//! The static table of artefacts the old installers left behind (task 2.6),
//! with the paths they wrote. Every path below was taken from
//! `deploy/install_local.ps1`, `deploy/install_local.sh`,
//! `deploy/manage_hooks.{ps1,py}` and `deploy/manage_routing.{ps1,py}`; the
//! line references are in the doc comment of each function.
//!
//! Nothing here touches the disk except [`desktop_profiles`], which lists
//! folders to find the Claude Desktop profiles.

use std::path::{Path, PathBuf};

use super::{ArtefactKind, ScanEnv};
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
    let settings_dir = env.claude_config_dir.clone().unwrap_or_else(|| dir.clone());
    vec![
        json_entry(
            id,
            join(&env.home, &[".claude.json"]),
            &["mcpServers", "mmhmem"],
            &["command", "args"],
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
/// in `~/.codex/config.toml` (ps1:314-316,354-358, sh:451-455); skills
/// `~/.agents/skills/unified-memory` and `~/.agents/skills/mmhmem-recap`
/// (ps1:608-609, sh:819-820); prompt `~/.codex/prompts/mmhmem-recap.md`
/// (ps1:610, sh:821); hook in `<CODEX_HOME or ~/.codex>/hooks.json`
/// (`manage_hooks.ps1:225,533`).
fn codex(env: &ScanEnv) -> Vec<Spec> {
    let id = AgentId::Codex;
    let dir = join(&env.home, &[".codex"]);
    let hooks_dir = env.codex_home.clone().unwrap_or_else(|| dir.clone());
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
        hook(id, join(&hooks_dir, &["hooks.json"]), HookHarness::Codex),
    ]
}

/// OpenCode: `mcp.servers.mmhmem` in `~/.config/opencode/opencode.json`
/// (ps1:288,560, sh:434,767); `opencode.jsonc` is also scanned because
/// OpenCode loads both (research 8); skill (ps1:563, sh:771); command in
/// `command/` (ps1:564, sh:772) and, for current OpenCode, `commands/`;
/// routing section in `~/.config/opencode/AGENTS.md`
/// (`routing_onboarding.ps1:33`); plugin
/// `<XDG_CONFIG_HOME or ~/.config>/opencode/plugins/mmhmem-read-routing.js`
/// (`manage_hooks.ps1:476-478`).
fn opencode(env: &ScanEnv) -> Vec<Spec> {
    let id = AgentId::OpenCode;
    let dir = join(&env.home, &[".config", "opencode"]);
    let plugin_dir = env
        .xdg_config_home
        .as_ref()
        .map_or_else(|| dir.clone(), |x| x.join("opencode"));
    let mcp = ["mcp", "servers", "mmhmem"];
    let mut plugin = file(
        id,
        ArtefactKind::ReadPlugin,
        join(&plugin_dir, &["plugins", "mmhmem-read-routing.js"]),
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
    for (config_dir, skill_dir) in desktop_profiles(env) {
        specs.push(json_entry(
            id,
            config_dir.join("claude_desktop_config.json"),
            &["mcpServers", "mmhmem"],
            &["command", "args"],
        ));
        specs.push(file(
            id,
            ArtefactKind::Skill,
            join(&skill_dir, &["unified-memory", "SKILL.md"]),
        ));
    }
    specs
}

/// The Claude Desktop folders that exist, as (config folder, skills folder).
///
/// Windows (ps1:290-303): `%APPDATA%\Claude` and every MSIX profile
/// `%LOCALAPPDATA%\Packages\Claude_*\LocalCache\Roaming\Claude`; skills go in
/// `<config>\skills`. Other systems (sh:438-441, 786-790):
/// `~/Library/Application Support/Claude` (skills in `~/Library/Claude/skills`)
/// and `~/.config/Claude` (skills in `~/.config/Claude/skills`).
fn desktop_profiles(env: &ScanEnv) -> Vec<(PathBuf, PathBuf)> {
    let mut found = Vec::new();
    if env.os == Os::Windows {
        let mut dirs = msix_profiles(env);
        dirs.extend(env.appdata.iter().map(|a| a.join("Claude")));
        for dir in dirs.into_iter().filter(|d| d.is_dir()) {
            found.push((dir.clone(), dir.join("skills")));
        }
    } else {
        let mac = join(&env.home, &["Library", "Application Support", "Claude"]);
        let linux = join(&env.home, &[".config", "Claude"]);
        if mac.is_dir() {
            found.push((mac, join(&env.home, &["Library", "Claude", "skills"])));
        }
        if linux.is_dir() {
            found.push((linux.clone(), linux.join("skills")));
        }
    }
    found
}

fn msix_profiles(env: &ScanEnv) -> Vec<PathBuf> {
    let Some(local) = &env.local_app_data else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(local.join("Packages")) else {
        return Vec::new();
    };
    let mut dirs: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .filter(|e| e.file_name().to_string_lossy().starts_with("Claude_"))
        .map(|e| join(&e.path(), &["LocalCache", "Roaming", "Claude"]))
        .collect();
    dirs.sort();
    dirs
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
