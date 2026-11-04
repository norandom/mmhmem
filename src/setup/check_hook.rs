//! The read-hook sample (reliable-installers task 4.2, design.md §6 step 4;
//! Requirement 6.4).
//!
//! Runs `<installed binary> hook <harness>` the way the agent runs it: the
//! agent's documented pre-tool event for a whole-file read of a sample file
//! on stdin, the sample's folder as working folder. The sample is a fresh
//! text file of [`SAMPLE_LINES`] lines in a temporary folder (removed
//! afterwards), well above the routing threshold, and every check uses a
//! new session id so the hook's repeat window never lets it through. The
//! step passes only on the harness's redirect answer: the `PreToolUse` deny
//! pointing to `smart_read` (Claude Code, Codex), or `{"decision": "deny"}`
//! with that reason (OpenCode plugin).

use std::fmt::Write as _;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use super::agent::{AgentFailure, CommandProblem, StepResult};
use crate::setup::agents::{Env, Invocation};
use crate::setup::record::HookHarness;

/// Lines in the sample file (the default routing threshold is 150).
pub const SAMPLE_LINES: usize = 5_000;
/// The hook's time limit (the agents allow it 10 s).
const HOOK_TIMEOUT: Duration = Duration::from_secs(10);
/// The sample file's name.
const SAMPLE_NAME: &str = "mmhmem-sample.txt";

/// What a hook answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Answer {
    Redirect,
    Pass,
    Other,
}

/// Step 4 for `harness`: run `binary hook <harness>` on a sample read.
pub fn hook_sample(env: &Env, binary: &Path, harness: HookHarness) -> StepResult {
    let not_run = StepResult::Fail(AgentFailure::HookNotRun {
        problem: CommandProblem::CouldNotRun,
    });
    let Some(dir) = sample_dir() else {
        return not_run;
    };
    let file = dir.path().join(SAMPLE_NAME);
    if crate::services::shunt::is_security_critical_path(&file) {
        // The hook never redirects such paths, so a sample there proves
        // nothing (a temp folder under a user name like "author").
        return StepResult::Skipped;
    }
    if std::fs::write(&file, sample_text()).is_err() {
        return not_run;
    }
    let event = sample_event(harness, dir.path(), &file, &session_id());
    let invocation = Invocation::new(binary, &["hook", harness.as_str()])
        .in_dir(dir.path())
        .stdin(event.into_bytes())
        .timeout(HOOK_TIMEOUT);
    match env.run(&invocation) {
        Err(error) => StepResult::Fail(AgentFailure::HookNotRun {
            problem: CommandProblem::of(&error),
        }),
        Ok(out) if !out.success() => StepResult::Fail(AgentFailure::HookWrongAnswer),
        Ok(out) => match answer(harness, &out.stdout) {
            Answer::Redirect => StepResult::Pass,
            Answer::Pass => StepResult::Fail(AgentFailure::HookLetReadThrough),
            Answer::Other => StepResult::Fail(AgentFailure::HookWrongAnswer),
        },
    }
}

/// A new temporary folder; a few tries for a random name the hook's
/// security rule does not match (`…lock…`, `…jwt…`).
fn sample_dir() -> Option<tempfile::TempDir> {
    let mut last = None;
    for _ in 0..5 {
        let dir = tempfile::Builder::new()
            .prefix("mmhmem-hook-check-")
            .tempdir()
            .ok()?;
        if !crate::services::shunt::is_security_critical_path(&dir.path().join(SAMPLE_NAME)) {
            return Some(dir);
        }
        last = Some(dir);
    }
    last
}

fn sample_text() -> String {
    let mut text = String::with_capacity(SAMPLE_LINES * 48);
    for n in 1..=SAMPLE_LINES {
        let _ = writeln!(text, "line {n} of the mmhmem setup hook check sample");
    }
    text
}

fn session_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    format!("mmhmem-setup-check-{}-{nanos}", std::process::id())
}

