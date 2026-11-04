//! More agent-check tests (reliable-installers task 4.2): the OpenCode
//! poll and race, its hook sample, an unreadable config, a failing server,
//! and Claude Desktop and Antigravity with pending user steps.

use std::time::Duration;

use serde_json::{Value, json};

use super::check_tests::{Fx, codex_fx, fx, outcome, want};
use super::doubles::Reply;
use super::doubles_cli::{self, OPENCODE_LIST_BEFORE_INIT};
use super::{AgentId, Feature, antigravity, claude_desktop, opencode};
use crate::setup::check::CheckFailure;
use crate::setup::record::{CheckResult, CheckStep, StepOutcome};

fn opencode_fx(features: &[Feature]) -> (Fx, opencode::OpenCode) {
    let mut fx = fx();
    fx.home.install_program("opencode");
    doubles_cli::opencode(&fx.home.runner, &fx.home.env);
    let agent = opencode::OpenCode::new(&fx.bin);
    fx.install(&agent, &want(AgentId::OpenCode, features));
    (fx, agent)
}

#[test]
fn opencode_race_first_answer_none_then_connected_passes() {
    let (fx, agent) = opencode_fx(&[Feature::Mcp]);
    let report = fx.check(&agent, &want(AgentId::OpenCode, &[Feature::Mcp]));
    assert_eq!(report.result, CheckResult::Pass, "{}", report.line);
    assert_eq!(report.line, "✔ OpenCode — mmhmem connected, 10 tools");
    let lists: Vec<_> = fx
        .home
        .runner
        .calls()
        .into_iter()
        .filter(|c| c.args[..2] == ["mcp", "list"])
        .collect();
    assert_eq!(lists.len(), 2);
    for call in &lists {
        assert_eq!(call.cwd.as_deref(), Some(fx.home.home()));
        assert!(!call.args.iter().any(|a| a == "--standalone"));
    }
    assert_eq!(fx.home.clock.slept(), Duration::from_secs(1));
    assert_eq!(
        outcome(&report, CheckStep::ConfigLoad),
        StepOutcome::Skipped
    );
}

#[test]
fn opencode_never_connected_fails_after_thirty_seconds() {
    let (fx, agent) = opencode_fx(&[Feature::Mcp]);
    let never = vec![Reply::ok(OPENCODE_LIST_BEFORE_INIT)];
    fx.home.runner.on("opencode", &["mcp", "list"], never);
    let report = fx.check(&agent, &want(AgentId::OpenCode, &[Feature::Mcp]));
    assert_eq!(report.result, CheckResult::Fail);
    assert_eq!(
        report.line,
        "✘ OpenCode — OpenCode did not show mmhmem within 30 seconds. What you can do: \
         close every OpenCode window, wait a moment, then run the installer again."
    );
    assert_eq!(fx.home.clock.slept(), Duration::from_secs(30));
    let failed = vec![Reply::ok("✗ mmhmem  failed\n")];
    fx.home.runner.on("opencode", &["mcp", "list"], failed);
    let report = fx.check(&agent, &want(AgentId::OpenCode, &[Feature::Mcp]));
    assert!(
        report
            .line
            .contains("OpenCode lists mmhmem but did not connect")
    );
}

#[test]
fn opencode_hook_sample_uses_the_plugin_event() {
    let features = [Feature::Mcp, Feature::ReadHook];
    let (fx, agent) = opencode_fx(&features);
    let redirect = json!({"decision": "deny", "reason": "Call smart_read"}).to_string();
    fx.home
        .runner
        .on("mmhmem", &["hook", "opencode"], vec![Reply::ok(&redirect)]);
    let report = fx.check(&agent, &want(AgentId::OpenCode, &features));
    assert_eq!(report.result, CheckResult::Pass, "{}", report.line);
    let call = fx.home.runner.calls().into_iter().last().unwrap();
    let event: Value = serde_json::from_slice(call.stdin.as_deref().unwrap()).unwrap();
    assert_eq!(event["tool"], "read");
    assert!(event["input"]["path"].is_string());
}

#[test]
fn an_unreadable_config_fails_the_registration_step_and_runs_no_command() {
    let (fx, agent) = opencode_fx(&[Feature::Mcp]);
    let file = fx.home.home().join(".config/opencode/opencode.json");
    std::fs::write(&file, "{ not json").unwrap();
    let before = fx.home.runner.calls().len();
    let report = fx.check(&agent, &want(AgentId::OpenCode, &[Feature::Mcp]));
    assert_eq!(report.result, CheckResult::Fail);
    assert!(report.line.starts_with(
        "✘ OpenCode — mmhmem did not start: the agent's settings file could not be read."
    ));
    assert_eq!(outcome(&report, CheckStep::Registration), StepOutcome::Fail);
    assert_eq!(
        outcome(&report, CheckStep::AgentCommand),
        StepOutcome::Skipped
    );
    assert_eq!(fx.home.runner.calls().len(), before);
    assert!(fx.home.probe.seen().is_empty());
}

#[test]
fn a_server_that_fails_skips_the_agent_steps() {
    let (fx, agent) = codex_fx(&[Feature::Mcp]);
    fx.home.probe.fail_with(Some(CheckFailure::NoAnswer {
        step: CheckStep::Handshake,
        seconds: 20,
    }));
    let before = fx.home.runner.calls().len();
    let report = fx.check(&agent, &want(AgentId::Codex, &[Feature::Mcp]));
    assert_eq!(report.result, CheckResult::Fail);
    assert!(
        report.line.starts_with(
            "✘ Codex — mmhmem did not start: mmhmem did not answer within 20 seconds."
        )
    );
    assert_eq!(
        outcome(&report, CheckStep::AgentCommand),
        StepOutcome::Skipped
    );
    assert_eq!(fx.home.runner.calls().len(), before);
}

#[test]
fn desktop_and_antigravity_pass_with_their_user_steps_pending() {
    let mut fx = fx();
    let config = fx.home.home().join(".config").join("Claude");
    std::fs::create_dir_all(&config).unwrap();
    let desktop = claude_desktop::ClaudeDesktop::new(&fx.bin);
    let mcp = want(AgentId::ClaudeDesktop, &[Feature::Mcp]);
    fx.install(&desktop, &mcp);
    let report = fx.check(&desktop, &mcp);
    assert_eq!(
        report.result,
        CheckResult::UserStepsPending,
        "{}",
        report.line
    );
    assert_eq!(
        report.lines(),
        [
            "✔ Claude Desktop — mmhmem connected, 10 tools",
            "● Claude Desktop — restart Claude Desktop so it starts mmhmem: quit it \
             completely, then open it again",
        ]
    );
    let gravity = antigravity::Antigravity::new(&fx.bin);
    let report = fx.check(&gravity, &want(AgentId::Antigravity, &[Feature::Mcp]));
    assert_eq!(report.result, CheckResult::UserStepsPending);
    assert!(report.pending[0].starts_with("● Antigravity — add mmhmem in Antigravity"));
    let seen = fx.home.probe.seen();
    assert_eq!(seen.len(), 2);
    assert!(
        seen.iter()
            .all(|r| r.program == fx.bin && r.args == ["stdio"])
    );
    std::fs::remove_file(config.join("claude_desktop_config.json")).unwrap();
    let report = fx.check(&desktop, &mcp);
    assert_eq!(report.result, CheckResult::Fail);
    assert!(
        report
            .line
            .contains("the agent's settings have no entry for mmhmem")
    );
}
