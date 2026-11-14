//! `mmhmem hook <harness>` (read-routing-hooks design.md "hook command
//! (`src/cli/hook.rs`)", "System Flows", "Error Handling"; Req 2.4, 2.5,
//! 3.1-3.4, 5.2-5.4).
//!
//! One pre-tool event in on stdin, one decision out on stdout, exit 0 in
//! every case. The command fails open: unknown harness, bad or oversized
//! input, a configuration that cannot load, any error or panic means no
//! output, so the tool call proceeds. Antigravity denies the call on empty
//! stdout, so for it the same cases print its pass answer `{"decision":"ask"}`
//! (antigravity-native-setup design.md "Antigravity hook adapter", Req 2.3,
//! 2.4). It is synchronous (no runtime, no network, no MCP server) and
//! initialises no logging, so stdout carries nothing but the harness decision.

use std::io::{Read, Write};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;
use std::process::ExitCode;

use serde_json::{Value, json};

use crate::config::Config;
use crate::hook::adapters::{self, Harness, MAX_INPUT_BYTES, ParsedCall};
use crate::hook::state::RepeatWindow;
use crate::hook::{Decision, HookSettings, LineRange, PassReason, ReadRequest, decide};
use crate::storage::audit::{AuditEventType, AuditWriter};

/// Project name recorded when the harness `cwd` has no usable basename.
const UNKNOWN_PROJECT: &str = "unknown";

/// Entry point: answers one event and always exits 0 (Req 3.1, 3.2).
pub fn run_hook(harness: Option<&str>) -> ExitCode {
    // The default hook prints the panic message and location; keep stderr
    // to one line and stdout untouched.
    // writeln! instead of eprintln!: a closed stderr must not panic again
    // inside the panic hook (that would abort with a non-zero exit).
    std::panic::set_hook(Box::new(|_| {
        let _ = writeln!(
            std::io::stderr(),
            "mmhmem hook: internal error, letting the call proceed"
        );
    }));
    let output = guarded(|| respond(harness), || audit_internal_error(harness));
    let output = if output.is_empty() {
        no_decision_output(harness)
    } else {
        output
    };
    if !output.is_empty() {
        let mut stdout = std::io::stdout().lock();
        let _ = stdout.write_all(output.as_bytes());
        let _ = stdout.flush();
    }
    ExitCode::SUCCESS
}

/// Runs `answer`; a panic yields the pass output (empty) after a
/// best-effort `on_error` that may itself not panic outward (Req 3.2).
fn guarded(answer: impl FnOnce() -> String, on_error: impl FnOnce()) -> String {
    match catch_unwind(AssertUnwindSafe(answer)) {
        Ok(output) => output,
        Err(_) => {
            let _ = catch_unwind(AssertUnwindSafe(on_error));
            String::new()
        }
    }
}

/// Stdout when `respond` rendered nothing (an early pass or a panic): the
/// harness's fail-open answer, empty except for Antigravity; an unsupported
/// harness name prints nothing, as before.
fn no_decision_output(harness: Option<&str>) -> String {
    harness
        .and_then(parse_harness)
        .map_or("", adapters::fail_open_output)
        .to_string()
}

/// Maps the CLI argument to a harness; `None` for anything unsupported.
fn parse_harness(name: &str) -> Option<Harness> {
    match name {
        "claude" => Some(Harness::Claude),
        "codex" => Some(Harness::Codex),
        "opencode" => Some(Harness::OpenCode),
        "antigravity" => Some(Harness::Antigravity),
        _ => None,
    }
}

