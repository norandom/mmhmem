//! The plain-language change summaries of the review screen (requirement
//! 1.4), one place for every agent and for the adoption plan. Each line says
//! what the change does for the user first and names the file in brackets.

use std::path::Path;

/// What a change is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum What {
    /// mmhmem's entry in the agent's list of memory servers.
    Server,
    /// The hook that sends large file reads through mmhmem.
    ReadHook,
    /// The OpenCode plugin that does the same.
    ReadPlugin,
    /// The guide that teaches the agent to use the memory.
    Guide,
    /// The command that shows the last session.
    Recap,
    /// The short notes in the agent's instructions file.
    Instructions,
}

/// Every kind, for tests.
pub const ALL_WHAT: [What; 6] = [
    What::Server,
    What::ReadHook,
    What::ReadPlugin,
    What::Guide,
    What::Recap,
    What::Instructions,
];

impl What {
    /// "mmhmem in Codex's server list" and the like.
    fn noun(self, agent: &str) -> String {
        match self {
            Self::Server => format!("mmhmem in {agent}'s server list"),
            Self::ReadHook => format!("mmhmem's read hook for {agent}"),
            Self::ReadPlugin => format!("mmhmem's read plugin for {agent}"),
            Self::Guide => format!("the memory guide for {agent}"),
            Self::Recap => format!("the recap command for {agent}"),
            Self::Instructions => format!("mmhmem's memory notes for {agent}"),
        }
    }

    /// What adding it does, then what is touched.
    fn add(self, agent: &str, file: &str) -> String {
        match self {
            Self::Server => {
                format!("Let {agent} use mmhmem (adds mmhmem to {agent}'s server list in {file})")
            }
            Self::ReadHook => {
                format!("Send {agent}'s large file reads through mmhmem (adds a hook to {file})")
            }
            Self::ReadPlugin => format!(
                "Send {agent}'s large file reads through mmhmem (adds the plugin file {file})"
            ),
            Self::Guide => format!("Give {agent} the memory guide (adds the file {file})"),
            Self::Recap => format!("Add the recap command to {agent} (adds the file {file})"),
            Self::Instructions => format!(
                "Tell {agent} how to use the shared memory (adds a short section to {file})"
            ),
        }
    }
}

/// "Let Codex use mmhmem (adds mmhmem to Codex's server list in <file>)".
pub fn add(what: What, agent: &str, file: &Path) -> String {
    what.add(agent, &file.display().to_string())
}

/// "Update mmhmem in Codex's server list (<file>)".
pub fn update(what: What, agent: &str, file: &Path) -> String {
    format!("Update {} ({})", what.noun(agent), file.display())
}

/// "Keep mmhmem in Codex's server list as it is and track it (<file>)".
pub fn keep(what: What, agent: &str, file: &Path) -> String {
    format!(
        "Keep {} as it is and track it ({})",
        what.noun(agent),
        file.display()
    )
}

/// "Remove mmhmem from Codex (changes <file>)".
pub fn remove(agent: &str, file: &Path) -> String {
    format!("Remove mmhmem from {agent} (changes {})", file.display())
}

/// The old installer's marker comment lines are taken out.
pub const ADOPT_MARKERS: &str = "Only the old installer's two marker comment lines are \
removed; the rest of the file stays as it is.";

/// "Found mmhmem in Codex's server list (made by the old installer); keep
/// and track it."
pub fn adopt(what: What, agent: &str, old_installer: bool, markers: bool) -> String {
    let origin = if old_installer {
        " (made by the old installer)"
    } else {
        ""
    };
    let mut line = format!("Found {}{origin}; keep and track it.", what.noun(agent));
    if markers {
        line.push(' ');
        line.push_str(ADOPT_MARKERS);
    }
    line
}

/// "Antigravity: the read hook cannot be installed because …." (a selected
/// feature left out while the agent's other changes go ahead).
pub fn left_out(agent: &str, reason: &str) -> String {
    format!("{agent}: {}.", reason.trim_end_matches('.'))
}

/// "Claude Code is not installed".
pub fn not_installed(agent: &str) -> String {
    format!("{agent} is not installed")
}
