//! Setup without a terminal (task 6.3; Requirement 4.9): nobody can answer
//! questions, so the default choices are installed and every result is
//! printed as a plain line. Only the install script gets here
//! (`--bootstrap <tag>`); any other run without a terminal is refused
//! before anything is read (`ui::refuse_without_terminal`, amended 4.9).
//! It never removes anything and never updates the program.
//!
//! The default choices are those of the first install: every agent found on
//! this computer with the required feature only (an existing install keeps
//! its recorded choices). The steps are the menu's: [`Flow::begin`], the
//! plan, [`Flow::confirm`]. After an earlier install was found, the first
//! confirm only takes it over and the choices are planned and confirmed
//! next, exactly as the menu does.
//!
//! Exit codes: the results' code (0 all passed or only user steps are left,
//! 1 an agent failed); [`EXIT_NO_AGENT`] (1) when no supported agent was
//! found, or all of them were skipped, and nothing was changed; 0 when
//! mmhmem is already installed and current, and nothing was changed.

use std::io::Write;

use super::{Console, print_state};
use crate::setup::ui::flow::{Confirmed, Flow, HomeAction, Next};
use crate::setup::ui::text;

/// The exit code when there is nothing to install into.
pub const EXIT_NO_AGENT: i32 = 1;

/// Installs the default choices; returns the exit code.
pub fn run_default(flow: &mut Flow, out: &mut dyn Write) -> i32 {
    let mut input = std::io::empty();
    let mut c = Console::new(&mut input, out);
    let view = flow.home();
    print_state(&mut c, &view);
    c.say("");
    c.say(text::NO_TERMINAL);
    if !view.actions.contains(&HomeAction::Install) {
        // The home notes already said why: current, or no agent found.
        return if flow.is_installed() {
            0
        } else {
            EXIT_NO_AGENT
        };
    }
    if flow.begin(&HomeAction::Install) == Next::Agents {
        flow.make_plan();
    }
    let mut first = true;
    loop {
        let review = flow.review();
        if review.is_empty() && first {
            return nothing_to_do(&mut c, &review.problems);
        }
        first = false;
        super::print_review(&mut c, &review, false);
        c.say("");
        c.say(text::CHECKING);
        if flow.confirm() == Confirmed::Done {
            break;
        }
    }
    c.say("");
    if let Some(progress) = flow.progress() {
        progress.all_lines().iter().for_each(|l| c.say(l));
    }
    flow.exit_code()
}

/// Nothing is planned: say why and change nothing.
fn nothing_to_do(c: &mut Console, problems: &[String]) -> i32 {
    problems.iter().for_each(|l| c.say(l));
    if problems.is_empty() {
        c.say(text::REVIEW_NOTHING);
    }
    i32::from(!problems.is_empty())
}
