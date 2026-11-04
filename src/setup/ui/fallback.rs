//! The numbered fallback (reliable-installers task 6.3, design.md §7;
//! Requirements 4.8 and 4.9): the same screens as the full-screen menu as
//! numbered prompts over the same [`Flow`], so no setup logic is repeated.
//! It is used when `TERM=dumb` or when the terminal cannot enter raw mode.
//!
//! Screens: Home (numbered actions), Agents and Features (type numbers to
//! switch on or off, Enter to continue, `b` back, `q` quit), Review (the
//! changes per file; `d <n>` shows one change's details, `y` makes the
//! changes; on a removal `m` switches "Also delete my saved memories") and
//! Progress. `q`, `n` or the end of the input before the review is
//! confirmed ends the run with [`EXIT_CANCELLED`] and nothing changed.
//! After an update the session ends so the new program can start; the
//! restarted program opens on the review. Input and output are injected so
//! tests drive whole sessions.
//!
//! The run without any terminal is in `fallback_default.rs`.

use std::io::{BufRead, Write};

use super::flow::{
    AgentState, Choice, Confirmed, EXIT_CANCELLED, FeatureRow, Flow, HomeAction, HomeView, Next,
    ReviewView,
};
use super::text;

#[path = "fallback_default.rs"]
mod unattended;

pub use unattended::{EXIT_NO_AGENT, run_default};

/// Where the session goes next.
enum Go {
    /// Show this screen (the current one goes on the back stack).
    To(Screen),
    /// Show the current screen again.
    Stay,
    /// Go to the screen before.
    Back,
    /// Leave with this exit code.
    Exit(i32),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Screen {
    Home,
    Agents,
    Features,
    Review,
    Progress,
}

/// What the user typed on a selection screen.
#[derive(Debug, PartialEq, Eq)]
enum Answer {
    /// Zero-based positions in the numbered list.
    Numbers(Vec<usize>),
    Enter,
    Back,
    Quit,
    Bad,
}

/// Reads one line and understands the common answers.
fn parse(line: &str) -> Answer {
    match line.trim().to_lowercase().as_str() {
        "" => Answer::Enter,
        "q" | "quit" => Answer::Quit,
        "b" | "back" => Answer::Back,
        other => {
            let numbers: Option<Vec<usize>> = other
                .split([' ', ','])
                .filter(|t| !t.is_empty())
                .map(|t| t.parse::<usize>().ok().and_then(|n| n.checked_sub(1)))
                .collect();
            numbers.map_or(Answer::Bad, |mut n| {
                n.dedup();
                Answer::Numbers(n)
            })
        }
    }
}

/// The injected input and output.
pub(super) struct Console<'a> {
    input: &'a mut dyn BufRead,
    out: &'a mut dyn Write,
}

impl<'a> Console<'a> {
    pub(super) fn new(input: &'a mut dyn BufRead, out: &'a mut dyn Write) -> Self {
        Self { input, out }
    }

    pub(super) fn say(&mut self, line: &str) {
        let _ = writeln!(self.out, "{line}");
    }

    /// Shows `prompt` and reads a line; `None` at the end of the input.
    fn ask(&mut self, prompt: &str) -> Option<String> {
        let _ = write!(self.out, "{prompt} ");
        let _ = self.out.flush();
        let mut line = String::new();
        match self.input.read_line(&mut line) {
            Ok(0) | Err(_) => None,
            Ok(_) => Some(line),
        }
    }

    /// The next answer; the end of the input counts as quitting.
    fn answer(&mut self, prompt: &str) -> Answer {
        self.ask(prompt).map_or(Answer::Quit, |l| parse(&l))
    }
}

/// Runs the numbered screens over `flow` and returns the exit code.
pub fn run(flow: &mut Flow, input: &mut dyn BufRead, out: &mut dyn Write) -> i32 {
    let mut console = Console::new(input, out);
    let code = session(flow, &mut console);
    if code == EXIT_CANCELLED {
        console.say(text::CANCELLED);
    }
    code
}

fn session(flow: &mut Flow, c: &mut Console) -> i32 {
    let (mut screen, mut back) = if flow.resume() {
        (Screen::Review, vec![Screen::Home])
    } else {
        (Screen::Home, Vec::new())
    };
    loop {
        let go = match screen {
            Screen::Home => home(flow, c),
            Screen::Agents => agents(flow, c),
            Screen::Features => features(flow, c),
            Screen::Review => review(flow, c, &mut back),
            Screen::Progress => progress(flow, c),
        };
        match go {
            Go::To(next) => {
                back.push(screen);
                screen = next;
            }
            Go::Stay => {}
            Go::Back => match back.pop() {
                Some(previous) => screen = previous,
                None => return EXIT_CANCELLED,
            },
            Go::Exit(code) => return code,
        }
    }
}

