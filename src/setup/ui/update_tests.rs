//! Tests of the update mode (Requirement 7.5): `mmhmem update` sessions on
//! temporary homes with the agent doubles, a local signed release
//! (`FakeRelease`, test keys) and a recorded restart instead of a real one,
//! and the install command run again on an installed system.

use super::*;
use crate::setup::release::fixtures::TRUSTED;
use crate::setup::release_keys::test_keys::public_key;
use crate::setup::ui::app::{App, Screen};
use crate::setup::ui::fixtures::{
    Fx, install, installed_old, new_release, recording_exec, release_dir, tag, with_clis,
};
use crate::setup::ui::flow::Restart;

fn v(a: &[&str]) -> Vec<String> {
    a.iter().map(|s| s.to_string()).collect()
}

/// One `mmhmem update` session over `flow` with `answers` typed and the
/// newest release `newest`; returns the exit code and everything printed.
fn run(flow: &mut Flow, newest: Newest, answers: &str) -> (i32, String) {
    let mut out = Vec::new();
    let code = session(flow, move || newest, &mut answers.as_bytes(), &mut out);
    (code, String::from_utf8(out).expect("utf8"))
}

/// An installed home updating to v9.9.9 from a local signed release; the
/// restart is recorded as `update …` and answers 0.
fn updating(dir: &std::path::Path) -> (Fx, crate::setup::ui::fixtures::Starts) {
    let mut fx = installed_old(dir, vec![public_key(TRUSTED)]);
    let (exec, seen) = recording_exec(0);
    fx.deps.restart = Restart {
        exec: Some(exec),
        command: Command::Update,
        ..Restart::default()
    };
    (fx, seen)
}

/// `fx` as the program started after the update to v9.9.9 sees it.
fn restarted(fx: &mut Fx) -> Flow {
    fx.deps.version = tag("v9.9.9");
    fx.deps.restart = Restart {
        command: Command::Update,
        resumed: Some(tag("v9.9.9")),
        ..Restart::default()
    };
    fx.flow(Newest::Unknown)
}

/// The OpenCode config, deleted so the recorded choices plan a change.
fn opencode_json(fx: &Fx) -> std::path::PathBuf {
    fx.home.home().join(".config/opencode/opencode.json")
}

#[test]
fn parses_only_the_internal_arguments() {
    let a = UpdateArgs::parse(&v(&["--release-dir", "d", "--restarted", "v1.0.0"])).unwrap();
    assert_eq!(a.release_dir, Some(PathBuf::from("d")));
    assert_eq!(a.restarted.as_deref(), Some("v1.0.0"));
    assert_eq!(UpdateArgs::parse(&[]).unwrap(), UpdateArgs::default());
    let bootstrap = UpdateArgs::parse(&v(&["--bootstrap", "v1.0.0"]));
    assert_eq!(bootstrap, Err("--bootstrap".into()));
    assert_eq!(UpdateArgs::parse(&v(&["--bogus"])), Err("--bogus".into()));
    assert_eq!(
        UpdateArgs::parse(&v(&["--restarted"])),
        Err("--restarted".into())
    );
}

#[test]
fn without_a_terminal_nothing_is_read_or_changed() {
    let mut out = Vec::new();
    let code = refuse_without_terminal(Mode::NoTerminal, &mut out);
    assert_eq!(code, Some(EXIT_NEEDS_TERMINAL));
    let said = String::from_utf8(out).unwrap();
    assert_eq!(said.trim_end(), text::update::NEEDS_TERMINAL);
    assert!(said.contains("run: mmhmem update"), "{said}");
    for mode in [Mode::Menu, Mode::Fallback] {
        let mut out = Vec::new();
        assert_eq!(refuse_without_terminal(mode, &mut out), None);
        assert!(out.is_empty());
    }
}

