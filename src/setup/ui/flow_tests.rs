//! Tests of the setup controller (task 6.2): the home model and its
//! actions, the default choices, a full install that applies, checks,
//! stores the record and shows the next steps, the second run that has
//! nothing to change, drift, recovery, adoption, agent problems and the
//! Claude Desktop profile choice. Temporary homes and the agent doubles.

use super::*;
use crate::setup::agents::{DISPLAY_ORDER, Feature};
use crate::setup::record::{CheckResult, EntryState, PendingUserStep};
use crate::setup::ui::fixtures::{agent_record, fx, record, tag, with_clis};

const FOUND: [AgentId; 3] = [AgentId::ClaudeCode, AgentId::Codex, AgentId::OpenCode];

/// Install the default choices through the flow; returns it after confirm.
fn install(fx: &crate::setup::ui::fixtures::Fx) -> Flow {
    let mut flow = fx.flow(Newest::Unknown);
    assert_eq!(flow.begin(&HomeAction::Install), Next::Agents);
    flow.make_plan();
    assert!(!flow.review().is_empty());
    assert_eq!(flow.confirm(), Confirmed::Done);
    flow
}

#[test]
fn a_fresh_home_offers_only_install() {
    let fx = with_clis();
    let home = fx.flow(Newest::Checking).home();
    assert_eq!(home.actions, [HomeAction::Install]);
    assert_eq!(home.installed, text::NOTHING_INSTALLED);
    assert_eq!(home.newest, text::NEWEST_CHECKING);
    assert_eq!(home.rows.len(), DISPLAY_ORDER.len());
    assert!(matches!(
        home.rows[1].state,
        AgentState::NotFound(ref reason) if reason.contains("Claude Desktop")
    ));
}

#[test]
fn a_new_install_preselects_detected_agents_with_only_memory_on() {
    let fx = with_clis();
    let mut flow = fx.flow(Newest::Checking);
    assert_eq!(flow.begin(&HomeAction::Install), Next::Agents);
    assert_eq!(flow.selection().agents(), FOUND);
    for id in FOUND {
        assert_eq!(flow.selection().features(id), [Feature::Mcp].into());
    }
    let rows = flow.feature_rows();
    let hook = rows
        .iter()
        .find(|r| r.feature == Some(Feature::ReadHook))
        .expect("hook row");
    assert!(!hook.on);
    let worker = rows.last().expect("worker row");
    assert_eq!(worker.label, text::FEATURE_WORKER);
    assert!(!worker.on);
    assert!(
        rows.iter()
            .filter(|r| r.locked && !r.is_heading())
            .all(|r| r.on)
    );
}

#[test]
fn no_agent_means_no_action() {
    let fx = fx();
    let home = fx.flow(Newest::Unknown).home();
    assert!(home.actions.is_empty());
    assert!(home.notes.contains(&text::NO_AGENT.to_owned()));
}

#[test]
fn a_full_install_applies_checks_stores_the_record_and_shows_next_steps() {
    let fx = with_clis();
    let flow = install(&fx);
    let progress = flow.progress().expect("results");
    assert_eq!(progress.code, 0, "{:?}", progress.lines);
    assert!(progress.lines.iter().any(|l| l.contains(text::CHECKING)));
    assert!(progress.lines.iter().any(|l| l.starts_with("✔ Codex")));
    for id in FOUND {
        assert!(
            progress
                .next_steps
                .contains(&text::next_step(id).to_owned())
        );
    }
    let stored = fx.deps.store.load().expect("load").expect("record");
    assert_eq!(stored.release, Some(tag("v0.4.1")));
    for id in FOUND {
        let rec = &stored.agents[&id];
        assert_eq!(rec.features.get(&Feature::Mcp), Some(&true));
        assert_eq!(
            rec.last_check.as_ref().map(|c| c.result),
            Some(CheckResult::Pass)
        );
    }
    assert_eq!(flow.exit_code(), 0);
}

