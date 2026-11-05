//! Tests of the numbered fallback and the no-terminal default (task 6.3;
//! Requirements 4.7–4.9): whole sessions driven line by line over the agent
//! doubles and temporary homes. Every way out before the review is
//! confirmed leaves the temporary folder byte-identical.

use super::*;
use crate::setup::agents::{AgentId, Feature};
use crate::setup::record::CheckResult;
use crate::setup::ui::fixtures::{Fx, fx, with_clis};
use crate::setup::ui::flow::{EXIT_CANCELLED, Newest};

const FOUND: [AgentId; 3] = [AgentId::ClaudeCode, AgentId::Codex, AgentId::OpenCode];

/// Runs the numbered screens on `input`; returns the exit code and output.
fn session(fx: &Fx, input: &str) -> (i32, String) {
    let mut flow = fx.flow(Newest::Unknown);
    let mut out = Vec::new();
    let code = run(&mut flow, &mut input.as_bytes(), &mut out);
    (code, shown(out))
}

/// Runs the no-terminal default; returns the exit code and output.
fn unattended(fx: &Fx) -> (i32, String) {
    let mut flow = fx.flow(Newest::Unknown);
    let mut out = Vec::new();
    let code = run_default(&mut flow, &mut out);
    (code, shown(out))
}

/// The output with `/` as the folder separator on every OS.
fn shown(out: Vec<u8>) -> String {
    String::from_utf8(out).expect("utf8").replace('\\', "/")
}

/// A fresh install's earlier-install config (the old installer's block).
fn old_codex_config(fx: &Fx) -> std::path::PathBuf {
    let bin = fx.deps.binary.to_string_lossy().replace('\\', "\\\\");
    let old = format!(
        "model = \"o3\"\n\n# BEGIN mmhmem (managed by install_local.sh -- do not edit)\n\
         [mcp_servers.mmhmem]\ncommand = \"{bin}\"\nargs = [\"stdio\"]\n# END mmhmem\n"
    );
    fx.write(&[".codex", "config.toml"], &old)
}

#[test]
fn quitting_at_any_screen_changes_nothing() {
    let fx = with_clis();
    let before = fx.digest();
    let scripts = [
        "",
        "q\n",
        "x\n0\nq\n",
        "1\nq\n",
        "1\n",
        "1\n1 2\nb\nq\n",
        "1\n\nq\n",
        "1\n\n\nq\n",
        "1\n\n\nn\n",
        "1\n\n\nno\n",
        "1\n\n\nd 1\nb\nq\n",
        "1\n\n\nb\nb\nb\nb\nb\n",
        "1\n\n\nmaybe\n",
        "1\n1 2 3\n\nq\n",
    ];
    for script in scripts {
        let (code, out) = session(&fx, script);
        assert_eq!(code, EXIT_CANCELLED, "{script:?}\n{out}");
        assert!(
            out.trim_end().ends_with(text::CANCELLED),
            "{script:?}\n{out}"
        );
        assert_eq!(fx.digest(), before, "{script:?} changed files");
    }
}

#[test]
fn the_home_screen_lists_the_state_and_numbered_actions() {
    let fx = with_clis();
    let (_, out) = session(&fx, "q\n");
    assert!(out.contains(text::TITLE_HOME), "{out}");
    assert!(out.contains(text::NOTHING_INSTALLED), "{out}");
    assert!(out.contains("Claude Code     not set up"), "{out}");
    assert!(
        out.contains("Claude Desktop  not found   (no Claude Desktop folder was found)"),
        "{out}"
    );
    assert!(
        out.contains(&format!("1) {}", text::ACTION_INSTALL)),
        "{out}"
    );
    assert!(out.contains(text::HELP_HOME), "{out}");
}

#[test]
fn bad_answers_are_met_with_a_hint_and_asked_again() {
    let fx = with_clis();
    let (code, out) = session(&fx, "7\nx\n1\n9\nzz\nq\n");
    assert_eq!(code, EXIT_CANCELLED);
    assert_eq!(out.matches(text::BAD_CHOICE).count(), 4, "{out}");
    assert!(out.contains(text::TITLE_AGENTS), "{out}");
}

#[test]
fn the_agents_screen_numbers_only_what_can_be_switched() {
    let fx = with_clis();
    let (_, out) = session(&fx, "1\nq\n");
    assert!(out.contains("1) [x] Claude Code"), "{out}");
    assert!(out.contains("2) [x] Codex"), "{out}");
    assert!(out.contains("3) [x] OpenCode"), "{out}");
    assert!(
        out.contains("    Antigravity: not found (Antigravity is not installed)"),
        "{out}"
    );
    assert!(out.contains(text::HELP_AGENTS), "{out}");
    let (_, out) = session(&fx, "1\n2\nq\n");
    assert!(out.contains("2) [ ] Codex"), "{out}");
    let (_, out) = session(&fx, "1\n1 2 3\n\nq\n");
    assert!(out.contains(text::NOTHING_SELECTED), "{out}");
}