#[test]
fn without_an_install_record_it_says_to_set_up_first_and_changes_nothing() {
    let fx = with_clis();
    let before = fx.digest();
    let mut flow = fx.flow(Newest::Checking);
    let mut out = Vec::new();
    let lookup = || -> Newest { panic!("no lookup without a record") };
    let code = session(&mut flow, lookup, &mut "y\n".as_bytes(), &mut out);
    assert_eq!(code, EXIT_NOT_SET_UP);
    let said = String::from_utf8(out).unwrap();
    assert_eq!(said.trim_end(), text::update::NOT_SET_UP);
    assert_eq!(fx.digest(), before);
}

#[test]
fn an_unknown_newest_release_changes_nothing() {
    let fx = with_clis();
    install(&fx);
    let before = fx.digest();
    let (code, said) = run(&mut fx.flow(Newest::Checking), Newest::Unknown, "y\n");
    assert_eq!(code, 1);
    assert!(
        said.ends_with(&format!("{}\n", text::update::NEWEST_UNKNOWN)),
        "{said}"
    );
    assert_eq!(fx.digest(), before);
}

#[test]
fn an_up_to_date_install_says_so_and_changes_nothing() {
    let fx = with_clis();
    install(&fx);
    let before = fx.digest();
    for newest in ["v0.4.1", "v0.4.0"] {
        let (code, said) = run(
            &mut fx.flow(Newest::Checking),
            Newest::Known(tag(newest)),
            "",
        );
        assert_eq!(code, 0);
        let want = format!(
            "{}\nmmhmem v0.4.1 is up to date.\n",
            text::update::CHECKING_NEWEST
        );
        assert_eq!(said, want);
    }
    assert_eq!(fx.digest(), before);
}

#[test]
fn a_declined_update_changes_nothing() {
    let fake = new_release();
    let dir = release_dir(&fake);
    let (fx, seen) = updating(dir.path());
    let before = fx.digest();
    for answers in ["n\n", "maybe\nno\n", ""] {
        let (code, said) = run(
            &mut fx.flow(Newest::Checking),
            Newest::Known(tag("v9.9.9")),
            answers,
        );
        assert_eq!(code, EXIT_CANCELLED, "{answers:?}");
        assert!(said.contains("mmhmem v0.4.1 is installed. v9.9.9 is available.\n"));
        assert!(said.contains(text::update::KEEPS_CHOICES));
        assert!(
            said.trim_end().ends_with(text::update::NOTHING_CHANGED),
            "{said}"
        );
    }
    assert_eq!(fx.digest(), before);
    assert_eq!(std::fs::read(&fx.deps.binary).unwrap(), b"OLD");
    assert!(seen.lock().unwrap().is_empty());
}

#[test]
fn without_a_trusted_release_key_nothing_is_downloaded() {
    let fake = new_release();
    let dir = release_dir(&fake);
    let fx = installed_old(dir.path(), Vec::new());
    let before = fx.digest();
    let (code, said) = run(
        &mut fx.flow(Newest::Checking),
        Newest::Known(tag("v9.9.9")),
        "y\n",
    );
    assert_eq!(code, 1);
    assert!(said.trim_end().ends_with(text::UPDATE_NO_KEY), "{said}");
    assert!(!said.contains("Downloading"), "{said}");
    assert_eq!(fx.digest(), before);
}

/// Answers Enter to "Update now?" on `fx`: the program is replaced and the
/// new one is started as `update --restarted v9.9.9`.
fn accept_update(fx: &Fx, seen: &crate::setup::ui::fixtures::Starts) {
    let mut flow = fx.flow(Newest::Checking);
    let (code, said) = run(&mut flow, Newest::Known(tag("v9.9.9")), "\n");
    assert_eq!(code, 0, "{said}");
    assert_eq!(flow.restart_now(), Some(Ok(0)));
    let want = format!(
        "{}\nmmhmem v0.4.1 is installed. v9.9.9 is available.\n{}\n{} \
         Downloading v9.9.9 ... signature and checksum OK.\n",
        text::update::CHECKING_NEWEST,
        text::update::KEEPS_CHOICES,
        text::update::PROMPT_UPDATE,
    );
    assert_eq!(said, want);
    assert_eq!(std::fs::read(&fx.deps.binary).unwrap(), b"NEW PROGRAM");
    let starts = seen.lock().unwrap().clone();
    assert_eq!(starts.len(), 1);
    assert_eq!(starts[0].args, v(&["update", "--restarted", "v9.9.9"]));
}

