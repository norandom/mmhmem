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

use super::{ElementMatcher, HookHarness, Os};

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
        None => shell_binary(command, &want).is_some(),
    }
}

/// The mmhmem binary a shell-form command `[& ]<path> hook <harness>` runs,
/// with the path bare or quoted for PowerShell or the POSIX shell
/// (`record_shell.rs`); `None` when it is not such a command.
fn shell_binary(command: &str, want: &[&str; 2]) -> Option<String> {
    super::shell::readings(command)
        .into_iter()
        .find_map(|words| {
            let [bin, rest @ ..] = words.as_slice() else {
                return None;
            };
            (is_mmhmem_binary(bin) && rest == want).then(|| bin.clone())
        })
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

/// Whether handler `have` already does what `want` describes: equal, or
/// equal but for a shell-form `command` that `os`'s shell reads as the same
/// words (a hand-edited quoting, which setup keeps rather than rewriting,
/// so Codex does not ask for the hook to be approved again).
pub fn same_handler(have: &Value, want: &Value, os: Os) -> bool {
    if have == want {
        return true;
    }
    let (Some(have), Some(want)) = (have.as_object(), want.as_object()) else {
        return false;
    };
    let (Some(theirs), Some(ours)) = (shell_form(have), shell_form(want)) else {
        return false;
    };
    let others_equal = have.len() == want.len()
        && have
            .iter()
            .all(|(k, v)| k == "command" || want.get(k) == Some(v));
    let words = |text| super::shell::words_for(text, os);
    others_equal && words(ours).is_some() && words(theirs) == words(ours)
}

/// The `command` of a shell-form handler (one without `args`).
fn shell_form(handler: &serde_json::Map<String, Value>) -> Option<&str> {
    let text = handler.get("command")?.as_str()?;
    handler.get("args").is_none().then_some(text)
}
