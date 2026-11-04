//! The words of the removal (task 7.2, Requirements 7.3 and 7.4), the
//! no-terminal refusal (amended Requirement 4.9) and the restart after an
//! update. Same plain-language rule as the rest of `text.rs`; [`ALL`] lists
//! every string for the test.

pub const INTRO: &str = "This removes everything setup added for mmhmem: its \
settings in your tools, the mmhmem program and its place on your PATH. Your other \
settings stay exactly as they are.";
/// Contains `{folder}`.
pub const CHANGE_PATH: &str = "Take {folder} out of your PATH again (setup added it)";
pub const CHANGE_PROGRAM: &str = "Delete the mmhmem program";
pub const CHANGE_PROGRAM_RUNNING: &str = "Delete the mmhmem program (it is running \
now, so it is deleted as soon as setup closes)";
pub const RECORD_FILE: &str = "Delete the list setup keeps of what it installed";
pub const MEMORIES_FILE: &str = "Delete all your saved memories in this folder";

pub const MEMORIES_OPTION: &str = "Also delete my saved memories";
/// Contains `{dir}`.
pub const MEMORIES_HELP: &str = "Off: your saved memories stay in {dir}, so you can \
install mmhmem again later and keep them. On: that folder, with every saved memory and \
the activity log, is deleted at the very end. This cannot be undone.";
pub const LEGEND: &str =
    "↑↓ move · space delete my memories too · d show details · enter remove · esc back · q quit";
pub const HELP: &str = "y = remove mmhmem, m = switch \"Also delete my saved memories\" \
on or off, d <number> = show the details of one change, b = back, q = quit.";
pub const CONFIRM: &str = "Remove mmhmem? (y/n)";

/// Contains `{file}` and `{reason}`.
pub const FILE_UNREADABLE: &str = "{file} could not be read ({reason}), so its mmhmem \
settings were left as they are";
pub const KEPT_FOR_SKIPPED: &str = "The mmhmem program and the list of what setup \
installed were kept, because not every tool could be cleaned up. Fix the file named \
above, then run mmhmem setup and choose Remove again.";
pub const RECORD_DELETED: &str = "Deleted the list of what setup installed.";
/// Contains `{dir}`.
pub const BACKUPS_KEPT: &str = "Copies of your settings from before the removal are \
kept in {dir}, in case you need them. You can delete that folder.";
/// Contains `{dir}`.
pub const MEMORIES_KEPT: &str = "Your saved memories were kept in {dir}.";
pub const MEMORIES_DELETED: &str = "Your saved memories were deleted.";
/// Contains `{dir}` and `{reason}`.
pub const MEMORIES_FAILED: &str = "Your saved memories could not be deleted \
({reason}). They are still in {dir}: close your AI tools, then delete that folder.";
/// Contains `{reason}`.
pub const RECORD_FAILED: &str = "The list of what setup installed could not be deleted \
({reason}). Everything else was removed.";
pub const REMOVED: &str = "mmhmem was removed.";
pub const PROGRAM_LATER: &str = "The mmhmem program is deleted as soon as setup closes.";

pub const NEEDS_TERMINAL: &str = "mmhmem setup asks before it changes anything, and \
there is no terminal window here to ask in, so nothing was changed. To install, update \
or remove mmhmem, open a terminal window and run: mmhmem setup";

/// Contains `{reason}`.
pub const RESTART_FAILED: &str = "mmhmem was updated, but the new program could not \
be started ({reason}). Run mmhmem setup again to finish updating your tools.";

fn fill(template: &str, key: &str, value: &str) -> String {
    template.replace(&format!("{{{key}}}"), value)
}

/// The review line of the `PATH` removal.
pub fn change_path(folder: &str) -> String {
    fill(CHANGE_PATH, "folder", folder)
}

/// The explanation under the memories option.
pub fn memories_help(dir: &str) -> String {
    fill(MEMORIES_HELP, "dir", dir)
}

/// "<file> could not be read (<reason>), so …".
pub fn file_unreadable(file: &str, reason: &str) -> String {
    fill(FILE_UNREADABLE, "file", file).replace("{reason}", reason)
}

/// Where the backups of the removal are.
pub fn backups_kept(dir: &str) -> String {
    fill(BACKUPS_KEPT, "dir", dir)
}

/// Where the memories were kept.
pub fn memories_kept(dir: &str) -> String {
    fill(MEMORIES_KEPT, "dir", dir)
}

/// The memories could not be deleted.
pub fn memories_failed(dir: &str, reason: &str) -> String {
    fill(MEMORIES_FAILED, "dir", dir).replace("{reason}", reason)
}

/// The record could not be deleted.
pub fn record_failed(reason: &str) -> String {
    fill(RECORD_FAILED, "reason", reason)
}

/// The new program could not be started after the update.
pub fn restart_failed(reason: &str) -> String {
    fill(RESTART_FAILED, "reason", reason)
}

/// Every string above (for the plain-language test).
pub const ALL: &[&str] = &[
    INTRO,
    CHANGE_PATH,
    CHANGE_PROGRAM,
    CHANGE_PROGRAM_RUNNING,
    RECORD_FILE,
    MEMORIES_FILE,
    MEMORIES_OPTION,
    MEMORIES_HELP,
    LEGEND,
    HELP,
    CONFIRM,
    FILE_UNREADABLE,
    KEPT_FOR_SKIPPED,
    RECORD_DELETED,
    BACKUPS_KEPT,
    MEMORIES_KEPT,
    MEMORIES_DELETED,
    MEMORIES_FAILED,
    RECORD_FAILED,
    REMOVED,
    PROGRAM_LATER,
    NEEDS_TERMINAL,
    RESTART_FAILED,
];
