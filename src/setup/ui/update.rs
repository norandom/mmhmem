//! Update mode (owner decision 2026-11-06, Requirement 7.5, design.md §8
//! "Update mode"): `mmhmem update`, and the install command run again on
//! an installed system. Plain questions, so it works in every terminal.
//!
//! **`mmhmem update`** ([`run`], [`session`]): without a terminal nothing
//! is read or changed ([`refuse_without_terminal`]); without an install
//! record it says to run `mmhmem setup` first ([`EXIT_NOT_SET_UP`]). It
//! looks up the newest release (at most [`NEWEST_WAIT`]): unknown → nothing
//! changed, exit 1; not newer → "up to date", exit 0. Otherwise it asks
//! `Update now? [Y/n]` once (Enter = yes; no, or the end of the input,
//! changes nothing and exits with [`EXIT_CANCELLED`], as quitting the menu
//! does) and takes the same verified path as the menu
//! ([`HomeAction::UpdateTo`]: no trusted key → refused; signature and
//! checksum; record updated), then starts the new program as
//! `update --restarted <tag>` and returns its exit code.
//!
//! **Ask only if needed** ([`finish`]): the restarted run plans the
//! recorded choices with its own files. No change → it only checks the
//! tools and prints the results. Changes → it lists them (the review's
//! summaries) and asks `Apply these changes? [Y/n]` once; no keeps the
//! files and says how to review them later (`mmhmem setup`), exit
//! [`EXIT_CANCELLED`].
//!
//! **The install command on an installed system** ([`bootstrap`]): when it
//! brought a newer release than the recorded one, the same question; yes
//! copies the program it already verified (the plan's program change) with
//! the recorded choices, asking only if other files change. An equal or
//! older release opens the normal menu. Without a terminal the install
//! command keeps its standard run (`fallback_default.rs`).
//!
//! Exit codes: the results' code (0 every tool passed or only has steps
//! left that only the user can do, 1 otherwise), 1 when nothing could be
//! done, [`EXIT_CANCELLED`] when the user said no.

use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::time::Duration;

use super::flow::{Command, EXIT_CANCELLED, Flow, HomeAction, Newest, Next, Resumed, ReviewView};
use super::{EXIT_NEEDS_TERMINAL, Mode, SetupArgs, text};

/// The exit code when mmhmem is not set up yet: nothing was changed.
pub const EXIT_NOT_SET_UP: i32 = 1;

/// How long `mmhmem update` waits for the newest-release lookup.
pub const NEWEST_WAIT: Duration = Duration::from_secs(15);

/// Internal arguments of `mmhmem update`; never shown in `--help`.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct UpdateArgs {
    /// Local release artefacts (`--release-dir <dir>`), as for setup.
    pub release_dir: Option<PathBuf>,
    /// Started by the previous program right after it updated itself
    /// (`--restarted <tag>`).
    pub restarted: Option<String>,
}

impl UpdateArgs {
    /// Parses the arguments after `update`. `Err` holds the unknown argument.
    pub fn parse(args: &[String]) -> Result<Self, String> {
        let setup = SetupArgs::parse(args)?;
        if setup.bootstrap.is_some() {
            return Err("--bootstrap".to_owned());
        }
        Ok(Self {
            release_dir: setup.release_dir,
            restarted: setup.restarted,
        })
    }

    fn setup_args(&self) -> SetupArgs {
        SetupArgs {
            bootstrap: None,
            release_dir: self.release_dir.clone(),
            restarted: self.restarted.clone(),
        }
    }
}

/// Without a terminal: says to run `mmhmem update` in a terminal window and
/// returns [`EXIT_NEEDS_TERMINAL`]; `None` when the run may go on.
pub fn refuse_without_terminal(mode: Mode, out: &mut dyn Write) -> Option<i32> {
    if mode != Mode::NoTerminal {
        return None;
    }
    let _ = writeln!(out, "{}", text::update::NEEDS_TERMINAL);
    Some(EXIT_NEEDS_TERMINAL)
}

