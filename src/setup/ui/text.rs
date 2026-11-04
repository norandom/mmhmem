//! Every user-facing string of the setup menu, the numbered fallback and the
//! no-terminal path, in one place (task 6.1, requirements 1.4 and 1.5).
//!
//! Written for someone who has never heard of mmhmem: no project jargon
//! unless the same string explains it. [`ALL`] lists every string so a test
//! can check that rule.

use crate::setup::record::AgentId;

pub const LEGEND: &str = "↑↓ move · space select · enter continue · esc back · q quit";

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

pub const REVIEW_NOTHING: &str = "Everything is current. Nothing needs to change.";
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
pub const PROMPT_CHOICE: &str = "Type a number and press enter:";
pub const PLACEHOLDER: &str = "The setup menu is coming soon. Nothing was changed.";
pub const BAD_ARGUMENT: &str = "mmhmem setup does not take any options. \
Just run: mmhmem setup";

pub const NEXT_CLAUDE_CODE: &str = "Claude Code: start a new session.";
pub const NEXT_CODEX: &str = "Codex: start a new session, and approve the read-routing \
hook when Codex asks, if you turned it on.";
pub const NEXT_OPENCODE: &str = "OpenCode: start a new session.";
pub const NEXT_DESKTOP: &str = "Claude Desktop: quit it completely and open it again.";
pub const NEXT_ANTIGRAVITY: &str = "Antigravity: import the snippet that setup saved, \
then start a new session.";

/// "Update to v1.2.3".
pub fn update_to(version: &str) -> String {
    ACTION_UPDATE_TO.replace("{version}", version)
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
#[cfg(test)]
pub(crate) const ALL: &[&str] = &[
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
    REVIEW_BACKUP,
    CONFIRM,
    CANCELLED,
    NO_AGENT,
    CHECKING,
    DONE,
    NO_TERMINAL,
    DUMB_TERMINAL,
    PROMPT_CHOICE,
    PLACEHOLDER,
    BAD_ARGUMENT,
    NEXT_CLAUDE_CODE,
    NEXT_CODEX,
    NEXT_OPENCODE,
    NEXT_DESKTOP,
    NEXT_ANTIGRAVITY,
];
