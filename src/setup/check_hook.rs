//! The read-hook sample (reliable-installers task 4.2, design.md §6 step 4;
//! Requirement 6.4; Antigravity: antigravity-native-setup task 2.2, design.md
//! "Hook sample", Requirement 4.2).
//!
//! Runs `<installed binary> hook <harness>` the way the agent runs it: the
//! agent's documented pre-tool event for a whole-file read of a sample file
//! on stdin, the sample's folder as working folder. The sample is a fresh
//! text file of [`SAMPLE_LINES`] lines in a temporary folder (removed
//! afterwards), well above the routing threshold, and every check uses a
//! new session id so the hook's repeat window never lets it through. The
//! step passes only on the harness's redirect answer: the `PreToolUse` deny
//! pointing to `smart_read` (Claude Code, Codex), or `{"decision": "deny"}`
//! with that reason (OpenCode plugin, Antigravity). Antigravity blocks the
//! tool call on anything but one JSON object with a `decision` (empty
//! output, `{}`, a non-zero exit, no answer in time), so for it any of those
//! means the hook did not run, and without a worker model its pass form
//! `{"decision": "ask"}` is the expected answer.

use std::fmt::Write as _;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use super::agent::{AgentFailure, CommandProblem, StepResult};
use crate::setup::agents::{CommandOutput, Env, Invocation};
use crate::setup::record::HookHarness;

/// Lines in the sample file, far above the default routing threshold
/// ([`crate::config::DEFAULT_SHUNT_MIN_LINES`]) and any usual override.
pub const SAMPLE_LINES: usize = 5_000;
/// The hook's time limit (the agents allow it 10 s).
const HOOK_TIMEOUT: Duration = Duration::from_secs(10);
/// The sample file's name.
const SAMPLE_NAME: &str = "mmhmem-sample.txt";

/// What a hook answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Answer {
    /// The redirect to `smart_read`.
    Redirect,
    /// The read may proceed.
    Pass,
    /// Something else the agent understands.
    Other,
    /// Nothing the agent can use: Antigravity blocks the tool call.
    Unusable,
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
        Ok(out) => judge(harness, env.worker_configured(), &out),
    }
}

/// The step's result for what the hook printed. Every harness must
/// redirect, except Antigravity without a worker model, which must pass;
/// an unusable Antigravity answer or exit is the hook not running.
fn judge(harness: HookHarness, worker: bool, out: &CommandOutput) -> StepResult {
    let antigravity = harness == HookHarness::Antigravity;
    let got = if out.success() {
        answer(harness, &out.stdout)
    } else {
        Answer::Unusable
    };
    let expected = if antigravity && !worker {
        Answer::Pass
    } else {
        Answer::Redirect
    };
    let failure = match got {
        _ if got == expected => return StepResult::Pass,
        Answer::Unusable if antigravity => AgentFailure::HookNotRun {
            problem: CommandProblem::CouldNotRun,
        },
        Answer::Pass => AgentFailure::HookLetReadThrough,
        Answer::Redirect | Answer::Other | Answer::Unusable => AgentFailure::HookWrongAnswer,
    };
    StepResult::Fail(failure)
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

/// The event the agent sends before a whole-file read of `file`. For
/// Antigravity, the captured `read_file` shape (research.md "Antigravity
/// hooks contract") with the check's own session id, so a repeated check
/// is never inside the hook's repeat window.
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
        HookHarness::Antigravity => json!({
            "conversationId": session, "workspacePaths": [cwd],
            "toolCall": {"name": "read_file", "args": {"Path": path}}, "stepIdx": 1,
        }),
    };
    event.to_string()
}

/// Whether a deny `reason` points the agent to `smart_read`.
fn names_smart_read(reason: Option<&Value>) -> bool {
    reason
        .and_then(Value::as_str)
        .is_some_and(|r| r.contains("smart_read"))
}

/// Classifies the hook's stdout for `harness`. Antigravity needs one JSON
/// object with a string `decision` (its pass form is `ask`); for the
/// others, empty output lets the read through.
fn answer(harness: HookHarness, stdout: &str) -> Answer {
    let text = stdout.trim();
    match (harness, serde_json::from_str::<Value>(text)) {
        (HookHarness::Antigravity, Ok(value)) if value["decision"].is_string() => {
            decision_answer(&value, "ask")
        }
        (HookHarness::Antigravity, _) => Answer::Unusable,
        _ if text.is_empty() => Answer::Pass,
        (_, Err(_)) => Answer::Other,
        (HookHarness::Claude | HookHarness::Codex, Ok(value)) => {
            let out = &value["hookSpecificOutput"];
            let deny = out["hookEventName"] == "PreToolUse" && out["permissionDecision"] == "deny";
            if deny && names_smart_read(out.get("permissionDecisionReason")) {
                Answer::Redirect
            } else {
                Answer::Other
            }
        }
        (HookHarness::Opencode, Ok(value)) => decision_answer(&value, "pass"),
    }
}