/// One agent line of the home screen.
fn agent_line(row: &super::flow::AgentRow) -> String {
    let label = format!("{:<16}", row.agent.label());
    match &row.state {
        AgentState::NotFound(reason) => {
            format!("{label}{:<12}({reason})", text::STATUS_NOT_FOUND)
        }
        state => {
            let status = if *state == AgentState::Installed {
                text::STATUS_INSTALLED
            } else {
                text::STATUS_NOT_SET_UP
            };
            let features = format!("{:<22}", row.features);
            format!("{label}{status:<12}{features}{}", row.last_check)
        }
    }
}

/// The state part of the home screen, also printed without a terminal.
pub(super) fn print_state(c: &mut Console, view: &HomeView) {
    c.say(text::TITLE_HOME);
    c.say(text::INTRO);
    c.say("");
    c.say(&format!("{}    {}", view.installed, view.newest));
    for row in &view.rows {
        c.say(&agent_line(row));
    }
    for note in &view.notes {
        c.say("");
        c.say(note);
    }
}

fn home(flow: &mut Flow, c: &mut Console) -> Go {
    let view = flow.home();
    c.say("");
    print_state(c, &view);
    c.say("");
    for (i, action) in view.actions.iter().enumerate() {
        c.say(&format!("{}) {}", i + 1, action.label()));
    }
    c.say(text::HELP_HOME);
    loop {
        let chosen = match c.answer(text::PROMPT_CHOICE) {
            Answer::Quit => return Go::Exit(EXIT_CANCELLED),
            Answer::Numbers(n) if n.len() == 1 => view.actions.get(n[0]),
            _ => None,
        };
        match chosen {
            Some(action) => return after_begin(flow, c, action),
            None => c.say(text::BAD_CHOICE),
        }
    }
}

fn after_begin(flow: &mut Flow, c: &mut Console, action: &HomeAction) -> Go {
    match flow.begin(action) {
        Next::Agents => Go::To(Screen::Agents),
        Next::Review => Go::To(Screen::Review),
        Next::Progress => Go::To(Screen::Progress),
        Next::Restart => Go::Exit(0),
        Next::Note(note) => {
            c.say(note);
            Go::Stay
        }
    }
}

/// The agent lines the user can switch, numbered from 1; others are shown
/// without a number.
fn agent_lines(choices: &[Choice]) -> (Vec<String>, Vec<&Choice>) {
    let mut lines = Vec::new();
    let mut numbered = Vec::new();
    for choice in choices {
        match choice {
            Choice::Agent {
                label,
                available: false,
                ..
            } => lines.push(format!("    {label}")),
            Choice::Agent { label, checked, .. } => {
                numbered.push(choice);
                let mark = if *checked { "[x]" } else { "[ ]" };
                lines.push(format!("{}) {mark} {label}", numbered.len()));
            }
            Choice::Profile { label, chosen, .. } => {
                numbered.push(choice);
                let mark = if *chosen { "(*)" } else { "( )" };
                lines.push(format!("{}) {mark} {label}", numbered.len()));
            }
        }
    }
    (lines, numbered)
}

fn agents(flow: &mut Flow, c: &mut Console) -> Go {
    loop {
        let choices = flow.agent_choices();
        let (lines, numbered) = agent_lines(&choices);
        c.say("");
        c.say(text::TITLE_AGENTS);
        lines.iter().for_each(|l| c.say(l));
        c.say(text::HELP_AGENTS);
        match c.answer(text::PROMPT_CHOICE) {
            Answer::Quit => return Go::Exit(EXIT_CANCELLED),
            Answer::Back => return Go::Back,
            Answer::Enter if flow.has_selection() => return Go::To(Screen::Features),
            Answer::Enter => c.say(text::NOTHING_SELECTED),
            Answer::Numbers(n) if n.iter().all(|i| *i < numbered.len()) => {
                n.iter().for_each(|i| flow.toggle_choice(numbered[*i]));
            }
            _ => c.say(text::BAD_CHOICE),
        }
    }
}

/// The feature lines; the rows the user can switch are numbered from 1.
fn feature_lines(rows: &[FeatureRow]) -> (Vec<String>, Vec<&FeatureRow>) {
    let mut lines = Vec::new();
    let mut numbered = Vec::new();
    for row in rows {
        if row.is_heading() {
            lines.push(row.label.clone());
            continue;
        }
        let mark = if row.on { "[x]" } else { "[ ]" };
        if row.locked {
            lines.push(format!("    {mark} {}", row.label));
        } else {
            numbered.push(row);
            lines.push(format!("{}) {mark} {}", numbered.len(), row.label));
        }
        lines.push(format!("        {}", row.help));
    }
    (lines, numbered)
}