/// Entry point of `mmhmem update`. Returns the process exit code.
pub fn run(args: UpdateArgs) -> i32 {
    if let Some(code) = refuse_without_terminal(super::current_mode(), &mut std::io::stdout()) {
        return code;
    }
    let setup = args.setup_args();
    if let Some(code) = super::missing_release_dir(&setup) {
        return code;
    }
    let Some(mut deps) = super::process_deps(&setup) else {
        eprintln!("{}", text::NO_HOME);
        return 1;
    };
    deps.restart.command = Command::Update;
    let mut flow = Flow::load(deps);
    let source = flow.release_source();
    let lookup = move || super::newest_within(source, NEWEST_WAIT);
    let code = session(
        &mut flow,
        lookup,
        &mut std::io::stdin().lock(),
        &mut std::io::stdout().lock(),
    );
    super::restarted(&flow).unwrap_or(code)
}

/// The plain questions and answers.
struct Prompt<'a> {
    input: &'a mut dyn BufRead,
    out: &'a mut dyn Write,
}

impl Prompt<'_> {
    fn say(&mut self, line: &str) {
        let _ = writeln!(self.out, "{line}");
    }

    /// Writes `text` without ending the line.
    fn start(&mut self, text: &str) {
        let _ = write!(self.out, "{text}");
        let _ = self.out.flush();
    }

    /// Asks `prompt` until the answer is yes (Enter too) or no; the end of
    /// the input counts as no.
    fn yes(&mut self, prompt: &str) -> bool {
        loop {
            self.start(&format!("{prompt} "));
            let mut line = String::new();
            match self.input.read_line(&mut line) {
                Ok(0) | Err(_) => {
                    self.say("");
                    return false;
                }
                Ok(_) => {}
            }
            match line.trim().to_lowercase().as_str() {
                "" | "y" | "yes" => return true,
                "n" | "no" => return false,
                _ => self.say(text::update::BAD_ANSWER),
            }
        }
    }
}

/// `mmhmem update` over `flow` (see the module docs); `lookup` answers the
/// newest release. Returns the exit code; after a verified update the
/// caller starts the new program ([`Flow::restart_now`]).
pub fn session(
    flow: &mut Flow,
    lookup: impl FnOnce() -> Newest,
    input: &mut dyn BufRead,
    out: &mut dyn Write,
) -> i32 {
    let mut p = Prompt { input, out };
    if let Some(Resumed::Check(tag) | Resumed::Review(tag)) = flow.resume() {
        return finish(flow, &mut p, tag.as_str(), Some(text::update::CHANGES_KEPT));
    }
    if flow.record().is_none() {
        p.say(text::update::NOT_SET_UP);
        return EXIT_NOT_SET_UP;
    }
    p.say(text::update::CHECKING_NEWEST);
    flow.set_newest(lookup());
    let installed = flow.installed_release();
    if *flow.newest() == Newest::Unknown {
        p.say(text::update::NEWEST_UNKNOWN);
        return 1;
    }
    let Some(newest) = flow.newer_release() else {
        p.say(&text::update::up_to_date(installed.as_str()));
        return 0;
    };
    p.say(&text::update::available(
        installed.as_str(),
        newest.as_str(),
    ));
    p.say(text::update::KEEPS_CHOICES);
    if !p.yes(text::update::PROMPT_UPDATE) {
        p.say(text::update::NOTHING_CHANGED);
        return EXIT_CANCELLED;
    }
    if let Some(note) = flow.update_refusal() {
        p.say(note);
        return 1;
    }
    // One progress line (bytes, percent, speed, time left) while the
    // release downloads (bugfix 2026-11-07: a frozen "Downloading…").
    let reporter = crate::setup::download_line::Reporter::stderr(newest.as_str());
    let hook = reporter.hook();
    p.start(&text::update::downloading(newest.as_str()));
    match flow.begin_with(&HomeAction::UpdateTo(newest.clone()), hook) {
        Next::Restart => {
            reporter.finish();
            p.say(&format!(" {}", text::update::VERIFIED));
            0
        }
        // The release is this program's own (or no restart is set up): go
        // on here with the files of this program.
        Next::Review => {
            reporter.finish();
            p.say(&format!(" {}", text::update::VERIFIED));
            finish(
                flow,
                &mut p,
                newest.as_str(),
                Some(text::update::CHANGES_KEPT),
            )
        }
        Next::Note(note) => {
            reporter.finish();
            p.say("");
            if note == text::UPDATE_FAILED
                && let Some(reason) = flow.last_note()
            {
                p.say(reason);
            }
            p.say(note);
            1
        }
        Next::Agents | Next::Progress => 1,
    }
}

