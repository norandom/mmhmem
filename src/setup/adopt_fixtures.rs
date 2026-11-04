//! Temporary homes that look like the output of the old installers (task
//! 2.6): the PowerShell installer on Windows (plain and MSIX Claude Desktop
//! profile) and the Bash installer on Linux. File shapes follow what the
//! scripts write: `claude mcp add` output in `~/.claude.json`, the
//! `# BEGIN mmhmem` block (`install_local.ps1:314`, `install_local.sh:451`),
//! `Merge-Json` / `json.dump` output, `New-RoutingPlan` sections and the
//! hook entries of `manage_hooks.ps1` (`Get-Entry`, `Get-CodexCommand`).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::ScanEnv;
use crate::setup::record::{Os, Sha256Hex};

pub(super) const MARKER_END: &str = "# END mmhmem";
pub(super) const MARKER_PS1: &str = "# BEGIN mmhmem (managed by install_local.ps1 -- do not edit)";
pub(super) const MARKER_SH: &str = "# BEGIN mmhmem (managed by install_local.sh -- do not edit)";
pub(super) const SKILL: &str = "---\nname: unified-memory\n---\nUse mmhmem.\n";
pub(super) const RECAP: &str = "Recover the latest mmhmem checkpoint.\n";
pub(super) const ROUTING_BODY: &str = "## mmhmem\nUse smart_read for large files.";
const PLUGIN_HEAD: &str =
    "// mmhmem read-routing bridge: installed only after explicit user opt-in.\n";

/// A seeded home: the temporary folder, the scan environment and key paths.
pub(super) struct Seeded {
    _tmp: tempfile::TempDir,
    pub(super) root: PathBuf,
    pub(super) env: ScanEnv,
    pub(super) bin: PathBuf,
    pub(super) desktop_dir: PathBuf,
}

impl Seeded {
    pub(super) fn home(&self) -> &Path {
        &self.env.home
    }

    pub(super) fn codex_config(&self) -> PathBuf {
        self.home().join(".codex").join("config.toml")
    }

    pub(super) fn opencode_config(&self) -> PathBuf {
        self.home()
            .join(".config")
            .join("opencode")
            .join("opencode.json")
    }
}

pub(super) fn write(path: &Path, content: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, content).unwrap();
}

fn json_text(value: &Value, python: bool) -> String {
    let mut text = serde_json::to_string_pretty(value).unwrap();
    if python {
        text.push('\n');
    }
    text
}

pub(super) fn routing_block(prefix: &str) -> String {
    format!(
        "<!-- mmhmem:routing:start -->\n<!-- mmhmem:routing:prefix={prefix} -->\n\
         {ROUTING_BODY}\n<!-- mmhmem:routing:end -->\n"
    )
}

pub(super) fn codex_block(begin: &str, bin: &Path) -> String {
    let esc = bin.to_string_lossy().replace('\\', "\\\\");
    format!(
        "{begin}\n[mcp_servers.mmhmem]\ncommand = \"{esc}\"\nargs = [\"stdio\"]\n{MARKER_END}\n"
    )
}

pub(super) fn codex_config_text(begin: &str, bin: &Path) -> String {
    format!(
        "model = \"o3\"\n\n[projects.'/work']\ntrust_level = \"trusted\"\n\n{}",
        codex_block(begin, bin)
    )
}

struct Layout<'a> {
    home: &'a Path,
    bin: &'a Path,
    desktop_dir: &'a Path,
    desktop_skills: &'a Path,
    marker: &'a str,
    python: bool,
}

/// Everything both installers write for a global install of all agents plus
/// the opt-in hooks.
fn seed(l: &Layout<'_>) {
    let bin = l.bin.to_string_lossy().into_owned();
    let (home, py) = (l.home, l.python);
    let stdio = json!({"command": bin, "args": ["stdio"]});
    // Claude Code: `claude mcp add --scope user`, skill, command, rules, hook.
    let entry = json!({"type": "stdio", "command": bin, "args": ["stdio"], "env": {}});
    let claude = json!({"numStartups": 7, "mcpServers": {"mmhmem": entry}, "projects": {}});
    write(&home.join(".claude.json"), &json_text(&claude, py));
    write(&home.join(".claude/skills/unified-memory/SKILL.md"), SKILL);
    write(&home.join(".claude/commands/mmhmem-recap.md"), RECAP);
    write(
        &home.join(".claude/rules/mmhmem.md"),
        &routing_block("none"),
    );
    let own = json!({"matcher": "Read|Bash|PowerShell", "hooks": [
        {"type": "command", "command": bin, "args": ["hook", "claude"], "timeout": 10}]});
    let theirs = json!({"matcher": "Bash", "hooks": [{"type": "command", "command": "audit.sh"}]});
    let settings = json!({"theme": "dark", "hooks": {"PreToolUse": [theirs, own]}});
    write(
        &home.join(".claude/settings.json"),
        &json_text(&settings, py),
    );
    // Claude Desktop.
    let desktop = json!({"globalShortcut": "Ctrl+Space",
        "mcpServers": {"other": {"command": "npx"}, "mmhmem": stdio}});
    write(
        &l.desktop_dir.join("claude_desktop_config.json"),
        &json_text(&desktop, py),
    );
    write(&l.desktop_skills.join("unified-memory/SKILL.md"), SKILL);
    // Codex: marker block, skills, prompt, hook.
    write(
        &home.join(".codex/config.toml"),
        &codex_config_text(l.marker, l.bin),
    );
    write(&home.join(".agents/skills/unified-memory/SKILL.md"), SKILL);
    write(&home.join(".agents/skills/mmhmem-recap/SKILL.md"), RECAP);
    write(&home.join(".codex/prompts/mmhmem-recap.md"), RECAP);
    let command = format!("& '{bin}' hook codex");
    let codex_hook = json!({"matcher": "^(Bash|shell)$", "hooks": [
        {"type": "command", "command": command, "timeout": 10}]});
    let hooks = json!({"hooks": {"PreToolUse": [codex_hook]}});
    write(&home.join(".codex/hooks.json"), &json_text(&hooks, py));
    // OpenCode.
    let oc = home.join(".config/opencode");
    let server = json!({"type": "local", "command": [bin, "stdio"]});
    let config = json!({"theme": "tokyonight", "mcp": {"servers": {"mmhmem": server}}});
    write(&oc.join("opencode.json"), &json_text(&config, py));
    write(&oc.join("skills/unified-memory/SKILL.md"), SKILL);
    write(&oc.join("command/mmhmem-recap.md"), RECAP);
    let agents = format!("# My rules\nBe terse.\n{}", routing_block("lf"));
    write(&oc.join("AGENTS.md"), &agents);
    write(
        &oc.join("plugins/mmhmem-read-routing.js"),
        &format!("{PLUGIN_HEAD}const binary = {};\n", json!(bin)),
    );
}

