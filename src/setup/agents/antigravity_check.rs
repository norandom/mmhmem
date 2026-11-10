//! Antigravity check helpers (antigravity-native-setup design.md
//! "Antigravity check"). For now: bringing earlier installs forward (task
//! 3.4; Requirements 5.1 and 5.2); task 3.5 moves the check itself here.
//!
//! - **Snippet era** (5.1): installs from 0.5.1 on saved an import snippet,
//!   [`SNIPPET_FILE`], next to the binary and recorded it as a whole file.
//!   It is matched by its file name, so the snippet next to a binary that has
//!   moved since is still taken out.
//! - **Stale location** (5.2): the snippet was pasted into
//!   `~/.gemini/antigravity/mcp_config.json`, which Antigravity does not
//!   read. mmhmem's entry there is taken out in every plan, whatever is
//!   selected, and on removal, when its `command` starts the mmhmem
//!   program. The file is not mmhmem's: an entry naming another program, a
//!   missing, unreadable or invalid file is left alone and never a problem,
//!   and everything else in the file stays byte-identical. The
//!   removal leaves nothing to record (a removed entry drops out of the
//!   record when it is applied), so a plan after it has nothing to do there.

use std::path::Path;

use super::{COMMAND, ENTRY, SCOPE, SNIPPET_FILE};
use crate::setup::edit::json as edit;
use crate::setup::plan::{Action, Change};
use crate::setup::record::{Target, key_path};

/// The mmhmem program's file names.
const PROGRAM_NAMES: [&str; 2] = ["mmhmem", "mmhmem.exe"];

/// Whether a recorded whole file is the snippet-era import snippet.
pub(super) fn is_snippet(file: &Path) -> bool {
    file.file_name().is_some_and(|name| name == SNIPPET_FILE)
}

/// The removal of mmhmem's whole entry from the stale server file `stale`
/// when its `command` starts the mmhmem program ([`starts_mmhmem`]);
/// `None` otherwise, and for a file that is missing, unreadable or not
/// valid JSON. Parent objects stay, even when left empty.
pub(super) fn stale_entry(stale: &Path, binary: &Path) -> Option<Change> {
    let before = std::fs::read(stale).ok()?;
    let text = std::str::from_utf8(&before).ok()?;
    let command = edit::get(text, &COMMAND).ok()??;
    if !starts_mmhmem(command.as_str()?, binary) {
        return None;
    }
    let after = edit::remove(text, &ENTRY).ok()?.into_bytes();
    Some(Change {
        scope: SCOPE,
        target: Target::JsonEntry {
            file: stale.to_path_buf(),
            pointer: key_path(&ENTRY).expect("static keys"),
            managed_keys: None,
        },
        file: stale.to_path_buf(),
        action: Action::Remove,
        before: Some(before),
        after: Some(after),
        owner_only: false,
        hide_diff: false,
        missing_parents: None,
        summary: format!(
            "Antigravity no longer reads this file; remove the mmhmem entry ({})",
            stale.display()
        ),
    })
}

/// Whether `command` starts mmhmem: it is the installed `binary`, or its
/// file name (after the last slash of either style, in any case) is one of
/// [`PROGRAM_NAMES`]. The file was written on this machine, but a path
/// pasted from another one may use the other slash style.
fn starts_mmhmem(command: &str, binary: &Path) -> bool {
    if Path::new(command) == binary {
        return true;
    }
    let name = command.rsplit(['/', '\\']).next().unwrap_or(command);
    PROGRAM_NAMES.iter().any(|p| name.eq_ignore_ascii_case(p))
}