/// The whole decision path; every early return is a pass (empty output).
fn respond(harness: Option<&str>) -> String {
    let Some(harness) = harness.and_then(parse_harness) else {
        let _ = writeln!(
            std::io::stderr(),
            "mmhmem hook: unsupported harness, letting the call proceed"
        );
        return String::new();
    };
    let Some(input) = read_bounded(&mut std::io::stdin().lock(), MAX_INPUT_BYTES) else {
        return String::new();
    };
    let call = adapters::parse(harness, &input);
    let Some(req) = call.read.as_ref() else {
        if harness == Harness::Claude
            && let Some(cmd) = call.command.as_deref()
        {
            return adapters::render_claude_compact(cmd);
        }
        return String::new();
    };
    // Req 3.1: a configuration that cannot load means pass, unrecorded
    // (the audit path itself comes from that configuration).
    let Ok(config) = Config::load_local() else {
        return String::new();
    };
    let window = RepeatWindow::with_default_window(&config.data_dir);
    let settings = settings_from(&config);
    let decision = decide_call(&call, req, &settings, &window, now_ms());
    record(&AuditWriter::from_config(&config), &call, req, &decision);
    let project = project_name(&call.cwd);
    adapters::render(
        harness,
        &decision,
        &req.path,
        project,
        settings.shunt_min_lines,
    )
}

/// Reads at most `cap` bytes; `None` on a read error or more than `cap`.
fn read_bounded(reader: &mut impl Read, cap: usize) -> Option<Vec<u8>> {
    let mut buf = Vec::new();
    reader
        .take(cap as u64 + 1)
        .read_to_end(&mut buf)
        .ok()
        .filter(|&n| n <= cap)?;
    Some(buf)
}

/// Only the two settings `decide` needs (Req 5.2, 5.3): the worker key and
/// every other setting stay in the dropped `Config`.
fn settings_from(config: &Config) -> HookSettings {
    HookSettings {
        shunt_min_lines: config.shunt_min_lines,
        worker_configured: config.worker_model.is_some(),
    }
}

/// Whether a worker model is set up, read from the same configuration and
/// with the same rule `respond` uses (the installer's hook check asks this,
/// since without a worker the hook lets every read through). A configuration
/// that cannot load counts as none, as it does for the hook.
pub fn worker_configured() -> bool {
    Config::load_local().is_ok_and(|config| settings_from(&config).worker_configured)
}

/// `decide`, then the repeat window for a redirect (Req 2.5): a repeat
/// passes; otherwise the redirect is remembered (errors ignored).
fn decide_call(
    call: &ParsedCall,
    req: &ReadRequest,
    settings: &HookSettings,
    window: &RepeatWindow,
    now_ms: u64,
) -> Decision {
    let decision = decide(req, settings, Path::new(&call.cwd));
    if !matches!(decision, Decision::Redirect { .. }) {
        return decision;
    }
    let harness = call.harness.name();
    let path = req.path.to_string_lossy();
    if window.is_repeat(harness, &call.session_id, &path, now_ms) {
        return Decision::Pass(PassReason::Repeat);
    }
    let _ = window.remember(harness, &call.session_id, &path, now_ms);
    decision
}

/// One audit event per read decision, never the file content (Req 5.4);
/// failures are ignored (fail open).
fn record(writer: &AuditWriter, call: &ParsedCall, req: &ReadRequest, decision: &Decision) {
    let (event, details) = audit_details(req, decision);
    let project = project_name(&call.cwd);
    let _ = writer.write_event(event, call.harness.agent_harness(), project, details);
}

/// Event type and `{"decision","reason","tool","file_path","lines"}`, plus
/// `"range"` when the read named one (shunt-cache-efficiency task 3.2).
fn audit_details(req: &ReadRequest, decision: &Decision) -> (AuditEventType, Value) {
    let (event, verdict, reason, lines) = match decision {
        Decision::Redirect { lines } => (
            AuditEventType::HookRedirect,
            "redirect",
            "large_file",
            json!(lines),
        ),
        Decision::Pass(reason) => (
            AuditEventType::HookPass,
            "pass",
            reason_name(*reason),
            Value::Null,
        ),
    };
    let mut details = json!({
        "decision": verdict,
        "reason": reason,
        "tool": req.tool,
        "file_path": req.path.display().to_string(),
        "lines": lines,
    });
    if let Some(range) = &req.range {
        details["range"] = range_details(range);
    }
    (event, details)
}

