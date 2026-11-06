//! mmhmem's read-routing hook handler inside shared hook groups (follow-up
//! G1, `.kiro/specs/reliable-installers/research.md` "2026-11-06 G1/G3").
//!
//! Claude Code `settings.json` and Codex `hooks.json` hold
//! `hooks.PreToolUse: [{matcher, hooks: [handler, ...]}, ...]`. mmhmem owns
//! exactly its handler, never the group: the old installers updated it in
//! place inside a group that may also hold the user's handlers
//! (`manage_hooks.ps1` `Find-Own`/`Remove-OwnHandler`), and adoption takes
//! such files over. The record's hash therefore describes the handler
//! ([`handler_view`]): the group's `matcher` (it decides when the handler
//! runs) plus that one handler, so the user's handlers are not drift. For a
//! group holding only mmhmem's handler the view is the group itself, so
//! records written before this change keep their hashes.

use serde_json::{Value, json};

use super::{ElementMatcher, HookHarness};

/// Whether `path` names the mmhmem binary (`mmhmem` or `mmhmem.exe`).
pub fn is_mmhmem_binary(path: &str) -> bool {
    let name = path.rsplit(['/', '\\']).next().unwrap_or("");
    name == "mmhmem" || name == "mmhmem.exe"
}

/// Whether `handler` runs `mmhmem hook <harness>`.
pub fn is_own_handler(handler: &Value, harness: HookHarness) -> bool {
    let Some(command) = handler.get("command").and_then(Value::as_str) else {
        return false;
    };
    let want = ["hook", harness.as_str()];
    match handler.get("args").and_then(Value::as_array) {
        Some(args) => {
            is_mmhmem_binary(command)
                && args.len() == 2
                && args.iter().zip(want).all(|(a, w)| a.as_str() == Some(w))
        }
        None => shell_form_matches(command, &want),
    }
}

/// `[& ]<path> hook <harness>` with the path bare or quoted.
fn shell_form_matches(command: &str, want: &[&str; 2]) -> bool {
    let rest = command.trim();
    let rest = rest.strip_prefix('&').map_or(rest, str::trim_start);
    let (path, tail) = match rest.chars().next() {
        Some(q @ ('\'' | '"')) => match rest[1..].find(q) {
            Some(end) => (&rest[1..=end], &rest[end + 2..]),
            None => return false,
        },
        _ => rest.split_once(' ').unwrap_or((rest, "")),
    };
    is_mmhmem_binary(path) && tail.split_whitespace().eq(want.iter().copied())
}

/// Every mmhmem handler in the hook groups `groups`, as
/// `(group index, handler index)` in document order.
pub fn own_handlers(groups: &[Value], matcher: ElementMatcher) -> Vec<(usize, usize)> {
    let mut found = Vec::new();
    for (g, group) in groups.iter().enumerate() {
        found.extend(matcher.own_handlers(group).into_iter().map(|h| (g, h)));
    }
    found
}

/// What the record hashes for mmhmem's handler `handler` of `group`: the
/// group's `matcher` (when it has one) and that handler alone.
pub fn handler_view(group: &Value, handler: usize) -> Value {
    let one = group["hooks"].get(handler).cloned().unwrap_or(Value::Null);
    match group.get("matcher") {
        Some(matcher) => json!({"matcher": matcher, "hooks": [one]}),
        None => json!({"hooks": [one]}),
    }
}

/// Whether handler `have` already does what `want` describes.
pub fn same_handler(have: &Value, want: &Value) -> bool {
    have == want
}
