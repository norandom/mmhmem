//! Confirm: apply, check, store and report (task 6.2, design.md §4, §6 and
//! §7 "Progress"; Requirements 1.5, 3.3, 3.5, 5.1, 5.5 and 6.5–6.7).
//!
//! [`Flow::confirm`] applies the reviewed plan unchanged
//! ([`crate::setup::plan::apply`]), runs [`Checker::run`] for the selected
//! agents that were not skipped at planning (which rolls back an agent whose
//! check fails and stores each `last_check`), records the release and
//! binary, drops agents that are no
//! longer selected and have nothing left, saves the record and builds the
//! progress lines: one line per agent's changes, where the backups are, the
//! check results with pending user steps, and the next step per agent.

use std::collections::BTreeSet;
use std::time::Duration;

use super::review::Planned;
use super::{Flow, text};
use crate::setup::agents::{AgentId, Feature, Selection};
use crate::setup::plan::{ApplyReport, Outcome, apply_with};
use crate::setup::record::{AgentRecord, CheckResult, InstallRecord, Scope};
use crate::setup::verify::{Checker, VerifyReport};

/// What a confirm did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Confirmed {
    /// The earlier install was taken over; the choices are planned now and
    /// wait for their own review.
    Review,
    /// Done; the results are in [`Flow::progress`].
    Done,
}

/// The results screen.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Progress {
    /// What was changed, skipped or restored, then the check results.
    pub lines: Vec<String>,
    /// What to do next in each agent that works.
    pub next_steps: Vec<String>,
    /// The exit code: 0 when every agent passed or only has user steps
    /// left, 1 otherwise.
    pub code: i32,
}

impl Progress {
    /// Every line, with the next steps under [`text::DONE`].
    pub fn all_lines(&self) -> Vec<String> {
        let mut lines = self.lines.clone();
        if !self.next_steps.is_empty() {
            lines.push(String::new());
            lines.push(text::DONE.to_owned());
            lines.extend(self.next_steps.iter().cloned());
        }
        lines
    }
}

impl Flow {
    /// Applies the reviewed plan (the only call besides the check that
    /// writes). Without a plan it does nothing.
    pub fn confirm(&mut self) -> Confirmed {
        let Some(planned) = self.planned.take() else {
            return Confirmed::Done;
        };
        if planned.removal {
            return self.confirm_removal(&planned);
        }
        let mut record = self.loaded.record.clone().unwrap_or_default();
        let paths = self.deps.paths.as_ref();
        let mut applied = apply_with(&planned.plan, &self.deps.store, &mut record, paths);
        if planned.adoption {
            if let Err(e) = self.deps.store.save(&record) {
                self.loaded.notes.push(self.tidy(&e.to_string()));
            }
            self.loaded.record = Some(record);
            self.loaded.scan = None;
            self.make_plan();
            return Confirmed::Review;
        }
        let registry = self.registry();
        let checker = Checker {
            registry: &registry,
            env: &self.deps.env,
            store: &self.deps.store,
        };
        // An agent skipped at planning changed nothing: it is not checked.
        let mut checked = self.selection.clone();
        for (id, _) in &planned.problems {
            for feature in self.selection.features(*id) {
                checked.deselect(*id, feature);
            }
        }
        let verify = checker.run(&checked, &mut applied, &mut record);
        self.finish_record(&mut record, &planned, &applied);
        let mut progress = self.progress_of(&applied, &verify, &planned.problems);
        if !record.agents.is_empty() {
            progress.next_steps.extend(self.path_next_step());
        }
        // The guidance the features screen promises (WORKER_LATER): only
        // when the worker line was ticked and none is set up yet.
        if self.worker && !self.deps.env.worker_configured() {
            progress.next_steps.extend(self.worker_guide());
        }
        if let Err(e) = self.deps.store.save(&record) {
            progress.lines.push(self.tidy(&e.to_string()));
            progress.code = 1;
        }
        if planned.plan.changes.iter().any(|c| c.before.is_some()) {
            let dir = self.deps.store.backup_dir(&planned.plan.run);
            if dir.is_dir() {
                let dir = self.tidy(&dir.display().to_string());
                progress.lines.insert(0, text::backups_in(&dir));
            }
        }
        self.loaded.record = Some(record);
        self.progress = Some(progress);
        Confirmed::Done
    }

    /// Checks every recorded agent again and stores the results.
    pub(super) fn run_check(&mut self) {
        let Some(mut record) = self.loaded.record.clone() else {
            return;
        };
        self.selection = Selection::from_record(&record);
        let registry = self.registry();
        let checker = Checker {
            registry: &registry,
            env: &self.deps.env,
            store: &self.deps.store,
        };
        let mut applied = ApplyReport { agents: Vec::new() };
        let verify = checker.run(&self.selection, &mut applied, &mut record);
        let worker_ready = self.deps.env.worker_configured();
        if record.worker_configured != worker_ready {
            record.worker_configured = worker_ready;
            if let Err(e) = self.deps.store.save(&record) {
                self.loaded.notes.push(self.tidy(&e.to_string()));
            }
        }
        self.progress = Some(self.progress_of(&applied, &verify, &[]));
        self.loaded.record = Some(record);
    }

