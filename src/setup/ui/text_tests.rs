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
    assert_eq!(text::update_to("v1.2.3"), "Update to v1.2.3");
    assert!(text::LEGEND.contains("esc back"));
}