/// The install command on an installed system brought a newer release:
/// asks once, then installs it with the recorded choices. `None` when this
/// run is not such a run, so the normal menu opens.
pub fn bootstrap(flow: &mut Flow, input: &mut dyn BufRead, out: &mut dyn Write) -> Option<i32> {
    let (recorded, brought) = flow.bootstrap_offer()?;
    let mut p = Prompt { input, out };
    let offer = text::update::bootstrap_available(recorded.as_str(), brought.as_str());
    p.say(&offer);
    p.say(text::update::KEEPS_CHOICES);
    if !p.yes(text::update::PROMPT_UPDATE) {
        p.say(text::update::NOTHING_CHANGED);
        return Some(EXIT_CANCELLED);
    }
    flow.plan_recorded();
    // A no here leaves even the program as it was.
    Some(finish(flow, &mut p, brought.as_str(), None))
}

/// Ask only if needed: with nothing to change besides the program, check
/// (and copy the program); otherwise list the changes and ask once.
/// `kept` is said when the user declines (`None`: nothing was changed).
fn finish(flow: &mut Flow, p: &mut Prompt, tag: &str, kept: Option<&str>) -> i32 {
    let view = flow.update_review();
    if view.groups.is_empty() && view.problems.is_empty() {
        p.say(&text::update::updated_checking(tag));
    } else {
        if kept.is_some() {
            p.say(&text::update::updated(tag));
        }
        p.say(text::update::CHANGES_NEEDED);
        changes(p, &view);
        if !p.yes(text::update::PROMPT_APPLY) {
            p.say(kept.unwrap_or(text::update::NOTHING_CHANGED));
            return EXIT_CANCELLED;
        }
        p.say(text::update::MAKING_CHANGES);
    }
    flow.apply_update();
    for line in result_lines(flow) {
        p.say(&line);
    }
    flow.exit_code()
}

/// The changes per file and the tools that will be skipped.
fn changes(p: &mut Prompt, view: &ReviewView) {
    for group in &view.groups {
        p.say(&group.file);
        for item in &group.items {
            p.say(&format!("  {} {}", item.mark, item.summary));
        }
    }
    if !view.problems.is_empty() {
        p.say(text::REVIEW_PROBLEMS);
        view.problems.iter().for_each(|l| p.say(l));
    }
}

/// The results without the "Checking …" heading said before the check.
fn result_lines(flow: &Flow) -> Vec<String> {
    let mut lines: Vec<String> = flow
        .progress()
        .map(|p| p.all_lines())
        .unwrap_or_default()
        .into_iter()
        .filter(|l| l != text::CHECKING)
        .collect();
    let blank = lines.iter().take_while(|l| l.is_empty()).count();
    lines.drain(..blank);
    if flow.exit_code() == 0 && flow.progress().is_some_and(|p| p.next_steps.is_empty()) {
        lines.push(text::update::DONE.to_owned());
    }
    lines
}

#[cfg(test)]
#[path = "update_tests.rs"]
mod tests;
