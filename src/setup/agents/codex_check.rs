//! Codex's functional check (reliable-installers task 4.2, design.md §5
//! table and §6; research.md §8).
//!
//! 1–2. The registration is read from `<CODEX_HOME>/config.toml`
//!    `[mcp_servers.mmhmem]` and started with the MCP check (Codex reports no
//!    connection state, so the handshake is the connectivity proof).
//! 3. `codex mcp list --json`, run from the home folder, must list an
//!    enabled `mmhmem` whose transport starts the installed program with
//!    `stdio`. **It prints env values unmasked**: the text is parsed in
//!    memory and dropped; only fixed failure codes leave this module.
//! 4. With the read hook on: `<binary> hook codex` on a sample shell read.
//! 5. The same call exits non-zero when `config.toml` does not load, so its
//!    exit code is the config-load step.

use std::path::Path;

use serde_json::Value;

use super::{Codex, ID};
use crate::setup::agents::{AgentRecord, CheckReport, DEFAULT_TIMEOUT, Env};
use crate::setup::check::Registration;
use crate::setup::check::agent::{
    AgentFailure, StepResult, Steps, hook_selected, read_registration, run_agent_command,
    same_program,
};
use crate::setup::record::{CheckStep, HookHarness, Os};

/// The server name mmhmem registers.
const NAME: &str = "mmhmem";

pub(super) fn check(agent: &Codex, env: &Env, rec: &AgentRecord) -> CheckReport {
    let config = env.codex_home().join("config.toml");
    let registration =
        read_registration(&config, |bytes| Registration::from_codex_toml(bytes, NAME));
    let mut steps = Steps::start(env, ID, registration);
    if steps.ok() {
        let (command, config) = mcp_list(env, &agent.binary);
        steps.set(CheckStep::AgentCommand, command);
        steps.set(CheckStep::ConfigLoad, config);
    }
    if hook_selected(rec) {
        steps.hook(env, &agent.binary, HookHarness::Codex);
    }
    steps.finish(rec)
}

/// Steps 3 and 5 from one `codex mcp list --json`.
fn mcp_list(env: &Env, binary: &Path) -> (StepResult, StepResult) {
    let args = ["mcp", "list", "--json"];
    match run_agent_command(env, "codex", &args, Some(env.home()), DEFAULT_TIMEOUT) {
        Err(result) => (result, StepResult::Skipped),
        Ok(out) if out.success() => (listed(&out.stdout, binary, env.os()), StepResult::Pass),
        Ok(_) => {
            let failed = || StepResult::Fail(AgentFailure::ConfigNotLoaded);
            (failed(), failed())
        }
    }
}

/// Checks mmhmem's entry in the `codex mcp list --json` array. Reads only
/// `name`, `enabled`, `transport.command` and `transport.args`.
pub(super) fn listed(json: &str, binary: &Path, os: Os) -> StepResult {
    let Ok(Value::Array(servers)) = serde_json::from_str::<Value>(json) else {
        return StepResult::Fail(AgentFailure::UnexpectedAnswer);
    };
    let Some(entry) = servers.iter().find(|s| s["name"] == NAME) else {
        return StepResult::Fail(AgentFailure::NotListed);
    };
    if entry["enabled"] != Value::Bool(true) {
        return StepResult::Fail(AgentFailure::Disabled);
    }
    let transport = &entry["transport"];
    let command = transport["command"].as_str().unwrap_or("");
    let args_ok = transport["args"]
        .as_array()
        .is_some_and(|a| a.len() == 1 && a[0] == "stdio");
    if same_program(command, binary, os) && args_ok {
        StepResult::Pass
    } else {
        StepResult::Fail(AgentFailure::WrongCommand)
    }
}
