//! Antigravity `PreToolUse` input and output mapping (antigravity-native-setup
//! design.md "Antigravity hook adapter (`src/hook/antigravity.rs`)", "Hook
//! call from Antigravity"; Req 2.2, 2.3, 2.4).
//!
//! Observed with agy 1.5.3 (research.md "Antigravity hooks contract",
//! payloads in `tests/fixtures/hook/antigravity/`): stdin is one camelCase
//! object with `conversationId`, `workspacePaths` and `toolCall` `{name,
//! args}`; the file-read tool is `read_file` with `Path` and an optional
//! `StartLine`/`EndLine`. Stdout must be one JSON object, because empty
//! output or `{}` denies the call: a pass is `{"decision":"ask"}`, never
//! `allow` (which would skip Antigravity's own permission prompts). Input is
//! untrusted: anything missing or of the wrong type is not a read.

use super::adapters::{Harness, ParsedCall, ascii_json, load_input};
use super::shell::resolve;
use super::{Decision, LineRange, ReadRequest};
use serde_json::{Value, json};
use std::path::Path;

/// The pass answer (research.md "Decision: Pass output"), also printed when
/// the hook reaches no decision.
pub(super) const PASS_OUTPUT: &str = r#"{"decision":"ask"}"#;

/// Antigravity's file-read tool.
const READ_TOOL: &str = "read_file";

/// Antigravity `PreToolUse` input: `conversationId` (session),
/// `workspacePaths[0]` (working folder; missing or empty means not a read)
/// and `toolCall`; every other field is ignored.
pub(super) fn parse(input: &[u8]) -> ParsedCall {
    let value = load_input(input);
    let session_id = text(value.get("conversationId"));
    let cwd = text(value.get("workspacePaths").and_then(|w| w.get(0)));
    let tool_call = value.get("toolCall").unwrap_or(&Value::Null);
    let tool = text(tool_call.get("name"));
    let args = tool_call.get("args").unwrap_or(&Value::Null);
    let read = match tool {
        READ_TOOL if !cwd.is_empty() => read_file(args, cwd),
        _ => None,
    };
    ParsedCall {
        harness: Harness::Antigravity,
        session_id: session_id.to_string(),
        cwd: cwd.to_string(),
        tool: tool.to_string(),
        read,
        command: None,
    }
}

/// A string field's text; empty when missing or of another type.
fn text(value: Option<&Value>) -> &str {
    value.and_then(Value::as_str).unwrap_or("")
}

/// `read_file`: `Path` in either slash style (a relative one joins `cwd`).
/// Either `StartLine` or `EndLine` makes it a ranged read from `StartLine`
/// (or 1) to `EndLine`, to the end when `EndLine` is absent or 0; only
/// non-negative integers count, and a range key of another type makes it
/// not a read.
fn read_file(args: &Value, cwd: &str) -> Option<ReadRequest> {
    let path = resolve(args.get("Path")?.as_str()?, Path::new(cwd))?;
    let line = |key: &str| match args.get(key) {
        None => Some(None),
        Some(value) => value.as_u64().map(Some),
    };
    let (start, end) = (line("StartLine")?, line("EndLine")?);
    let range = (start.is_some() || end.is_some())
        .then(|| LineRange::new(start.unwrap_or(1), end.filter(|&n| n > 0)));
    Some(ReadRequest {
        path,
        range,
        tool: READ_TOOL.to_string(),
    })
}

