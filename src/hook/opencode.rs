//! OpenCode v2 read/shell adapter (read-routing-hooks Req 6.3).
use super::adapters::{Harness, ParsedCall, load_input};
use super::{Decision, ReadRequest};
use serde_json::{Value, json};
use std::path::Path;

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
fn read(input: &Value, cwd: &str, tool: &str) -> Option<ReadRequest> {
    let path = match tool {
        "read" => super::shell::resolve(input.get("path")?.as_str()?, Path::new(cwd))?,
        "shell" => {
            let dir = match input.get("workdir") {
                None => Path::new(cwd).to_path_buf(),
                Some(Value::String(dir)) => super::shell::resolve(dir, Path::new(cwd))?,
                _ => return None,
            };
            super::shell::recognise(input.get("command")?.as_str()?, &dir)?
        }
        _ => return None,
    };
    if tool == "read"
        && ["offset", "limit"]
            .iter()
            .any(|key| input.get(key).is_some_and(|v| v.as_u64().is_none()))
    {
        return None;
    }
    let number = |key| input.get(key).and_then(Value::as_u64);
    let (offset, limit) = (number("offset"), number("limit"));
    let range = (tool == "read" && (offset.is_some() || limit.is_some()))
        .then(|| (offset.unwrap_or(1), limit.unwrap_or(0)));
    Some(ReadRequest {
        path,
        range,
        tool: tool.into(),
    })
}
pub(super) fn render(decision: &Decision, path: &Path, project: &str) -> String {
    match decision {
        Decision::Pass(_) => json!({"decision":"pass"}).to_string(),
        Decision::Redirect { lines } => json!({"decision":"deny","reason":format!(
            "mmhmem: {} has at least {lines} lines. Call smart_read with project {project:?}, agent_harness \"opencode\" and file_path {:?} for a summary. For exact text, use read with offset and limit. Only if the summary is not enough, repeat this full read within 2 minutes.", path.display(), path.to_string_lossy())}).to_string(),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_and_shell_ranges_and_untrusted_input() {
        let event = |tool: &str, args| {
            json!({"tool":tool,"cwd":"/work","sessionID":"s","input":args}).to_string()
        };
        let full = parse(event("read", json!({"path":"big.rs"})).as_bytes());
        assert_eq!(full.read.unwrap().path, Path::new("/work").join("big.rs"));
        let ranged = parse(event("read", json!({"path":"big.rs","limit":10})).as_bytes());
        assert_eq!(ranged.read.unwrap().range, Some((1, 10)));
        assert!(
            parse(event("shell", json!({"command":"cat big.rs","workdir":"sub"})).as_bytes())
                .read
                .is_some()
        );
        assert!(
            parse(event("edit", json!({"path":"big.rs"})).as_bytes())
                .read
                .is_none()
        );
        assert!(parse(b"garbage").read.is_none());
    }
    #[test]
    fn deny_and_pass_protocol() {
        let output: Value = serde_json::from_str(&render(
            &Decision::Redirect { lines: 150 },
            Path::new("/big.rs"),
            "p",
        ))
        .unwrap();
        assert_eq!(output["decision"], "deny");
        assert!(output["reason"].as_str().unwrap().contains("smart_read"));
        assert_eq!(
            render(
                &Decision::Pass(super::super::PassReason::Range),
                Path::new("/big.rs"),
                "p"
            ),
            "{\"decision\":\"pass\"}"
        );
    }
}
