//! Entry normalization, drift detection and the recovery scan (task 2.4,
//! design.md §3 "Entry-level hashes" and "Atomic writes"; Requirements 5.2
//! and 5.3).
//!
//! **Normalization rule.** A managed entry's hash is SHA-256 over:
//!
//! - `json_entry`: the value at the pointer (read with `edit::json::get`,
//!   so JSONC comments and a BOM do not matter) in canonical JSON: object
//!   keys sorted by code point, no whitespace, `serde_json` number and string
//!   formatting.
//! - `toml_table`: the item at the key path as JSON via
//!   `edit::toml::get_item_json` (typed floats and date-times, see there),
//!   in the same canonical JSON. Formatting, comments and key order are
//!   ignored.
//! - With `managed_keys` set, a `json_entry` or `toml_table` object is cut
//!   down to those top-level keys first (an absent key stays absent), so
//!   keys the user adds inside mmhmem's entry are not drift.
//! - `json_array_element`: the one array element its `ElementMatcher`
//!   accepts, in canonical JSON. No match is `Missing`; more than one is
//!   `Unreadable`.
//! - `markdown_section`: the bytes from the start-marker line through the
//!   end-marker line, CRLF turned into LF.
//! - `file`: the file bytes, with CRLF turned into LF when the file is UTF-8
//!   text without NUL bytes; other files are hashed raw.
//! - `binary`: the raw file bytes. `path_entry`: the folder's UTF-8 text.
//!
//! So edits elsewhere in a shared file never count as drift. Nothing here
//! returns file contents: reasons are fixed messages or positions.

use std::io;
use std::path::{Path, PathBuf};

use super::{
    ElementMatcher, InstallRecord, KeySegment, ManagedEntry, Scope, SectionId, Sha256Hex, Target,
};
use crate::setup::edit;
use serde_json::Value;

/// What is on disk for a managed entry right now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Observation {
    /// The entry exists; the hash of its normalized value.
    Present(Sha256Hex),
    /// The file or the entry inside it is gone.
    Missing,
    /// The file cannot be read or parsed (fixed reason, no contents).
    Unreadable(String),
}

/// Outside inputs the disk check needs besides files.
#[derive(Debug, Clone, Copy, Default)]
pub struct Probe<'a> {
    /// The folders on the user's `PATH` (`None`: not inspected, so a
    /// `path_entry` is reported unreadable).
    pub user_path: Option<&'a [PathBuf]>,
}

/// Drift of one `done` entry (Requirement 5.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Drift {
    /// Disk matches the record.
    Unchanged,
    /// The entry was changed outside the installer.
    Changed {
        /// Hash in the record.
        expected: Sha256Hex,
        /// Hash on disk.
        actual: Sha256Hex,
    },
    /// The file or entry was deleted outside the installer.
    Missing,
    /// The file cannot be read or parsed.
    Unreadable {
        /// Fixed reason, no contents.
        reason: String,
    },
}

/// One entry's drift.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriftReport {
    /// Whose entry.
    pub scope: Scope,
    /// The entry.
    pub target: Target,
    /// What changed.
    pub drift: Drift,
}

/// What happened to an `applying` entry after an interruption.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Recovery {
    /// The target was written; finish with `finish_change`.
    Applied,
    /// The target was not written; undo with `undo_change`.
    NotApplied,
    /// Disk matches neither the old nor the new state; ask the user.
    Diverged {
        /// Hash on disk (`None`: the entry is absent).
        actual: Option<Sha256Hex>,
    },
    /// The file cannot be read or parsed.
    Unreadable {
        /// Fixed reason, no contents.
        reason: String,
    },
}

/// One `applying` entry's recovery state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryReport {
    /// Whose entry.
    pub scope: Scope,
    /// The entry.
    pub target: Target,
    /// What to do.
    pub recovery: Recovery,
}

/// Turn CRLF into LF.
pub fn normalize_text(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\r' && bytes.get(i + 1) == Some(&b'\n') {
            i += 1;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    out
}

/// Canonical JSON: keys sorted, no whitespace.
pub fn normalize_json(value: &Value) -> String {
    let mut out = String::new();
    write_canonical(value, &mut out);
    out
}

