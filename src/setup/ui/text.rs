//! Every user-facing string of the setup menu, the numbered fallback and the
//! no-terminal path, in one place (task 6.1, requirements 1.4 and 1.5).
//!
//! Written for someone who has never heard of mmhmem: no project jargon
//! unless the same string explains it. [`ALL`] lists every string so a test
//! can check that rule.

use crate::setup::record::{AgentId, Feature, PendingUserStep};

#[path = "text_changes.rs"]
pub mod changes;
#[path = "text_remove.rs"]
pub mod remove;

pub const LEGEND: &str = "↑↓ move · space select · enter continue · esc back · q quit";
pub const LEGEND_HOME: &str = "↑↓ move · enter continue · q quit";
pub const LEGEND_REVIEW: &str =
    "↑↓ move · d show details · enter make these changes · esc back · q quit";
pub const LEGEND_DIFF: &str = "↑↓ scroll · d or esc close the details · q quit";
pub const LEGEND_WORKING: &str = "Please wait...";
pub const LEGEND_DONE: &str = "↑↓ scroll · enter or q close";

pub const TITLE_HOME: &str = "mmhmem setup";
pub const TITLE_AGENTS: &str = "Which tools should mmhmem work with?";
pub const TITLE_FEATURES: &str = "Which features do you want?";
pub const TITLE_REVIEW: &str = "Review: this is what will change";
pub const TITLE_PROGRESS: &str = "Setting things up";

pub const INTRO: &str = "mmhmem gives your AI coding tools a shared memory, so a new \
session can pick up where the last one stopped.";

pub const ACTION_INSTALL: &str = "Install or update";
pub const ACTION_CHANGE: &str = "Change agents and features";
pub const ACTION_CHECK: &str = "Check that everything works";
pub const ACTION_REMOVE: &str = "Remove mmhmem";
/// Contains `{version}`; see [`update_to`].
pub const ACTION_UPDATE_TO: &str = "Update to {version}";

pub const FEATURE_MEMORY: &str = "Shared memory (required)";
pub const FEATURE_MEMORY_HELP: &str =
    "Lets your tools save notes and find them again in later sessions.";
pub const FEATURE_SKILL: &str = "Memory guide";
pub const FEATURE_SKILL_HELP: &str =
    "A short guide your tool reads to learn how to use the shared memory.";
pub const FEATURE_RECAP: &str = "Recap command";
pub const FEATURE_RECAP_HELP: &str =
    "Adds a command that shows what you were working on last time.";
pub const FEATURE_INSTRUCTIONS: &str = "Agent instructions";
pub const FEATURE_INSTRUCTIONS_HELP: &str =
    "Short notes that tell each tool how to use the shared memory.";
pub const FEATURE_HOOK: &str = "Read-routing hook";
pub const FEATURE_HOOK_HELP: &str = "Sends large file reads through mmhmem to save tokens \
(off by default).";
pub const FEATURE_WORKER: &str = "Worker model (optional)";
pub const FEATURE_WORKER_HELP: &str = "Worker model: a small AI model that summarizes long \
files for you. It needs extra setup, so it is left off unless you ask for it.";

pub const WORKER_LATER: &str = "Worker model: setup will show you how to connect one \
after the install. Nothing about it changes now.";

pub const COL_AGENT: &str = "Tool";
pub const COL_STATUS: &str = "mmhmem";
pub const COL_FEATURES: &str = "Features";
pub const COL_CHECK: &str = "Last check";
pub const STATUS_INSTALLED: &str = "installed";
pub const STATUS_NOT_SET_UP: &str = "not set up";
pub const STATUS_NOT_FOUND: &str = "not found";
pub const SHORT_MEMORY: &str = "memory";
pub const SHORT_SKILL: &str = "guide";
pub const SHORT_RECAP: &str = "recap";
pub const SHORT_INSTRUCTIONS: &str = "instructions";
pub const SHORT_HOOK: &str = "read hook";
pub const PENDING_APPROVE_HOOK: &str = "approve the hook in Codex";
pub const PENDING_RESTART_DESKTOP: &str = "restart Claude Desktop";
pub const PENDING_IMPORT_SNIPPET: &str = "import the snippet in Antigravity";
/// Contains `{version}`.
pub const INSTALLED_VERSION: &str = "Installed: {version}";
pub const NOTHING_INSTALLED: &str = "Installed: nothing yet";
/// Contains `{version}`.
pub const NEWEST: &str = "Newest release: {version}";
pub const NEWEST_CHECKING: &str = "Newest release: checking...";
pub const NEWEST_UNKNOWN: &str = "Newest release: unknown (offline?)";

