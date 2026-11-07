//! The real terminal for the menu (task 6.2; Requirement 4.1): raw mode and
//! the alternate screen through `ratatui::try_init` (which also installs a
//! panic hook that restores the terminal), a drop guard that restores it on
//! every other way out, and the event loop. The newest-release lookup runs
//! on its own thread so an offline machine never blocks the menu.
//!
//! **Keys typed ahead of the Review.** Enter on the Review makes the
//! changes, so the Review only takes keys pressed after it is on the
//! screen: keys typed before it is drawn (while the update downloads, while
//! the plan is made, or while the program restarts after an update) are
//! discarded, whichever process read them. Without this, an Enter pressed
//! twice on "Update to …" was either lost with the old program or confirmed
//! a Review nobody had seen, depending on timing (research.md §8, fourth
//! e2e run).

use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::Duration;

use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use super::{App, Control, Key, Screen};
use crate::setup::download_line;
use crate::setup::ui::flow::{Flow, Newest};
use crate::setup::ui::text;

/// How often the loop looks for the release answer while no key comes.
const POLL: Duration = Duration::from_millis(200);
/// How long one look for typed-ahead keys waits. Not zero: crossterm's
/// `/dev/tty` reader (macOS) does not look at the terminal at all for a
/// zero timeout.
const DISCARD_POLL: Duration = Duration::from_millis(1);

/// The newest-release lookup, run off the UI thread.
pub type Lookup = Box<dyn FnOnce() -> Newest + Send>;

/// Raw mode or the alternate screen could not be entered. The flow comes
/// back unchanged for the numbered fallback.
pub struct TerminalUnavailable(pub Box<Flow>);

/// Restores the terminal when dropped (normal exit, error, unwinding).
struct Restore;

impl Drop for Restore {
    fn drop(&mut self) {
        let _ = ratatui::try_restore();
    }
}

/// Runs the menu on the real terminal. Returns the exit code and the flow
/// (its results to print, or the restart after an update) once the normal
/// screen is back.
pub fn run_terminal(flow: Flow, lookup: Lookup) -> Result<(i32, Box<Flow>), TerminalUnavailable> {
    let mut terminal = match ratatui::try_init() {
        Ok(terminal) => terminal,
        Err(_) => {
            let _ = ratatui::try_restore();
            return Err(TerminalUnavailable(Box::new(flow)));
        }
    };
    let restore = Restore;
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let _ = tx.send(lookup());
    });
    let mut app = App::new(flow);
    let result = event_loop(&mut terminal, &mut app, &rx);
    drop(restore);
    // Said on the normal screen, so the user sees why the menu closed.
    let code = result.unwrap_or_else(|error| {
        eprintln!("{}", text::menu_stopped(&error.to_string()));
        1
    });
    Ok((code, Box::new(app.into_flow())))
}

fn event_loop(
    terminal: &mut DefaultTerminal,
    app: &mut App,
    newest: &Receiver<Newest>,
) -> std::io::Result<i32> {
    let mut shown = None;
    loop {
        if let Ok(answer) = newest.try_recv() {
            app.set_newest(answer);
        }
        if discards_typed_ahead(shown, app.screen()) {
            discard_pending_keys()?;
        }
        terminal.draw(|frame| app.draw(frame))?;
        shown = Some(app.screen());
        if app.is_working() {
            app.work();
            continue;
        }
        if !event::poll(POLL)? {
            continue;
        }
        let Event::Key(press) = event::read()? else {
            continue;
        };
        if press.kind != KeyEventKind::Press {
            continue;
        }
        let key = key_of(press);
        // "Update to …": the full-screen menu gives way to one progress
        // line on the plain screen while the release downloads (bugfix
        // 2026-11-07: the screen used to freeze for the whole download),
        // then the menu comes back.
        if key == Key::Enter
            && let Some(tag) = app.selected_update()
        {
            let _ = ratatui::try_restore();
            let reporter = download_line::Reporter::stderr(tag.as_str());
            let control = app.update_key(reporter.hook());
            reporter.finish();
            let restored = ratatui::try_init().and_then(|new_terminal| {
                *terminal = new_terminal;
                terminal.clear()
            });
            if restored.is_err() {
                return Err(std::io::Error::other(
                    "the terminal could not be reopened after the download",
                ));
            }
            if let Control::Exit(code) = control {
                return Ok(code);
            }
            continue;
        }
        if let Control::Exit(code) = app.key(key) {
            return Ok(code);
        }
    }
}

/// Whether keys typed before `next` is drawn must be dropped: only when
/// the Review (where Enter makes the changes) is about to appear.
fn discards_typed_ahead(shown: Option<Screen>, next: Screen) -> bool {
    next == Screen::Review && shown != Some(Screen::Review)
}

/// Reads and drops every key that is already waiting.
fn discard_pending_keys() -> std::io::Result<()> {
    while event::poll(DISCARD_POLL)? {
        let _ = event::read()?;
    }
    Ok(())
}

fn key_of(press: KeyEvent) -> Key {
    if press.modifiers.contains(KeyModifiers::CONTROL) && press.code == KeyCode::Char('c') {
        return Key::Quit;
    }
    match press.code {
        KeyCode::Up | KeyCode::Char('k') => Key::Up,
        KeyCode::Down | KeyCode::Char('j') => Key::Down,
        KeyCode::Char(' ') => Key::Space,
        KeyCode::Enter => Key::Enter,
        KeyCode::Esc | KeyCode::Backspace => Key::Esc,
        KeyCode::Char('q' | 'Q') => Key::Quit,
        KeyCode::Char('d' | 'D') => Key::Diff,
        _ => Key::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::{Screen, discards_typed_ahead};

    #[test]
    fn only_a_review_that_appears_drops_typed_ahead_keys() {
        // The restarted program opens on the Review.
        assert!(discards_typed_ahead(None, Screen::Review));
        assert!(discards_typed_ahead(Some(Screen::Home), Screen::Review));
        assert!(discards_typed_ahead(Some(Screen::Features), Screen::Review));
        // Keys pressed while the Review is shown are kept.
        assert!(!discards_typed_ahead(Some(Screen::Review), Screen::Review));
        for screen in [
            Screen::Home,
            Screen::Agents,
            Screen::Features,
            Screen::Progress,
        ] {
            assert!(!discards_typed_ahead(None, screen));
            assert!(!discards_typed_ahead(Some(Screen::Review), screen));
        }
    }
}
