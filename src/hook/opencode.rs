//! OpenCode v2 read/shell adapter (read-routing-hooks Req 6.3).
use super::adapters::{Harness, ParsedCall, load_input};
use super::{Decision, LineRange, ReadRequest};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

pub(super) fn parse(input: &[u8]) -> ParsedCall {
    let value = load_input(input);
    let text = |key: &str| value.get(key).and_then(Value::as_str).unwrap_or("");
    let (cwd, tool) = (text("cwd"), text("tool"));
    let args = value.get("input").unwrap_or(&Value::Null);
    let read = if cwd.is_empty() {
        None
    } else {
        read(args, cwd, tool)
    };
    ParsedCall {
        harness: Harness::OpenCode,
        session_id: text("sessionID").into(),
        cwd: cwd.into(),
        tool: tool.into(),
        read,
    }
}

/// `read` (native, with `offset`/`limit`) or `shell` (the recogniser, with
/// the range of a partial reader); anything else is not a read.
fn read(input: &Value, cwd: &str, tool: &str) -> Option<ReadRequest> {
    let (path, range) = match tool {
        "read" => native_read(input, cwd)?,
        "shell" => {
            let dir = match input.get("workdir") {
                None => Path::new(cwd).to_path_buf(),
                Some(Value::String(dir)) => super::shell::resolve(dir, Path::new(cwd))?,
                _ => return None,
            };
            let read = super::shell::recognise(input.get("command")?.as_str()?, &dir)?;
            (read.path, read.range)
        }
        _ => return None,
    };
    Some(ReadRequest {
        path,
        range,
        tool: tool.into(),
    })
}

/// `read`: `path` and the [`LineRange::from_offset_limit`] range; a range
/// key that is not a non-negative integer makes it not a read.
fn native_read(input: &Value, cwd: &str) -> Option<(PathBuf, Option<LineRange>)> {
    let path = super::shell::resolve(input.get("path")?.as_str()?, Path::new(cwd))?;
    let number = |key: &str| match input.get(key) {
        None => Some(None),
        Some(value) => value.as_u64().map(Some),
    };
    let (offset, limit) = (number("offset")?, number("limit")?);
    Some((path, LineRange::from_offset_limit(offset, limit)))
}

/// Redirect: `{"decision":"deny","reason":...}` (shunt-cache-efficiency Req
/// 3.4, 3.6: summary first, then a range under the threshold or a
/// `smart_read` slice, the repeat last); pass: `{"decision":"pass"}`.
pub(super) fn render(decision: &Decision, path: &Path, project: &str, threshold: u32) -> String {
    let Decision::Redirect { lines } = decision else {
        return json!({"decision":"pass"}).to_string();
    };
    let reason = format!(
        "mmhmem: {} has at least {lines} lines. Read it through the mmhmem MCP tool \
         instead: call smart_read with project {project:?}, agent_harness \"opencode\" and \
         file_path {:?} for a summary with exact line ranges. For exact text, for example \
         before an edit, use read with offset and limit for a range under {threshold} lines \
         around what you need, or ask smart_read for a slice with offset and limit. Only if \
         the summary is not enough, repeat this full read within 2 minutes.",
        path.display(),
        path.to_string_lossy()
    );
    json!({"decision": "deny", "reason": reason}).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const T: u32 = crate::config::DEFAULT_SHUNT_MIN_LINES;

    fn event(tool: &str, args: Value) -> String {
        json!({"tool":tool,"cwd":"/work","sessionID":"s","input":args}).to_string()
    }

    fn read_of(tool: &str, args: Value) -> Option<ReadRequest> {
        parse(event(tool, args).as_bytes()).read
    }

    #[test]
    fn native_and_shell_ranges_and_untrusted_input() {
        let full = read_of("read", json!({"path":"big.rs"})).unwrap();
        assert_eq!(full.path, Path::new("/work").join("big.rs"));
        assert_eq!(full.range, None);
        let shell = read_of("shell", json!({"command":"cat big.rs","workdir":"sub"}));
        assert_eq!(shell.unwrap().range, None);
        assert!(read_of("edit", json!({"path":"big.rs"})).is_none());
        assert!(parse(b"garbage").read.is_none());
    }

    #[test]
    fn offset_limit_and_shell_partial_reads_become_line_ranges() {
        let range = |tool: &str, args: Value| read_of(tool, args).expect("read").range;
        let lines = |a, b| Some(LineRange::new(a, b));
        assert_eq!(
            range("read", json!({"path":"a","limit":10})),
            lines(1, Some(10))
        );
        let both = json!({"path":"a","offset":3,"limit":5});
        assert_eq!(range("read", both), lines(3, Some(7)));
        assert_eq!(
            range("read", json!({"path":"a","offset":4})),
            lines(4, None)
        );
        let sed = json!({"command":"sed -n 1,400p a.rs"});
        assert_eq!(range("shell", sed), lines(1, Some(400)));
        let tail = json!({"command":"tail -n 50 a.rs"});
        assert_eq!(range("shell", tail), Some(LineRange::from_tail(50)));
        // A range key of another type is not a read (as before).
        for bad in [json!("x"), json!(-1), json!(1.5), json!(null)] {
            assert!(read_of("read", json!({"path":"a","limit":bad})).is_none());
        }
    }

    #[test]
    fn deny_and_pass_protocol() {
        let lines = u64::from(T);
        let out = render(&Decision::Redirect { lines }, Path::new("/big.rs"), "p", T);
        let output: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(output.as_object().unwrap().len(), 2, "{out}");
        assert_eq!(output["decision"], "deny");
        let reason = output["reason"].as_str().unwrap();
        let head = format!("mmhmem: /big.rs has at least {T} lines.");
        assert!(reason.starts_with(&head), "{reason}");
        let call = "smart_read with project \"p\", agent_harness \"opencode\" and \
                    file_path \"/big.rs\"";
        assert!(reason.contains(call), "{reason}");
        super::super::adapters::assert_redirect_text(reason, T, "read with offset and limit");
        let pass = Decision::Pass(super::super::PassReason::Range);
        assert_eq!(
            render(&pass, Path::new("/big.rs"), "p", T),
            "{\"decision\":\"pass\"}"
        );
    }
}
