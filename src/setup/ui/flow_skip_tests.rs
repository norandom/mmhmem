//! Tests of a tool whose config is invalid (task 6.2): it is skipped, not
//! checked, offered again, set up once fixed; what was chosen for it is
//! remembered across a failed check and when every feature is switched off.

use super::*;
use crate::setup::agents::Feature;
use crate::setup::agents::doubles::Reply;
use crate::setup::agents::doubles_cli::OPENCODE_LIST_BEFORE_INIT;
use crate::setup::record::CheckResult;
use crate::setup::ui::fixtures::with_clis;

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
    // Codex waits only for its hook approval.
    for (id, result) in [
        (AgentId::ClaudeCode, CheckResult::Pass),
        (AgentId::Codex, CheckResult::UserStepsPending),
    ] {
        assert_eq!(
            stored.agents[&id].last_check.as_ref().map(|c| c.result),
            Some(result)
        );
    }
    let opencode = &stored.agents[&AgentId::OpenCode];
    assert!(!opencode.features.values().any(|on| *on), "{opencode:?}");
    assert!(opencode.managed.is_empty() && opencode.last_check.is_none());
    assert_eq!(
        opencode.skipped,
        Some([Feature::Mcp, Feature::ReadHook].into())
    );
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
    assert!(opencode.skipped.is_none());
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
fn a_fixed_tool_whose_check_fails_keeps_its_remembered_choice() {
    let fx = with_clis();
    let config = install_with_invalid_opencode(&fx);
    std::fs::write(&config, "{}\n").expect("fix");
    let never = vec![Reply::ok(OPENCODE_LIST_BEFORE_INIT)];
    fx.home.runner.on("opencode", &["mcp", "list"], never);
    let mut flow = fx.flow(Newest::Unknown);
    assert_eq!(flow.begin(&HomeAction::Install), Next::Review);
    assert!(flow.review().problems.is_empty());
    assert_eq!(flow.confirm(), Confirmed::Done);
    assert_eq!(flow.exit_code(), 1, "{:?}", flow.progress());
    let stored = fx.deps.store.load().expect("load").expect("record");
    let opencode = &stored.agents[&AgentId::OpenCode];
    assert!(opencode.managed.is_empty(), "{opencode:?}");
    assert_eq!(
        opencode.skipped,
        Some([Feature::Mcp, Feature::ReadHook].into())
    );
    // The next run offers the same choice again.
    let mut flow = fx.flow(Newest::Unknown);
    assert_eq!(flow.begin(&HomeAction::Install), Next::Review);
    let review = flow.review();
    let files: Vec<&str> = review.groups.iter().map(|g| g.file.as_str()).collect();
    assert!(
        files.iter().any(|f| f.contains("opencode.json")),
        "{files:?}"
    );
}

#[test]
fn switching_a_tool_off_while_its_config_is_invalid_is_remembered() {
    let fx = with_clis();
    install(&fx);
    let config = fx.home.env.config_home().join("opencode/opencode.json");
    std::fs::write(&config, "{ not json").expect("break");
    let mut flow = fx.flow(Newest::Unknown);
    assert_eq!(flow.begin(&HomeAction::Change), Next::Agents);
    let opencode = flow
        .agent_choices()
        .into_iter()
        .find(|c| {
            matches!(
                c,
                Choice::Agent {
                    agent: AgentId::OpenCode,
                    ..
                }
            )
        })
        .expect("row");
    flow.toggle_choice(&opencode);
    flow.make_plan();
    assert_eq!(flow.review().problems.len(), 1);
    assert_eq!(flow.confirm(), Confirmed::Done);
    let stored = fx.deps.store.load().expect("load").expect("record");
    assert_eq!(
        stored.agents[&AgentId::OpenCode].skipped,
        Some(Default::default())
    );
    // The next run does not offer the earlier features as chosen.
    let flow = fx.flow(Newest::Unknown);
    assert!(
        flow.recorded_selection()
            .features(AgentId::OpenCode)
            .is_empty()
    );
}
