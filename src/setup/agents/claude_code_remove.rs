//! Removal of Claude Code entries from the install record (task 3.2):
//! mmhmem's keys of a JSON entry, its elements of a shared JSON array, its
//! Markdown section and its own files. Everything else stays byte-identical.

use std::io;
use std::path::Path;

use serde_json::Value;

use super::{SCOPE, hides_contents, json_problem};
use crate::setup::agents::{AgentProblem, FileChain};
use crate::setup::edit::json::{self as edit_json, JsonEditError};
use crate::setup::edit::markdown::{self, routing};
use crate::setup::plan::{Action, Change, target_file};
use crate::setup::record::{ElementMatcher, KeySegment, ManagedEntry, Target};

fn text_of(file: &Path, bytes: &[u8]) -> Result<String, AgentProblem> {
    String::from_utf8(bytes.to_vec()).map_err(|_| AgentProblem::Unreadable {
        file: file.to_path_buf(),
        kind: io::ErrorKind::InvalidData,
    })
}

fn segments(path: &[KeySegment]) -> Vec<&str> {
    path.iter().map(KeySegment::as_str).collect()
}

/// Takes mmhmem's keys out of a JSON entry (the whole entry without
/// `managed_keys`); an entry left empty goes too, then `missing` parents.
fn remove_entry(
    text: &str,
    pointer: &[KeySegment],
    managed: Option<&[KeySegment]>,
    missing: usize,
) -> Result<String, JsonEditError> {
    let pointer = segments(pointer);
    let Some(managed) = managed else {
        return edit_json::remove_and_prune(text, &pointer, missing);
    };
    let mut out = text.to_owned();
    for key in managed {
        let mut path = pointer.clone();
        path.push(key.as_str());
        out = edit_json::remove(&out, &path)?;
    }
    match edit_json::get(&out, &pointer)? {
        Some(Value::Object(map)) if map.is_empty() => {
            edit_json::remove_and_prune(&out, &pointer, missing)
        }
        _ => Ok(out),
    }
}

/// Takes every element `matcher` accepts out of the array; the last removal
/// prunes up to `missing` containers.
fn remove_elements(
    text: &str,
    pointer: &[KeySegment],
    matcher: ElementMatcher,
    missing: usize,
) -> Result<String, JsonEditError> {
    let pointer = segments(pointer);
    let own = edit_json::array_find(text, &pointer, |v| matcher.matches(v))?;
    let mut out = text.to_owned();
    for (n, index) in own.iter().rev().enumerate() {
        let prune = if n + 1 == own.len() { missing } else { 0 };
        out = edit_json::array_remove_and_prune(&out, &pointer, *index, prune)?;
    }
    Ok(out)
}

/// The change that takes `entry` out of its file, `None` when nothing is
/// left to take out.
pub(super) fn removal(
    chain: &mut FileChain,
    entry: &ManagedEntry,
) -> Result<Option<Change>, AgentProblem> {
    let Some(file) = target_file(&entry.target).map(Path::to_path_buf) else {
        return Ok(None);
    };
    let Some(before) = chain.bytes(&file)? else {
        return Ok(None);
    };
    let missing = entry.missing_parents.unwrap_or(0) as usize;
    let err = json_problem(&file);
    let after = match &entry.target {
        Target::File { .. } => None,
        Target::JsonEntry {
            pointer,
            managed_keys,
            ..
        } => {
            let text = text_of(&file, &before)?;
            let out = remove_entry(&text, pointer, managed_keys.as_deref(), missing);
            Some(out.map_err(&err)?.into_bytes())
        }
        Target::JsonArrayElement {
            pointer, matcher, ..
        } => {
            let text = text_of(&file, &before)?;
            let out = remove_elements(&text, pointer, *matcher, missing);
            Some(out.map_err(&err)?.into_bytes())
        }
        Target::MarkdownSection { .. } => {
            let text = text_of(&file, &before)?;
            let out = markdown::remove_section(&text, &routing())
                .map_err(AgentProblem::markdown(&file))?;
            Some(out.into_bytes())
        }
        _ => return Ok(None),
    };
    if after.as_ref() == Some(&before) {
        return Ok(None);
    }
    Ok(Some(Change {
        scope: SCOPE,
        target: entry.target.clone(),
        owner_only: false,
        hide_diff: hides_contents(&file),
        summary: format!("remove mmhmem's entry from {}", file.display()),
        file,
        action: Action::Remove,
        before: Some(before),
        after,
        missing_parents: None,
    }))
}