#[test]
fn a_second_run_has_nothing_to_change() {
    let fx = with_clis();
    install(&fx);
    let before = fx.digest();
    let mut flow = fx.flow(Newest::Known(tag("v0.4.1")));
    let home = flow.home();
    assert!(home.notes.contains(&text::REVIEW_NOTHING.to_owned()));
    assert_eq!(
        home.actions,
        [HomeAction::Change, HomeAction::Check, HomeAction::Remove]
    );
    assert!(home.rows[0].features.contains(text::SHORT_MEMORY));
    assert_eq!(flow.begin(&HomeAction::Install), Next::Review);
    assert!(flow.review().is_empty());
    assert_eq!(fx.digest(), before);
}

#[test]
fn a_newer_release_offers_the_update_hook_and_changes_nothing() {
    let fx = with_clis();
    install(&fx);
    let before = fx.digest();
    let mut flow = fx.flow(Newest::Known(tag("v0.5.0")));
    let actions = flow.home().actions;
    assert_eq!(actions[0], HomeAction::UpdateTo(tag("v0.5.0")));
    assert!(!actions.contains(&HomeAction::Install));
    assert_eq!(flow.begin(&actions[0]), Next::Note(text::UPDATE_NO_KEY));
    assert_eq!(flow.begin(&HomeAction::Remove), Next::Review);
    assert_eq!(fx.digest(), before);
}

#[test]
fn check_runs_again_and_stores_the_result() {
    let fx = with_clis();
    install(&fx);
    let mut flow = fx.flow(Newest::Unknown);
    assert_eq!(flow.begin(&HomeAction::Check), Next::Progress);
    let progress = flow.progress().expect("results");
    assert_eq!(progress.code, 0, "{:?}", progress.lines);
    assert!(progress.lines.iter().any(|l| l.starts_with("✔ OpenCode")));
}

#[test]
fn drift_is_reported_and_install_is_offered_again() {
    let fx = with_clis();
    install(&fx);
    let config = fx.home.env.codex_home().join("config.toml");
    let text = std::fs::read_to_string(&config).expect("config");
    std::fs::write(&config, text.replace("stdio", "other")).expect("edit");
    let flow = fx.flow(Newest::Unknown);
    let home = flow.home();
    assert!(
        home.notes
            .iter()
            .any(|n| n.contains("config.toml was changed outside setup")),
        "{:?}",
        home.notes
    );
    assert!(home.actions.contains(&HomeAction::Install));
}

#[test]
fn an_interrupted_change_is_resolved_in_memory_only() {
    let fx = with_clis();
    install(&fx);
    let mut stored = fx.deps.store.load().expect("load").expect("record");
    let entry = &mut stored
        .agents
        .get_mut(&AgentId::Codex)
        .expect("codex")
        .managed[0];
    entry.state = EntryState::Applying;
    fx.save(&stored);
    let before = fx.digest();
    let flow = fx.flow(Newest::Unknown);
    assert_eq!(flow.home().notes[0], text::RECOVERED);
    let codex = &flow.record().expect("record").agents[&AgentId::Codex];
    assert_eq!(codex.managed[0].state, EntryState::Done);
    assert_eq!(fx.digest(), before);
}

#[test]
fn an_invalid_config_skips_only_that_agent() {
    let fx = with_clis();
    fx.write(&[".config", "opencode", "opencode.json"], "{ not json");
    let mut flow = fx.flow(Newest::Unknown);
    flow.begin(&HomeAction::Install);
    flow.make_plan();
    let review = flow.review();
    assert_eq!(review.problems.len(), 1);
    assert!(review.problems[0].starts_with("OpenCode was skipped"));
    assert!(!review.is_empty());
}

/// Installs the default choices with OpenCode's config invalid; returns
/// the config file.
fn install_with_invalid_opencode(fx: &crate::setup::ui::fixtures::Fx) -> std::path::PathBuf {
    let config = fx.write(&[".config", "opencode", "opencode.json"], "{ not json");
    let flow = install(fx);
    let progress = flow.progress().expect("results");
    assert_eq!(progress.code, 1, "{:?}", progress.lines);
    assert!(
        progress.lines[0].starts_with("OpenCode was skipped:"),
        "{:?}",
        progress.lines
    );
    config
}

