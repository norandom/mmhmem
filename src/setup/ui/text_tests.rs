//! The plain-language rule (requirement 1.4): no project-internal term in a
//! user-facing string unless the same string explains it.

use super::text::{self, ALL};
use crate::setup::record::AgentId;

/// (term, phrase that must be in the same string, or "" = never allowed).
const JARGON: &[(&str, &str)] = &[
    ("playbook", ""),
    ("shunt", ""),
    ("routing instructions", ""),
    ("smart_read", ""),
    ("breadcrumb", ""),
    ("mcp", "Model Context Protocol"),
    ("worker", "Worker model"),
    ("on path", ""),
];

fn violations(strings: &[&str]) -> Vec<String> {
    let mut bad = Vec::new();
    for s in strings {
        let lower = s.to_lowercase();
        for (term, explained_by) in JARGON {
            if lower.contains(term) && (explained_by.is_empty() || !s.contains(explained_by)) {
                bad.push(format!("{term:?} in {s:?}"));
            }
        }
    }
    bad
}

#[test]
fn text_uses_no_unexplained_project_terms() {
    assert_eq!(violations(ALL), Vec::<String>::new());
    assert_eq!(violations(text::remove::ALL), Vec::<String>::new());
    assert_eq!(violations(text::update::ALL), Vec::<String>::new());
}

#[test]
fn the_rule_catches_jargon() {
    assert_eq!(
        violations(&["Install the playbook", "Worker model: x"]).len(),
        1
    );
    assert_eq!(violations(&["The worker runs"]).len(), 1);
    assert_eq!(violations(&["MCP server"]).len(), 1);
}

#[test]
fn next_steps_and_update_label_are_listed() {
    for agent in [
        AgentId::ClaudeCode,
        AgentId::ClaudeDesktop,
        AgentId::Codex,
        AgentId::OpenCode,
        AgentId::Antigravity,
    ] {
        assert!(ALL.contains(&text::next_step(agent)));
    }
    assert_eq!(
        text::update_to("v1.2.3"),
        "Update to v1.2.3 (keeps your choices)"
    );
    assert!(text::LEGEND.contains("esc back"));
}

/// Every kind of change summary the text module builds, for every agent.
fn generated_summaries() -> Vec<String> {
    use super::text::changes::{self as say, ALL_WHAT};
    let file = std::path::Path::new("~/some/config.file");
    let mut all = Vec::new();
    for agent in [
        "Claude Code",
        "Claude Desktop",
        "Codex",
        "OpenCode",
        "Antigravity",
    ] {
        all.push(say::remove(agent, file));
        all.push(say::not_installed(agent));
        for what in ALL_WHAT {
            all.push(say::add(what, agent, file));
            all.push(say::update(what, agent, file));
            all.push(say::keep(what, agent, file));
            all.push(say::adopt(what, agent, true, true));
            all.push(say::adopt(what, agent, false, false));
        }
    }
    all
}

#[test]
fn generated_change_summaries_use_no_unexplained_project_terms() {
    let all = generated_summaries();
    let refs: Vec<&str> = all.iter().map(String::as_str).collect();
    assert_eq!(violations(&refs), Vec::<String>::new());
    assert!(all.iter().any(|s| s.starts_with("Let Codex use mmhmem")));
}

/// Plans and reviews every feature of every agent, then their removal, and
/// returns every summary and every reason an agent is not found.
fn planned_summaries() -> Vec<String> {
    use crate::setup::ui::fixtures::{fx, with_clis};
    use crate::setup::ui::flow::{HomeAction, Newest};
    let mut seen = Vec::new();
    let empty = fx().flow(Newest::Unknown).home();
    for row in empty.rows {
        if let crate::setup::ui::flow::AgentState::NotFound(reason) = row.state {
            seen.push(reason);
        }
    }
    let fx = with_clis();
    let mut flow = fx.flow(Newest::Unknown);
    flow.begin(&HomeAction::Install);
    for row in flow.feature_rows() {
        flow.toggle_feature(&row);
    }
    flow.make_plan();
    let summaries = |flow: &crate::setup::ui::flow::Flow| -> Vec<String> {
        flow.review()
            .items()
            .iter()
            .map(|i| i.summary.clone())
            .collect()
    };
    seen.extend(summaries(&flow));
    // The hook cannot start under the doubles, so install without it.
    for row in flow.feature_rows() {
        if row.feature == Some(crate::setup::agents::Feature::ReadHook) {
            flow.toggle_feature(&row);
        }
    }
    flow.make_plan();
    flow.confirm();
    flow.begin(&HomeAction::Change);
    for row in flow.feature_rows().iter().filter(|r| r.on) {
        flow.toggle_feature(row);
    }
    flow.make_plan();
    let removals = summaries(&flow);
    assert!(
        removals.iter().any(|s| s.starts_with("Remove mmhmem")),
        "{removals:?}"
    );
    seen.extend(removals);
    seen
}

#[test]
fn what_the_agents_plan_and_report_uses_no_unexplained_project_terms() {
    let seen = planned_summaries();
    assert!(seen.len() > 8, "{seen:?}");
    let refs: Vec<&str> = seen.iter().map(String::as_str).collect();
    assert_eq!(violations(&refs), Vec::<String>::new());
    for s in &seen {
        assert!(!s.contains("PATH"), "{s}");
    }
}
