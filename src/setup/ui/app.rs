//! The full-screen menu (reliable-installers task 6.2, design.md §7;
//! Requirements 4.1–4.7): a key-driven state machine over [`Flow`], drawn
//! with ratatui. Submodules:
//!
//! - `app_draw.rs`: the screens and the always-visible key legend.
//! - `app_term.rs`: the crossterm terminal (raw mode and the alternate
//!   screen, restored on every exit and on panic) and the event loop.
//!
//! Screens: Home → Agents → Features → Review (`d` shows a change's diff,
//! scrollable) → Progress. Keys: ↑↓ move, space toggles, enter continues,
//! esc goes back, q quits. On a removal's Review, space switches "Also
//! delete my saved memories". Esc on Home or q before the Review is
//! confirmed exits with [`EXIT_CANCELLED`] and nothing changed. After an
//! update the menu closes so the new program can start; the restarted
//! program opens on the Review when the recorded choices change files, and
//! otherwise goes straight to Progress and only checks the tools
//! (Requirement 7.5). The state machine has
//! no terminal in it, so tests drive it key by key and draw it on a
//! `TestBackend`.

use std::cell::Cell;

use super::flow::{Confirmed, EXIT_CANCELLED, Flow, Newest, Next, Resumed};
use super::text;

#[path = "app_draw.rs"]
mod draw;
#[path = "app_term.rs"]
mod term;

pub use term::{TerminalUnavailable, run_terminal};

/// A screen of the menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    /// State and actions.
    Home,
    /// Which agents.
    Agents,
    /// Which features.
    Features,
    /// What will change.
    Review,
    /// What happened.
    Progress,
}

/// A key, as the menu understands it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    /// ↑.
    Up,
    /// ↓.
    Down,
    /// Space: toggle.
    Space,
    /// Enter: continue.
    Enter,
    /// Esc: back.
    Esc,
    /// q: quit.
    Quit,
    /// d: show or hide the diff.
    Diff,
    /// Anything else.
    Other,
}

/// What the caller does after a key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Control {
    /// Draw again and wait for the next key.
    Continue,
    /// Leave the menu with this exit code.
    Exit(i32),
}

/// The menu state.
pub struct App {
    flow: Flow,
    screen: Screen,
    back: Vec<Screen>,
    cursor: usize,
    scroll: usize,
    max_scroll: Cell<usize>,
    diff: Option<usize>,
    note: Option<&'static str>,
    working: Option<Work>,
    /// The line above the results (after an update that changed no file).
    heading: Option<String>,
}

/// The slow step [`App::work`] does next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Work {
    /// Apply the confirmed plan and check.
    Confirm,
    /// Only check (after an update that changes no file).
    Check,
}

impl App {
    /// The menu on its home screen. In the run started right after an
    /// update ([`Flow::resume`]): on the review when the recorded choices
    /// change files, else straight to the results, where only the check
    /// runs (ask only if needed, Requirement 7.5).
    pub fn new(mut flow: Flow) -> Self {
        let resumed = flow.resume();
        let (screen, back, working, heading) = match resumed {
            Some(Resumed::Review(_)) => (Screen::Review, vec![Screen::Home], None, None),
            Some(Resumed::Check(tag)) => (
                Screen::Progress,
                Vec::new(),
                Some(Work::Check),
                Some(text::update::updated_no_changes(tag.as_str())),
            ),
            None => (Screen::Home, Vec::new(), None, None),
        };
        Self {
            flow,
            screen,
            back,
            cursor: 0,
            scroll: 0,
            max_scroll: Cell::new(0),
            diff: None,
            note: None,
            working,
            heading,
        }
    }

    /// The controller.
    pub fn flow(&self) -> &Flow {
        &self.flow
    }

    /// The controller, when the menu is closed.
    pub fn into_flow(self) -> Flow {
        self.flow
    }

    /// The screen shown.
    pub fn screen(&self) -> Screen {
        self.screen
    }

    /// Hands over the newest-release answer.
    pub fn set_newest(&mut self, newest: Newest) {
        self.flow.set_newest(newest);
    }

    /// A confirmed plan waits for [`App::work`] (drawn as "please wait").
    pub fn is_working(&self) -> bool {
        self.working.is_some()
    }

    /// Applies the confirmed plan and checks the agents, or only checks
    /// them after an update that changes no file.
    pub fn work(&mut self) {
        match self.working.take() {
            Some(Work::Check) => self.flow.apply_update(),
            Some(Work::Confirm) if self.flow.confirm() == Confirmed::Review => {
                self.screen = Screen::Review;
                self.cursor = 0;
            }
            Some(Work::Confirm) | None => {}
        }
    }

    /// The line above the results, if any.
    pub fn heading(&self) -> Option<&str> {
        self.heading.as_deref()
    }

