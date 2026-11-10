//! Doubles of the `claude`, `codex` and `opencode` CLIs (reliable-installers
//! task 3.1) and of Antigravity's `agy` (antigravity-native-setup task 3.5).
//! They replay the outputs captured in task 1.1 of each spec
//! (`tests/fixtures/setup/agents/`, research.md §8 and "Antigravity CLI
//! surface") and, for `mcp add` and `mcp remove`, edit the agent's file the
//! way the confirmed real CLI does. The text a real `mcp add`/`mcp remove`
//! prints was not captured, so those answer with empty output and exit 0.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{Value, json};

use super::doubles::{FakeRunner, Reply};
use super::{Env, Invocation};

/// `claude mcp get mmhmem`, connected (Claude Code 2.1.302).
pub(crate) const CLAUDE_MCP_GET: &str =
    include_str!("../../../tests/fixtures/setup/agents/claude-code/mcp-get-connected.txt");
/// `claude mcp list`, connected.
pub(crate) const CLAUDE_MCP_LIST: &str =
    include_str!("../../../tests/fixtures/setup/agents/claude-code/mcp-list-connected.txt");
/// `codex mcp get mmhmem` (codex-cli 0.166.0).
pub(crate) const CODEX_MCP_GET: &str =
    include_str!("../../../tests/fixtures/setup/agents/codex/mcp-get.txt");
/// `codex mcp list --json` with the mmhmem entry.
pub(crate) const CODEX_MCP_LIST_JSON: &str =
    include_str!("../../../tests/fixtures/setup/agents/codex/mcp-list-json-mmhmem.json");
/// The first `opencode mcp list` of a new instance (OpenCode 2.2.5).
pub(crate) const OPENCODE_LIST_BEFORE_INIT: &str =
    include_str!("../../../tests/fixtures/setup/agents/opencode/mcp-list-before-init.txt");
/// `opencode mcp list` once the instance is warm.
pub(crate) const OPENCODE_LIST_CONNECTED: &str =
    include_str!("../../../tests/fixtures/setup/agents/opencode/mcp-list-connected.txt");
/// `opencode service status` with the background service stopped.
pub(crate) const OPENCODE_SERVICE_STOPPED: &str =
    include_str!("../../../tests/fixtures/setup/agents/opencode/service-status-stopped.txt");
/// `opencode service status` while the service runs (its address).
pub(crate) const OPENCODE_SERVICE_RUNNING: &str =
    include_str!("../../../tests/fixtures/setup/agents/opencode/service-status-running.txt");

/// `agy --version` (agy 1.5.3).
pub(crate) const AGY_VERSION: &str =
    include_str!("../../../tests/fixtures/setup/agents/antigravity/agy-version.txt");
/// `agy mcp list` with mmhmem added (a space-aligned table).
pub(crate) const AGY_MCP_LIST_MMHMEM: &str =
    include_str!("../../../tests/fixtures/setup/agents/antigravity/agy-mcp-list-mmhmem.txt");
/// `agy mcp list` with no server configured.
pub(crate) const AGY_MCP_LIST_EMPTY: &str =
    include_str!("../../../tests/fixtures/setup/agents/antigravity/agy-mcp-list-empty.txt");

/// `name` and command of `… mcp add [flags] <name> -- <command…>`.
fn added(inv: &Invocation) -> (String, Vec<String>) {
    let split = inv
        .args
        .iter()
        .position(|a| a == "--")
        .expect("mcp add needs `--`");
    assert!(split > 0, "mcp add needs a name before `--`");
    (inv.args[split - 1].clone(), inv.args[split + 1..].to_vec())
}

/// `name` of `… mcp remove <name> [flags]`.
fn removed(inv: &Invocation) -> String {
    inv.args.get(2).cloned().expect("mcp remove needs a name")
}

fn read_json(file: &Path) -> Value {
    std::fs::read_to_string(file).ok().map_or_else(
        || json!({}),
        |t| serde_json::from_str(&t).expect("valid JSON"),
    )
}