    /// The release, the program and the `PATH` entry of this install;
    /// agents that are no longer selected and manage nothing leave the
    /// record. An agent skipped at planning keeps its entry as it was (no
    /// feature is claimed for it) and only remembers what was chosen, so the
    /// next run plans it again ([`Flow::recorded_selection`]); choosing no
    /// feature at all is remembered too. An agent whose changes were undone
    /// (its check failed) keeps the choice it remembered before this run.
    fn finish_record(&self, record: &mut InstallRecord, planned: &Planned, applied: &ApplyReport) {
        let skipped: Vec<AgentId> = planned.problems.iter().map(|(id, _)| *id).collect();
        let undone = |id: AgentId| {
            applied
                .agent(Scope::Agent(id))
                .is_some_and(|a| a.outcome != Outcome::Applied)
        };
        for (id, entry) in &mut record.agents {
            if !skipped.contains(id) && !undone(*id) {
                entry.skipped = None;
            }
        }
        for id in &skipped {
            let chosen = self.selection.features(*id);
            if let Some(entry) = record.agents.get_mut(id) {
                let on: BTreeSet<Feature> = entry
                    .features
                    .iter()
                    .filter_map(|(f, on)| on.then_some(*f))
                    .collect();
                entry.skipped = (chosen != on).then_some(chosen);
            } else if !chosen.is_empty() {
                let entry = AgentRecord {
                    skipped: Some(chosen),
                    ..AgentRecord::default()
                };
                record.agents.insert(*id, entry);
            }
        }
        let selected = self.selection.agents();
        record
            .agents
            .retain(|id, a| selected.contains(id) || !a.managed.is_empty());
        if record.agents.is_empty() {
            return;
        }
        record.worker_configured = self.deps.env.worker_configured();
        record.release = Some(self.deps.version.clone());
        self.finish_install(record, &planned.plan);
    }

    /// The promised "how to connect one" step (see [`text::WORKER_LATER`]):
    /// what to put where, plus the models a local endpoint offers.
    fn worker_guide(&self) -> Vec<String> {
        let url = crate::config::Config::load_local()
            .ok()
            .map(|c| c.worker_api_url)
            .unwrap_or_else(|| crate::config::DEFAULT_WORKER_API_URL.to_owned());
        let found = (self.deps.models)(&url, Duration::from_millis(1500));
        let file = self
            .deps
            .env
            .home()
            .join(crate::config::local_dirs::CONFIG_FILE_NAME);
        text::worker_guide(&file.display().to_string(), found.as_deref())
    }

    fn progress_of(
        &self,
        applied: &ApplyReport,
        verify: &VerifyReport,
        problems: &[(AgentId, String)],
    ) -> Progress {
        let mut lines = self.problem_lines(problems);
        let mut failed = !problems.is_empty();
        for report in &applied.agents {
            let checked = match report.scope {
                Scope::Agent(id) => verify.agent(id).is_some(),
                Scope::Install => false,
            };
            let line = self.outcome_line(report.scope, &report.outcome, report.files().len());
            failed |= !checked && report.outcome != Outcome::Applied;
            lines.extend(line.filter(|_| !checked || report.outcome == Outcome::Applied));
        }
        if !verify.agents.is_empty() {
            lines.push(String::new());
            lines.push(text::CHECKING.to_owned());
            lines.extend(verify.lines().iter().map(|l| self.tidy(l)));
        }
        lines.extend(verify.record_errors.iter().map(|e| self.tidy(e)));
        let mut next_steps: Vec<String> = Vec::new();
        for v in verify
            .agents
            .iter()
            .filter(|v| v.report.result != CheckResult::Fail)
        {
            next_steps.push(text::next_step(v.agent()).to_owned());
        }
        let ok = !failed && verify.all_passed() && verify.record_errors.is_empty();
        Progress {
            lines,
            next_steps,
            code: i32::from(!ok),
        }
    }

    pub(super) fn outcome_line(
        &self,
        scope: Scope,
        outcome: &Outcome,
        files: usize,
    ) -> Option<String> {
        let label = match scope {
            Scope::Agent(id) => id.label(),
            Scope::Install => "mmhmem",
        };
        match outcome {
            Outcome::Applied if files == 0 => None,
            Outcome::Applied => Some(text::changes_made(label, files)),
            Outcome::Skipped(skip) => {
                Some(text::agent_skipped(label, &self.tidy(&skip.to_string())))
            }
            Outcome::RolledBack { reason } => Some(self.tidy(reason)),
            Outcome::RollbackIncomplete { reason, problems } => Some(format!(
                "{} {}",
                self.tidy(reason),
                self.tidy(&problems.join("; "))
            )),
        }
    }
}
