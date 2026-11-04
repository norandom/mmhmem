//! The plan and the review screen (task 6.2, design.md §4 and §7 "Review";
//! Requirements 4.6, 5.4, 5.6 and 8.5).
//!
//! Every agent that is selected or recorded is planned with its own
//! `plan()`; an agent that cannot be planned is listed with its problem and
//! the others go on. The review lists every change grouped by file with its
//! summary, and [`Flow::diff`] shows one change's diff (hidden diffs stay
//! hidden: [`Change::diff`] decides).
//!
//! **Adoption first.** With no usable record and an earlier install found,
//! the first plan only takes over what that install made
//! ([`adoption_plan`]); after the user confirms it, the choices are planned
//! on top of the new record and reviewed again. One plan never touches the
//! same entry twice.

use std::path::PathBuf;

use super::{Flow, text};
use crate::setup::adopt::adoption_plan;
use crate::setup::agents::{AgentId, Selection};
use crate::setup::plan::{Action, Change, Plan};

/// A plan and the agents that could not be planned.
pub(super) struct Planned {
    pub(super) plan: Plan,
    pub(super) problems: Vec<(AgentId, String)>,
    pub(super) adoption: bool,
}

impl Planned {
    pub(super) fn is_empty(&self) -> bool {
        self.plan.changes.is_empty() && self.problems.is_empty()
    }
}

/// One change on the review screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewItem {
    /// The change's position in the plan (for [`Flow::diff`]).
    pub index: usize,
    /// `+` add, `~` change, `-` remove.
    pub mark: char,
    /// The plain-language summary.
    pub summary: String,
}

/// The changes to one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileGroup {
    /// The file, with the home folder shown as `~`.
    pub file: String,
    /// Its changes, in plan order.
    pub items: Vec<ReviewItem>,
}

/// Everything the review screen shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewView {
    /// Lines above the changes (the adoption step, drift).
    pub intro: Vec<String>,
    /// The changes, grouped by file in the order files first appear.
    pub groups: Vec<FileGroup>,
    /// One line per agent that will be skipped.
    pub problems: Vec<String>,
    /// Lines below the changes (backups).
    pub notes: Vec<String>,
}

impl ReviewView {
    /// Nothing will change.
    pub fn is_empty(&self) -> bool {
        self.groups.is_empty()
    }

    /// Every item, in screen order.
    pub fn items(&self) -> Vec<&ReviewItem> {
        self.groups.iter().flat_map(|g| &g.items).collect()
    }
}

impl Flow {
    /// Plans the current choices (or the adoption step first; see the
    /// module docs). Reads only.
    pub fn make_plan(&mut self) {
        let scan = self
            .loaded
            .scan
            .as_ref()
            .filter(|s| self.loaded.record.is_none() && s.agents.iter().any(|a| a.can_adopt()));
        let planned = match scan {
            Some(scan) => Planned {
                plan: adoption_plan(scan, None, (self.deps.now)()),
                problems: Vec::new(),
                adoption: true,
            },
            None => self.plan_for(&self.selection),
        };
        self.planned = Some(planned);
    }

    /// The review screen of the last [`Flow::make_plan`].
    pub fn review(&self) -> ReviewView {
        let Some(planned) = &self.planned else {
            return ReviewView {
                intro: Vec::new(),
                groups: Vec::new(),
                problems: Vec::new(),
                notes: Vec::new(),
            };
        };
        let mut intro = Vec::new();
        if planned.adoption {
            intro.push(text::ADOPT_REVIEW.to_owned());
        }
        let notes = if planned.plan.changes.is_empty() {
            vec![text::REVIEW_NOTHING.to_owned()]
        } else {
            vec![text::REVIEW_BACKUP.to_owned()]
        };
        ReviewView {
            intro,
            groups: self.groups(&planned.plan),
            problems: self.problem_lines(&planned.problems),
            notes,
        }
    }

    /// The diff of the change at `index` of the plan ("" when there is none).
    pub fn diff(&self, index: usize) -> String {
        self.planned
            .as_ref()
            .and_then(|p| p.plan.changes.get(index))
            .map(|c| self.tidy(&c.diff()))
            .unwrap_or_default()
    }

    /// The file of the change at `index`, for the diff title.
    pub fn diff_file(&self, index: usize) -> String {
        self.planned
            .as_ref()
            .and_then(|p| p.plan.changes.get(index))
            .map(|c| self.tidy(&c.file.display().to_string()))
            .unwrap_or_default()
    }

    /// Whether the recorded choices plan no change (the "everything is
    /// current" home screen).
    pub(super) fn is_current(&self) -> bool {
        self.is_installed() && self.plan_for(&self.recorded_selection()).is_empty()
    }

    /// Plans `want` for every agent that is selected or recorded.
    pub(super) fn plan_for(&self, want: &Selection) -> Planned {
        let registry = self.registry();
        let record = self.loaded.record.as_ref();
        let mut plan = Plan::new((self.deps.now)());
        let mut problems = Vec::new();
        for agent in registry.agents() {
            let id = agent.id();
            let rec = record.and_then(|r| r.agents.get(&id));
            if want.features(id).is_empty() && rec.is_none() {
                continue;
            }
            match agent.plan(&self.deps.env, want, rec) {
                Ok(changes) => plan.changes.extend(changes),
                Err(problem) => problems.push((id, problem.to_string())),
            }
        }
        Planned {
            plan,
            problems,
            adoption: false,
        }
    }

    pub(super) fn problem_lines(&self, problems: &[(AgentId, String)]) -> Vec<String> {
        problems
            .iter()
            .map(|(id, p)| text::agent_skipped(id.label(), &self.tidy(p)))
            .collect()
    }

    fn groups(&self, plan: &Plan) -> Vec<FileGroup> {
        let mut files: Vec<PathBuf> = Vec::new();
        let mut groups: Vec<FileGroup> = Vec::new();
        for (index, change) in plan.changes.iter().enumerate() {
            let at = files.iter().position(|f| f == &change.file);
            let group = match at {
                Some(i) => &mut groups[i],
                None => {
                    files.push(change.file.clone());
                    groups.push(FileGroup {
                        file: self.tidy(&change.file.display().to_string()),
                        items: Vec::new(),
                    });
                    groups.last_mut().expect("just pushed")
                }
            };
            group.items.push(item(index, change, self));
        }
        groups
    }
}

fn item(index: usize, change: &Change, flow: &Flow) -> ReviewItem {
    ReviewItem {
        index,
        mark: match change.action {
            Action::Add => '+',
            Action::Update => '~',
            Action::Remove => '-',
        },
        summary: flow.tidy(&change.summary),
    }
}
