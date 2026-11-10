//! Antigravity check (antigravity-native-setup design.md "Antigravity
//! check"): the functional check from Antigravity's own files (task 3.5;
//! Requirements 2.6 and 4.1–4.3) and bringing earlier installs forward
//! (task 3.4; Requirements 5.1 and 5.2).
//!
//! - **Check** (4.1–4.3): the registration is read back from the bytes of
//!   the global server file (`mcpServers.mmhmem`, the shape Claude Desktop
//!   uses) and started with the standard MCP steps; a file that is missing,
//!   unreadable or has no usable entry fails the registration step with the
//!   usual reason, never its text. With `agy` on PATH, `agy mcp list` must
//!   have a line naming mmhmem (the output is read in memory and dropped);
//!   without `agy` that step is skipped and the report says why. With the
//!   read hook selected and its entry recorded, the hook sample runs
//!   whether or not a worker model is set up, because a hook that does not
//!   answer blocks every `read_file`: without a worker the pass form is the
//!   expected answer and the report still says the hook stays inactive
//!   (2.6). A selected hook that setup left out is not sampled and the
//!   report says so.
//! - **Snippet era** (5.1): installs from 0.5.1 on saved an import snippet,
//!   [`SNIPPET_FILE`], next to the binary and recorded it as a whole file.
//!   It is matched by its file name, so the snippet next to a binary that has
//!   moved since is still taken out.
//! - **Stale location** (5.2): the snippet was pasted into
//!   `~/.gemini/antigravity/mcp_config.json`, which Antigravity does not
//!   read. mmhmem's entry there is taken out in every plan, whatever is
//!   selected, and on removal, when its `command` starts the mmhmem
//!   program. The file is not mmhmem's: an entry naming another program, a
//!   missing, unreadable or invalid file is left alone and never a problem,
//!   and everything else in the file stays byte-identical. The
//!   removal leaves nothing to record (a removed entry drops out of the
//!   record when it is applied), so a plan after it has nothing to do there.

use std::path::Path;

use super::hook::is_hook;
use super::{Antigravity, COMMAND, ENTRY, ID, PROGRAM, SCOPE, SNIPPET_FILE};
use crate::setup::agents::{AgentRecord, CheckReport, DEFAULT_TIMEOUT, Env};
use crate::setup::check::Registration;
use crate::setup::check::agent::{
    AgentFailure, StepResult, Steps, hook_selected, read_registration, run_agent_command,
};
use crate::setup::edit::json as edit;
use crate::setup::plan::{Action, Change};
use crate::setup::record::{CheckResult, CheckStep, HookHarness, Target, key_path};

/// The mmhmem program's file names.
const PROGRAM_NAMES: [&str; 2] = ["mmhmem", "mmhmem.exe"];

/// The server name mmhmem registers.
const NAME: &str = "mmhmem";

/// Why the listing step was skipped (design.md "Antigravity check").
const NO_CLI: &str = "Antigravity CLI not found; checked the config file only";

/// Why the hook sample was skipped although the read hook is selected.
const HOOK_LEFT_OUT: &str = "the read-routing hook was left out, so it was not checked";

/// The functional check from Antigravity's own files (see the module
/// comment). The notes for skipped steps follow the result line only when
/// the check did not fail.
pub(super) fn check(agent: &Antigravity, env: &Env, rec: &AgentRecord) -> CheckReport {
    let file = Antigravity::mcp_file(env);
    let registration =
        read_registration(&file, |bytes| Registration::from_claude_json(bytes, NAME));
    let mut steps = Steps::start(env, ID, registration);
    let mut notes = Vec::new();
    if steps.ok() {
        let listed = mcp_list(env);
        if listed == StepResult::Skipped {
            notes.push(NO_CLI);
        }
        steps.set(CheckStep::AgentCommand, listed);
    }
    if hook_selected(rec) {
        if hook_recorded(env, rec) {
            steps.hook_always(env, &agent.binary, HookHarness::Antigravity);
        } else {
            notes.push(HOOK_LEFT_OUT);
        }
    }
    let mut report = steps.finish(rec);
    if report.result != CheckResult::Fail {
        let label = ID.label();
        let mut lines: Vec<String> = notes.iter().map(|n| format!("● {label} — {n}")).collect();
        lines.append(&mut report.notes);
        report.notes = lines;
    }
    report
}

