//! Per-harness hook input and output mapping (read-routing-hooks design.md
//! "adapters (`src/hook/adapters.rs`)", Req 2.6, 2.7, 5.5, 6.1).
//!
//! Input is untrusted: unknown fields are ignored and anything missing or of
//! the wrong type means "not a read", so the hook passes. Claude Code is mapped
//! here, Codex in [`super::codex`], OpenCode in [`super::opencode`] and
//! Antigravity in [`super::antigravity`]; another harness adds a [`Harness`]
//! variant and a branch in [`parse`] and [`render`].

use super::{Decision, LineRange, ReadRequest};
use serde_json::{Value, json};
use std::fmt::Write as _;
use std::path::Path;

/// Stdin larger than this is never parsed (treated as not a read).
pub const MAX_INPUT_BYTES: usize = 1024 * 1024;

/// The harness whose hook protocol is being spoken.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Harness {
    Claude,
    Codex,
    OpenCode,
    Antigravity,
}

impl Harness {
    /// Stable lowercase name, used for state keys and audit records.
    pub fn name(self) -> &'static str {
        match self {
            Harness::Claude => "claude",
            Harness::Codex => "codex",
            Harness::OpenCode => "opencode",
            Harness::Antigravity => "antigravity",
        }
    }

    /// The `agent_harness` this harness passes to `smart_read` (per the
    /// installed routing guidance), recorded in audit events so hook
    /// decisions and summaries share one `mmhmem stats` row.
    pub fn agent_harness(self) -> &'static str {
        match self {
            Harness::Claude => "claude-code",
            Harness::Codex => "codex",
            Harness::OpenCode => "opencode",
            Harness::Antigravity => "antigravity",
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
        Harness::Codex => super::codex::parse(input),
        Harness::OpenCode => super::opencode::parse(input),
        Harness::Antigravity => super::antigravity::parse(input),
    }
}

/// Renders a decision as the harness's stdout; a pass is the empty string
/// for Claude Code and Codex. `threshold` (`shunt_min_lines`) is named in
/// the redirect's ranged fallback: a range under it passes the hook.
pub fn render(
    harness: Harness,
    decision: &Decision,
    path: &Path,
    project: &str,
    threshold: u32,
) -> String {
    match harness {
        Harness::Claude => render_claude(decision, path, project, threshold),
        Harness::Codex => super::codex::render(decision, path, project, threshold),
        Harness::OpenCode => super::opencode::render(decision, path, project, threshold),
        Harness::Antigravity => super::antigravity::render(decision, path, project, threshold),
    }
}

/// Stdout when the hook reaches no decision (not a read, input or
/// configuration it cannot use, an internal error): nothing, except for
/// Antigravity, where empty stdout denies the tool call
/// (antigravity-native-setup research.md "Decision: Pass output").
pub fn fail_open_output(harness: Harness) -> &'static str {
    match harness {
        Harness::Antigravity => super::antigravity::PASS_OUTPUT,
        Harness::Claude | Harness::Codex | Harness::OpenCode => "",
    }
}

/// The event as JSON; `Null` when over [`MAX_INPUT_BYTES`] or malformed.
pub(super) fn load_input(input: &[u8]) -> Value {
    if input.len() > MAX_INPUT_BYTES {
        return Value::Null;
    }
    serde_json::from_slice(input).unwrap_or(Value::Null)
}

/// The documented `PreToolUse` deny (Claude Code and Codex share it).
pub(super) fn deny_output(reason: String) -> Value {
    json!({"hookSpecificOutput": {
        "hookEventName": "PreToolUse",
        "permissionDecision": "deny",
        "permissionDecisionReason": reason,
    }})
}