/// The event the agent sends before a whole-file read of `file`.
pub(super) fn sample_event(harness: HookHarness, dir: &Path, file: &Path, session: &str) -> String {
    let cwd = dir.to_string_lossy();
    let path = file.to_string_lossy();
    let event = match harness {
        HookHarness::Claude => json!({
            "session_id": session, "cwd": cwd, "hook_event_name": "PreToolUse",
            "tool_name": "Read", "tool_input": {"file_path": path},
        }),
        HookHarness::Codex => json!({
            "session_id": session, "cwd": cwd, "hook_event_name": "PreToolUse",
            "tool_name": "Bash", "tool_input": {"command": format!("cat {SAMPLE_NAME}")},
        }),
        HookHarness::Opencode => json!({
            "tool": "read", "sessionID": session, "cwd": cwd, "input": {"path": path},
        }),
    };
    event.to_string()
}

/// Classifies the hook's stdout for `harness`.
fn answer(harness: HookHarness, stdout: &str) -> Answer {
    let text = stdout.trim();
    if text.is_empty() {
        return Answer::Pass;
    }
    let Ok(value) = serde_json::from_str::<Value>(text) else {
        return Answer::Other;
    };
    let points_to_smart_read = |reason: Option<&Value>| {
        reason
            .and_then(Value::as_str)
            .is_some_and(|r| r.contains("smart_read"))
    };
    match harness {
        HookHarness::Claude | HookHarness::Codex => {
            let out = &value["hookSpecificOutput"];
            let deny = out["hookEventName"] == "PreToolUse" && out["permissionDecision"] == "deny";
            if deny && points_to_smart_read(out.get("permissionDecisionReason")) {
                Answer::Redirect
            } else {
                Answer::Other
            }
        }
        HookHarness::Opencode => match value["decision"].as_str() {
            Some("pass") => Answer::Pass,
            Some("deny") if points_to_smart_read(value.get("reason")) => Answer::Redirect,
            _ => Answer::Other,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answers_are_classified_per_harness() {
        let deny = json!({"hookSpecificOutput": {"hookEventName": "PreToolUse",
            "permissionDecision": "deny", "permissionDecisionReason": "call smart_read"}})
        .to_string();
        assert_eq!(answer(HookHarness::Claude, &deny), Answer::Redirect);
        assert_eq!(answer(HookHarness::Codex, &deny), Answer::Redirect);
        assert_eq!(answer(HookHarness::Opencode, &deny), Answer::Other);
        assert_eq!(answer(HookHarness::Claude, "\n"), Answer::Pass);
        assert_eq!(answer(HookHarness::Claude, "not json"), Answer::Other);
        let oc = json!({"decision": "deny", "reason": "Call smart_read"}).to_string();
        assert_eq!(answer(HookHarness::Opencode, &oc), Answer::Redirect);
        let pass = json!({"decision": "pass"}).to_string();
        assert_eq!(answer(HookHarness::Opencode, &pass), Answer::Pass);
        let other = json!({"hookSpecificOutput": {"hookEventName": "PreToolUse",
            "permissionDecision": "allow"}})
        .to_string();
        assert_eq!(answer(HookHarness::Claude, &other), Answer::Other);
    }

    #[test]
    fn sample_events_are_whole_file_reads_the_hook_accepts() {
        let dir = Path::new("/work");
        let file = dir.join(SAMPLE_NAME);
        for (harness, h) in [
            (HookHarness::Claude, crate::hook::adapters::Harness::Claude),
            (HookHarness::Codex, crate::hook::adapters::Harness::Codex),
            (
                HookHarness::Opencode,
                crate::hook::adapters::Harness::OpenCode,
            ),
        ] {
            let event = sample_event(harness, dir, &file, "s-1");
            let call = crate::hook::adapters::parse(h, event.as_bytes());
            let read = call.read.expect("a read");
            assert_eq!(read.path, file, "{harness:?}");
            assert_eq!(read.range, None, "{harness:?}");
            assert_eq!(call.session_id, "s-1");
        }
        assert_eq!(sample_text().lines().count(), SAMPLE_LINES);
    }
}
