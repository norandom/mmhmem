//! Codex `PreToolUse` input and output mapping (read-routing-hooks design.md
//! "adapters (`src/hook/adapters.rs`)", codex row; Req 2.6, 2.7, 5.5, 6.2).
//!
//! Codex has no native read tool: reads are shell commands. Observed with
//! codex-cli 0.166.0 on Windows, the shell tool arrives as `tool_name`
//! `"Bash"` with `tool_input.command` holding the script as a string; an
//! argv array such as `["bash", "-lc", "cat x"]` is unwrapped to its script.
//! Input is untrusted: anything missing or of the wrong type is not a read.

use super::adapters::{Harness, ParsedCall, ascii_json, deny_output, load_input};
use super::shell::{recognise, resolve, unwrap_invocation};
use super::{Decision, ReadRequest};
use serde_json::Value;
use std::path::Path;

/// Codex `PreToolUse` input: `tool_name`, `tool_input`, `cwd`, `session_id`.
pub(super) fn parse(input: &[u8]) -> ParsedCall {
    let value = load_input(input);
    let text = |key: &str| value.get(key).and_then(Value::as_str).unwrap_or("");
    let (session_id, cwd, tool) = (text("session_id"), text("cwd"), text("tool_name"));
    let tool_input = value.get("tool_input").unwrap_or(&Value::Null);
    let read = match tool {
        "Bash" | "shell" => shell_read(tool_input, cwd, tool),
        _ => None,
    };
    ParsedCall {
        harness: Harness::Codex,
        session_id: session_id.to_string(),
        cwd: cwd.to_string(),
        tool: tool.to_string(),
        read,
    }
}

/// A read only if the recogniser accepts the script, with the range of a
/// partial reader (`sed -n A,Bp`, `head`, `tail`); an empty `cwd` is not a
/// read.
fn shell_read(input: &Value, cwd: &str, tool: &str) -> Option<ReadRequest> {
    if cwd.is_empty() {
        return None;
    }
    let script = command_script(input.get("command")?)?;
    let dir = working_dir(input.get("workdir"), Path::new(cwd))?;
    let read = recognise(&script, &dir)?;
    Some(ReadRequest {
        path: read.path,
        range: read.range,
        tool: tool.to_string(),
    })
}

/// The script of a string command, or of a shell-invocation argv.
fn command_script(command: &Value) -> Option<String> {
    match command {
        Value::String(script) => Some(script.clone()),
        Value::Array(items) => {
            let argv: Option<Vec<String>> = items
                .iter()
                .map(|v| v.as_str().map(str::to_owned))
                .collect();
            unwrap_invocation(&argv?)
        }
        _ => None,
    }
}

/// Where the shell tool runs: an optional `workdir` resolved against `cwd`
/// (as the tool would), else `cwd`; a `workdir` of another type is `None`.
fn working_dir(workdir: Option<&Value>, cwd: &Path) -> Option<std::path::PathBuf> {
    match workdir {
        None | Some(Value::Null) => Some(cwd.to_path_buf()),
        Some(Value::String(dir)) => resolve(dir, cwd),
        Some(_) => None,
    }
}