/// Redirect: `{"decision":"deny","reason":...}` naming the `smart_read`
/// call, ASCII only (the reason reaches the agent as "tool call denied by
/// pre-tool hook: <reason>"); pass: [`PASS_OUTPUT`].
pub(super) fn render(decision: &Decision, path: &Path, project: &str, threshold: u32) -> String {
    let Decision::Redirect { lines } = decision else {
        return PASS_OUTPUT.to_string();
    };
    let reason = format!(
        "mmhmem: {path} has at least {lines} lines. Read it through the mmhmem MCP tool \
         instead: call smart_read with project \"{project}\", agent_harness \"{harness}\" \
         and file_path \"{path}\" for a summary with exact line ranges. For exact text, for \
         example before an edit, use read_file with StartLine and EndLine for a range under \
         {threshold} lines around what you need, or ask smart_read for a slice with offset \
         and limit. Only if the summary is not enough, repeat this full read within 2 \
         minutes.",
        path = path.display(),
        harness = Harness::Antigravity.agent_harness(),
    );
    ascii_json(&json!({"decision": "deny", "reason": reason}).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hook::adapters::{self, MAX_INPUT_BYTES};
    use crate::hook::{HookSettings, PassReason, ReadRequest};
    use serde_json::{Value, json};
    use std::path::PathBuf;

    const T: u32 = crate::config::DEFAULT_SHUNT_MIN_LINES;

    /// A payload captured live from agy 1.5.3 (task 1.1; see the README in
    /// `tests/fixtures/hook/antigravity/`).
    fn fixture(name: &str) -> Vec<u8> {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/hook/antigravity")
            .join(format!("pretooluse-{name}.json"));
        std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
    }

    fn call(input: &[u8]) -> ParsedCall {
        adapters::parse(Harness::Antigravity, input)
    }

    /// The captured shape with only the tool call replaced.
    fn event(tool: &str, args: Value) -> Vec<u8> {
        json!({"conversationId":"c-1","workspacePaths":["D:/work/ws"],"modelName":"m",
               "toolCall":{"name":tool,"args":args},"stepIdx":2})
        .to_string()
        .into_bytes()
    }

    fn read_of(tool: &str, args: Value) -> Option<ReadRequest> {
        call(&event(tool, args)).read
    }

    #[test]
    fn captured_whole_file_reads_are_full_requests() {
        for (name, path) in [
            ("read_file-whole", r"D:\work\ws\notes.txt"),
            ("read_file-whole-forward-slashes", "D:/work/ws/notes.txt"),
        ] {
            let p = call(&fixture(name));
            assert_eq!(p.harness, Harness::Antigravity);
            assert_eq!(p.session_id, "<conversation-id>", "{name}");
            assert_eq!(p.cwd, "D:/work/ws", "{name}");
            assert_eq!(p.tool, "read_file", "{name}");
            let r = p.read.expect(name);
            assert_eq!(r.path, PathBuf::from(path), "{name}");
            assert_eq!((r.range, r.tool.as_str()), (None, "read_file"), "{name}");
        }
    }

    #[test]
    fn captured_ranged_read_is_a_ranged_request_that_passes() {
        let r = call(&fixture("read_file-ranged")).read.expect("read");
        assert_eq!(r.path, PathBuf::from(r"D:\work\ws\notes.txt"));
        assert_eq!(r.range, Some(LineRange::new(10, Some(20))));
        let settings = HookSettings {
            shunt_min_lines: T,
            worker_configured: true,
        };
        let decision = crate::hook::decide(&r, &settings, Path::new("D:/work/ws"));
        assert_eq!(decision, Decision::Pass(PassReason::Range));
    }

    #[test]
    fn one_range_key_is_a_ranged_read_and_relative_paths_join_the_workspace() {
        let range = |args: Value| read_of("read_file", args).expect("read").range;
        let lines = |a, b| Some(LineRange::new(a, b));
        let open = range(json!({"Path":"D:/w/a.rs","StartLine":5}));
        assert_eq!(open, lines(5, None), "no EndLine reads to the end");
        assert_eq!(
            range(json!({"Path":"D:/w/a.rs","EndLine":7})),
            lines(1, Some(7))
        );
        let zero_end = json!({"Path":"D:/w/a.rs","StartLine":5,"EndLine":0});
        assert_eq!(range(zero_end), lines(5, None));
        let zero_start = json!({"Path":"D:/w/a.rs","StartLine":0,"EndLine":9});
        assert_eq!(range(zero_start), lines(1, Some(9)));
        let rel = read_of("read_file", json!({"Path":"src/a.rs"})).expect("read");
        assert_eq!(rel.path, Path::new("D:/work/ws").join("src/a.rs"));
        assert_eq!(rel.range, None);
    }

    #[test]
    fn captured_other_tools_are_not_reads() {
        for (name, tool) in [
            ("call_mcp_tool-smart_read", "call_mcp_tool"),
            ("run_command", "run_command"),
        ] {
            let p = call(&fixture(name));
            assert!(p.read.is_none(), "{name}");
            assert_eq!((p.tool.as_str(), p.cwd.as_str()), (tool, "D:/work/ws"));
            assert_eq!(p.session_id, "<conversation-id>");
        }
        let args = json!({"Path":"D:/w/a.rs","CommandLine":"cat a.rs"});
        for tool in [
            "view_file",
            "Read",
            "READ_FILE",
            "write_to_file",
            "grep_search",
        ] {
            assert!(read_of(tool, args.clone()).is_none(), "{tool}");
        }
    }

    #[test]
    fn malformed_input_is_not_a_read() {
        let path = "D:/w/a.rs";
        let mut cases: Vec<Vec<u8>> =
            [&b""[..], b"not json", b"[]", b"null", b"{", b"\xff\xfe\x00"]
                .iter()
                .map(|c| c.to_vec())
                .collect();
        for args in [
            json!({}),
            json!(path),
            json!({"Path":5}),
            json!({"Path":""}),
            json!({"Path":path,"StartLine":"10"}),
            json!({"Path":path,"EndLine":-1}),
            json!({"Path":path,"StartLine":1.5}),
            json!({"Path":path,"EndLine":null}),
        ] {
            cases.push(event("read_file", args));
        }
        let tool_call = json!({"name":"read_file","args":{"Path":path}});
        for workspaces in [
            json!(null),
            json!([]),
            json!([""]),
            json!([5]),
            json!("D:/w"),
        ] {
            let v = json!({"conversationId":"c","workspacePaths":workspaces,"toolCall":tool_call});
            cases.push(v.to_string().into_bytes());
        }
        cases.push(
            json!({"workspacePaths":["D:/w"],"toolCall":{"name":7}})
                .to_string()
                .into(),
        );
        cases.push(
            json!({"workspacePaths":["D:/w"],"toolCall":"read_file"})
                .to_string()
                .into(),
        );
        for c in &cases {
            assert!(call(c).read.is_none(), "{}", String::from_utf8_lossy(c));
        }
    }

    #[test]
    fn oversized_input_is_not_a_read() {
        let mut v: Value = serde_json::from_slice(&fixture("read_file-whole")).expect("json");
        assert!(call(v.to_string().as_bytes()).read.is_some());
        v["pad"] = json!("x".repeat(MAX_INPUT_BYTES));
        assert!(call(v.to_string().as_bytes()).read.is_none());
    }

    fn reason_of(out: &str) -> String {
        assert!(!out.contains('\n'), "{out}");
        let v: Value = serde_json::from_str(out).expect("one JSON object");
        assert_eq!(v.as_object().expect("object").len(), 2, "{out}");
        assert_eq!(v["decision"], "deny");
        v["reason"].as_str().expect("reason").to_string()
    }

    #[test]
    fn redirect_renders_a_deny_naming_the_smart_read_call() {
        let path = Path::new(r"D:\work\ws\notes.txt");
        let d = Decision::Redirect {
            lines: u64::from(T),
        };
        let out = adapters::render(Harness::Antigravity, &d, path, "ws", T);
        assert!(out.is_ascii(), "{out}");
        let reason = reason_of(&out);
        let head = format!("mmhmem: {} has at least {T} lines.", path.display());
        assert!(reason.starts_with(&head), "{reason}");
        let smart_read = format!(
            "smart_read with project \"ws\", agent_harness \"antigravity\" and file_path \"{}\"",
            path.display()
        );
        assert!(reason.contains(&smart_read), "{reason}");
        assert!(!reason.contains("claude-code"), "{reason}");
        let tool = "read_file with StartLine and EndLine";
        adapters::assert_redirect_text(&reason, T, tool);
    }

    #[test]
    fn redirect_output_is_ascii_and_round_trips_non_ascii_paths() {
        let path = Path::new("D:/work/Übersicht \"q\" 😀.rs");
        let out = render(&Decision::Redirect { lines: 200 }, path, "projé", 200);
        assert!(out.is_ascii(), "{out}");
        let reason = reason_of(&out);
        assert!(reason.contains("D:/work/Übersicht \"q\" 😀.rs"), "{reason}");
        assert!(reason.contains("project \"projé\""), "{reason}");
    }

    #[test]
    fn every_pass_renders_ask_and_never_allow() {
        for reason in [
            PassReason::Range,
            PassReason::Short,
            PassReason::SecurityCritical,
            PassReason::NotText,
            PassReason::Missing,
            PassReason::NoWorker,
            PassReason::Repeat,
            PassReason::NotARead,
        ] {
            let d = Decision::Pass(reason);
            let out = adapters::render(Harness::Antigravity, &d, Path::new("/x"), "p", T);
            assert_eq!(out, r#"{"decision":"ask"}"#, "{reason:?}");
        }
        assert_eq!(
            adapters::fail_open_output(Harness::Antigravity),
            r#"{"decision":"ask"}"#
        );
        for other in [Harness::Claude, Harness::Codex, Harness::OpenCode] {
            assert_eq!(adapters::fail_open_output(other), "", "{other:?}");
        }
    }

    #[test]
    fn harness_names_are_antigravity() {
        assert_eq!(Harness::Antigravity.name(), "antigravity");
        assert_eq!(Harness::Antigravity.agent_harness(), "antigravity");
    }
}
