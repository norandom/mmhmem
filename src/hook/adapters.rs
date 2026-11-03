//! Per-harness hook input and output mapping (read-routing-hooks design.md
//! "adapters (`src/hook/adapters.rs`)", Req 2.6, 2.7, 5.5, 6.1).
//!
//! Input is untrusted: unknown fields are ignored and anything missing or of
//! the wrong type means "not a read", so the hook passes. Only Claude Code is
//! mapped so far; other harnesses add a [`Harness`] variant and a branch in
//! [`parse`] and [`render`].

use super::{Decision, ReadRequest};
use serde_json::{Value, json};
use std::path::Path;

/// Stdin larger than this is never parsed (treated as not a read).
pub const MAX_INPUT_BYTES: usize = 1024 * 1024;

/// The harness whose hook protocol is being spoken.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Harness {
    Claude,
}

impl Harness {
    /// Stable lowercase name, used for state keys and audit records.
    pub fn name(self) -> &'static str {
        match self {
            Harness::Claude => "claude",
        }
    }
}

/// One parsed pre-tool event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedCall {
    pub harness: Harness,
    pub session_id: String,
    pub cwd: String,
    pub tool: String,
    /// `Some` only for a recognised read; `None` means not a read.
    pub read: Option<ReadRequest>,
}

/// Parses raw stdin for `harness`; over [`MAX_INPUT_BYTES`] or malformed
/// input yields a call with no read.
pub fn parse(harness: Harness, input: &[u8]) -> ParsedCall {
    match harness {
        Harness::Claude => parse_claude(input),
    }
}

/// Renders a decision as the harness's stdout; a pass is the empty string.
pub fn render(harness: Harness, decision: &Decision, path: &Path, project: &str) -> String {
    match harness {
        Harness::Claude => render_claude(decision, path, project),
    }
}

/// Claude Code `PreToolUse` input: `tool_name`, `tool_input`, `cwd`,
/// `session_id`; every other field is ignored.
fn parse_claude(input: &[u8]) -> ParsedCall {
    let value: Value = if input.len() > MAX_INPUT_BYTES {
        Value::Null
    } else {
        serde_json::from_slice(input).unwrap_or(Value::Null)
    };
    let text = |key: &str| value.get(key).and_then(Value::as_str).unwrap_or("");
    let (session_id, cwd, tool) = (text("session_id"), text("cwd"), text("tool_name"));
    let tool_input = value.get("tool_input").unwrap_or(&Value::Null);
    let read = match tool {
        "Read" => claude_read(tool_input, cwd),
        "Bash" | "PowerShell" => claude_shell(tool_input, cwd, tool),
        _ => None,
    };
    ParsedCall {
        harness: Harness::Claude,
        session_id: session_id.to_string(),
        cwd: cwd.to_string(),
        tool: tool.to_string(),
        read,
    }
}

/// Native `Read`. Either `offset` or `limit` makes it a ranged read:
/// `(offset or 1, limit or 0)` where 0 means "to the end". Only
/// non-negative integers count; other types are ignored. A relative
/// `file_path` (Claude Code sends absolute ones) joins `cwd`, and with no
/// `cwd` it is not a read.
fn claude_read(input: &Value, cwd: &str) -> Option<ReadRequest> {
    let file = input.get("file_path")?.as_str().filter(|s| !s.is_empty())?;
    let path = Path::new(file);
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else if cwd.is_empty() {
        return None;
    } else {
        Path::new(cwd).join(path)
    };
    let num = |key: &str| input.get(key).and_then(Value::as_u64);
    let (offset, limit) = (num("offset"), num("limit"));
    let range =
        (offset.is_some() || limit.is_some()).then(|| (offset.unwrap_or(1), limit.unwrap_or(0)));
    Some(ReadRequest {
        path,
        range,
        tool: "Read".to_string(),
    })
}

/// `Bash` / `PowerShell`: a whole-file read only if the shell recogniser
/// accepts the command (an empty `cwd` is not a read).
fn claude_shell(input: &Value, cwd: &str, tool: &str) -> Option<ReadRequest> {
    let command = input.get("command")?.as_str()?;
    if cwd.is_empty() {
        return None;
    }
    let path = super::shell::recognise(command, Path::new(cwd))?;
    Some(ReadRequest {
        path,
        range: None,
        tool: tool.to_string(),
    })
}

