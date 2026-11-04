//! The agents and features screens (task 6.2, design.md §7; Requirements
//! 4.4 and 4.5): one checkbox per detected agent, undetected agents listed
//! with their reason, a Claude Desktop profile choice when there are
//! several, and per agent the features with one-line explanations. Only the
//! required feature is on by default; the worker model is a toggle whose
//! setup comes in a later step.

use std::path::PathBuf;

use super::{Flow, text};
use crate::setup::agents::{AgentId, DISPLAY_ORDER, Detection, Feature};

/// One line of the agents screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Choice {
    /// An agent checkbox.
    Agent {
        /// The agent.
        agent: AgentId,
        /// The line: the agent's name, or "<name>: not found (<reason>)".
        label: String,
        /// Found on this computer (only then can it be selected).
        available: bool,
        /// Selected.
        checked: bool,
    },
    /// One Claude Desktop profile (shown when there are several).
    Profile {
        /// The profile's config folder.
        path: PathBuf,
        /// "Use the Claude Desktop profile in <path>".
        label: String,
        /// The profile in use.
        chosen: bool,
    },
}

/// One line of the features screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeatureRow {
    /// The agent (`None`: the install-wide worker model line).
    pub agent: Option<AgentId>,
    /// The feature (`None`: an agent heading or the worker model line).
    pub feature: Option<Feature>,
    /// The name.
    pub label: String,
    /// The one-line explanation (empty for a heading).
    pub help: String,
    /// Switched on.
    pub on: bool,
    /// Cannot be switched (the required feature, a heading).
    pub locked: bool,
}

impl FeatureRow {
    /// An agent heading line.
    pub fn is_heading(&self) -> bool {
        self.agent.is_some() && self.feature.is_none()
    }
}

impl Flow {
    /// The agents screen, in display order.
    pub fn agent_choices(&self) -> Vec<Choice> {
        let mut rows = Vec::new();
        for id in DISPLAY_ORDER {
            let checked = !self.selection.features(id).is_empty();
            let (available, label) = match self.detection(id) {
                Some(Detection::NotFound { reason }) => (
                    false,
                    text::agent_unavailable(id.label(), &self.tidy(reason)),
                ),
                _ => (true, id.label().to_owned()),
            };
            rows.push(Choice::Agent {
                agent: id,
                label,
                available,
                checked,
            });
            if id == AgentId::ClaudeDesktop && checked && self.loaded.profiles.len() > 1 {
                rows.extend(self.profile_choices());
            }
        }
        rows
    }

    fn profile_choices(&self) -> Vec<Choice> {
        self.loaded
            .profiles
            .iter()
            .map(|path| Choice::Profile {
                path: path.clone(),
                label: text::profile_choice(&self.tidy(&path.display().to_string())),
                chosen: self.profile.as_ref() == Some(path),
            })
            .collect()
    }

    /// Space on an agents-screen line: toggles an available agent (on with
    /// the required feature only) or picks a profile.
    pub fn toggle_choice(&mut self, choice: &Choice) {
        match choice {
            Choice::Agent {
                agent,
                available: true,
                checked,
                ..
            } => {
                if *checked {
                    for feature in self.selection.features(*agent) {
                        self.selection.deselect(*agent, feature);
                    }
                } else {
                    self.selection.select(*agent, Feature::Mcp);
                }
            }
            Choice::Profile { path, .. } => self.profile = Some(path.clone()),
            Choice::Agent { .. } => {}
        }
    }

    /// Whether at least one agent is selected.
    pub fn has_selection(&self) -> bool {
        !self.selection.agents().is_empty()
    }

    /// The features screen: per selected agent a heading and its features,
    /// then the worker model line.
    pub fn feature_rows(&self) -> Vec<FeatureRow> {
        let registry = self.registry();
        let mut rows = Vec::new();
        for id in self.selection.agents() {
            let Some(agent) = registry.get(id) else {
                continue;
            };
            rows.push(FeatureRow {
                agent: Some(id),
                feature: None,
                label: id.label().to_owned(),
                help: String::new(),
                on: true,
                locked: true,
            });
            for feature in agent.features() {
                let (label, help) = text::feature_label(*feature);
                rows.push(FeatureRow {
                    agent: Some(id),
                    feature: Some(*feature),
                    label: label.to_owned(),
                    help: help.to_owned(),
                    on: self.selection.wants(id, *feature),
                    locked: *feature == Feature::Mcp,
                });
            }
        }
        rows.push(self.worker_row());
        rows
    }

    fn worker_row(&self) -> FeatureRow {
        let help = if self.worker {
            text::WORKER_LATER
        } else {
            text::FEATURE_WORKER_HELP
        };
        FeatureRow {
            agent: None,
            feature: None,
            label: text::FEATURE_WORKER.to_owned(),
            help: help.to_owned(),
            on: self.worker,
            locked: false,
        }
    }

    /// Space on a features-screen line.
    pub fn toggle_feature(&mut self, row: &FeatureRow) {
        if row.locked {
            return;
        }
        match (row.agent, row.feature) {
            (Some(agent), Some(feature)) if row.on => self.selection.deselect(agent, feature),
            (Some(agent), Some(feature)) => self.selection.select(agent, feature),
            // The worker model is only noted here; its setup is a later step.
            (None, None) => self.worker = !self.worker,
            _ => {}
        }
    }
}