/// Contains `{agent}` and `{reason}`.
pub const AGENT_UNAVAILABLE: &str = "{agent}: not found ({reason})";
/// Contains `{path}`.
pub const PROFILE_CHOICE: &str = "Use the Claude Desktop profile in {path}";
pub const NOTHING_SELECTED: &str = "Choose at least one tool, or press q to quit.";
pub const UPDATE_NO_KEY: &str = "Updates are turned off until the release signing key \
is added to mmhmem, so nothing was downloaded. Nothing was changed.";
pub const UPDATE_FAILED: &str = "The update did not finish; the reason is shown above.";
pub const UPDATE_NO_PLATFORM: &str = "There are no mmhmem releases for this computer's \
system, so mmhmem cannot update itself. Nothing was changed.";
/// Contains `{version}`.
pub const UPDATED: &str = "The mmhmem program was updated to {version}. Below is what \
changes in your tools.";
/// Contains `{sha}`.
pub const CHANGE_BINARY_ADD: &str = "Install the mmhmem program here (SHA-256 {sha})";
/// Contains `{sha}`.
pub const CHANGE_BINARY_UPDATE: &str = "Replace the mmhmem program here with this \
release's (SHA-256 {sha})";
/// Contains `{folder}`.
pub const CHANGE_PATH_ADD: &str = "Add {folder} to your PATH, so a terminal can start \
mmhmem by name";
/// Contains `{folder}`.
pub const PATH_HINT: &str = "Terminal: to start mmhmem by name, add this line to your \
shell's start-up file (for example ~/.profile), then open a new terminal: \
export PATH=\"{folder}:$PATH\"";

pub const RECORD_UNREADABLE: &str = "The file where setup keeps track of what it \
installed could not be read, so setup looked for an earlier install instead. \
Nothing changes until you confirm.";
pub const RECOVERED: &str = "An earlier setup was interrupted. Setup worked out \
where it stopped.";
/// Contains `{file}`.
pub const RECOVERY_UNCLEAR: &str = "An earlier setup was interrupted while changing \
{file}, and that file now matches neither the old nor the new version. Please check it.";
/// Contains `{file}`.
pub const DRIFT_CHANGED: &str = "{file} was changed outside setup. The review shows \
the difference before anything is overwritten.";
/// Contains `{file}`.
pub const DRIFT_MISSING: &str = "{file} was deleted outside setup.";
/// Contains `{file}` and `{reason}`.
pub const DRIFT_UNREADABLE: &str = "{file} could not be read: {reason}.";
/// Contains `{agent}`.
pub const ADOPT_FOUND: &str = "Found an earlier mmhmem setup for {agent}. It will be \
kept and tracked.";
pub const ADOPT_REVIEW: &str = "First, setup takes over what an earlier install made. \
After that you will see the changes for your choices.";
pub const BINARY_CHANGED: &str = "The installed mmhmem program is not the one setup \
installed.";
pub const BINARY_MISSING: &str = "The installed mmhmem program is missing.";

pub const PATH_SETTING: &str = "your PATH setting";
pub const NO_HOME: &str = "Setup could not find your home folder, so nothing was changed.";

