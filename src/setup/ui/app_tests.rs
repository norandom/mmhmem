//! Key-sequence tests of the menu state machine (task 6.2; Requirement
//! 4.7): cancel or quit at every screen before the review is confirmed
//! exits with code 2 and leaves the temporary folder byte-identical; enter
//! through every screen installs and closes with the results' code.

use super::*;
use crate::setup::ui::fixtures::{Fx, press, with_clis};
use crate::setup::ui::flow::Newest;

use Key::{Diff, Down, Enter, Esc, Quit, Space, Up};

/// Runs `keys` on a fresh menu over `fx`; returns the exit and the screen.
fn run(fx: &Fx, keys: &[Key]) -> (Control, Screen, bool) {
    let mut app = fx.app(Newest::Unknown);
    let control = app.keys(keys);
    let done = app.flow().progress().is_some();
    (control, app.screen(), done)
}

fn assert_cancels(fx: &Fx, keys: &[Key]) {
    let before = fx.digest();
    let (control, screen, done) = run(fx, keys);
    assert_eq!(
        control,
        Control::Exit(EXIT_CANCELLED),
        "{keys:?} on {screen:?}"
    );
    assert!(!done, "{keys:?}");
    assert_eq!(fx.digest(), before, "{keys:?} changed files");
}

/// Every way out before confirm, on a fresh install.
const FRESH_EXITS: &[&[Key]] = &[
    &[Quit],
    &[Esc],
    &[Down, Up, Quit],
    &[Enter, Quit],
    &[Enter, Space, Down, Space, Esc, Esc],
    &[Enter, Enter, Quit],
    &[Enter, Enter, Down, Space, Down, Down, Space, Esc, Esc, Esc],
    &[Enter, Enter, Enter, Quit],
    &[Enter, Enter, Enter, Down, Esc, Esc, Esc, Esc],
    &[Enter, Enter, Enter, Diff, Down, Down, Quit],
    &[Enter, Enter, Enter, Diff, Esc, Esc, Esc, Esc, Esc],
    &[Enter, Enter, Enter, Diff, Diff, Quit],
];

#[test]
fn cancel_or_quit_at_every_screen_of_a_fresh_install_changes_nothing() {
    let fx = with_clis();
    for keys in FRESH_EXITS {
        assert_cancels(&fx, keys);
    }
}

#[test]
fn cancel_or_quit_at_every_screen_of_an_installed_home_changes_nothing() {
    let fx = with_clis();
    let mut app = fx.app(Newest::Unknown);
    press(&mut app, &[Enter, Enter, Enter, Enter]);
    assert_eq!(app.screen(), Screen::Progress);
    // Installed and current: Change, Check, Remove. Change walks every
    // screen; Remove reviews the removal and quits there.
    let exits: &[&[Key]] = &[
        &[Quit],
        &[Enter, Quit],
        &[Enter, Space, Esc, Esc],
        &[Enter, Enter, Quit],
        &[Enter, Enter, Esc, Esc, Esc],
        &[Enter, Space, Enter, Enter, Quit],
        &[Enter, Space, Enter, Enter, Diff, Quit],
        &[Down, Down, Enter, Quit],
        &[Down, Down, Enter, Space, Diff, Esc, Esc, Quit],
    ];
    for keys in exits {
        assert_cancels(&fx, keys);
    }
}

#[test]
fn nothing_selected_stays_on_the_agents_screen() {
    let fx = with_clis();
    let mut app = fx.app(Newest::Unknown);
    press(
        &mut app,
        &[Enter, Space, Down, Down, Space, Down, Space, Enter],
    );
    assert_eq!(app.screen(), Screen::Agents);
    assert!(!app.flow().has_selection());
}

#[test]
fn enter_through_every_screen_installs_and_closes_with_the_result() {
    let fx = with_clis();
    let mut app = fx.app(Newest::Unknown);
    assert_eq!(app.keys(&[Enter, Enter, Enter, Enter]), Control::Continue);
    assert_eq!(app.screen(), Screen::Progress);
    assert!(!app.is_working());
    let progress = app.flow().progress().expect("results");
    assert_eq!(progress.code, 0, "{:?}", progress.lines);
    assert!(fx.deps.store.load().expect("load").is_some());
    assert_eq!(app.key(Down), Control::Continue);
    assert_eq!(app.key(Enter), Control::Exit(0));
}

#[test]
fn keys_wait_while_the_plan_is_applied() {
    let fx = with_clis();
    let mut app = fx.app(Newest::Unknown);
    for key in [Enter, Enter, Enter, Enter] {
        app.key(key);
    }
    assert!(app.is_working());
    assert_eq!(app.key(Quit), Control::Continue);
    app.work();
    assert!(!app.is_working());
    assert_eq!(app.key(Quit), Control::Exit(0));
}