fn write_canonical(value: &Value, out: &mut String) {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            out.push('{');
            for (i, k) in keys.into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&Value::String(k.clone()).to_string());
                out.push(':');
                write_canonical(&map[k.as_str()], out);
            }
            out.push('}');
        }
        Value::Array(items) => {
            out.push('[');
            for (i, v) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_canonical(v, out);
            }
            out.push(']');
        }
        scalar => out.push_str(&scalar.to_string()),
    }
}

/// Keep only `managed` top-level keys of an object entry (absent keys stay
/// absent, so they are part of the hash as absent). Non-objects and `None`
/// are returned whole.
fn managed_view(value: Value, managed: Option<&[KeySegment]>) -> Value {
    match (value, managed) {
        (Value::Object(mut map), Some(keys)) => Value::Object(
            keys.iter()
                .filter_map(|k| map.remove_entry(k.as_str()))
                .collect(),
        ),
        (value, _) => value,
    }
}

fn hash_value(value: &Value) -> Observation {
    Observation::Present(Sha256Hex::of(normalize_json(value).as_bytes()))
}

/// The hash recorded for a `path_entry` target.
pub fn path_entry_hash(value: &Path) -> Sha256Hex {
    Sha256Hex::of(value.to_string_lossy().as_bytes())
}

/// Observe `target` in the file content `bytes` (for example the planned
/// bytes before they are written, to get the `after` hash). A `path_entry`
/// target is always `Present` with [`path_entry_hash`].
pub fn observe_in(target: &Target, bytes: &[u8]) -> Observation {
    match target {
        Target::File { .. } => Observation::Present(Sha256Hex::of(&normalize_file(bytes))),
        Target::Binary { .. } => Observation::Present(Sha256Hex::of(bytes)),
        Target::PathEntry { value } => Observation::Present(path_entry_hash(value)),
        Target::JsonEntry {
            pointer,
            managed_keys,
            ..
        } => with_text(bytes, |t| observe_json(t, pointer, managed_keys.as_deref())),
        Target::TomlTable {
            key, managed_keys, ..
        } => with_text(bytes, |t| observe_toml(t, key, managed_keys.as_deref())),
        Target::JsonArrayElement {
            pointer, matcher, ..
        } => with_text(bytes, |t| observe_element(t, pointer, *matcher)),
        Target::MarkdownSection { section, .. } => {
            with_text(bytes, |t| observe_section(t, section))
        }
    }
}

/// Observe `target` on disk now.
pub fn observe(target: &Target, probe: &Probe<'_>) -> Observation {
    let file = match target {
        Target::PathEntry { value } => return observe_path_entry(value, probe),
        Target::Binary { path } => path,
        Target::File { file }
        | Target::JsonEntry { file, .. }
        | Target::TomlTable { file, .. }
        | Target::JsonArrayElement { file, .. }
        | Target::MarkdownSection { file, .. } => file,
    };
    match std::fs::read(file) {
        Ok(bytes) => observe_in(target, &bytes),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Observation::Missing,
        Err(e) => Observation::Unreadable(format!("the file could not be read ({})", e.kind())),
    }
}

fn normalize_file(bytes: &[u8]) -> Vec<u8> {
    if !bytes.contains(&0) && std::str::from_utf8(bytes).is_ok() {
        normalize_text(bytes)
    } else {
        bytes.to_vec()
    }
}

fn with_text(bytes: &[u8], f: impl FnOnce(&str) -> Observation) -> Observation {
    match std::str::from_utf8(bytes) {
        Ok(text) => f(text),
        Err(_) => Observation::Unreadable("the file is not UTF-8 text".to_owned()),
    }
}

fn segments(path: &[KeySegment]) -> Vec<&str> {
    path.iter().map(KeySegment::as_str).collect()
}

fn observe_json(text: &str, pointer: &[KeySegment], managed: Option<&[KeySegment]>) -> Observation {
    match edit::json::get(text, &segments(pointer)) {
        Ok(Some(value)) => hash_value(&managed_view(value, managed)),
        Ok(None) => Observation::Missing,
        Err(e) => Observation::Unreadable(e.to_string()),
    }
}

fn observe_toml(text: &str, key: &[KeySegment], managed: Option<&[KeySegment]>) -> Observation {
    match edit::toml::get_item_json(text, &segments(key)) {
        Ok(Some(value)) => hash_value(&managed_view(value, managed)),
        Ok(None) => Observation::Missing,
        Err(e) => Observation::Unreadable(e.to_string()),
    }
}

