//! OpenCode's functional check (reliable-installers task 4.2, design.md §5
//! table and §6; research.md §8 "OpenCode `mcp list` answers before servers
//! start").
//!
//! 1–2. The registration is read from the file the record says holds
//!    `mcp.servers.mmhmem` (else `opencode.jsonc` when it exists, else
//!    `opencode.json`) and started with the MCP check, the primary proof.
//! 3. `opencode mcp list`, run from the home folder (never `--standalone`,
//!    never in a new folder), polled every [`POLL_EVERY`] until it shows
//!    `✓ mmhmem connected` or [`POLL_LIMIT`] passes: the first answer of a
//!    new instance comes before servers start, so a missing entry counts as
//!    a failure only after the limit. The clock is the [`Env`]'s.
//! 4. With the read hook (plugin) on: `<binary> hook opencode` with the
//!    plugin's event for a sample `read`.
//! 5. OpenCode has no command that validates its config: skipped (an
//!    unreadable file already fails step 1).

use std::path::PathBuf;
use std::time::Duration;

use super::{MCP, OpenCode};
use crate::setup::agents::{AgentRecord, CheckReport, DEFAULT_TIMEOUT, Env, Invocation, RunError};
use crate::setup::check::Registration;
use crate::setup::check::agent::{
    AgentFailure, StepResult, Steps, hook_selected, read_registration, recorded_json_file,
};
use crate::setup::record::{AgentId, CheckStep, HookHarness};

/// The server name mmhmem registers.
const NAME: &str = "mmhmem";
/// How long the poll waits for mmhmem to show as connected.
pub const POLL_LIMIT: Duration = Duration::from_secs(30);
/// The pause between two `opencode mcp list` calls.
pub const POLL_EVERY: Duration = Duration::from_secs(1);

pub(super) fn check(agent: &OpenCode, env: &Env, rec: &AgentRecord) -> CheckReport {
    let file = recorded_json_file(rec, &MCP).unwrap_or_else(|| default_file(env));
    let registration =
        read_registration(&file, |bytes| Registration::from_opencode_json(bytes, NAME));
    let mut steps = Steps::start(env, AgentId::OpenCode, registration);
    steps.run(CheckStep::AgentCommand, || poll(env));
    if hook_selected(rec) {
        steps.hook(env, &agent.binary, HookHarness::Opencode);
    }
    steps.finish(rec)
}

fn default_file(env: &Env) -> PathBuf {
    let dir = OpenCode::config_dir(env);
    let jsonc = dir.join("opencode.jsonc");
    if jsonc.is_file() {
        jsonc
    } else {
        dir.join("opencode.json")
    }
}

/// What one `opencode mcp list` said about mmhmem.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Shown {
    Connected,
    Listed,
    Missing,
}

/// Finds mmhmem's line (`✓ mmhmem   connected`) in `opencode mcp list`.
pub(super) fn shown(text: &str) -> Shown {
    let mut result = Shown::Missing;
    for line in text.lines() {
        let mut words = line.split_whitespace();
        let (mark, name) = (words.next(), words.next());
        if name != Some(NAME) {
            continue;
        }
        if mark == Some("\u{2713}") && words.any(|w| w == "connected") {
            return Shown::Connected;
        }
        result = Shown::Listed;
    }
    result
}

/// Step 3: polls `opencode mcp list` from the home folder.
fn poll(env: &Env) -> StepResult {
    let Some(program) = env.find_program("opencode") else {
        return StepResult::Skipped;
    };
    let clock = env.clock();
    let deadline = clock.now() + POLL_LIMIT;
    let mut listed = false;
    loop {
        let left = deadline.saturating_duration_since(clock.now());
        let invocation = Invocation::new(&program, &["mcp", "list"])
            .in_dir(env.home())
            .timeout(left.clamp(POLL_EVERY, DEFAULT_TIMEOUT));
        match env.run(&invocation) {
            Ok(out) => match shown(&out.stdout) {
                Shown::Connected => return StepResult::Pass,
                Shown::Listed => listed = true,
                Shown::Missing => {}
            },
            Err(RunError::NotFound { .. }) => return StepResult::Skipped,
            Err(_) => {}
        }
        if clock.now() >= deadline {
            return StepResult::Fail(AgentFailure::NotConnectedInTime {
                listed,
                seconds: POLL_LIMIT.as_secs(),
            });
        }
        clock.sleep(POLL_EVERY);
    }
}