fn tmp_root() -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().to_path_buf();
    (tmp, root)
}

/// The home the PowerShell installer leaves (Windows rules). With `msix` the
/// Claude Desktop profile is the MSIX package folder, else `%APPDATA%\Claude`.
pub(super) fn powershell(msix: bool) -> Seeded {
    let (tmp, root) = tmp_root();
    let (home, appdata, local) = (root.join("home"), root.join("appdata"), root.join("local"));
    let bin_dir = local.join("Programs").join("mmhmem");
    let bin = bin_dir.join("mmhmem.exe");
    let desktop_dir = if msix {
        local.join("Packages/Claude_pzs8sxrjxfjjc/LocalCache/Roaming/Claude")
    } else {
        appdata.join("Claude")
    };
    std::fs::create_dir_all(&desktop_dir).unwrap();
    write(&bin, "MZ fake binary");
    seed(&Layout {
        home: &home,
        bin: &bin,
        desktop_dir: &desktop_dir,
        desktop_skills: &desktop_dir.join("skills"),
        marker: MARKER_PS1,
        python: false,
    });
    write(&bin_dir.join("manage_hooks.ps1"), "# helper copy\n");
    write(
        &bin_dir.join("opencode/mmhmem-read-routing.js"),
        "// template\n",
    );
    write(
        &home.join(".codex/config.toml.mmhmem.bak"),
        "model = \"o3\"\n",
    );
    let mut env = ScanEnv::new(Os::Windows, &home);
    env.appdata = Some(appdata);
    env.local_app_data = Some(local);
    env.user_path = Some(vec![PathBuf::from("C:\\Windows"), bin_dir]);
    Seeded {
        _tmp: tmp,
        root,
        env,
        bin,
        desktop_dir,
    }
}

/// The home the Bash installer leaves (Linux rules).
pub(super) fn bash() -> Seeded {
    let (tmp, root) = tmp_root();
    let home = root.join("home");
    let bin_dir = home.join(".local/bin");
    let bin = bin_dir.join("mmhmem");
    write(&bin, "ELF fake binary");
    let desktop_dir = home.join(".config/Claude");
    seed(&Layout {
        home: &home,
        bin: &bin,
        desktop_dir: &desktop_dir,
        desktop_skills: &desktop_dir.join("skills"),
        marker: MARKER_SH,
        python: true,
    });
    write(&bin_dir.join("mmhmem-manage-hooks.py"), "# helper copy\n");
    write(&bin_dir.join("mmhmem.old"), "ELF older binary");
    let mut env = ScanEnv::new(Os::Linux, &home);
    env.user_path = Some(vec![PathBuf::from("/usr/bin"), bin_dir]);
    Seeded {
        _tmp: tmp,
        root,
        env,
        bin,
        desktop_dir,
    }
}

/// An empty home (the folders exist, nothing is installed).
pub(super) fn empty(os: Os) -> Seeded {
    let (tmp, root) = tmp_root();
    let home = root.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let env = ScanEnv::new(os, &home);
    Seeded {
        _tmp: tmp,
        root: root.clone(),
        env,
        bin: root.join("none"),
        desktop_dir: root.join("none"),
    }
}

/// Every file under `root` with its bytes.
pub(super) fn snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn walk(dir: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(&path, out);
            } else {
                out.insert(path.clone(), std::fs::read(&path).unwrap());
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, &mut out);
    out
}

/// One hash over every path and byte under `root`.
pub(super) fn tree_hash(root: &Path) -> Sha256Hex {
    let mut all = Vec::new();
    for (path, bytes) in snapshot(root) {
        all.extend_from_slice(path.to_string_lossy().as_bytes());
        all.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
        all.extend_from_slice(&bytes);
    }
    Sha256Hex::of(&all)
}