pub const REVIEW_PROBLEMS: &str = "These tools will be skipped:";
/// Contains `{agent}` and `{reason}`.
pub const AGENT_SKIPPED: &str = "{agent} was skipped: {reason}.";
/// Contains `{agent}` and `{count}`.
pub const CHANGES_MADE: &str = "{agent}: {count} change(s) made.";
/// Contains `{dir}`.
pub const BACKUPS_IN: &str = "Your original files were backed up to {dir}.";

pub const REVIEW_NOTHING: &str = "Everything is current. Nothing needs to change.";
/// The home line instead of [`REVIEW_NOTHING`] when a newer release is known.
pub const HOME_UPDATE_AVAILABLE: &str =
    "Your tools are set up as you chose. A newer version of mmhmem is available.";
pub const REVIEW_BACKUP: &str = "Every file is backed up before it is changed.";
pub const CONFIRM: &str = "Make these changes? (y/n)";
pub const CANCELLED: &str = "Nothing was changed.";
pub const NO_AGENT: &str = "No supported AI tool was found on this computer, \
so nothing was changed.";

pub const CHECKING: &str = "Checking that each tool can start and use mmhmem...";
pub const DONE: &str = "Done. Next steps:";
pub const NO_TERMINAL: &str = "No interactive terminal was found, so mmhmem is \
using the standard choices.";
pub const DUMB_TERMINAL: &str = "This terminal cannot show a menu, so mmhmem will \
ask numbered questions instead.";
/// Contains `{reason}`.
pub const MENU_STOPPED: &str = "The menu stopped because the terminal did not answer \
({reason}).";
pub const PROMPT_CHOICE: &str = "Type a number and press enter:";
pub const BAD_CHOICE: &str = "That is not one of the choices shown. Please try again.";
pub const HELP_HOME: &str = "Type a number to choose, or q to quit.";
pub const HELP_AGENTS: &str =
    "Type numbers to switch tools on or off, Enter to continue, b = back, q = quit.";
pub const HELP_FEATURES: &str =
    "Type numbers to switch features on or off, Enter to continue, b = back, q = quit.";
pub const HELP_REVIEW: &str =
    "y = make these changes, d <number> = show the details of one change, b = back, q = quit.";
pub const BAD_ARGUMENT: &str = "mmhmem setup does not take any options. \
Just run: mmhmem setup";

pub const NEXT_CLAUDE_CODE: &str = "Claude Code: start a new session.";
pub const NEXT_CODEX: &str = "Codex: start a new session, and approve the read-routing \
hook when Codex asks, if you turned it on.";
pub const NEXT_OPENCODE: &str = "OpenCode: start a new session.";
pub const NEXT_DESKTOP: &str = "Claude Desktop: quit it completely and open it again.";
pub const NEXT_ANTIGRAVITY: &str = "Antigravity: import the snippet that setup saved, \
then start a new session.";

/// Fills `{key}` placeholders in `template`.
fn fill(template: &str, values: &[(&str, &str)]) -> String {
    values
        .iter()
        .fold(template.to_owned(), |text, (key, value)| {
            text.replace(&format!("{{{key}}}"), value)
        })
}

/// "Installed: v1.2.3", or "Installed: nothing yet".
pub fn installed_version(version: Option<&str>) -> String {
    version.map_or_else(
        || NOTHING_INSTALLED.to_owned(),
        |v| fill(INSTALLED_VERSION, &[("version", v)]),
    )
}

/// "Newest release: v1.2.3".
pub fn newest(version: &str) -> String {
    fill(NEWEST, &[("version", version)])
}

/// "Codex: not found (reason)".
pub fn agent_unavailable(agent: &str, reason: &str) -> String {
    fill(AGENT_UNAVAILABLE, &[("agent", agent), ("reason", reason)])
}

/// "Use the Claude Desktop profile in <path>".
pub fn profile_choice(path: &str) -> String {
    fill(PROFILE_CHOICE, &[("path", path)])
}

/// The interrupted-change line for `file`.
pub fn recovery_unclear(file: &str) -> String {
    fill(RECOVERY_UNCLEAR, &[("file", file)])
}