/// Escapes every non-ASCII character as `\uXXXX`: Codex and Antigravity
/// run the hook through a shell (PowerShell or `cmd /c` on Windows), which
/// may re-encode non-ASCII stdout. Valid because JSON has non-ASCII only
/// inside strings.
pub(super) fn ascii_json(json: &str) -> String {
    let mut out = String::with_capacity(json.len());
    for c in json.chars() {
        if c.is_ascii() {
            out.push(c);
            continue;
        }
        let mut units = [0u16; 2];
        for unit in c.encode_utf16(&mut units) {
            let _ = write!(out, "\\u{unit:04x}");
        }
    }
    out
}

/// Claude Code `PreToolUse` input: `tool_name`, `tool_input`, `cwd`,
/// `session_id`; every other field is ignored.
fn parse_claude(input: &[u8]) -> ParsedCall {
    let value = load_input(input);
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

/// Native `Read`. Either `offset` or `limit` makes it a ranged read
/// ([`LineRange::from_offset_limit`]: from `offset`, `limit` lines, to the
/// end without a positive `limit`). Only non-negative integers count; other
/// types are ignored, so such a call stays a whole-file read. A relative
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
    Some(ReadRequest {
        path,
        range: LineRange::from_offset_limit(num("offset"), num("limit")),
        tool: "Read".to_string(),
    })
}

/// `Bash` / `PowerShell`: a read only if the shell recogniser accepts the
/// command, with the range of a partial reader (an empty `cwd` is not a
/// read).
fn claude_shell(input: &Value, cwd: &str, tool: &str) -> Option<ReadRequest> {
    let command = input.get("command")?.as_str()?;
    if cwd.is_empty() {
        return None;
    }
    let read = super::shell::recognise(command, Path::new(cwd))?;
    Some(ReadRequest {
        path: read.path,
        range: read.range,
        tool: tool.to_string(),
    })
}

/// Redirect: the documented `PreToolUse` deny with a one-line reason and no
/// file content (shunt-cache-efficiency Req 3.4, 3.6: summary first, then a
/// range under the threshold or a `smart_read` slice, the repeat last);
/// pass: no output.
fn render_claude(decision: &Decision, path: &Path, project: &str, threshold: u32) -> String {
    let Decision::Redirect { lines } = decision else {
        return String::new();
    };
    let reason = format!(
        "mmhmem: {path} has at least {lines} lines. Read it through the mmhmem MCP tool \
         instead: call smart_read with project \"{project}\", agent_harness \"claude-code\" \
         and file_path \"{path}\" for a summary with exact line ranges. For exact text, for \
         example before an edit, use Read with offset and limit for a range under \
         {threshold} lines around what you need, or ask smart_read for a slice with offset \
         and limit. Only if the summary is not enough, repeat this full read within 2 \
         minutes.",
        path = path.display()
    );
    deny_output(reason).to_string()
}

