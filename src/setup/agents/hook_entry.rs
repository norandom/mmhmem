//! Install and removal of mmhmem's read-routing hook HANDLER in a shared
//! `hooks.PreToolUse` array, for Claude Code and Codex (follow-up G1,
//! `.kiro/specs/reliable-installers/research.md` "2026-11-06 G1/G3").
//!
//! mmhmem owns exactly its handler, never the group (`record_hook.rs`), as
//! the old `manage_hooks.ps1`/`.py` did:
//!
//! - **Install**: without a handler of mmhmem's, its own group is appended
//!   (never merged into a user group, even one with the same matcher). With
//!   one, it is kept where it is and updated in place, key by key, when its
//!   group has mmhmem's matcher or holds nothing else; in a user group with
//!   another matcher it moves to mmhmem's own group. Further handlers of
//!   mmhmem's (duplicates) are removed.
//! - **Removal**: only mmhmem's handlers go. A group goes only when mmhmem's
//!   handlers were all it held; then up to `levels` emptied containers.
//!
//! Everything else stays byte-identical (`edit::json` nested edits). A
//! handler that is not identifiably mmhmem's is never touched.

use std::collections::BTreeMap;

use serde_json::Value;

use crate::setup::edit::json::{self as ejson, JsonEditError};
use crate::setup::record::{ElementMatcher, own_handlers, same_handler};

/// The planned text of an install.
pub(crate) struct Installed {
    /// The file text afterwards.
    pub(crate) after: String,
    /// Whether a handler of mmhmem's was there before.
    pub(crate) found: bool,
    /// How many containers of `path` the append created (0 when `found`).
    pub(crate) missing: usize,
}

fn groups_at(text: &str, path: &[&str]) -> Result<Vec<Value>, JsonEditError> {
    Ok(match ejson::get(text, path)? {
        Some(Value::Array(groups)) => groups,
        _ => Vec::new(),
    })
}

fn handler_count(group: &Value) -> usize {
    group["hooks"].as_array().map_or(0, Vec::len)
}

/// Removes the handlers `drop` (`(group, handler)` pairs) from `text`: a
/// group whose handlers all go is removed whole (pruning up to `levels`
/// emptied containers), otherwise only those handlers.
fn drop_handlers(
    text: &str,
    path: &[&str],
    groups: &[Value],
    drop: &[(usize, usize)],
    levels: usize,
) -> Result<String, JsonEditError> {
    let mut by_group: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for (g, h) in drop {
        by_group.entry(*g).or_default().push(*h);
    }
    let mut out = text.to_owned();
    for (g, handlers) in by_group.iter().rev() {
        if handlers.len() == handler_count(&groups[*g]) {
            out = ejson::array_remove_and_prune(&out, path, *g, levels)?;
            continue;
        }
        for h in handlers.iter().rev() {
            out = ejson::nested_remove(&out, path, *g, "hooks", *h)?;
        }
    }
    Ok(out)
}

/// Makes handler `h` of group `g` equal to `want`, key by key.
fn update_handler(
    text: &str,
    path: &[&str],
    (g, h): (usize, usize),
    have: &Value,
    want: &Value,
) -> Result<String, JsonEditError> {
    let mut out = text.to_owned();
    if same_handler(have, want) {
        return Ok(out);
    }
    let inner = Some(("hooks", h));
    if let (Some(have), Some(want)) = (have.as_object(), want.as_object()) {
        for key in have.keys().filter(|k| !want.contains_key(*k)) {
            out = ejson::nested_remove_key(&out, path, g, inner, key)?;
        }
        for (key, value) in want {
            out = ejson::nested_set(&out, path, g, inner, key, value)?;
        }
    }
    Ok(out)
}

/// Plans mmhmem's handler `want_group` (`{matcher, hooks: [handler]}`) into
/// the array at `path` of `text`.
pub(crate) fn install(
    text: &str,
    path: &[&str],
    matcher: ElementMatcher,
    want_group: &Value,
) -> Result<Installed, JsonEditError> {
    let groups = groups_at(text, path)?;
    let own = own_handlers(&groups, matcher);
    if own.is_empty() {
        let missing = ejson::missing_keys(text, path)?;
        let after = ejson::array_append(text, path, want_group)?;
        return Ok(Installed {
            after,
            found: false,
            missing,
        });
    }
    let want_matcher = want_group.get("matcher");
    let right_matcher = |g: usize| groups[g].get("matcher") == want_matcher;
    let keep = own
        .iter()
        .copied()
        .find(|(g, _)| right_matcher(*g))
        .unwrap_or(own[0]);
    let (g, h) = keep;
    let alone = own.iter().filter(|(og, _)| *og == g).count() == handler_count(&groups[g]);
    let in_place = right_matcher(g) || alone;
    let mut drop: Vec<(usize, usize)> = own.iter().copied().filter(|l| *l != keep).collect();
    let mut after = text.to_owned();
    if in_place {
        let want = &want_group["hooks"][0];
        after = update_handler(&after, path, keep, &groups[g]["hooks"][h], want)?;
        if let Some(m) = want_matcher.filter(|_| !right_matcher(g)) {
            after = ejson::nested_set(&after, path, g, None, "matcher", m)?;
        }
    } else {
        drop.push(keep);
    }
    // The kept handler's group is never removed whole: it keeps `keep`, or
    // (when it moves) the user's handlers with another matcher.
    after = drop_handlers(&after, path, &groups, &drop, 0)?;
    if !in_place {
        after = ejson::array_append(&after, path, want_group)?;
    }
    Ok(Installed {
        after,
        found: true,
        missing: 0,
    })
}

/// Takes every handler of mmhmem's out of the array at `path` of `text`;
/// a group left without handlers goes, then up to `levels` containers.
pub(crate) fn remove(
    text: &str,
    path: &[&str],
    matcher: ElementMatcher,
    levels: usize,
) -> Result<String, JsonEditError> {
    let groups = groups_at(text, path)?;
    let own = own_handlers(&groups, matcher);
    drop_handlers(text, path, &groups, &own, levels)
}
