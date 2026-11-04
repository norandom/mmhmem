//! The real terminal for the menu (task 6.2; Requirement 4.1): raw mode and
//! the alternate screen through `ratatui::try_init` (which also installs a
//! panic hook that restores the terminal), a drop guard that restores it on
//! every other way out, and the event loop. The newest-release lookup runs
//! on its own thread so an offline machine never blocks the menu.

use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::Duration;

use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use super::{App, Control, Key};
use crate::setup::ui::flow::{Flow, Newest};

/// How often the loop looks for the release answer while no key comes.
const POLL: Duration = Duration::from_millis(200);

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
    let code = event_loop(&mut terminal, &mut app, &rx).unwrap_or(1);
    drop(restore);
    Ok((code, Box::new(app.into_flow())))
}

fn event_loop(
    terminal: &mut DefaultTerminal,
    app: &mut App,
    newest: &Receiver<Newest>,
) -> std::io::Result<i32> {
    loop {
        if let Ok(answer) = newest.try_recv() {
            app.set_newest(answer);
        }
        terminal.draw(|frame| app.draw(frame))?;
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
        if let Control::Exit(code) = app.key(key_of(press)) {
            return Ok(code);
        }
    }
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