/// Every harness's redirect text (shunt-cache-efficiency Req 3.4, 3.6):
/// `smart_read` for the summary first, then the native `tool` for a range
/// under `threshold` lines around what is needed, then the `smart_read`
/// slice, and the repeat hint last.
#[cfg(test)]
pub(super) fn assert_redirect_text(reason: &str, threshold: u32, tool: &str) {
    let at = |needle: &str| {
        reason
            .find(needle)
            .unwrap_or_else(|| panic!("{needle:?} missing: {reason}"))
    };
    let summary = at("call smart_read with project");
    let native = at(tool);
    let range = at(&format!(
        "a range under {threshold} lines around what you need"
    ));
    let slice = at("ask smart_read for a slice with offset and limit");
    let repeat = at("Only if the summary is not enough");
    assert!(summary < native && summary < range, "{reason}");
    assert!(
        native < slice && range < slice && slice < repeat,
        "{reason}"
    );
    assert!(
        reason.ends_with("repeat this full read within 2 minutes."),
        "{reason}"
    );
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

    fn range_of(input: Value) -> Option<LineRange> {
        let mut input = input;
        input["file_path"] = json!("/work/a.rs");
        read_of(&event("Read", input)).expect("read").range
    }

    #[test]
    fn read_with_offset_or_limit_is_ranged() {
        let lines = |a, b| Some(LineRange::new(a, b));
        assert_eq!(
            range_of(json!({"offset":10,"limit":20})),
            lines(10, Some(29))
        );
        assert_eq!(range_of(json!({"offset":5})), lines(5, None), "open-ended");
        assert_eq!(range_of(json!({"limit":7})), lines(1, Some(7)));
        assert_eq!(range_of(json!({"offset":0,"limit":3})), lines(1, Some(3)));
        assert_eq!(range_of(json!({"offset":4,"limit":0})), lines(4, None));
        // Values that are not non-negative integers are ignored (Req 3.5):
        // the call is not sized and stays what it was without them.
        assert_eq!(range_of(json!({"limit":"x"})), None);
        assert_eq!(range_of(json!({"offset":-1,"limit":1.5})), None);
        assert_eq!(range_of(json!({"offset":10,"limit":"x"})), lines(10, None));
    }

    #[test]
    fn ranged_requests_are_sized_by_decide() {
        let s = crate::hook::HookSettings {
            shunt_min_lines: 300,
            worker_configured: true,
        };
        let decide = |input: Value| {
            let r = read_of(&event("Read", input)).expect("read");
            crate::hook::decide(&r, &s, Path::new("/work"))
        };
        let small = json!({"file_path":"/nope","offset":5,"limit":10});
        assert_eq!(decide(small), Decision::Pass(PassReason::Range));
        // Open-ended on a missing file: the count fails, the read passes.
        let open = json!({"file_path":"/nope","offset":5});
        assert_eq!(decide(open), Decision::Pass(PassReason::Missing));
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
        let sed = read_of(&event("Bash", json!({"command":"sed -n 1,400p big.rs"}))).unwrap();
        assert_eq!(sed.range, Some(LineRange::new(1, Some(400))));
        let tail = read_of(&event("Bash", json!({"command":"tail -n 50 big.rs"}))).unwrap();
        assert_eq!(tail.range, Some(LineRange::from_tail(50)));
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

    const T: u32 = crate::config::DEFAULT_SHUNT_MIN_LINES;

    fn claude_reason(path: &Path, threshold: u32) -> String {
        let lines = u64::from(threshold);
        let out = render(
            Harness::Claude,
            &Decision::Redirect { lines },
            path,
            "proj",
            threshold,
        );
        assert!(!out.contains('\n'));
        let v: Value = serde_json::from_str(&out).expect("valid json");
        let h = &v["hookSpecificOutput"];
        assert_eq!(v.as_object().unwrap().len(), 1);
        assert_eq!(h["hookEventName"], "PreToolUse");
        assert_eq!(h["permissionDecision"], "deny");
        h["permissionDecisionReason"].as_str().unwrap().to_string()
    }

    #[test]
    fn redirect_renders_the_documented_deny_json() {
        let path = Path::new(r#"C:\Users\a "b"\x.rs"#);
        let reason = claude_reason(path, T);
        // Live check 2.3: agents need every required smart_read argument, and a
        // "repeat to read anyway" hint must come last, as a last resort.
        let call = format!(
            "smart_read with project \"proj\", agent_harness \"claude-code\" and file_path \"{}\"",
            path.display()
        );
        assert!(reason.starts_with(&format!(
            "mmhmem: {} has at least {T} lines.",
            path.display()
        )));
        assert!(reason.contains(&call), "{reason}");
        assert_redirect_text(&reason, T, "Read with offset and limit");
        // The threshold comes from the settings, not from a constant.
        assert_redirect_text(&claude_reason(path, 40), 40, "Read with offset and limit");
    }

    #[test]
    fn pass_renders_nothing() {
        let d = Decision::Pass(PassReason::Short);
        assert_eq!(render(Harness::Claude, &d, Path::new("/x"), "proj", T), "");
    }
}