/// A `{"decision", "reason"}` answer whose pass form is `{"decision": pass}`.
fn decision_answer(value: &Value, pass: &str) -> Answer {
    match value["decision"].as_str() {
        Some(decision) if decision == pass => Answer::Pass,
        Some("deny") if names_smart_read(value.get("reason")) => Answer::Redirect,
        _ => Answer::Other,
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::hook::adapters::{self, Harness};
    use crate::hook::{Decision, PassReason};
    use crate::setup::agents::doubles::{Effect, Reply, TestHome};
    use crate::setup::record::Os;

    /// Each setup-side harness with the hook's own.
    const HARNESSES: [(HookHarness, Harness); 4] = [
        (HookHarness::Claude, Harness::Claude),
        (HookHarness::Codex, Harness::Codex),
        (HookHarness::Opencode, Harness::OpenCode),
        (HookHarness::Antigravity, Harness::Antigravity),
    ];

    const T: u32 = crate::config::DEFAULT_SHUNT_MIN_LINES;

    /// What the real hook prints for a redirect of a sample read.
    fn redirect(harness: Harness) -> String {
        let file = Path::new("/work").join(SAMPLE_NAME);
        let lines = u64::from(T);
        adapters::render(harness, &Decision::Redirect { lines }, &file, "p", T)
    }

    #[test]
    fn the_sample_is_far_above_the_default_threshold() {
        assert!(SAMPLE_LINES >= 10 * T as usize, "{SAMPLE_LINES} vs {T}");
    }

    fn not_run(problem: CommandProblem) -> StepResult {
        StepResult::Fail(AgentFailure::HookNotRun { problem })
    }

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
    fn antigravity_answers_need_one_object_with_a_decision() {
        let ag = |stdout: &str| answer(HookHarness::Antigravity, stdout);
        assert_eq!(ag(&redirect(Harness::Antigravity)), Answer::Redirect);
        let file = Path::new("/work").join(SAMPLE_NAME);
        let pass = Decision::Pass(PassReason::NoWorker);
        let ask = adapters::render(Harness::Antigravity, &pass, &file, "p", T);
        assert_eq!(ag(&ask), Answer::Pass);
        let fail_open = adapters::fail_open_output(Harness::Antigravity);
        assert_eq!(ag(fail_open), Answer::Pass);
        let claude = redirect(Harness::Claude);
        for blocking in [
            "",
            "\n",
            "not json",
            "{}",
            "[]",
            "\"deny\"",
            r#"{"decision": 1}"#,
            r#"{"reason": "call smart_read"}"#,
            r#"{"decision":"ask"}{"decision":"ask"}"#,
            &claude,
        ] {
            assert_eq!(ag(blocking), Answer::Unusable, "{blocking:?}");
        }
        for other in [
            r#"{"decision":"allow"}"#,
            r#"{"decision":"force_ask"}"#,
            r#"{"decision":"pass"}"#,
            r#"{"decision":"deny"}"#,
            r#"{"decision":"deny","reason":"no"}"#,
        ] {
            assert_eq!(ag(other), Answer::Other, "{other:?}");
        }
    }

    #[test]
    fn sample_events_are_whole_file_reads_the_hook_accepts() {
        let dir = Path::new("/work");
        let file = dir.join(SAMPLE_NAME);
        for (harness, h) in HARNESSES {
            let event = sample_event(harness, dir, &file, "s-1");
            let call = adapters::parse(h, event.as_bytes());
            let read = call.read.expect("a read");
            assert_eq!(read.path, file, "{harness:?}");
            assert_eq!(read.range, None, "{harness:?}");
            assert_eq!(call.session_id, "s-1", "{harness:?}");
            assert_eq!(call.cwd, dir.to_string_lossy(), "{harness:?}");
        }
        assert_eq!(sample_text().lines().count(), SAMPLE_LINES);
    }

    #[test]
    fn harness_names_match_the_hook_command_and_the_record() {
        for (harness, h) in HARNESSES {
            assert_eq!(harness.as_str(), h.name());
            assert_eq!(serde_json::to_value(harness).unwrap(), json!(h.name()));
            let back: HookHarness = serde_json::from_value(json!(h.name())).unwrap();
            assert_eq!(back, harness);
        }
        assert_eq!(HookHarness::Antigravity.as_str(), "antigravity");
    }

    /// One sample run against a scripted `mmhmem hook <harness>`.
    struct Ran {
        result: StepResult,
        event: Value,
        cwd: Option<PathBuf>,
        /// The sample's line count while the hook ran.
        sample_lines: usize,
    }

    fn run_sample(harness: HookHarness, worker: bool, reply: Reply) -> Ran {
        let home = TestHome::new(Os::current());
        let seen = Arc::new(Mutex::new(0));
        let count = seen.clone();
        let effect: Effect = Arc::new(move |call: &Invocation| {
            let file = call.cwd.as_ref().map(|d| d.join(SAMPLE_NAME));
            let text = file.and_then(|f| std::fs::read_to_string(f).ok());
            *count.lock().unwrap() = text.map_or(0, |t| t.lines().count());
        });
        let args = ["hook", harness.as_str()];
        home.runner.on_with("mmhmem", &args, vec![reply], effect);
        let env = home.env.clone().with_worker_configured(worker);
        let result = hook_sample(&env, &home.bin().join("mmhmem"), harness);
        let call = home.runner.calls().pop().expect("the hook ran");
        assert_eq!(call.args, args);
        assert_eq!(call.timeout, HOOK_TIMEOUT);
        let event = serde_json::from_slice(call.stdin.as_deref().expect("stdin")).unwrap();
        let sample_lines = *seen.lock().unwrap();
        Ran {
            result,
            event,
            cwd: call.cwd,
            sample_lines,
        }
    }

    #[test]
    fn antigravity_sample_is_a_whole_file_read_file_call() {
        let deny = redirect(Harness::Antigravity);
        let ran = run_sample(HookHarness::Antigravity, true, Reply::ok(&deny));
        assert_eq!(ran.result, StepResult::Pass);
        let tool = &ran.event["toolCall"];
        assert_eq!(tool["name"], "read_file");
        assert_eq!(tool["args"].as_object().map(|a| a.len()), Some(1));
        let sample = PathBuf::from(tool["args"]["Path"].as_str().expect("Path"));
        assert!(sample.is_absolute(), "{}", sample.display());
        let cwd = ran.cwd.expect("working folder");
        assert_eq!(sample.parent(), Some(cwd.as_path()));
        assert_eq!(ran.event["workspacePaths"], json!([cwd.to_string_lossy()]));
        let session = ran.event["conversationId"].as_str().unwrap_or_default();
        assert!(session.starts_with("mmhmem-setup-check-"), "{session}");
        assert_eq!(ran.event["stepIdx"], 1);
        assert_eq!(ran.sample_lines, SAMPLE_LINES);
        assert!(!sample.exists(), "the sample folder is removed afterwards");
    }

    #[test]
    fn antigravity_sample_wants_deny_with_a_worker_and_ask_without() {
        let deny = redirect(Harness::Antigravity);
        let ask = adapters::fail_open_output(Harness::Antigravity);
        let ag = |worker, stdout: &str| {
            run_sample(HookHarness::Antigravity, worker, Reply::ok(stdout)).result
        };
        assert_eq!(ag(true, &deny), StepResult::Pass);
        assert_eq!(ag(false, ask), StepResult::Pass);
        let let_through = StepResult::Fail(AgentFailure::HookLetReadThrough);
        assert_eq!(ag(true, ask), let_through);
        let wrong = StepResult::Fail(AgentFailure::HookWrongAnswer);
        assert_eq!(ag(false, &deny), wrong);
        assert_eq!(ag(true, r#"{"decision":"allow"}"#), wrong);
        assert_eq!(ag(false, r#"{"decision":"allow"}"#), wrong);
    }

    #[test]
    fn an_antigravity_answer_that_would_block_reads_is_not_run() {
        let ask = adapters::fail_open_output(Harness::Antigravity);
        let timed_out = not_run(CommandProblem::TimedOut { seconds: 10 });
        let could_not = not_run(CommandProblem::CouldNotRun);
        for worker in [true, false] {
            let run = |reply| run_sample(HookHarness::Antigravity, worker, reply).result;
            assert_eq!(run(Reply::TimedOut), timed_out, "worker {worker}");
            for stdout in ["", "\n", "{}", "garbage", "[]"] {
                let result = run(Reply::ok(stdout));
                assert_eq!(result, could_not, "{stdout:?}, worker {worker}");
            }
            assert_eq!(run(Reply::exit(1, ask, "")), could_not, "worker {worker}");
        }
    }

    #[test]
    fn other_harnesses_keep_their_acceptance() {
        let deny = redirect(Harness::Claude);
        let let_through = StepResult::Fail(AgentFailure::HookLetReadThrough);
        let wrong = StepResult::Fail(AgentFailure::HookWrongAnswer);
        for worker in [true, false] {
            let claude = |reply| run_sample(HookHarness::Claude, worker, reply).result;
            assert_eq!(claude(Reply::ok(&deny)), StepResult::Pass);
            assert_eq!(claude(Reply::ok("")), let_through);
            assert_eq!(claude(Reply::ok("{}")), wrong);
            assert_eq!(claude(Reply::exit(1, &deny, "")), wrong);
            let timed_out = not_run(CommandProblem::TimedOut { seconds: 10 });
            assert_eq!(claude(Reply::TimedOut), timed_out);
            let oc = |stdout| run_sample(HookHarness::Opencode, worker, Reply::ok(stdout)).result;
            assert_eq!(oc(r#"{"decision":"pass"}"#), let_through);
            assert_eq!(oc(""), let_through);
        }
    }
}