    /// Handles one key.
    pub fn key(&mut self, key: Key) -> Control {
        if self.working.is_some() {
            return Control::Continue;
        }
        if key != Key::Other {
            self.note = None;
        }
        match self.screen {
            Screen::Home => self.home_key(key),
            Screen::Agents => self.agents_key(key),
            Screen::Features => self.features_key(key),
            Screen::Review => self.review_key(key),
            Screen::Progress => self.progress_key(key),
        }
    }

    /// Feeds `keys` in order; the first exit wins.
    pub fn keys(&mut self, keys: &[Key]) -> Control {
        for key in keys {
            if let Control::Exit(code) = self.key(*key) {
                return Control::Exit(code);
            }
            self.work();
        }
        Control::Continue
    }

    fn go(&mut self, screen: Screen) {
        self.back.push(self.screen);
        self.screen = screen;
        self.cursor = 0;
        self.scroll = 0;
    }

    fn go_back(&mut self) -> Control {
        match self.back.pop() {
            Some(screen) => {
                self.screen = screen;
                self.cursor = 0;
                Control::Continue
            }
            None => Control::Exit(EXIT_CANCELLED),
        }
    }

    fn step(&mut self, key: Key, len: usize) {
        match key {
            Key::Up => self.cursor = self.cursor.saturating_sub(1),
            Key::Down if self.cursor + 1 < len => self.cursor += 1,
            _ => {}
        }
    }

    fn home_key(&mut self, key: Key) -> Control {
        let actions = self.flow.home().actions;
        match key {
            Key::Quit | Key::Esc => Control::Exit(EXIT_CANCELLED),
            Key::Enter => {
                if let Some(action) = actions.get(self.cursor) {
                    match self.flow.begin(action) {
                        Next::Agents => self.go(Screen::Agents),
                        Next::Review => self.go(Screen::Review),
                        Next::Progress => self.go(Screen::Progress),
                        Next::Note(note) => self.note = Some(note),
                        Next::Restart => return Control::Exit(0),
                    }
                }
                Control::Continue
            }
            _ => {
                self.step(key, actions.len());
                Control::Continue
            }
        }
    }

    fn agents_key(&mut self, key: Key) -> Control {
        let choices = self.flow.agent_choices();
        match key {
            Key::Quit => return Control::Exit(EXIT_CANCELLED),
            Key::Esc => return self.go_back(),
            Key::Space => {
                if let Some(choice) = choices.get(self.cursor) {
                    self.flow.toggle_choice(choice);
                }
            }
            Key::Enter if self.flow.has_selection() => self.go(Screen::Features),
            Key::Enter => self.note = Some(text::NOTHING_SELECTED),
            _ => self.step(key, choices.len()),
        }
        Control::Continue
    }

    fn features_key(&mut self, key: Key) -> Control {
        let rows = self.flow.feature_rows();
        match key {
            Key::Quit => return Control::Exit(EXIT_CANCELLED),
            Key::Esc => return self.go_back(),
            Key::Space => {
                if let Some(row) = rows.get(self.cursor) {
                    self.flow.toggle_feature(row);
                }
            }
            Key::Enter => {
                self.flow.make_plan();
                self.go(Screen::Review);
            }
            _ => self.step(key, rows.len()),
        }
        Control::Continue
    }

    fn review_key(&mut self, key: Key) -> Control {
        if self.diff.is_some() {
            return self.diff_key(key);
        }
        let items = self.flow.review().items().len();
        match key {
            Key::Quit => return Control::Exit(EXIT_CANCELLED),
            Key::Esc => return self.go_back(),
            Key::Space => self.flow.toggle_memories(),
            Key::Diff => {
                let review = self.flow.review();
                self.diff = review.items().get(self.cursor).map(|i| i.index);
                self.scroll = 0;
            }
            Key::Enter => {
                self.working = Some(Work::Confirm);
                self.go(Screen::Progress);
                self.back.clear();
            }
            _ => self.step(key, items),
        }
        Control::Continue
    }

    fn diff_key(&mut self, key: Key) -> Control {
        match key {
            Key::Quit => return Control::Exit(EXIT_CANCELLED),
            Key::Esc | Key::Diff => self.diff = None,
            Key::Up => self.scroll = self.scroll.saturating_sub(1),
            Key::Down => self.scroll_down(),
            _ => {}
        }
        Control::Continue
    }

    fn scroll_down(&mut self) {
        if self.scroll < self.max_scroll.get() {
            self.scroll += 1;
        }
    }

    fn progress_key(&mut self, key: Key) -> Control {
        match key {
            Key::Enter | Key::Quit | Key::Esc => Control::Exit(self.flow.exit_code()),
            Key::Up => {
                self.scroll = self.scroll.saturating_sub(1);
                Control::Continue
            }
            Key::Down => {
                self.scroll_down();
                Control::Continue
            }
            _ => Control::Continue,
        }
    }
}

#[cfg(test)]
#[path = "app_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "app_snapshots.rs"]
mod snapshots;
