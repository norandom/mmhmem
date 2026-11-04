//! The MCP functional check (reliable-installers task 4.1,
//! `.kiro/specs/reliable-installers/design.md` §6 steps 1–2; Requirements
//! 6.1 and 6.5).
//!
//! [`check_mcp`] starts mmhmem exactly as an agent's config starts it
//! ([`Registration`], read back from the config file's bytes), with the
//! `rmcp` child-process client on a small current-thread runtime, and then:
//! completes `initialize`, runs `tools/list` and requires every expected
//! tool, and calls the read-only `status` tool for project
//! [`CHECK_PROJECT`] and harness [`CHECK_HARNESS`], requiring `store ok` and
//! the expected version (research.md §10). One overall timeout covers all
//! steps; the child is always stopped at the end.
//!
//! The result ([`McpCheck`]) maps onto the record's [`CheckDetail`]s, plus
//! one plain-language reason and a "what you can do" line for the first
//! failing step (wording in `check_text.rs`). The child's error output is
//! kept, at most [`MAX_DIAGNOSTICS`] bytes with environment values blanked,
//! for a log only; it is never part of `Display` or `Debug`.
//!
//! Submodules: `check_registration.rs` ([`Registration`] and its readers),
//! `check_session.rs` (the async client session), `check_text.rs` (every
//! sentence of the check, MCP and agent steps), `check_agent.rs` ([`agent`]:
//! the steps every agent check shares, task 4.2) and `check_hook.rs` (the
//! read-hook sample).

use std::fmt;
use std::io::ErrorKind;
use std::path::PathBuf;
use std::time::Duration;

use crate::setup::agents::AgentId;
use crate::setup::record::{CheckDetail, CheckResult, CheckStep, StepOutcome};

#[path = "check_agent.rs"]
pub mod agent;
#[path = "check_hook.rs"]
mod hook;
#[path = "check_registration.rs"]
mod registration;
#[path = "check_session.rs"]
mod session;
#[path = "check_text.rs"]
mod text;

pub use registration::{Registration, RegistrationError};

/// The tools this release offers over MCP (compared with the real server's
/// `tools/list` in `tests/setup_check.rs`).
pub const EXPECTED_TOOLS: [&str; 10] = [
    "session_resume_or_handoff",
    "smart_read",
    "boilerplate_write",
    "mem_search",
    "mem_get_observations",
    "mem_record_observation",
    "shunt_warmup",
    "shunt_prune",
    "status",
    "mem_search_breadcrumbs",
];

/// The project name the check passes to the `status` tool.
pub const CHECK_PROJECT: &str = "mmhmem-install-check";
/// The harness name the check passes to the `status` tool.
pub const CHECK_HARNESS: &str = "mmhmem-setup";
/// The overall limit for one check (design.md §6).
pub const DEFAULT_CHECK_TIMEOUT: Duration = Duration::from_secs(20);
/// At most this many bytes of the child's error output are kept.
pub const MAX_DIAGNOSTICS: usize = 4096;

/// The MCP steps, in the order they run.
pub const MCP_STEPS: [CheckStep; 4] = [
    CheckStep::Registration,
    CheckStep::Handshake,
    CheckStep::ToolsList,
    CheckStep::StatusTool,
];

/// Why the check failed. Holds only paths, tool names, versions, steps and
/// fixed codes, never program output or environment values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckFailure {
    /// The agent's config could not be read back.
    Registration(RegistrationError),
    /// The registered program does not exist.
    ProgramMissing {
        /// The program as registered.
        program: PathBuf,
    },
    /// The registered start folder does not exist.
    FolderMissing {
        /// The folder as registered.
        cwd: PathBuf,
    },
    /// The system refused to start the program.
    CannotStart {
        /// The program as registered.
        program: PathBuf,
        /// The kind of error the system gave.
        kind: ErrorKind,
    },
    /// The program ended during this step.
    ExitedEarly {
        /// The step that was running.
        step: CheckStep,
    },
    /// No answer before the overall timeout.
    NoAnswer {
        /// The step that was running.
        step: CheckStep,
        /// The limit in whole seconds.
        seconds: u64,
    },
    /// The program answered, but not as an MCP server.
    NotMcp {
        /// The step that was running.
        step: CheckStep,
    },
    /// Expected tools are missing from `tools/list`.
    MissingTools {
        /// The missing names (from the expected list).
        names: Vec<String>,
    },
    /// The `status` tool returned an error.
    StatusFailed,
    /// The `status` tool did not report `store ok`.
    StoreNotReady,
    /// The `status` tool reported another version.
    WrongVersion {
        /// The version the check wanted.
        expected: String,
    },
    /// The check could not run (no async runtime).
    Internal,
}

impl CheckFailure {
    /// The step this failure belongs to.
    pub fn step(&self) -> CheckStep {
        match self {
            Self::Registration(_)
            | Self::ProgramMissing { .. }
            | Self::FolderMissing { .. }
            | Self::CannotStart { .. }
            | Self::Internal => CheckStep::Registration,
            Self::ExitedEarly { step } | Self::NoAnswer { step, .. } | Self::NotMcp { step } => {
                *step
            }
            Self::MissingTools { .. } => CheckStep::ToolsList,
            Self::StatusFailed | Self::StoreNotReady | Self::WrongVersion { .. } => {
                CheckStep::StatusTool
            }
        }
    }
}