fn write_json(file: &Path, value: &Value) {
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent).expect("config folder");
    }
    let text = serde_json::to_string_pretty(value).expect("serialize") + "\n";
    std::fs::write(file, text).expect("write config");
}

/// Sets `root[keys…] = entry`, creating objects on the way (as the CLIs do).
fn put(root: &mut Value, keys: &[&str], name: &str, entry: Value) {
    let mut at = root;
    for key in keys {
        at = at
            .as_object_mut()
            .expect("object")
            .entry(*key)
            .or_insert_with(|| json!({}));
    }
    at.as_object_mut()
        .expect("object")
        .insert(name.to_owned(), entry);
}

/// Scripts `claude` (`--version`, `mcp get`, `mcp list`, `mcp add --scope
/// user`, `mcp remove`) for `env`'s home: `add` and `remove` edit
/// `mcpServers` in `~/.claude.json`.
pub(crate) fn claude(runner: &FakeRunner, env: &Env) {
    runner.on(
        "claude",
        &["--version"],
        vec![Reply::ok("2.1.302 (Claude Code)\n")],
    );
    runner.on(
        "claude",
        &["mcp", "get", "mmhmem"],
        vec![Reply::ok(CLAUDE_MCP_GET)],
    );
    runner.on("claude", &["mcp", "list"], vec![Reply::ok(CLAUDE_MCP_LIST)]);
    let file = env.home().join(".claude.json");
    let add_file = file.clone();
    let add = Arc::new(move |inv: &Invocation| {
        let (name, mut command) = added(inv);
        let program = command.remove(0);
        let mut root = read_json(&add_file);
        let entry = json!({"type": "stdio", "command": program, "args": command, "env": {}});
        put(&mut root, &["mcpServers"], &name, entry);
        write_json(&add_file, &root);
    });
    runner.on_with("claude", &["mcp", "add"], vec![Reply::ok("")], add);
    let remove = Arc::new(move |inv: &Invocation| {
        let mut root = read_json(&file);
        if let Some(servers) = root.get_mut("mcpServers").and_then(Value::as_object_mut) {
            servers.remove(&removed(inv));
        }
        write_json(&file, &root);
    });
    runner.on_with("claude", &["mcp", "remove"], vec![Reply::ok("")], remove);
}

/// Scripts `codex` (`--version`, `mcp get`, `mcp list --json`, `mcp add`,
/// `mcp remove`): `add` and `remove` edit `[mcp_servers.<name>]` in
/// `$CODEX_HOME/config.toml`.
pub(crate) fn codex(runner: &FakeRunner, env: &Env) {
    runner.on(
        "codex",
        &["--version"],
        vec![Reply::ok("codex-cli 0.166.0\n")],
    );
    runner.on(
        "codex",
        &["mcp", "get", "mmhmem"],
        vec![Reply::ok(CODEX_MCP_GET)],
    );
    runner.on(
        "codex",
        &["mcp", "list", "--json"],
        vec![Reply::ok(CODEX_MCP_LIST_JSON)],
    );
    let file = env.codex_home().join("config.toml");
    let add_file = file.clone();
    let add = Arc::new(move |inv: &Invocation| {
        let (name, mut command) = added(inv);
        let program = command.remove(0);
        edit_toml(&add_file, |doc| {
            let servers = doc
                .entry("mcp_servers")
                .or_insert(toml_edit::table())
                .as_table_mut()
                .expect("mcp_servers table");
            servers.set_implicit(true);
            let mut entry = toml_edit::Table::new();
            entry["command"] = toml_edit::value(program);
            entry["args"] = toml_edit::value(
                command
                    .iter()
                    .map(String::as_str)
                    .collect::<toml_edit::Array>(),
            );
            servers.insert(&name, toml_edit::Item::Table(entry));
        });
    });
    runner.on_with("codex", &["mcp", "add"], vec![Reply::ok("")], add);
    let remove = Arc::new(move |inv: &Invocation| {
        edit_toml(&file, |doc| {
            if let Some(servers) = doc.get_mut("mcp_servers").and_then(|i| i.as_table_mut()) {
                servers.remove(&removed(inv));
            }
        });
    });
    runner.on_with("codex", &["mcp", "remove"], vec![Reply::ok("")], remove);
}