/// `{"start", "end"}` (`end` null: to the end of the file), or `{"last": n}`
/// for the last `n` lines.
fn range_details(range: &LineRange) -> Value {
    match range.end {
        Some(n) if range.is_tail() => json!({"last": n}),
        end => json!({"start": range.start, "end": end}),
    }
}

/// Stable snake_case name of a pass reason for the audit record.
fn reason_name(reason: PassReason) -> &'static str {
    match reason {
        PassReason::Range => "range",
        PassReason::Short => "short",
        PassReason::SecurityCritical => "security_critical",
        PassReason::NotText => "not_text",
        PassReason::Missing => "missing",
        PassReason::NoWorker => "no_worker",
        PassReason::Repeat => "repeat",
        PassReason::NotARead => "not_a_read",
    }
}

/// Basename of the harness `cwd`, or `unknown` when it has none.
fn project_name(cwd: &str) -> &str {
    Path::new(cwd)
        .file_name()
        .and_then(|n| n.to_str())
        .filter(|n| !n.is_empty())
        .unwrap_or(UNKNOWN_PROJECT)
}

/// Best-effort `ERROR` record after a panic, without any tool input
/// (design.md "Error Handling", Req 3.2).
fn audit_internal_error(harness: Option<&str>) {
    let Ok(config) = Config::load_local() else {
        return;
    };
    let name = harness
        .and_then(parse_harness)
        .map_or("unknown", Harness::agent_harness);
    let details = json!({"component": "hook", "error": "panic", "decision": "pass"});
    let _ = AuditWriter::from_config(&config).write_event(
        AuditEventType::Error,
        name,
        UNKNOWN_PROJECT,
        details,
    );
}