/// Redirect: the documented deny JSON with a Codex-specific reason, ASCII
/// only; pass: no output.
pub(super) fn render(decision: &Decision, path: &Path, project: &str, threshold: u32) -> String {
    let Decision::Redirect { lines } = decision else {
        return String::new();
    };
    let reason = format!(
        "mmhmem: {path} has at least {lines} lines. Read it through the mmhmem MCP tool \
         instead: call smart_read with project \"{project}\", agent_harness \"codex\" and \
         file_path \"{path}\" for a summary with exact line ranges. For exact text, for \
         example before an edit, read a range under {threshold} lines around what you need \
         with a ranged shell read such as sed -n 'A,Bp' \"{path}\" or Get-Content \
         -LiteralPath \"{path}\" -TotalCount B | Select-Object -Skip (A-1) (or -Tail N for \
         the last lines), or ask smart_read for a slice with offset and limit. Only if the \
         summary is not enough, repeat this full read within 2 minutes.",
        path = path.display()
    );
    ascii_json(&deny_output(reason).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hook::adapters;
    use crate::hook::{LineRange, PassReason};
    use serde_json::{Value, json};
    use std::path::PathBuf;

    const T: u32 = crate::config::DEFAULT_SHUNT_MIN_LINES;

    /// The event shape observed from codex-cli 0.166.0 (extra fields kept).
    fn event(tool: &str, input: Value) -> Value {
        json!({"session_id":"s1","turn_id":"t1","transcript_path":"/x.jsonl",
               "cwd":"/work","hook_event_name":"PreToolUse","model":"m",
               "permission_mode":"bypassPermissions","tool_name":tool,
               "tool_input":input,"tool_use_id":"exec-1"})
    }

    fn call(v: &Value) -> ParsedCall {
        adapters::parse(Harness::Codex, v.to_string().as_bytes())
    }

    fn read_of(v: &Value) -> Option<ReadRequest> {
        call(v).read
    }

    #[test]
    fn bash_string_command_is_a_whole_file_read() {
        let p = call(&event("Bash", json!({"command":"cat big.rs"})));
        assert_eq!((p.harness, p.tool.as_str()), (Harness::Codex, "Bash"));
        assert_eq!((p.session_id.as_str(), p.cwd.as_str()), ("s1", "/work"));
        let r = p.read.expect("read");
        assert_eq!(r.path, Path::new("/work").join("big.rs"));
        assert_eq!((r.range, r.tool.as_str()), (None, "Bash"));
        let ps = read_of(&event("Bash", json!({"command":"Get-Content -Path x.rs"})));
        assert_eq!(ps.unwrap().path, Path::new("/work").join("x.rs"));
        let legacy = read_of(&event("shell", json!({"command":"type notes.txt"})));
        assert_eq!(legacy.unwrap().tool, "shell");
    }

    #[test]
    fn argv_commands_are_unwrapped() {
        let bash = json!({"command":["bash","-lc","cat 'a b.txt'"]});
        let r = read_of(&event("Bash", bash)).expect("read");
        assert_eq!(r.path, Path::new("/work").join("a b.txt"));
        let pwsh = json!({"command":["pwsh","-NoProfile","-Command","gc big.rs"]});
        assert!(read_of(&event("Bash", pwsh)).is_some());
        for argv in [
            json!(["cat", "big.rs"]),
            json!(["bash", "-lc", "cat a | wc -l"]),
            json!(["bash", "-lc", 7]),
            json!([]),
        ] {
            let e = event("Bash", json!({"command": argv}));
            assert!(read_of(&e).is_none(), "{e}");
        }
    }

    #[test]
    fn sized_partial_reads_carry_their_range() {
        let range = |cmd: &str| read_of(&event("Bash", json!({"command": cmd}))).map(|r| r.range);
        let lines = |a, b| Some(Some(LineRange::new(a, b)));
        assert_eq!(range("sed -n 1,400p big.rs"), lines(1, Some(400)));
        assert_eq!(range("sed -n '1,20p' big.rs"), lines(1, Some(20)));
        assert_eq!(range("head -n 50 big.rs"), lines(1, Some(50)));
        assert_eq!(range("tail -n +2 big.rs"), lines(2, None), "open-ended");
        let tail = Some(Some(LineRange::from_tail(10)));
        assert_eq!(range("tail big.rs"), tail);
        let argv = json!({"command":["bash","-lc","tail -n 50 big.rs"]});
        let r = read_of(&event("Bash", argv)).expect("read");
        assert_eq!(r.range, Some(LineRange::from_tail(50)));
        assert_eq!(range("cat big.rs"), Some(None), "whole-file read");
    }

    #[test]
    fn unsized_filtered_and_compound_commands_pass() {
        for cmd in [
            "head -n x big.rs",
            "tail -f big.rs",
            "sed -n '1,20p;30q' big.rs",
            "Get-Content big.rs -TotalCount 20",
            "cat big.rs | grep x",
            "cat a.rs b.rs",
            "cat big.rs && ls",
            "rg foo",
        ] {
            assert!(
                read_of(&event("Bash", json!({"command": cmd}))).is_none(),
                "{cmd}"
            );
        }
    }

    #[test]
    fn workdir_is_resolved_like_the_shell_tool() {
        let rel = json!({"command":"cat a.rs","workdir":"sub"});
        let r = read_of(&event("Bash", rel)).expect("read");
        assert_eq!(r.path, Path::new("/work").join("sub").join("a.rs"));
        let abs = json!({"command":"cat a.rs","workdir":"/other"});
        let r = read_of(&event("Bash", abs)).expect("read");
        assert_eq!(r.path, Path::new("/other").join("a.rs"));
        let absolute_file = json!({"command":"cat /etc/x.txt","workdir":"sub"});
        let r = read_of(&event("Bash", absolute_file)).expect("read");
        assert_eq!(r.path, PathBuf::from("/etc/x.txt"));
        let bad = json!({"command":"cat a.rs","workdir":5});
        assert!(read_of(&event("Bash", bad)).is_none());
    }

    #[test]
    fn other_tools_and_malformed_input_are_not_reads() {
        let input = json!({"command":"cat a.rs","file_path":"/work/a.rs"});
        for tool in [
            "apply_patch",
            "Edit",
            "Write",
            "Read",
            "bash",
            "mcp__mmhmem__smart_read",
        ] {
            let p = call(&event(tool, input.clone()));
            assert!(p.read.is_none(), "{tool}");
            assert_eq!(p.tool, tool);
        }
        let cases: Vec<&[u8]> = vec![
            b"",
            b"not json",
            b"[]",
            b"{\"tool_name\":\"Bash\"}",
            b"{\"tool_name\":\"Bash\",\"tool_input\":{\"command\":5}}",
            b"{\"tool_name\":\"Bash\",\"tool_input\":{\"command\":\"cat a.rs\"}}",
            b"\xff\xfe\x00",
        ];
        for c in cases {
            assert!(adapters::parse(Harness::Codex, c).read.is_none(), "{c:?}");
        }
        let mut big = event("Bash", json!({"command":"cat a.rs"}));
        big["pad"] = json!("x".repeat(adapters::MAX_INPUT_BYTES));
        assert!(call(&big).read.is_none());
    }

    fn reason_of(out: &str) -> String {
        let v: Value = serde_json::from_str(out).expect("valid json");
        assert_eq!(v.as_object().unwrap().len(), 1);
        let h = &v["hookSpecificOutput"];
        assert_eq!(h["hookEventName"], "PreToolUse");
        assert_eq!(h["permissionDecision"], "deny");
        h["permissionDecisionReason"].as_str().unwrap().to_string()
    }

    #[test]
    fn redirect_names_the_codex_smart_read_call_and_ranged_shell_reads() {
        let path = Path::new("/work/big notes.rs");
        let d = Decision::Redirect {
            lines: u64::from(T),
        };
        let out = adapters::render(Harness::Codex, &d, path, "proj", T);
        assert!(!out.contains('\n'));
        let reason = reason_of(&out);
        let head = format!("mmhmem: /work/big notes.rs has at least {T} lines.");
        assert!(reason.starts_with(&head), "{reason}");
        let call = "smart_read with project \"proj\", agent_harness \"codex\" and \
                    file_path \"/work/big notes.rs\"";
        assert!(reason.contains(call), "{reason}");
        assert!(
            reason.contains("sed -n 'A,Bp' \"/work/big notes.rs\""),
            "{reason}"
        );
        assert!(reason.contains("-TotalCount"), "{reason}");
        assert!(reason.contains("-Tail"), "{reason}");
        assert!(!reason.contains("claude-code"), "{reason}");
        adapters::assert_redirect_text(&reason, T, "sed -n");
    }

    #[test]
    fn redirect_output_is_ascii_and_round_trips_non_ascii_paths() {
        let path = Path::new("/work/Übersicht \"q\" 😀.rs");
        let out = render(&Decision::Redirect { lines: 200 }, path, "projé", 200);
        assert!(out.is_ascii(), "{out}");
        let reason = reason_of(&out);
        assert!(reason.contains("/work/Übersicht \"q\" 😀.rs"), "{reason}");
        assert!(reason.contains("project \"projé\""), "{reason}");
    }

    #[test]
    fn pass_renders_nothing() {
        for reason in [PassReason::Short, PassReason::Repeat, PassReason::Range] {
            let d = Decision::Pass(reason);
            assert_eq!(
                adapters::render(Harness::Codex, &d, Path::new("/x"), "p", T),
                ""
            );
        }
    }

    #[test]
    fn harness_name_is_codex() {
        assert_eq!(Harness::Codex.name(), "codex");
    }
}
