//! Every sentence the functional check shows the user (task 4.1, design.md
//! §6 result lines). Plain words for people who don't know the project:
//! what is wrong, then what they can do. Only paths, tool names, versions and
//! numbers are filled in, never file text, program output or environment
//! values.

use std::io::ErrorKind;
use std::path::Path;

use super::CheckFailure;
use super::registration::RegistrationError;
use crate::setup::record::CheckStep;

/// What the user sees for a check that passed: `mmhmem connected, 10 tools`.
pub(super) fn passed(tools: usize) -> String {
    format!("mmhmem connected, {tools} tools")
}

/// The start of a failure line, by the step that failed.
pub(super) fn headline(step: CheckStep) -> &'static str {
    match step {
        CheckStep::Registration | CheckStep::Handshake => "mmhmem did not start",
        _ => "mmhmem started but did not work",
    }
}

/// What is wrong, in one plain sentence (no full stop).
pub(super) fn reason(failure: &CheckFailure) -> String {
    match failure {
        CheckFailure::Registration(error) => registration_reason(error),
        CheckFailure::ProgramMissing { program } => {
            format!("the program at {} does not exist", shown(program))
        }
        CheckFailure::FolderMissing { cwd } => format!(
            "the folder {} that the agent starts mmhmem in does not exist",
            shown(cwd)
        ),
        CheckFailure::CannotStart { program, kind } => format!(
            "the program at {} could not be started ({})",
            shown(program),
            start_problem(*kind)
        ),
        CheckFailure::ExitedEarly { step } => match step {
            CheckStep::Handshake => "mmhmem stopped right after it started".to_owned(),
            _ => "mmhmem stopped while it was answering".to_owned(),
        },
        CheckFailure::NoAnswer { seconds, .. } => {
            format!("mmhmem did not answer within {seconds} seconds")
        }
        CheckFailure::NotMcp { .. } => "the program did not answer the way mmhmem does".to_owned(),
        CheckFailure::MissingTools { names } => format!(
            "mmhmem does not offer {}, so it is probably a different version than this installer",
            names.join(", ")
        ),
        CheckFailure::StatusFailed => "mmhmem could not report its status".to_owned(),
        CheckFailure::StoreNotReady => {
            "mmhmem started, but the place where it keeps its memory is not ready".to_owned()
        }
        CheckFailure::WrongVersion { expected } => {
            format!("mmhmem answered, but it is not version {expected}")
        }
        CheckFailure::Internal => "the check itself could not run".to_owned(),
    }
}

/// What the user can do about it, in one plain sentence (no full stop).
pub(super) fn what_you_can_do(failure: &CheckFailure, program: &Path) -> String {
    const AGAIN: &str = "run the installer again";
    match failure {
        CheckFailure::Registration(RegistrationError::Disabled) => {
            format!("switch mmhmem on in the agent's settings, or {AGAIN}")
        }
        CheckFailure::FolderMissing { .. } => {
            format!("create that folder or remove it from mmhmem's entry, then {AGAIN}")
        }
        CheckFailure::CannotStart { .. } => {
            format!(
                "make sure that file is the mmhmem program and that you may run it, then {AGAIN}"
            )
        }
        CheckFailure::ExitedEarly { .. } | CheckFailure::NotMcp { .. } => format!(
            "{AGAIN}; if this happens again, open a terminal and run {} stdio to see the error",
            shown(program)
        ),
        CheckFailure::NoAnswer { .. } => {
            format!("wait a moment and {AGAIN}; if it happens again, restart the computer first")
        }
        CheckFailure::MissingTools { .. } | CheckFailure::WrongVersion { .. } => {
            format!("{AGAIN} so the agent starts the mmhmem version this installer brings")
        }
        CheckFailure::StatusFailed | CheckFailure::StoreNotReady => format!(
            "make sure mmhmem's data folder can be written and no other program locks it, then {AGAIN}"
        ),
        _ => AGAIN.to_owned(),
    }
}

/// Why the registration could not be read, in one plain sentence.
pub(super) fn registration_reason(error: &RegistrationError) -> String {
    match error {
        RegistrationError::Unreadable => "the agent's settings file could not be read".to_owned(),
        RegistrationError::Missing => "the agent's settings have no entry for mmhmem".to_owned(),
        RegistrationError::NoCommand => {
            "mmhmem's entry in the agent's settings does not say which program to start".to_owned()
        }
        RegistrationError::NotLocal => {
            "mmhmem's entry in the agent's settings does not start a program on this computer"
                .to_owned()
        }
        RegistrationError::Disabled => "mmhmem is switched off in the agent's settings".to_owned(),
        RegistrationError::BadField(field) => {
            format!("the setting \"{field}\" in mmhmem's entry is not in the expected form")
        }
    }
}

fn start_problem(kind: ErrorKind) -> &'static str {
    match kind {
        ErrorKind::PermissionDenied => "it is not allowed to run",
        ErrorKind::IsADirectory => "it is a folder, not a program",
        _ => "the system refused to start it",
    }
}

fn shown(path: &Path) -> String {
    format!("\"{}\"", path.display())
}