impl fmt::Display for CheckFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&text::reason(self))
    }
}

/// The result of one MCP functional check.
#[derive(Clone, PartialEq, Eq)]
pub struct McpCheck {
    /// One outcome per [`MCP_STEPS`] entry, in that order.
    pub details: Vec<CheckDetail>,
    /// The first failure, `None` when every step passed.
    pub failure: Option<CheckFailure>,
    /// The tool names the server listed, sorted (empty when the list did
    /// not run). Names that are not plain identifiers are left out.
    pub tools: Vec<String>,
    /// The child's process id while it ran (for diagnostics and tests).
    pub pid: Option<u32>,
    program: PathBuf,
    diagnostics: String,
}

impl fmt::Debug for McpCheck {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("McpCheck")
            .field("details", &self.details)
            .field("failure", &self.failure)
            .field("tools", &self.tools)
            .field("pid", &self.pid)
            .field("diagnostics_bytes", &self.diagnostics.len())
            .finish()
    }
}

impl fmt::Display for McpCheck {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.summary())
    }
}

impl McpCheck {
    /// The result for a registration that could not be read back.
    pub fn registration_failed(error: RegistrationError) -> Self {
        Self::new(PathBuf::new(), Some(CheckFailure::Registration(error)))
    }

    pub(super) fn new(program: PathBuf, failure: Option<CheckFailure>) -> Self {
        let failed_at = failure.as_ref().map(CheckFailure::step);
        let mut seen_failure = false;
        let details = MCP_STEPS
            .iter()
            .map(|&step| {
                let outcome = if seen_failure {
                    StepOutcome::Skipped
                } else if Some(step) == failed_at {
                    seen_failure = true;
                    StepOutcome::Fail
                } else {
                    StepOutcome::Pass
                };
                CheckDetail { step, outcome }
            })
            .collect();
        Self {
            details,
            failure,
            tools: Vec::new(),
            pid: None,
            program,
            diagnostics: String::new(),
        }
    }

    /// A result as the probe would give it, for tests of agent checks that
    /// do not start a server.
    #[cfg(test)]
    pub(crate) fn fake(program: &str, failure: Option<CheckFailure>, tools: &[&str]) -> Self {
        let mut check = Self::new(PathBuf::from(program), failure);
        check.tools = tools.iter().map(|t| (*t).to_owned()).collect();
        check
    }

    /// Every step passed.
    pub fn passed(&self) -> bool {
        self.failure.is_none()
    }

    /// The overall result of these steps.
    pub fn result(&self) -> CheckResult {
        if self.passed() {
            CheckResult::Pass
        } else {
            CheckResult::Fail
        }
    }

    /// The plain reason for the failure, if any.
    pub fn reason(&self) -> Option<String> {
        self.failure.as_ref().map(text::reason)
    }

    /// What the user can do about the failure, if any.
    pub fn what_you_can_do(&self) -> Option<String> {
        self.failure
            .as_ref()
            .map(|f| text::what_you_can_do(f, &self.program))
    }

    /// `mmhmem connected, 10 tools`, or `mmhmem did not start: <reason>.
    /// What you can do: <step>.`
    pub fn summary(&self) -> String {
        match &self.failure {
            None => text::passed(self.tools.len()),
            Some(f) => format!(
                "{}: {}. What you can do: {}.",
                text::headline(f.step()),
                text::reason(f),
                text::what_you_can_do(f, &self.program)
            ),
        }
    }

    /// The result line for `agent`, e.g. `✔ Codex — mmhmem connected, 10
    /// tools` (design.md §6).
    pub fn line(&self, agent: AgentId) -> String {
        let mark = if self.passed() { '✔' } else { '✘' };
        format!("{mark} {} — {}", agent.label(), self.summary())
    }

    /// The child's error output (at most [`MAX_DIAGNOSTICS`] bytes, control
    /// characters and environment values removed). Untrusted text: for a log
    /// file only, never for the screen.
    pub fn diagnostics(&self) -> &str {
        &self.diagnostics
    }
}

/// Starts `registration` and runs the MCP steps within `timeout`
/// (normally [`DEFAULT_CHECK_TIMEOUT`]). `expected_version` is this
/// release's version, with or without a leading `v`. Blocking; safe to call
/// from inside or outside an async runtime (it runs on its own thread).
pub fn check_mcp(
    registration: &Registration,
    expected_tools: &[&str],
    expected_version: &str,
    timeout: Duration,
) -> McpCheck {
    let run = || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build();
        match runtime {
            Ok(rt) => rt.block_on(session::run(
                registration,
                expected_tools,
                expected_version,
                timeout,
            )),
            Err(_) => McpCheck::new(registration.program.clone(), Some(CheckFailure::Internal)),
        }
    };
    std::thread::scope(|scope| {
        scope.spawn(run).join().unwrap_or_else(|_| {
            McpCheck::new(registration.program.clone(), Some(CheckFailure::Internal))
        })
    })
}

#[cfg(test)]
#[path = "check_tests.rs"]
mod tests;