/// "<file> was changed outside setup. ...".
pub fn drift_changed(file: &str) -> String {
    fill(DRIFT_CHANGED, &[("file", file)])
}

/// "<file> was deleted outside setup.".
pub fn drift_missing(file: &str) -> String {
    fill(DRIFT_MISSING, &[("file", file)])
}

/// "<file> could not be read: <reason>.".
pub fn drift_unreadable(file: &str, reason: &str) -> String {
    fill(DRIFT_UNREADABLE, &[("file", file), ("reason", reason)])
}

/// "Found an earlier mmhmem setup for <agent>. ...".
pub fn adopt_found(agent: &str) -> String {
    fill(ADOPT_FOUND, &[("agent", agent)])
}

/// "<agent> was skipped: <reason>.".
pub fn agent_skipped(agent: &str, reason: &str) -> String {
    let reason = reason.trim_end_matches('.');
    fill(AGENT_SKIPPED, &[("agent", agent), ("reason", reason)])
}

/// "<agent>: <n> change(s) made.".
pub fn changes_made(agent: &str, count: usize) -> String {
    fill(
        CHANGES_MADE,
        &[("agent", agent), ("count", &count.to_string())],
    )
}

/// "Your original files were backed up to <dir>.".
pub fn backups_in(dir: &str) -> String {
    fill(BACKUPS_IN, &[("dir", dir)])
}

/// The short name of a feature for the home screen.
pub fn feature_short(feature: Feature) -> &'static str {
    match feature {
        Feature::Mcp => SHORT_MEMORY,
        Feature::Skill => SHORT_SKILL,
        Feature::RecapCommand => SHORT_RECAP,
        Feature::Instructions => SHORT_INSTRUCTIONS,
        Feature::ReadHook => SHORT_HOOK,
    }
}

/// The name and one-line explanation of a feature (features screen).
pub fn feature_label(feature: Feature) -> (&'static str, &'static str) {
    match feature {
        Feature::Mcp => (FEATURE_MEMORY, FEATURE_MEMORY_HELP),
        Feature::Skill => (FEATURE_SKILL, FEATURE_SKILL_HELP),
        Feature::RecapCommand => (FEATURE_RECAP, FEATURE_RECAP_HELP),
        Feature::Instructions => (FEATURE_INSTRUCTIONS, FEATURE_INSTRUCTIONS_HELP),
        Feature::ReadHook => (FEATURE_HOOK, FEATURE_HOOK_HELP),
    }
}

/// A step only the user can do, in a few words.
pub fn pending_short(step: PendingUserStep) -> &'static str {
    match step {
        PendingUserStep::ApproveHookInCodex => PENDING_APPROVE_HOOK,
        PendingUserStep::RestartClaudeDesktop => PENDING_RESTART_DESKTOP,
        PendingUserStep::ImportAntigravitySnippet => PENDING_IMPORT_SNIPPET,
    }
}

/// The line telling Linux and macOS users how to put `folder` on `PATH`.
pub fn path_hint(folder: &str) -> String {
    fill(PATH_HINT, &[("folder", folder)])
}

/// "The mmhmem program was updated to v1.2.3. …".
pub fn updated(version: &str) -> String {
    fill(UPDATED, &[("version", version)])
}

/// The review line of the program copy; `sha` is shortened to 12 digits.
pub fn change_binary(update: bool, sha: &str) -> String {
    let short = &sha[..sha.len().min(12)];
    let template = if update {
        CHANGE_BINARY_UPDATE
    } else {
        CHANGE_BINARY_ADD
    };
    fill(template, &[("sha", short)])
}

/// The review line of the `PATH` change.
pub fn change_path(folder: &str) -> String {
    fill(CHANGE_PATH_ADD, &[("folder", folder)])
}

/// Why the menu stopped (reading keys or drawing failed).
pub fn menu_stopped(reason: &str) -> String {
    fill(MENU_STOPPED, &[("reason", reason)])
}

/// "Update to v1.2.3".
pub fn update_to(version: &str) -> String {
    fill(ACTION_UPDATE_TO, &[("version", version)])
}