/// Wall-clock epoch milliseconds (0 if the clock is before 1970).
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::collections::HashMap;
    use std::path::PathBuf;

    fn config(pairs: &[(&str, &str)]) -> Config {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        Config::from_env_local(&|k: &str| map.get(k).cloned()).expect("config")
    }

    #[test]
    fn settings_take_only_threshold_and_worker_presence() {
        let base = [("AGENT_MEM_DATA_DIR", "/data"), ("WORKER_API_KEY", "sk-x")];
        let none = settings_from(&config(&base));
        assert_eq!(none.shunt_min_lines, crate::config::DEFAULT_SHUNT_MIN_LINES);
        assert!(!none.worker_configured);
        let mut armed = base.to_vec();
        armed.extend([("WORKER_MODEL", "m"), ("SHUNT_MIN_LINES", "40")]);
        let armed = settings_from(&config(&armed));
        assert_eq!((armed.shunt_min_lines, armed.worker_configured), (40, true));
        let blank = [("AGENT_MEM_DATA_DIR", "/data"), ("WORKER_MODEL", "  ")];
        assert!(!settings_from(&config(&blank)).worker_configured);
    }

    #[test]
    fn project_name_is_the_cwd_basename_or_unknown() {
        let cwd = Path::new("work").join("proj");
        assert_eq!(project_name(cwd.to_str().unwrap()), "proj");
        assert_eq!(project_name(""), "unknown");
        assert_eq!(project_name("/"), "unknown");
    }

    #[test]
    fn a_panic_passes_and_runs_the_error_hook() {
        let called = Cell::new(false);
        let out = guarded(|| panic!("boom"), || called.set(true));
        assert!(out.is_empty());
        assert!(called.get());
        let both = guarded(|| panic!("boom"), || panic!("again"));
        assert!(both.is_empty());
        assert_eq!(guarded(|| "ok".to_string(), || unreachable!()), "ok");
    }

    #[test]
    fn stdin_is_read_up_to_the_cap() {
        let at_cap = vec![b'a'; 16];
        assert_eq!(read_bounded(&mut at_cap.as_slice(), 16), Some(at_cap));
        assert_eq!(read_bounded(&mut [b'a'; 17].as_slice(), 16), None);
        assert_eq!(read_bounded(&mut [].as_slice(), 16), Some(Vec::new()));
    }

    #[test]
    fn harness_names_and_reason_names_are_stable() {
        assert_eq!(parse_harness("claude"), Some(Harness::Claude));
        assert_eq!(parse_harness("Claude"), None);
        assert_eq!(parse_harness("codex"), Some(Harness::Codex));
        assert_eq!(parse_harness("Codex"), None);
        assert_eq!(parse_harness("opencode"), Some(Harness::OpenCode));
        assert_eq!(parse_harness("antigravity"), Some(Harness::Antigravity));
        assert_eq!(parse_harness("Antigravity"), None);
        assert_eq!(parse_harness("agy"), None);
        // The CLI argument stays (Req 4.8); audit records carry the
        // agent_harness each harness passes to smart_read (task 5.2 joins them).
        assert_eq!(Harness::Claude.agent_harness(), "claude-code");
        assert_eq!(Harness::Codex.agent_harness(), "codex");
        assert_eq!(Harness::Claude.name(), "claude");
        assert_eq!(
            reason_name(PassReason::SecurityCritical),
            "security_critical"
        );
        assert_eq!(reason_name(PassReason::NoWorker), "no_worker");
    }

    #[test]
    fn only_antigravity_prints_a_pass_when_no_decision_was_rendered() {
        // antigravity-native-setup task 2.1: empty stdout denies Antigravity's
        // tool call; every other harness (and an unknown name) keeps no output.
        assert_eq!(
            no_decision_output(Some("antigravity")),
            r#"{"decision":"ask"}"#
        );
        for other in [
            Some("claude"),
            Some("codex"),
            Some("opencode"),
            Some("x"),
            None,
        ] {
            assert_eq!(no_decision_output(other), "", "{other:?}");
        }
    }

    #[test]
    fn audit_records_the_range_of_ranged_reads_on_both_outcomes() {
        let req = |range| ReadRequest {
            path: PathBuf::from("/w/a.rs"),
            range,
            tool: "Read".to_string(),
        };
        let pass = Decision::Pass(PassReason::Range);
        let redirect = Decision::Redirect { lines: 300 };
        let closed = Some(LineRange::new(10, Some(109)));
        let (_, details) = audit_details(&req(closed), &pass);
        assert_eq!(details["range"], json!({"start": 10, "end": 109}));
        let open = Some(LineRange::new(2, None));
        let (_, details) = audit_details(&req(open), &redirect);
        assert_eq!(details["range"], json!({"start": 2, "end": null}));
        let tail = Some(LineRange::from_tail(50));
        let (_, details) = audit_details(&req(tail), &pass);
        assert_eq!(details["range"], json!({"last": 50}));
        let (_, details) = audit_details(&req(None), &redirect);
        assert!(details.get("range").is_none(), "{details}");
    }

    #[test]
    fn redirect_is_remembered_and_the_repeat_passes() {
        // A temp path whose random suffix never trips the security rule.
        let dir = loop {
            let dir = tempfile::tempdir().expect("tempdir");
            if !crate::services::shunt::is_security_critical_path(dir.path()) {
                break dir;
            }
        };
        let window = RepeatWindow::with_default_window(dir.path());
        let call = ParsedCall {
            harness: Harness::Claude,
            session_id: "s".to_string(),
            cwd: String::new(),
            tool: "Read".to_string(),
            read: None,
            command: None,
        };
        let file = dir.path().join("plain.txt");
        std::fs::write(&file, "x\n".repeat(20)).expect("write");
        let req = ReadRequest {
            path: PathBuf::from(&file),
            range: None,
            tool: "Read".to_string(),
        };
        let settings = HookSettings {
            shunt_min_lines: 5,
            worker_configured: true,
        };
        let first = decide_call(&call, &req, &settings, &window, 1_000);
        assert_eq!(first, Decision::Redirect { lines: 5 });
        let again = decide_call(&call, &req, &settings, &window, 2_000);
        assert_eq!(again, Decision::Pass(PassReason::Repeat));
        let later = decide_call(&call, &req, &settings, &window, 1_000_000);
        assert_eq!(later, Decision::Redirect { lines: 5 });
    }
}