#[test]
fn an_accepted_update_restarts_as_update_and_the_restarted_run_only_checks() {
    let fake = new_release();
    let dir = release_dir(&fake);
    let (mut fx, seen) = updating(dir.path());
    let choices = fx.deps.store.load().unwrap().unwrap();
    accept_update(&fx, &seen);
    // Forget the earlier results (timestamps are to the second, so a check
    // in the same second would look unchanged): the restarted run must
    // store a new one for every tool.
    let mut before = fx.deps.store.load().unwrap().unwrap();
    for agent in before.agents.values_mut() {
        agent.last_check = None;
    }
    fx.deps.store.save(&before).unwrap();

    // The new program: same choices, no change, so no question; it checks.
    let mut flow = restarted(&mut fx);
    let (code, said) = run(&mut flow, Newest::Unknown, "");
    assert_eq!(code, 0, "{said}");
    assert!(
        said.starts_with("Updated to v9.9.9. Checking your tools...\n✔ Claude Code"),
        "{said}"
    );
    assert!(said.contains("\n✔ Codex"), "{said}");
    assert!(!said.contains(text::update::PROMPT_APPLY), "{said}");
    assert!(!said.contains(text::CHECKING), "{said}");
    let stored = fx.deps.store.load().unwrap().unwrap();
    assert_eq!(stored.release, Some(tag("v9.9.9")));
    let selection = crate::setup::agents::Selection::from_record;
    assert_eq!(selection(&stored), selection(&choices));
    assert!(
        stored.agents.values().all(|a| a.last_check.is_some()),
        "checked again: {stored:?}"
    );
    assert_eq!(flow.restart_now(), None);
}

#[test]
fn a_restarted_update_with_changes_lists_them_and_asks_once() {
    let fake = new_release();
    let dir = release_dir(&fake);
    let (mut fx, _seen) = updating(dir.path());
    run(
        &mut fx.flow(Newest::Checking),
        Newest::Known(tag("v9.9.9")),
        "y\n",
    );
    std::fs::remove_file(opencode_json(&fx)).expect("delete");

    let before = fx.digest();
    let (code, said) = run(&mut restarted(&mut fx), Newest::Unknown, "n\n");
    let said = said.replace('\\', "/");
    assert_eq!(code, EXIT_CANCELLED);
    assert!(said.starts_with("Updated to v9.9.9.\n"), "{said}");
    assert!(said.contains(text::update::CHANGES_NEEDED), "{said}");
    assert!(
        said.contains("~/.config/opencode/opencode.json\n  + "),
        "{said}"
    );
    assert!(
        said.trim_end().ends_with(text::update::CHANGES_KEPT),
        "{said}"
    );
    assert_eq!(fx.digest(), before);

    let (code, said) = run(&mut restarted(&mut fx), Newest::Unknown, "\n");
    assert_eq!(code, 0, "{said}");
    assert_eq!(said.matches(text::update::PROMPT_APPLY).count(), 1);
    assert!(said.contains("OpenCode: 1 change(s) made."), "{said}");
    assert!(said.contains("\n✔ OpenCode"), "{said}");
    assert!(opencode_json(&fx).is_file());
}