/// What the user still has to do for `agent` after an install.
pub fn next_step(agent: AgentId) -> &'static str {
    match agent {
        AgentId::ClaudeCode => NEXT_CLAUDE_CODE,
        AgentId::ClaudeDesktop => NEXT_DESKTOP,
        AgentId::Codex => NEXT_CODEX,
        AgentId::OpenCode => NEXT_OPENCODE,
        AgentId::Antigravity => NEXT_ANTIGRAVITY,
    }
}

/// Every string above (for the plain-language test).
pub const ALL: &[&str] = &[
    LEGEND,
    TITLE_HOME,
    TITLE_AGENTS,
    TITLE_FEATURES,
    TITLE_REVIEW,
    TITLE_PROGRESS,
    INTRO,
    ACTION_INSTALL,
    ACTION_CHANGE,
    ACTION_CHECK,
    ACTION_REMOVE,
    ACTION_UPDATE_TO,
    FEATURE_MEMORY,
    FEATURE_MEMORY_HELP,
    FEATURE_RECAP,
    FEATURE_RECAP_HELP,
    FEATURE_INSTRUCTIONS,
    FEATURE_INSTRUCTIONS_HELP,
    FEATURE_HOOK,
    FEATURE_HOOK_HELP,
    FEATURE_WORKER,
    FEATURE_WORKER_HELP,
    REVIEW_NOTHING,
    HOME_UPDATE_AVAILABLE,
    REVIEW_BACKUP,
    CONFIRM,
    CANCELLED,
    NO_AGENT,
    CHECKING,
    DONE,
    NO_TERMINAL,
    DUMB_TERMINAL,
    MENU_STOPPED,
    PROMPT_CHOICE,
    BAD_CHOICE,
    HELP_HOME,
    HELP_AGENTS,
    HELP_FEATURES,
    HELP_REVIEW,
    BAD_ARGUMENT,
    NEXT_CLAUDE_CODE,
    NEXT_CODEX,
    NEXT_OPENCODE,
    NEXT_DESKTOP,
    NEXT_ANTIGRAVITY,
    LEGEND_HOME,
    LEGEND_REVIEW,
    LEGEND_DIFF,
    LEGEND_WORKING,
    LEGEND_DONE,
    FEATURE_SKILL,
    FEATURE_SKILL_HELP,
    WORKER_LATER,
    COL_AGENT,
    COL_STATUS,
    COL_FEATURES,
    COL_CHECK,
    STATUS_INSTALLED,
    STATUS_NOT_SET_UP,
    STATUS_NOT_FOUND,
    SHORT_MEMORY,
    SHORT_SKILL,
    SHORT_RECAP,
    SHORT_INSTRUCTIONS,
    SHORT_HOOK,
    PENDING_APPROVE_HOOK,
    PENDING_RESTART_DESKTOP,
    PENDING_IMPORT_SNIPPET,
    INSTALLED_VERSION,
    NOTHING_INSTALLED,
    NEWEST,
    NEWEST_CHECKING,
    NEWEST_UNKNOWN,
    AGENT_UNAVAILABLE,
    PROFILE_CHOICE,
    NOTHING_SELECTED,
    RECORD_UNREADABLE,
    RECOVERED,
    RECOVERY_UNCLEAR,
    DRIFT_CHANGED,
    DRIFT_MISSING,
    DRIFT_UNREADABLE,
    ADOPT_FOUND,
    ADOPT_REVIEW,
    BINARY_CHANGED,
    BINARY_MISSING,
    REVIEW_PROBLEMS,
    AGENT_SKIPPED,
    CHANGES_MADE,
    BACKUPS_IN,
    PATH_SETTING,
    NO_HOME,
    UPDATE_NO_KEY,
    UPDATE_FAILED,
    UPDATE_NO_PLATFORM,
    UPDATED,
    CHANGE_BINARY_ADD,
    CHANGE_BINARY_UPDATE,
    CHANGE_PATH_ADD,
    PATH_HINT,
];
