//! The home screen model and its actions (task 6.2, design.md §7 "Home";
//! Requirements 4.2 and 4.3): the installed and newest version, one row per
//! agent, notes, and only the actions that apply.

use super::{Flow, Newest, Next, text};
use crate::setup::agents::{AgentId, AgentRecord, DISPLAY_ORDER, Detection, Feature, Selection};
use crate::setup::record::{CheckResult, ReleaseTag};

/// One action on the home screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HomeAction {
    /// Update to a newer release, then review the agents again.
    UpdateTo(ReleaseTag),
    /// Install, or bring the recorded choices up to date.
    Install,
    /// Change agents and features.
    Change,
    /// Run the functional check again.
    Check,
    /// Remove mmhmem (task 7.2, `flow_remove.rs`).
    Remove,
}

impl HomeAction {
    /// The menu line.
    pub fn label(&self) -> String {
        match self {
            Self::UpdateTo(tag) => text::update_to(tag.as_str()),
            Self::Install => text::ACTION_INSTALL.to_owned(),
            Self::Change => text::ACTION_CHANGE.to_owned(),
            Self::Check => text::ACTION_CHECK.to_owned(),
            Self::Remove => text::ACTION_REMOVE.to_owned(),
        }
    }
}

/// Whether mmhmem is set up in an agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentState {
    /// The record lists features for it.
    Installed,
    /// Found on this computer, not set up.
    NotSetUp,
    /// Not found, with the plain reason.
    NotFound(String),
}

/// One agent row of the home screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentRow {
    /// The agent.
    pub agent: AgentId,
    /// Installed, not set up or not found.
    pub state: AgentState,
    /// The features that are on, e.g. "memory, recap" ("–" for none).
    pub features: String,
    /// The last check, e.g. "✔ 2026-11-04 09:31" or "● approve the hook…".
    pub last_check: String,
}

/// Everything the home screen shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HomeView {
    /// "Installed: v0.4.1".
    pub installed: String,
    /// "Newest release: v0.5.0".
    pub newest: String,
    /// One row per agent, in display order.
    pub rows: Vec<AgentRow>,
    /// Recovery, drift, adoption and status lines.
    pub notes: Vec<String>,
    /// Only the actions that apply.
    pub actions: Vec<HomeAction>,
}

impl Flow {
    /// The home screen.
    pub fn home(&self) -> HomeView {
        let record = self.loaded.record.as_ref();
        let mut notes = self.loaded.notes.clone();
        if self.loaded.current {
            // "Everything is current" would contradict "Update to …" below.
            notes.push(if self.newer_release().is_some() {
                text::HOME_UPDATE_AVAILABLE.to_owned()
            } else {
                text::REVIEW_NOTHING.to_owned()
            });
        }
        if !self.is_installed() && !self.any_detected() {
            notes.push(text::NO_AGENT.to_owned());
        }
        HomeView {
            installed: text::installed_version(
                record.and_then(|r| r.release.as_ref()).map(|t| t.as_str()),
            ),
            newest: match &self.newest {
                Newest::Checking => text::NEWEST_CHECKING.to_owned(),
                Newest::Known(tag) => text::newest(tag.as_str()),
                Newest::Unknown => text::NEWEST_UNKNOWN.to_owned(),
            },
            rows: DISPLAY_ORDER.iter().map(|id| self.row(*id)).collect(),
            notes,
            actions: self.actions(),
        }
    }

    /// Starts `action`; only [`Flow::confirm`] and the check write.
    pub fn begin(&mut self, action: &HomeAction) -> Next {
        self.planned = None;
        self.delete_memories = false;
        self.relaunch = None;
        self.progress = None;
        self.intro = None;
        match action {
            HomeAction::UpdateTo(tag) => self.update_to(tag),
            HomeAction::Remove => self.plan_removal(),
            HomeAction::Check => {
                self.run_check();
                Next::Progress
            }
            HomeAction::Change => {
                self.selection = self.recorded_selection();
                Next::Agents
            }
            HomeAction::Install if self.is_installed() => {
                self.selection = self.recorded_selection();
                self.make_plan();
                Next::Review
            }
            HomeAction::Install => {
                self.selection = self.default_selection();
                self.profile = self.loaded.profiles.first().cloned();
                Next::Agents
            }
        }
    }

    /// Whether the record lists any agent with features on.
    pub fn is_installed(&self) -> bool {
        self.loaded
            .record
            .as_ref()
            .is_some_and(|r| r.agents.values().any(installed))
    }

    fn any_detected(&self) -> bool {
        self.loaded.detections.iter().any(|(_, d)| d.is_found())
    }

