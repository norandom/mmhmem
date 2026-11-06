//! The words of the update mode (owner decision 2026-11-06, Requirement
//! 7.5): `mmhmem update`, the home title of an installed system and the
//! install command run again on an installed system. Same plain-language
//! rule as the rest of `text.rs`; [`ALL`] lists every string for the test.

/// Contains `{version}`; see [`set_up_title`].
pub const TITLE_SET_UP: &str = "mmhmem {version} is set up";

pub const NOT_SET_UP: &str = "mmhmem is not set up on this computer yet, so there is \
nothing to update and nothing was changed. To install it, run: mmhmem setup";
pub const NEEDS_TERMINAL: &str = "mmhmem update asks before it changes anything, and \
there is no terminal window here to ask in, so nothing was changed. To update mmhmem, \
open a terminal window and run: mmhmem update";
pub const BAD_ARGUMENT: &str = "mmhmem update does not take any options. \
Just run: mmhmem update";
pub const CHECKING_NEWEST: &str = "Looking for a newer release...";
pub const NEWEST_UNKNOWN: &str = "Could not check for a newer release (no internet \
connection, or the release server did not answer), so nothing was changed. Please try \
again later.";
/// Contains `{version}`.
pub const UP_TO_DATE: &str = "mmhmem {version} is up to date.";
/// Contains `{installed}` and `{newest}`.
pub const AVAILABLE: &str = "mmhmem {installed} is installed. {newest} is available.";
/// Contains `{installed}` and `{newest}`.
pub const BOOTSTRAP_AVAILABLE: &str = "mmhmem {installed} is set up. This installer \
has {newest}.";
pub const KEEPS_CHOICES: &str = "Your tools and choices stay as they are.";
pub const PROMPT_UPDATE: &str = "Update now? [Y/n]";
pub const PROMPT_APPLY: &str = "Apply these changes? [Y/n]";
pub const BAD_ANSWER: &str = "Please type y for yes or n for no, then press enter.";
/// Contains `{version}`.
pub const DOWNLOADING: &str = "Downloading {version} ...";
pub const VERIFIED: &str = "signature and checksum OK.";
/// Contains `{version}`.
pub const UPDATED_CHECKING: &str = "Updated to {version}. Checking your tools...";
/// Contains `{version}`.
pub const UPDATED: &str = "Updated to {version}.";
/// Contains `{version}`.
pub const UPDATED_NO_CHANGES: &str = "mmhmem was updated to {version}. Your tools \
needed no changes, so setup only checked that they still work.";
pub const CHANGES_NEEDED: &str = "The new version also needs to change these files \
in your tools (each file is backed up first):";
pub const MAKING_CHANGES: &str = "Making the changes, then checking your tools...";
pub const CHANGES_KEPT: &str = "Your tools' files were left as they are. To see these \
changes again and make them, run: mmhmem setup";
pub const NOTHING_CHANGED: &str = "Nothing was changed.";
pub const DONE: &str = "Done.";

fn fill(template: &str, values: &[(&str, &str)]) -> String {
    values
        .iter()
        .fold(template.to_owned(), |text, (key, value)| {
            text.replace(&format!("{{{key}}}"), value)
        })
}

/// "mmhmem v1.2.3 is set up".
pub fn set_up_title(version: &str) -> String {
    fill(TITLE_SET_UP, &[("version", version)])
}

/// "mmhmem v1.2.3 is up to date.".
pub fn up_to_date(version: &str) -> String {
    fill(UP_TO_DATE, &[("version", version)])
}

/// "mmhmem v1 is installed. v2 is available.".
pub fn available(installed: &str, newest: &str) -> String {
    fill(AVAILABLE, &[("installed", installed), ("newest", newest)])
}

/// "mmhmem v1 is set up. This installer has v2.".
pub fn bootstrap_available(installed: &str, newest: &str) -> String {
    fill(
        BOOTSTRAP_AVAILABLE,
        &[("installed", installed), ("newest", newest)],
    )
}

/// "Downloading v1.2.3 ...".
pub fn downloading(version: &str) -> String {
    fill(DOWNLOADING, &[("version", version)])
}

/// "Updated to v1.2.3. Checking your tools...".
pub fn updated_checking(version: &str) -> String {
    fill(UPDATED_CHECKING, &[("version", version)])
}

/// "Updated to v1.2.3.".
pub fn updated(version: &str) -> String {
    fill(UPDATED, &[("version", version)])
}

/// The results heading of the menu after an update that changed no file.
pub fn updated_no_changes(version: &str) -> String {
    fill(UPDATED_NO_CHANGES, &[("version", version)])
}

/// Every string above (for the plain-language test).
pub const ALL: &[&str] = &[
    TITLE_SET_UP,
    NOT_SET_UP,
    NEEDS_TERMINAL,
    BAD_ARGUMENT,
    CHECKING_NEWEST,
    NEWEST_UNKNOWN,
    UP_TO_DATE,
    AVAILABLE,
    BOOTSTRAP_AVAILABLE,
    KEEPS_CHOICES,
    PROMPT_UPDATE,
    PROMPT_APPLY,
    BAD_ANSWER,
    DOWNLOADING,
    VERIFIED,
    UPDATED_CHECKING,
    UPDATED,
    UPDATED_NO_CHANGES,
    CHANGES_NEEDED,
    MAKING_CHANGES,
    CHANGES_KEPT,
    NOTHING_CHANGED,
    DONE,
];
