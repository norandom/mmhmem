//! `TestBackend` snapshots of every menu screen and home state (task 6.2;
//! Requirements 4.1–4.6): a fresh home, an installed one, an update
//! available, drift, a pending user step, an agent problem and several
//! Claude Desktop profiles. 80×24, home folder shown as `~`.

use super::*;
use crate::setup::agents::{AgentId, Feature, PendingUserStep};
use crate::setup::record::CheckResult;
use crate::setup::ui::fixtures::{Fx, agent_record, press, record, render, tag, with_clis};
use crate::setup::ui::flow::Newest;

use Key::{Diff, Down, Enter};

/// A record with Claude Code (passed) and Codex (hook approval pending).
fn installed(fx: &Fx) {
    let claude = agent_record(
        &[Feature::Mcp, Feature::RecapCommand],
        CheckResult::Pass,
        &[],
    );
    let codex = agent_record(
        &[Feature::Mcp, Feature::ReadHook],
        CheckResult::UserStepsPending,
        &[PendingUserStep::ApproveHookInCodex],
    );
    fx.save(&record(
        "v0.4.1",
        vec![(AgentId::ClaudeCode, claude), (AgentId::Codex, codex)],
    ));
}

fn snap(fx: &Fx, newest: Newest, keys: &[Key]) -> String {
    let mut app = fx.app(newest);
    press(&mut app, keys);
    render(&app)
}

#[test]
fn home_fresh() {
    let fx = with_clis();
    insta::assert_snapshot!(snap(&fx, Newest::Checking, &[]));
}

#[test]
fn home_installed_with_pending_step() {
    let fx = with_clis();
    installed(&fx);
    insta::assert_snapshot!(snap(&fx, Newest::Known(tag("v0.4.1")), &[]));
}

#[test]
fn home_update_available() {
    let fx = with_clis();
    installed(&fx);
    insta::assert_snapshot!(snap(&fx, Newest::Known(tag("v0.5.0")), &[]));
}

#[test]
fn home_update_note() {
    let fx = with_clis();
    installed(&fx);
    insta::assert_snapshot!(snap(&fx, Newest::Known(tag("v0.5.0")), &[Enter]));
}

#[test]
fn home_current_with_update_available() {
    let fx = with_clis();
    let mut app = fx.app(Newest::Unknown);
    press(&mut app, &[Enter, Enter, Enter, Enter]);
    // The check times are real ones; show the home on a record without them.
    let mut stored = fx.deps.store.load().expect("load").expect("record");
    for rec in stored.agents.values_mut() {
        rec.last_check = None;
    }
    fx.save(&stored);
    insta::assert_snapshot!(snap(&fx, Newest::Known(tag("v0.5.0")), &[]));
}

#[test]
fn home_drift() {
    let fx = with_clis();
    let mut app = fx.app(Newest::Unknown);
    press(&mut app, &[Enter, Enter, Enter, Enter]);
    let config = fx.home.env.codex_home().join("config.toml");
    let text = std::fs::read_to_string(&config).expect("config");
    std::fs::write(&config, text.replace("stdio", "other")).expect("edit");
    // The check times are real ones; show the drift on a record without them.
    let mut stored = fx.deps.store.load().expect("load").expect("record");
    for rec in stored.agents.values_mut() {
        rec.last_check = None;
    }
    fx.save(&stored);
    insta::assert_snapshot!(snap(&fx, Newest::Unknown, &[]));
}

#[test]
fn home_nothing_found() {
    let fx = crate::setup::ui::fixtures::fx();
    insta::assert_snapshot!(snap(&fx, Newest::Unknown, &[]));
}

#[test]
fn agents_fresh() {
    let fx = with_clis();
    insta::assert_snapshot!(snap(&fx, Newest::Unknown, &[Enter, Down]));
}

#[test]
fn agents_several_desktop_profiles() {
    let fx = with_clis();
    for dir in ["Library/Application Support/Claude", ".config/Claude"] {
        std::fs::create_dir_all(fx.home.home().join(dir)).expect("profile");
    }
    insta::assert_snapshot!(snap(&fx, Newest::Unknown, &[Enter]));
}

#[test]
fn features_fresh() {
    let fx = with_clis();
    insta::assert_snapshot!(snap(&fx, Newest::Unknown, &[Enter, Enter, Down, Down]));
}

#[test]
fn review_fresh() {
    let fx = with_clis();
    insta::assert_snapshot!(snap(&fx, Newest::Unknown, &[Enter, Enter, Enter]));
}

#[test]
fn review_diff() {
    let fx = with_clis();
    let keys = [Enter, Enter, Down, Down, Key::Space, Enter, Down, Diff];
    insta::assert_snapshot!(snap(&fx, Newest::Unknown, &keys));
}

#[test]
fn review_agent_problem() {
    let fx = with_clis();
    fx.write(&[".config", "opencode", "opencode.json"], "{ not json");
    insta::assert_snapshot!(snap(&fx, Newest::Unknown, &[Enter, Enter, Enter]));
}

/// The program started after an update, when the recorded choices change
/// no file: straight to the results of the check (Requirement 7.5).
#[test]
fn progress_after_an_update_without_changes() {
    let mut fx = with_clis();
    press(&mut fx.app(Newest::Unknown), &[Enter, Enter, Enter, Enter]);
    fx.deps.restart.resumed = Some(tag("v0.5.0"));
    let mut app = fx.app(Newest::Unknown);
    assert_eq!(app.screen(), Screen::Progress);
    app.work();
    insta::assert_snapshot!(render(&app));
}

#[test]
fn progress_done() {
    let fx = with_clis();
    insta::assert_snapshot!(snap(&fx, Newest::Unknown, &[Enter, Enter, Enter, Enter]));
}
