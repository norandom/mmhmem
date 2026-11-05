//! Claude Code's functional check (reliable-installers task 4.2, design.md
//! §5 table and §6; research.md §8).
//!
//! 1–2. The registration is read from `~/.claude.json` `mcpServers.mmhmem`
//!    (the bytes Claude Code loads) and started with the MCP check.
//! 3. `claude mcp get mmhmem`, run from the home folder, must show
//!    `Scope: User config`, `Status: ✔ Connected` (or the console fallback
//!    `√ Connected`) and the installed program with `Args: stdio`. Not `claude mcp list`, which health-checks every
//!    remote server and is slow.
//! 4. With the read hook on: `<binary> hook claude` on a sample `Read`.
//! 5. `claude mcp get` exits 0 only when Claude Code loaded its settings, so
//!    its exit code is the config-load step; "No MCP server found" means the
//!    settings loaded but mmhmem is not listed.

use std::path::Path;

use super::{ClaudeCode, Paths};
use crate::setup::agents::{AgentRecord, CheckReport, DEFAULT_TIMEOUT, Env};
use crate::setup::check::Registration;
use crate::setup::check::agent::{
    AgentFailure, StepResult, Steps, hook_selected, read_registration, run_agent_command,
    same_program,
};
use crate::setup::record::{AgentId, CheckStep, HookHarness, Os};

/// The server name mmhmem registers.
const NAME: &str = "mmhmem";

pub(super) fn check(agent: &ClaudeCode, env: &Env, rec: &AgentRecord) -> CheckReport {
    let paths = Paths::of(env);
    let registration = read_registration(&paths.claude_json, |bytes| {
        Registration::from_claude_json(bytes, NAME)
    });
    let mut steps = Steps::start(env, AgentId::ClaudeCode, registration);
    if steps.ok() {
        let (command, config) = mcp_get(env, &agent.binary);
        steps.set(CheckStep::AgentCommand, command);
        steps.set(CheckStep::ConfigLoad, config);
    }
    if hook_selected(rec) {
        steps.hook(env, &agent.binary, HookHarness::Claude);
    }
    steps.finish(rec)
}

/// Steps 3 and 5 from one `claude mcp get mmhmem`.
fn mcp_get(env: &Env, binary: &Path) -> (StepResult, StepResult) {
    let args = ["mcp", "get", NAME];
    match run_agent_command(env, "claude", &args, Some(env.home()), DEFAULT_TIMEOUT) {
        Err(result) => (result, StepResult::Skipped),
        Ok(out) if out.success() => (parse_get(&out.stdout, binary, env.os()), StepResult::Pass),
        Ok(out) => {
            let both = format!("{}\n{}", out.stdout, out.stderr).to_lowercase();
            if both.contains("no mcp server found") {
                (StepResult::Fail(AgentFailure::NotListed), StepResult::Pass)
            } else {
                let failed = || StepResult::Fail(AgentFailure::ConfigNotLoaded);
                (failed(), failed())
            }
        }
    }
}

/// The value after `key:` on its own line of `claude mcp get` output.
fn field<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    text.lines()
        .find_map(|line| line.trim().strip_prefix(key)?.strip_prefix(':'))
        .map(str::trim)
}

/// The tick Claude Code puts before `Connected`. It prints `✔` (U+2714) where
/// it trusts the terminal with Unicode (Windows Terminal, macOS, Linux) and
/// the fallback `√` (U+221A) in other Windows consoles and pipes without
/// Windows Terminal: what the CI runners and a plain `powershell.exe` window
/// get (research.md §8). `✓` (U+2713) is accepted as the same mark.
const CONNECTED_TICKS: [char; 3] = ['\u{2714}', '\u{221A}', '\u{2713}'];

/// `✔ Connected` (or a fallback tick): the status of a reachable server.
fn shows_connected(status: &str) -> bool {
    let mut chars = status.chars();
    chars
        .next()
        .is_some_and(|tick| CONNECTED_TICKS.contains(&tick))
        && chars.as_str().trim_start().starts_with("Connected")
}

/// Checks scope, status and command of a `claude mcp get` answer.
pub(super) fn parse_get(text: &str, binary: &Path, os: Os) -> StepResult {
    let (Some(scope), Some(status)) = (field(text, "Scope"), field(text, "Status")) else {
        return StepResult::Fail(AgentFailure::UnexpectedAnswer);
    };
    if !scope.starts_with("User config") {
        return StepResult::Fail(AgentFailure::WrongScope);
    }
    if !shows_connected(status) {
        return StepResult::Fail(AgentFailure::NotConnected);
    }
    let command = field(text, "Command").unwrap_or("");
    let args = field(text, "Args").unwrap_or("");
    if same_program(command, binary, os) && args == "stdio" {
        StepResult::Pass
    } else {
        StepResult::Fail(AgentFailure::WrongCommand)
    }
}