#[test]
fn a_tool_skipped_for_an_invalid_config_is_not_checked_or_recorded_as_set_up() {
    let fx = with_clis();
    install_with_invalid_opencode(&fx);
    let stored = fx.deps.store.load().expect("load").expect("record");
    for id in [AgentId::ClaudeCode, AgentId::Codex] {
        assert_eq!(
            stored.agents[&id].last_check.as_ref().map(|c| c.result),
            Some(CheckResult::Pass)
        );
    }
    let opencode = &stored.agents[&AgentId::OpenCode];
    assert!(!opencode.features.values().any(|on| *on), "{opencode:?}");
    assert!(opencode.managed.is_empty() && opencode.last_check.is_none());
    assert_eq!(opencode.skipped, [Feature::Mcp].into());
}

#[test]
fn the_results_of_a_skipped_tool_have_no_check_line() {
    let fx = with_clis();
    fx.write(&[".config", "opencode", "opencode.json"], "{ not json");
    let flow = install(&fx);
    let lines = &flow.progress().expect("results").lines;
    assert!(!lines.iter().any(|l| l.contains("OpenCode —")), "{lines:?}");
    assert!(lines.iter().any(|l| l.starts_with("✔ Claude Code")));
    assert!(lines.iter().any(|l| l.starts_with("✔ Codex")));
    assert_eq!(flow.exit_code(), 1);
}

#[test]
fn a_check_after_a_skip_checks_only_what_is_set_up() {
    let fx = with_clis();
    install_with_invalid_opencode(&fx);
    let mut flow = fx.flow(Newest::Unknown);
    assert_eq!(flow.begin(&HomeAction::Check), Next::Progress);
    let lines = &flow.progress().expect("results").lines;
    assert!(!lines.iter().any(|l| l.contains("OpenCode")), "{lines:?}");
}

#[test]
fn a_skipped_tool_is_offered_again_and_skipped_again_while_its_config_is_invalid() {
    let fx = with_clis();
    install_with_invalid_opencode(&fx);
    let before = fx.digest();
    let mut flow = fx.flow(Newest::Unknown);
    let home = flow.home();
    assert!(home.actions.contains(&HomeAction::Install));
    let row = home.rows.iter().find(|r| r.agent == AgentId::OpenCode);
    assert_eq!(row.map(|r| &r.state), Some(&AgentState::NotSetUp));
    assert_eq!(flow.begin(&HomeAction::Install), Next::Review);
    assert_eq!(flow.review().problems.len(), 1);
    assert!(flow.review().is_empty());
    assert_eq!(fx.digest(), before);
}

#[test]
fn a_skipped_tool_is_set_up_once_its_config_is_fixed() {
    let fx = with_clis();
    let config = install_with_invalid_opencode(&fx);
    std::fs::write(&config, "{}\n").expect("fix");
    let mut flow = fx.flow(Newest::Unknown);
    assert!(flow.home().actions.contains(&HomeAction::Install));
    assert_eq!(flow.begin(&HomeAction::Install), Next::Review);
    let review = flow.review();
    assert!(review.problems.is_empty(), "{:?}", review.problems);
    let files: Vec<&str> = review.groups.iter().map(|g| g.file.as_str()).collect();
    assert!(
        files.iter().any(|f| f.contains("opencode.json")),
        "{files:?}"
    );
    assert_eq!(flow.confirm(), Confirmed::Done);
    let progress = flow.progress().expect("results");
    assert_eq!(progress.code, 0, "{:?}", progress.lines);
    assert!(progress.lines.iter().any(|l| l.starts_with("✔ OpenCode")));
    let stored = fx.deps.store.load().expect("load").expect("record");
    let opencode = &stored.agents[&AgentId::OpenCode];
    assert_eq!(opencode.features.get(&Feature::Mcp), Some(&true));
    assert!(opencode.skipped.is_empty());
    assert_eq!(
        opencode.last_check.as_ref().map(|c| c.result),
        Some(CheckResult::Pass)
    );
}