#[test]
fn the_review_lists_changes_per_file_and_shows_details_on_request() {
    let fx = with_clis();
    let (code, out) = session(&fx, "1\n\n\nd 2\nd 9\nq\n");
    assert_eq!(code, EXIT_CANCELLED);
    assert!(out.contains(text::TITLE_REVIEW), "{out}");
    assert!(out.contains("~/.codex/config.toml"), "{out}");
    assert!(
        out.contains("Let Codex use mmhmem (adds mmhmem to Codex's server list in"),
        "{out}"
    );
    assert!(out.contains(text::REVIEW_BACKUP), "{out}");
    assert!(out.contains("[mcp_servers.mmhmem]"), "{out}");
    assert_eq!(out.matches(text::BAD_CHOICE).count(), 1, "{out}");
    let (_, hidden) = session(&fx, "1\n\n\nd 1\nq\n");
    assert!(hidden.contains(crate::setup::plan::HIDDEN_DIFF), "{hidden}");
}

#[test]
fn a_full_install_through_the_fallback_checks_and_records() {
    let fx = with_clis();
    let (code, out) = session(&fx, "1\n\n\ny\n");
    assert_eq!(code, 0, "{out}");
    assert!(out.contains(text::CHECKING), "{out}");
    assert!(out.contains("✔ Codex"), "{out}");
    assert!(out.contains(text::DONE), "{out}");
    assert!(out.contains(text::NEXT_CODEX), "{out}");
    assert!(!out.contains(text::CANCELLED), "{out}");
    let record = fx.deps.store.load().expect("load").expect("record");
    assert_eq!(record.agents.keys().copied().collect::<Vec<_>>().len(), 3);
    for id in FOUND {
        assert_eq!(record.agents[&id].features.get(&Feature::Mcp), Some(&true));
    }
}

#[test]
fn switching_a_tool_off_and_a_feature_on_changes_the_install() {
    let fx = with_clis();
    // Tool 1 (Claude Code) off; the first switchable feature of the first
    // remaining tool (Codex) on.
    let (code, out) = session(&fx, "1\n1\n\n1\n\ny\n");
    assert_eq!(code, 0, "{out}");
    let record = fx.deps.store.load().expect("load").expect("record");
    assert!(!record.agents.contains_key(&AgentId::ClaudeCode), "{out}");
    let codex = &record.agents[&AgentId::Codex];
    assert!(
        codex.features.values().filter(|on| **on).count() >= 2,
        "{out}"
    );
    assert!(out.contains(text::HELP_FEATURES), "{out}");
}

#[test]
fn a_second_run_offers_check_and_change_not_install() {
    let fx = with_clis();
    session(&fx, "1\n\n\ny\n");
    let before = fx.digest();
    let (code, out) = session(&fx, "q\n");
    assert_eq!(code, EXIT_CANCELLED);
    assert!(out.contains(text::REVIEW_NOTHING), "{out}");
    assert!(!out.contains(text::ACTION_INSTALL), "{out}");
    assert!(
        out.contains(&format!("1) {}", text::ACTION_CHANGE)),
        "{out}"
    );
    assert_eq!(fx.digest(), before);
    let (code, out) = session(&fx, "2\n");
    assert_eq!(code, 0, "{out}");
    assert!(out.contains(text::CHECKING), "{out}");
}

#[test]
fn an_earlier_install_is_taken_over_in_two_reviews() {
    let fx = with_clis();
    let config = old_codex_config(&fx);
    let (code, out) = session(&fx, "1\n\n\ny\ny\n");
    assert_eq!(code, 0, "{out}");
    assert!(out.contains(text::ADOPT_REVIEW), "{out}");
    assert_eq!(out.matches(text::TITLE_REVIEW).count(), 2, "{out}");
    let after = std::fs::read_to_string(config).expect("config");
    assert!(!after.contains("BEGIN mmhmem"));
}

#[test]
fn notes_without_an_action_are_printed_and_quitting_changes_nothing() {
    let fx = fx();
    let before = fx.digest();
    let (code, out) = session(&fx, "");
    assert_eq!(code, EXIT_CANCELLED);
    assert!(out.contains(text::NO_AGENT), "{out}");
    assert_eq!(fx.digest(), before);
}