/// Redirect: the documented `PreToolUse` deny with a one-line reason and no
/// file content; pass: no output.
fn render_claude(decision: &Decision, path: &Path, project: &str) -> String {
    let Decision::Redirect { lines } = decision else {
        return String::new();
    };
    let reason = format!(
        "mmhmem: {path} has at least {lines} lines. Read it through the mmhmem MCP tool \
         instead: call smart_read with project \"{project}\", agent_harness \"claude-code\" \
         and file_path \"{path}\" for a summary with exact line ranges. For exact text, for \
         example before an edit, use Read with offset and limit. Only if the summary is not \
         enough, repeat this full read within 2 minutes.",
        path = path.display()
    );
    json!({"hookSpecificOutput": {
        "hookEventName": "PreToolUse",
        "permissionDecision": "deny",
        "permissionDecisionReason": reason,
    }})
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hook::PassReason;
    use serde_json::{Value, json};
    use std::path::PathBuf;

    fn call(v: &Value) -> ParsedCall {
        parse(Harness::Claude, v.to_string().as_bytes())
    }

    fn event(tool: &str, input: Value) -> Value {
        json!({"session_id":"s1","cwd":"/work","hook_event_name":"PreToolUse",
               "tool_name":tool,"tool_input":input,"tool_use_id":"t","extra":1})
    }

    fn read_of(v: &Value) -> Option<ReadRequest> {
        call(v).read
    }

    #[test]
    fn read_without_range_is_a_full_request() {
        let p = call(&event("Read", json!({"file_path":"/work/a.rs"})));
        assert_eq!((p.session_id.as_str(), p.cwd.as_str()), ("s1", "/work"));
        assert_eq!((p.harness, p.tool.as_str()), (Harness::Claude, "Read"));
        let r = p.read.expect("read");
        assert_eq!(r.path, PathBuf::from("/work/a.rs"));
        assert_eq!((r.range, r.tool.as_str()), (None, "Read"));
    }

    #[test]
    fn read_with_offset_or_limit_is_ranged() {
        let both = json!({"file_path":"/work/a.rs","offset":10,"limit":20});
        assert_eq!(read_of(&event("Read", both)).unwrap().range, Some((10, 20)));
        let off = json!({"file_path":"/work/a.rs","offset":5});
        assert_eq!(read_of(&event("Read", off)).unwrap().range, Some((5, 0)));
        let lim = json!({"file_path":"/work/a.rs","limit":7});
        assert_eq!(read_of(&event("Read", lim)).unwrap().range, Some((1, 7)));
    }

    #[test]
    fn ranged_requests_pass_decide() {
        let r = read_of(&event("Read", json!({"file_path":"/nope","offset":5}))).unwrap();
        let s = crate::hook::HookSettings {
            shunt_min_lines: 1,
            worker_configured: true,
        };
        assert_eq!(
            crate::hook::decide(&r, &s),
            Decision::Pass(PassReason::Range)
        );
    }

    #[test]
    fn relative_file_path_joins_cwd() {
        let r = read_of(&event("Read", json!({"file_path":"src/a.rs"}))).unwrap();
        assert_eq!(r.path, Path::new("/work").join("src/a.rs"));
    }

    #[test]
    fn shell_tools_use_the_recogniser() {
        let bash = read_of(&event("Bash", json!({"command":"cat big.rs"}))).unwrap();
        assert_eq!(bash.path, Path::new("/work").join("big.rs"));
        assert_eq!(bash.range, None);
        assert_eq!(bash.tool, "Bash");
        let pipe = event("Bash", json!({"command":"cat a | grep x"}));
        assert!(read_of(&pipe).is_none());
        let ps = event("PowerShell", json!({"command":"Get-Content -Path x.rs"}));
        let ps = read_of(&ps).unwrap();
        assert_eq!(ps.path, Path::new("/work").join("x.rs"));
        assert_eq!(ps.tool, "PowerShell");
    }

    #[test]
    fn other_tools_are_never_reads() {
        let input = json!({"file_path":"/work/a.rs","command":"cat a.rs","pattern":"x"});
        for tool in ["Edit", "Write", "Grep", "mcp__mmhmem__smart_read", "read"] {
            let p = call(&event(tool, input.clone()));
            assert!(p.read.is_none(), "{tool}");
            assert_eq!(p.tool, tool);
        }
    }

    #[test]
    fn malformed_input_is_not_a_read() {
        let cases: Vec<&[u8]> = vec![
            b"",
            b"not json",
            b"[]",
            b"null",
            b"{",
            b"{\"tool_name\":\"Read\"}",
            b"{\"tool_name\":7,\"tool_input\":{}}",
            b"{\"tool_name\":\"Read\",\"tool_input\":\"x\"}",
            b"{\"tool_name\":\"Read\",\"tool_input\":{\"file_path\":5}}",
            b"{\"tool_name\":\"Read\",\"tool_input\":{\"file_path\":\"\"}}",
            b"{\"tool_name\":\"Bash\",\"tool_input\":{\"command\":[1]}}",
            b"\xff\xfe\x00",
        ];
        for c in cases {
            assert!(parse(Harness::Claude, c).read.is_none(), "{c:?}");
        }
        let missing_cwd = json!({"tool_name":"Read","tool_input":{"file_path":"rel.rs"}});
        assert!(read_of(&missing_cwd).is_none());
    }

    #[test]
    fn oversized_input_is_not_a_read() {
        let mut v = event("Read", json!({"file_path":"/work/a.rs"}));
        v["pad"] = json!("x".repeat(MAX_INPUT_BYTES));
        assert!(call(&v).read.is_none());
    }

    #[test]
    fn redirect_renders_the_documented_deny_json() {
        let path = Path::new(r#"C:\Users\a "b"\x.rs"#);
        let out = render(
            Harness::Claude,
            &Decision::Redirect { lines: 150 },
            path,
            "proj",
        );
        assert!(!out.contains('\n'));
        let v: Value = serde_json::from_str(&out).expect("valid json");
        let h = &v["hookSpecificOutput"];
        assert_eq!(v.as_object().unwrap().len(), 1);
        assert_eq!(h["hookEventName"], "PreToolUse");
        assert_eq!(h["permissionDecision"], "deny");
        let reason = h["permissionDecisionReason"].as_str().unwrap();
        // Live check 2.3: agents need every required smart_read argument, and a
        // "repeat to read anyway" hint must come last, as a last resort.
        let call = format!(
            "smart_read with project \"proj\", agent_harness \"claude-code\" and file_path \"{}\"",
            path.display()
        );
        assert!(reason.starts_with(&format!(
            "mmhmem: {} has at least 150 lines.",
            path.display()
        )));
        assert!(reason.contains(&call), "{reason}");
        assert!(reason.contains("Read with offset and limit"), "{reason}");
        let smart = reason.find("smart_read").unwrap();
        let repeat = reason.find("Only if the summary is not enough").unwrap();
        assert!(smart < repeat, "{reason}");
    }

    #[test]
    fn pass_renders_nothing() {
        let d = Decision::Pass(PassReason::Short);
        assert_eq!(render(Harness::Claude, &d, Path::new("/x"), "proj"), "");
    }
}