fn features(flow: &mut Flow, c: &mut Console) -> Go {
    loop {
        let rows = flow.feature_rows();
        let (lines, numbered) = feature_lines(&rows);
        c.say("");
        c.say(text::TITLE_FEATURES);
        lines.iter().for_each(|l| c.say(l));
        c.say(text::HELP_FEATURES);
        match c.answer(text::PROMPT_CHOICE) {
            Answer::Quit => return Go::Exit(EXIT_CANCELLED),
            Answer::Back => return Go::Back,
            Answer::Enter => {
                flow.make_plan();
                return Go::To(Screen::Review);
            }
            Answer::Numbers(n) if n.iter().all(|i| *i < numbered.len()) => {
                n.iter().for_each(|i| flow.toggle_feature(numbered[*i]));
            }
            _ => c.say(text::BAD_CHOICE),
        }
    }
}

/// The review screen; changes are numbered across files from 1.
pub(super) fn print_review(c: &mut Console, view: &ReviewView, numbers: bool) {
    c.say("");
    c.say(text::TITLE_REVIEW);
    view.intro.iter().for_each(|l| c.say(l));
    let mut n = 0;
    for group in &view.groups {
        c.say(&group.file);
        for item in &group.items {
            n += 1;
            let prefix = if numbers {
                format!("{n:>2}) ")
            } else {
                "  ".into()
            };
            c.say(&format!("{prefix}{} {}", item.mark, item.summary));
        }
    }
    if !view.problems.is_empty() {
        c.say(text::REVIEW_PROBLEMS);
        view.problems.iter().for_each(|l| c.say(l));
    }
    if let Some(option) = &view.memories {
        let mark = if option.on { "[x]" } else { "[ ]" };
        c.say(&format!("m) {mark} {}", option.label));
        c.say(&format!("       {}", option.help));
    }
    view.notes.iter().for_each(|l| c.say(l));
}

/// What the user typed on the review screen.
#[derive(Debug, PartialEq, Eq)]
enum ReviewAnswer {
    Yes,
    Quit,
    Back,
    Diff(usize),
    Memories,
    Bad,
}

fn parse_review(line: &str) -> ReviewAnswer {
    let lower = line.trim().to_lowercase();
    match lower.as_str() {
        "y" | "yes" => ReviewAnswer::Yes,
        "n" | "no" | "q" | "quit" => ReviewAnswer::Quit,
        "b" | "back" => ReviewAnswer::Back,
        "m" => ReviewAnswer::Memories,
        other => other
            .strip_prefix('d')
            .and_then(|rest| rest.trim().parse::<usize>().ok())
            .and_then(|n| n.checked_sub(1))
            .map_or(ReviewAnswer::Bad, ReviewAnswer::Diff),
    }
}

fn show_diff(flow: &Flow, c: &mut Console, view: &ReviewView, at: usize) {
    let Some(item) = view.items().get(at).map(|i| i.index) else {
        c.say(text::BAD_CHOICE);
        return;
    };
    c.say("");
    c.say(&flow.diff_file(item));
    c.say(&flow.diff(item));
}

fn review(flow: &mut Flow, c: &mut Console, back: &mut Vec<Screen>) -> Go {
    let view = flow.review();
    print_review(c, &view, true);
    let (help, confirm) = if view.memories.is_some() {
        (text::remove::HELP, text::remove::CONFIRM)
    } else {
        (text::HELP_REVIEW, text::CONFIRM)
    };
    loop {
        c.say(help);
        let answer = c
            .ask(confirm)
            .map_or(ReviewAnswer::Quit, |l| parse_review(&l));
        match answer {
            ReviewAnswer::Quit => return Go::Exit(EXIT_CANCELLED),
            ReviewAnswer::Back => return Go::Back,
            ReviewAnswer::Memories if view.memories.is_some() => {
                flow.toggle_memories();
                return Go::Stay;
            }
            ReviewAnswer::Diff(at) => show_diff(flow, c, &view, at),
            ReviewAnswer::Bad | ReviewAnswer::Memories => c.say(text::BAD_CHOICE),
            ReviewAnswer::Yes => {
                c.say(text::CHECKING);
                back.clear();
                return match flow.confirm() {
                    Confirmed::Review => Go::Stay,
                    Confirmed::Done => Go::To(Screen::Progress),
                };
            }
        }
    }
}

fn progress(flow: &mut Flow, c: &mut Console) -> Go {
    c.say("");
    if let Some(progress) = flow.progress() {
        progress.all_lines().iter().for_each(|l| c.say(l));
    }
    Go::Exit(flow.exit_code())
}

#[cfg(test)]
#[path = "fallback_tests.rs"]
mod tests;