#[test]
fn a_set_up_tool_whose_config_broke_keeps_its_record_and_is_not_checked() {
    let fx = with_clis();
    install(&fx);
    let config = fx.home.env.config_home().join("opencode/opencode.json");
    assert!(config.is_file());
    std::fs::write(&config, "{ not json").expect("break");
    let earlier = fx.deps.store.load().expect("load").expect("record");
    let mut flow = fx.flow(Newest::Unknown);
    assert_eq!(flow.begin(&HomeAction::Change), Next::Agents);
    flow.make_plan();
    assert_eq!(flow.review().problems.len(), 1);
    assert_eq!(flow.confirm(), Confirmed::Done);
    let progress = flow.progress().expect("results");
    assert_eq!(progress.code, 1, "{:?}", progress.lines);
    assert!(!progress.lines.iter().any(|l| l.contains("OpenCode —")));
    let stored = fx.deps.store.load().expect("load").expect("record");
    assert_eq!(
        stored.agents[&AgentId::OpenCode],
        earlier.agents[&AgentId::OpenCode]
    );
    assert_eq!(
        std::fs::read_to_string(&config).expect("config"),
        "{ not json"
    );
}

#[test]
fn several_desktop_profiles_are_offered_and_the_chosen_one_is_planned() {
    let fx = with_clis();
    let mac = fx.home.home().join("Library/Application Support/Claude");
    let linux = fx.home.home().join(".config/Claude");
    std::fs::create_dir_all(&mac).expect("mac");
    std::fs::create_dir_all(&linux).expect("linux");
    let mut flow = fx.flow(Newest::Unknown);
    flow.begin(&HomeAction::Install);
    let profiles: Vec<Choice> = flow
        .agent_choices()
        .into_iter()
        .filter(|c| matches!(c, Choice::Profile { .. }))
        .collect();
    assert_eq!(profiles.len(), 2);
    flow.toggle_choice(&profiles[1]);
    flow.make_plan();
    let review = flow.review();
    assert!(review.problems.is_empty(), "{:?}", review.problems);
    assert!(
        review
            .groups
            .iter()
            .any(|g| g.file.contains(".config") && g.file.contains("claude_desktop_config.json"))
    );
}

#[test]
fn pending_user_steps_show_on_the_home_screen() {
    let fx = with_clis();
    let rec = agent_record(
        &[Feature::Mcp, Feature::ReadHook],
        CheckResult::UserStepsPending,
        &[PendingUserStep::ApproveHookInCodex],
    );
    fx.save(&record("v0.4.1", vec![(AgentId::Codex, rec)]));
    let home = fx.flow(Newest::Unknown).home();
    let codex = &home.rows[2];
    assert_eq!(codex.state, AgentState::Installed);
    assert_eq!(codex.features, "memory, read hook");
    assert_eq!(
        codex.last_check,
        format!("● {}", text::PENDING_APPROVE_HOOK)
    );
}

#[test]
fn an_earlier_install_is_adopted_before_the_choices_are_planned() {
    let fx = with_clis();
    let bin = fx.deps.binary.to_string_lossy().replace('\\', "\\\\");
    let old = format!(
        "model = \"o3\"\n\n# BEGIN mmhmem (managed by install_local.sh -- do not edit)\n\
         [mcp_servers.mmhmem]\ncommand = \"{bin}\"\nargs = [\"stdio\"]\n# END mmhmem\n"
    );
    let config = fx.write(&[".codex", "config.toml"], &old);
    let mut flow = fx.flow(Newest::Unknown);
    assert!(flow.home().notes.iter().any(|n| n.contains("for Codex")));
    flow.begin(&HomeAction::Install);
    flow.make_plan();
    assert_eq!(flow.review().intro, [text::ADOPT_REVIEW]);
    assert_eq!(flow.confirm(), Confirmed::Review);
    assert!(
        flow.record()
            .is_some_and(|r| r.agents.contains_key(&AgentId::Codex))
    );
    assert!(flow.review().intro.is_empty());
    assert_eq!(flow.confirm(), Confirmed::Done);
    let after = std::fs::read_to_string(&config).expect("config");
    assert!(!after.contains("BEGIN mmhmem"));
    assert!(after.starts_with("model = \"o3\""));
    assert_eq!(flow.exit_code(), 0, "{:?}", flow.progress());
}