/// Step 3: `agy mcp list`, run from the home folder, must name mmhmem.
/// Skipped when `agy` is not on PATH; an exit other than 0 is Antigravity
/// not loading its settings.
fn mcp_list(env: &Env) -> StepResult {
    let args = ["mcp", "list"];
    match run_agent_command(env, PROGRAM, &args, Some(env.home()), DEFAULT_TIMEOUT) {
        Err(result) => result,
        Ok(out) if out.success() => listed(&out.stdout),
        Ok(_) => StepResult::Fail(AgentFailure::ConfigNotLoaded),
    }
}

/// Whether the `agy mcp list` table (`NAME TYPE STATUS COMMAND/URL`, or
/// `No MCP servers configured.`) has a line with the word `mmhmem`; a
/// command path that merely contains it does not count.
fn listed(stdout: &str) -> StepResult {
    let names_mmhmem = |line: &str| line.split_whitespace().any(|word| word == NAME);
    if stdout.lines().any(names_mmhmem) {
        StepResult::Pass
    } else {
        StepResult::Fail(AgentFailure::NotListed)
    }
}

/// Whether the record lists mmhmem's named hook in Antigravity's hook file.
fn hook_recorded(env: &Env, rec: &AgentRecord) -> bool {
    let hooks = Antigravity::hooks_file(env);
    rec.managed.iter().any(|entry| {
        matches!(&entry.target,
            Target::JsonEntry { file, pointer, .. } if *file == hooks && is_hook(pointer))
    })
}

/// Whether a recorded whole file is the snippet-era import snippet.
pub(super) fn is_snippet(file: &Path) -> bool {
    file.file_name().is_some_and(|name| name == SNIPPET_FILE)
}

/// The removal of mmhmem's whole entry from the stale server file `stale`
/// when its `command` starts the mmhmem program ([`starts_mmhmem`]);
/// `None` otherwise, and for a file that is missing, unreadable or not
/// valid JSON. Parent objects stay, even when left empty.
pub(super) fn stale_entry(stale: &Path, binary: &Path) -> Option<Change> {
    let before = std::fs::read(stale).ok()?;
    let text = std::str::from_utf8(&before).ok()?;
    let command = edit::get(text, &COMMAND).ok()??;
    if !starts_mmhmem(command.as_str()?, binary) {
        return None;
    }
    let after = edit::remove(text, &ENTRY).ok()?.into_bytes();
    Some(Change {
        scope: SCOPE,
        target: Target::JsonEntry {
            file: stale.to_path_buf(),
            pointer: key_path(&ENTRY).expect("static keys"),
            managed_keys: None,
        },
        file: stale.to_path_buf(),
        action: Action::Remove,
        before: Some(before),
        after: Some(after),
        owner_only: false,
        hide_diff: false,
        missing_parents: None,
        summary: format!(
            "Antigravity no longer reads this file; remove the mmhmem entry ({})",
            stale.display()
        ),
    })
}

/// Whether `command` starts mmhmem: it is the installed `binary`, or its
/// file name (after the last slash of either style, in any case) is one of
/// [`PROGRAM_NAMES`]. The file was written on this machine, but a path
/// pasted from another one may use the other slash style.
fn starts_mmhmem(command: &str, binary: &Path) -> bool {
    if Path::new(command) == binary {
        return true;
    }
    let name = command.rsplit(['/', '\\']).next().unwrap_or(command);
    PROGRAM_NAMES.iter().any(|p| name.eq_ignore_ascii_case(p))
}