    fn actions(&self) -> Vec<HomeAction> {
        let installed = self.is_installed();
        let newer = self.newer_release().filter(|_| installed);
        let mut actions = Vec::new();
        if let Some(tag) = newer.clone() {
            actions.push(HomeAction::UpdateTo(tag));
        }
        let install = if installed {
            newer.is_none() && !self.loaded.current
        } else {
            self.any_detected()
        };
        if install {
            actions.push(HomeAction::Install);
        }
        if installed {
            actions.extend([HomeAction::Change, HomeAction::Check, HomeAction::Remove]);
        }
        actions
    }

    /// The newest release when it is newer than the installed one.
    fn newer_release(&self) -> Option<ReleaseTag> {
        let Newest::Known(newest) = &self.newest else {
            return None;
        };
        let installed = self
            .loaded
            .record
            .as_ref()
            .and_then(|r| r.release.clone())
            .unwrap_or_else(|| self.deps.version.clone());
        (version(newest) > version(&installed)).then(|| newest.clone())
    }

    fn row(&self, id: AgentId) -> AgentRow {
        let rec = self.loaded.record.as_ref().and_then(|r| r.agents.get(&id));
        let state = match (rec.filter(|r| installed(r)), self.detection(id)) {
            (Some(_), _) => AgentState::Installed,
            (None, Some(Detection::NotFound { reason })) => AgentState::NotFound(self.tidy(reason)),
            (None, _) => AgentState::NotSetUp,
        };
        let features: Vec<&str> = rec
            .map(|r| r.features.iter().filter(|(_, on)| **on).collect::<Vec<_>>())
            .unwrap_or_default()
            .into_iter()
            .map(|(f, _)| text::feature_short(*f))
            .collect();
        AgentRow {
            agent: id,
            state,
            features: if features.is_empty() {
                "–".to_owned()
            } else {
                features.join(", ")
            },
            last_check: rec.map_or_else(|| "–".to_owned(), last_check),
        }
    }

    /// The recorded choices (for updates and changes): the features that
    /// are on, or, for an agent an earlier run skipped, what was chosen then.
    pub(super) fn recorded_selection(&self) -> Selection {
        let Some(record) = self.loaded.record.as_ref() else {
            return Selection::new();
        };
        let mut selection = Selection::from_record(record);
        for (id, rec) in record.agents.iter().filter(|(_, r)| !r.skipped.is_empty()) {
            for feature in selection.features(*id) {
                selection.deselect(*id, feature);
            }
            for feature in &rec.skipped {
                selection.select(*id, *feature);
            }
        }
        selection
    }

    /// Every detected agent with its default features, plus what an earlier
    /// install had switched on (so adopting keeps it).
    fn default_selection(&self) -> Selection {
        let registry = self.registry();
        let mut selection = Selection::new();
        for agent in registry.agents() {
            let id = agent.id();
            if !self.detection(id).is_some_and(Detection::is_found) {
                continue;
            }
            selection.select(id, Feature::Mcp);
            if agent.features().contains(&Feature::ReadHook) {
                selection.select(id, Feature::ReadHook);
            }
            let found = self.loaded.scan.as_ref().and_then(|s| s.agent(id));
            for feature in found.map(|s| s.features()).unwrap_or_default() {
                if agent.features().contains(&feature) {
                    selection.select(id, feature);
                }
            }
        }
        selection
    }
}

fn installed(rec: &AgentRecord) -> bool {
    rec.features.values().any(|on| *on)
}

/// "✔ 2026-11-04 09:31", "✘ …" or "● <first pending step>".
fn last_check(rec: &AgentRecord) -> String {
    let Some(check) = &rec.last_check else {
        return "–".to_owned();
    };
    let at = check.at.as_str();
    let when = format!("{} {}", &at[..10], &at[11..16]);
    match check.result {
        CheckResult::Pass => format!("✔ {when}"),
        CheckResult::Fail => format!("✘ {when}"),
        CheckResult::UserStepsPending => rec.pending_user_steps.first().map_or_else(
            || format!("● {when}"),
            |s| format!("● {}", text::pending_short(*s)),
        ),
    }
}

/// `(major, minor, patch)` of a `vX.Y.Z` tag (anything else sorts first).
fn version(tag: &ReleaseTag) -> (u64, u64, u64) {
    let mut parts = tag
        .as_str()
        .trim_start_matches('v')
        .split('.')
        .map(|p| p.parse::<u64>().unwrap_or(0));
    let mut next = || parts.next().unwrap_or(0);
    (next(), next(), next())
}