#[test]
fn the_default_install_needs_no_questions() {
    let fx = with_clis();
    let (code, out) = unattended(&fx);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains(text::NO_TERMINAL), "{out}");
    assert!(out.contains("Let Claude Code use mmhmem"), "{out}");
    assert!(out.contains(text::CHECKING), "{out}");
    assert!(out.contains("✔ Codex"), "{out}");
    assert!(out.contains(text::DONE), "{out}");
    assert!(out.contains(text::NEXT_OPENCODE), "{out}");
    let record = fx.deps.store.load().expect("load").expect("record");
    for id in FOUND {
        let features = &record.agents[&id].features;
        assert_eq!(features.values().filter(|on| **on).count(), 1);
        assert_eq!(features.get(&Feature::Mcp), Some(&true));
    }
    let runs = fx.home.runner.args_of("claude");
    assert!(
        runs.iter()
            .any(|a| a.starts_with(&["mcp".into(), "get".into()]))
    );
}

#[test]
fn the_default_install_skips_a_tool_with_an_invalid_config_and_checks_only_the_others() {
    let fx = with_clis();
    let config = fx.write(&[".config", "opencode", "opencode.json"], "{ not json");
    let (code, out) = unattended(&fx);
    assert_eq!(code, 1, "{out}");
    assert!(
        out.lines()
            .any(|l| l.starts_with("OpenCode was skipped:") && l.contains("opencode.json")),
        "{out}"
    );
    assert!(!out.contains("✘ OpenCode"), "{out}");
    assert!(!out.contains("✔ OpenCode"), "{out}");
    assert!(!out.contains(text::NEXT_OPENCODE), "{out}");
    assert!(out.contains("✔ Claude Code"), "{out}");
    assert!(out.contains("✔ Codex"), "{out}");
    assert_skipped_opencode_unchanged(&fx, &config);
}

/// Claude Code and Codex are set up and passed; OpenCode is not set up,
/// its folder holds only the untouched `config`, and its CLI was not used
/// for a check.
fn assert_skipped_opencode_unchanged(fx: &Fx, config: &std::path::Path) {
    let record = fx.deps.store.load().expect("load").expect("record");
    for id in [AgentId::ClaudeCode, AgentId::Codex] {
        let rec = &record.agents[&id];
        assert_eq!(rec.features.get(&Feature::Mcp), Some(&true));
        assert_eq!(
            rec.last_check.as_ref().map(|c| c.result),
            Some(CheckResult::Pass)
        );
    }
    let opencode = &record.agents[&AgentId::OpenCode];
    assert!(!opencode.features.values().any(|on| *on), "{opencode:?}");
    assert!(opencode.managed.is_empty() && opencode.last_check.is_none());
    assert_eq!(
        std::fs::read_to_string(config).expect("config"),
        "{ not json"
    );
    let folder: Vec<_> = std::fs::read_dir(config.parent().expect("folder"))
        .expect("list")
        .collect();
    assert_eq!(folder.len(), 1, "only opencode.json is there");
    let runs = fx.home.runner.args_of("opencode");
    assert!(!runs.iter().any(|a| a.contains(&"mcp".into())), "{runs:?}");
}

#[test]
fn the_default_install_without_an_agent_changes_nothing() {
    let fx = fx();
    let before = fx.digest();
    let (code, out) = unattended(&fx);
    assert_eq!(code, EXIT_NO_AGENT);
    assert!(out.contains(text::NO_AGENT), "{out}");
    assert!(!out.contains(text::CHECKING), "{out}");
    assert_eq!(fx.digest(), before);
}

#[test]
fn the_default_install_when_everything_is_current_changes_nothing() {
    let fx = with_clis();
    unattended(&fx);
    let before = fx.digest();
    let (code, out) = unattended(&fx);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains(text::REVIEW_NOTHING), "{out}");
    assert!(!out.contains(text::CHECKING), "{out}");
    assert_eq!(fx.digest(), before);
}

#[test]
fn the_default_install_takes_over_an_earlier_install_first() {
    let fx = with_clis();
    let config = old_codex_config(&fx);
    let (code, out) = unattended(&fx);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains(text::ADOPT_REVIEW), "{out}");
    assert_eq!(out.matches(text::TITLE_REVIEW).count(), 2, "{out}");
    let after = std::fs::read_to_string(config).expect("config");
    assert!(!after.contains("BEGIN mmhmem"));
}

#[test]
fn answers_are_understood() {
    assert_eq!(parse(""), Answer::Enter);
    assert_eq!(parse(" Q "), Answer::Quit);
    assert_eq!(parse("back"), Answer::Back);
    assert_eq!(parse("1, 3 2"), Answer::Numbers(vec![0, 2, 1]));
    assert_eq!(parse("0"), Answer::Bad);
    assert_eq!(parse("x"), Answer::Bad);
    assert_eq!(parse_review("Y"), ReviewAnswer::Yes);
    assert_eq!(parse_review("d 3"), ReviewAnswer::Diff(2));
    assert_eq!(parse_review("d3"), ReviewAnswer::Diff(2));
    assert_eq!(parse_review("d"), ReviewAnswer::Bad);
    assert_eq!(parse_review("d 0"), ReviewAnswer::Bad);
}