#[test]
fn the_menu_after_an_update_with_changes_opens_the_review() {
    let fake = new_release();
    let dir = release_dir(&fake);
    let (mut fx, _seen) = updating(dir.path());
    run(
        &mut fx.flow(Newest::Checking),
        Newest::Known(tag("v9.9.9")),
        "y\n",
    );
    std::fs::remove_file(opencode_json(&fx)).expect("delete");
    fx.deps.restart = Restart {
        resumed: Some(tag("v9.9.9")),
        ..Restart::default()
    };
    let app = App::new(fx.flow(Newest::Unknown));
    assert_eq!(app.screen(), Screen::Review);
    assert!(!app.is_working());
    assert_eq!(app.flow().review().intro, [text::updated("v9.9.9")]);
    assert!(!app.flow().review().is_empty());
}

/// An installed home (v0.4.1) started by the install command of `brought`,
/// whose program holds `NEW`.
fn bootstrapped(brought: &str, temp: &std::path::Path) -> Fx {
    let mut fx = with_clis();
    install(&fx);
    let program = temp.join("mmhmem");
    std::fs::write(&program, b"NEW").expect("program");
    fx.deps.install_from = Some(program);
    fx.deps.version = tag(brought);
    fx
}

#[test]
fn the_install_command_with_a_newer_release_updates_in_update_mode() {
    let temp = tempfile::tempdir().expect("temp");
    let fx = bootstrapped("v0.5.0", temp.path());
    let before = fx.digest();
    let mut out = Vec::new();
    let code = bootstrap(
        &mut fx.flow(Newest::Unknown),
        &mut "n\n".as_bytes(),
        &mut out,
    );
    assert_eq!(code, Some(EXIT_CANCELLED));
    assert_eq!(fx.digest(), before);

    let mut out = Vec::new();
    let code = bootstrap(
        &mut fx.flow(Newest::Unknown),
        &mut "y\n".as_bytes(),
        &mut out,
    );
    let said = String::from_utf8(out).unwrap();
    assert_eq!(code, Some(0), "{said}");
    let start = format!(
        "mmhmem v0.4.1 is set up. This installer has v0.5.0.\n{}\n{} \
         Updated to v0.5.0. Checking your tools...\n",
        text::update::KEEPS_CHOICES,
        text::update::PROMPT_UPDATE,
    );
    assert!(said.starts_with(&start), "{said}");
    assert!(!said.contains(text::update::PROMPT_APPLY), "{said}");
    assert!(said.contains("\n✔ Codex"), "{said}");
    assert_eq!(std::fs::read(&fx.deps.binary).unwrap(), b"NEW");
    let stored = fx.deps.store.load().unwrap().unwrap();
    assert_eq!(stored.release, Some(tag("v0.5.0")));
}

#[test]
fn the_install_command_of_the_same_or_an_older_release_opens_the_menu() {
    let temp = tempfile::tempdir().expect("temp");
    for brought in ["v0.4.1", "v0.4.0"] {
        let fx = bootstrapped(brought, temp.path());
        let mut out = Vec::new();
        let code = bootstrap(
            &mut fx.flow(Newest::Unknown),
            &mut "y\n".as_bytes(),
            &mut out,
        );
        assert_eq!(code, None, "{brought}");
        assert!(out.is_empty());
    }
    // A first install is no update either.
    let mut fx = with_clis();
    fx.deps.install_from = Some(temp.path().join("mmhmem"));
    fx.deps.version = tag("v0.5.0");
    let code = bootstrap(
        &mut fx.flow(Newest::Unknown),
        &mut "y\n".as_bytes(),
        &mut Vec::new(),
    );
    assert_eq!(code, None);
}

#[test]
fn the_home_title_names_the_installed_release() {
    let fx = with_clis();
    assert_eq!(fx.flow(Newest::Unknown).home().title, text::TITLE_HOME);
    install(&fx);
    let home = fx.flow(Newest::Known(tag("v0.5.0"))).home();
    assert_eq!(home.title, "mmhmem v0.4.1 is set up");
    assert_eq!(
        home.actions[0].label(),
        "Update to v0.5.0 (keeps your choices)"
    );
}