fn observe_element(text: &str, pointer: &[KeySegment], matcher: ElementMatcher) -> Observation {
    let array = match edit::json::get(text, &segments(pointer)) {
        Ok(Some(Value::Array(items))) => items,
        Ok(Some(_)) => return Observation::Unreadable("the value is not a list".to_owned()),
        Ok(None) => return Observation::Missing,
        Err(e) => return Observation::Unreadable(e.to_string()),
    };
    let mut own = array.iter().filter(|e| matcher.matches(e));
    match (own.next(), own.next()) {
        (Some(element), None) => hash_value(element),
        (None, _) => Observation::Missing,
        (Some(_), Some(_)) => {
            Observation::Unreadable("mmhmem's entry appears more than once".to_owned())
        }
    }
}

fn observe_section(text: &str, id: &SectionId) -> Observation {
    let (start, end) = (id.start_marker(), id.end_marker());
    let mut offset = 0;
    let mut begin = None;
    let mut found = None;
    for line in text.split_inclusive('\n') {
        let bare = line.trim_end_matches(['\n', '\r']);
        if bare == start {
            if begin.is_some() || found.is_some() {
                return Observation::Unreadable(
                    "the mmhmem section appears more than once".to_owned(),
                );
            }
            begin = Some(offset);
        } else if bare == end
            && found.is_none()
            && let Some(b) = begin.take()
        {
            found = Some(b..offset + bare.len());
        }
        offset += line.len();
    }
    match (found, begin) {
        (_, Some(_)) => Observation::Unreadable("the mmhmem section is not closed".to_owned()),
        (Some(range), None) => {
            Observation::Present(Sha256Hex::of(&normalize_text(&text.as_bytes()[range])))
        }
        (None, None) => Observation::Missing,
    }
}

/// Compare folder paths the way `PATH` lookups do: trailing separators are
/// ignored, and on Windows ASCII case too.
fn same_folder(a: &Path, b: &Path) -> bool {
    let trim = |p: &Path| p.to_string_lossy().trim_end_matches(['/', '\\']).to_owned();
    let (a, b) = (trim(a), trim(b));
    if cfg!(windows) {
        a.eq_ignore_ascii_case(&b)
    } else {
        a == b
    }
}

fn observe_path_entry(value: &Path, probe: &Probe<'_>) -> Observation {
    match probe.user_path {
        None => Observation::Unreadable("the PATH was not inspected".to_owned()),
        Some(list) if list.iter().any(|p| same_folder(p, value)) => {
            Observation::Present(path_entry_hash(value))
        }
        Some(_) => Observation::Missing,
    }
}

fn drift_of(entry: &ManagedEntry, probe: &Probe<'_>) -> Drift {
    let Some(expected) = entry.sha256.clone() else {
        return Drift::Missing;
    };
    match observe(&entry.target, probe) {
        Observation::Present(actual) if actual == expected => Drift::Unchanged,
        Observation::Present(actual) => Drift::Changed { expected, actual },
        Observation::Missing => Drift::Missing,
        Observation::Unreadable(reason) => Drift::Unreadable { reason },
    }
}

/// Compare every `done` entry with the disk (Requirement 5.3). `applying`
/// entries are left to [`recovery_scan`].
pub fn detect_drift(record: &InstallRecord, probe: &Probe<'_>) -> Vec<DriftReport> {
    record
        .done()
        .into_iter()
        .map(|(scope, e)| DriftReport {
            scope,
            target: e.target.clone(),
            drift: drift_of(e, probe),
        })
        .collect()
}

fn recovery_of(entry: &ManagedEntry, probe: &Probe<'_>) -> Recovery {
    let actual = match observe(&entry.target, probe) {
        Observation::Present(h) => Some(h),
        Observation::Missing => None,
        Observation::Unreadable(reason) => return Recovery::Unreadable { reason },
    };
    if actual == entry.sha256 {
        Recovery::Applied
    } else if actual == entry.before_sha256 {
        Recovery::NotApplied
    } else {
        Recovery::Diverged { actual }
    }
}

/// List every `applying` entry with what the disk shows (Requirement 5.2):
/// the next run finishes `Applied` ones, undoes `NotApplied` ones and asks
/// about the rest.
pub fn recovery_scan(record: &InstallRecord, probe: &Probe<'_>) -> Vec<RecoveryReport> {
    record
        .applying()
        .into_iter()
        .map(|(scope, e)| RecoveryReport {
            scope,
            target: e.target.clone(),
            recovery: recovery_of(e, probe),
        })
        .collect()
}