fn edit_toml(file: &Path, edit: impl FnOnce(&mut toml_edit::DocumentMut)) {
    let text = std::fs::read_to_string(file).unwrap_or_default();
    let mut doc: toml_edit::DocumentMut = text.parse().expect("valid TOML");
    edit(&mut doc);
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent).expect("config folder");
    }
    std::fs::write(file, doc.to_string()).expect("write config");
}

/// The file `opencode mcp add` writes: `opencode.json` in the global config
/// folder with `--global`, else in the working folder.
fn opencode_target(inv: &Invocation, global_dir: &Path) -> PathBuf {
    if inv.args.iter().any(|a| a == "--global") {
        global_dir.join("opencode.json")
    } else {
        inv.cwd
            .clone()
            .expect("project add needs a working folder")
            .join("opencode.json")
    }
}

/// Scripts `opencode` (`--version`, `mcp list`, `mcp add [--global]`,
/// `service status` and `service stop`).
/// `mcp list` reproduces the race of research.md §8: the first answer is
/// "No MCP servers configured", every later one is connected. The
/// background service starts out stopped; `service stop` prints nothing
/// (both as captured). `mcp add`
/// writes the confirmed `mcp.servers.<name> = {type: "local", command: […]}`.
/// OpenCode 2.2.5 has no `mcp remove`, so that call is not scripted.
pub(crate) fn opencode(runner: &FakeRunner, env: &Env) {
    runner.on("opencode", &["--version"], vec![Reply::ok("2.2.5\n")]);
    let list = vec![
        Reply::ok(OPENCODE_LIST_BEFORE_INIT),
        Reply::ok(OPENCODE_LIST_CONNECTED),
    ];
    runner.on("opencode", &["mcp", "list"], list);
    let status = vec![Reply::ok(OPENCODE_SERVICE_STOPPED)];
    runner.on("opencode", &["service", "status"], status);
    runner.on("opencode", &["service", "stop"], vec![Reply::ok("")]);
    let global_dir = env.config_home().join("opencode");
    let add = Arc::new(move |inv: &Invocation| {
        let (name, command) = added(inv);
        let file = opencode_target(inv, &global_dir);
        let mut root = read_json(&file);
        let entry = json!({"type": "local", "command": command});
        put(&mut root, &["mcp", "servers"], &name, entry);
        write_json(&file, &root);
    });
    runner.on_with("opencode", &["mcp", "add"], vec![Reply::ok("")], add);
}

/// Scripts `agy` (`--version`, and `mcp list` listing mmhmem). Setup never
/// writes through `agy`, so `mcp add` and `mcp remove` are not scripted.
pub(crate) fn agy(runner: &FakeRunner) {
    runner.on("agy", &["--version"], vec![Reply::ok(AGY_VERSION)]);
    let list = vec![Reply::ok(AGY_MCP_LIST_MMHMEM)];
    runner.on("agy", &["mcp", "list"], list);
}

/// Scripts all four CLIs.
pub(crate) fn all(runner: &FakeRunner, env: &Env) {
    claude(runner, env);
    codex(runner, env);
    opencode(runner, env);
    agy(runner);
}

#[test]
fn every_double_reports_its_captured_version() {
    use crate::setup::record::Os;
    let home = super::doubles::TestHome::new(Os::Windows);
    all(&home.runner, &home.env);
    let versions = [
        ("claude", "2.1.302"),
        ("codex", "codex-cli 0.166.0"),
        ("opencode", "2.2.5"),
        ("agy", "1.5.3"),
    ];
    for (program, version) in versions {
        let out = home
            .env
            .run(&Invocation::new(program, &["--version"]))
            .expect("ran");
        assert!(out.stdout.starts_with(version), "{program}");
    }
}
