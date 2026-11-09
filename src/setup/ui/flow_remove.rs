//! "Remove mmhmem" (task 7.2, design.md §8 "Remove"; Requirements 7.3,
//! 7.4, 4.6 and 4.7).
//!
//! **The review lists every change, in apply order:**
//!
//! 1. per recorded agent, its own `plan_remove`: only the entries the
//!    record lists, taken out of shared files with the same parsers that
//!    added them, so every other setting (also ones the user changed after
//!    the install) stays byte-identical;
//! 2. the program's folder out of the user's `PATH`, only when this
//!    install added it (`path_entry.added`), with every other byte of the
//!    value kept ([`PathValue::without_folder`], exact with the recorded
//!    `trailing_separator`); a value mmhmem created (`created_value`) is
//!    deleted when nothing else is left in it;
//! 3. the program at its permanent place (deleting the running program on
//!    Windows is deferred until setup exits, see `plan_apply.rs`), then the
//!    folders the install created for it (the entry's `missing_parents`),
//!    only while they are empty;
//! 4. the install record itself (not a planned change: deleted after
//!    everything above succeeded);
//! 5. only when "Also delete my saved memories" is ticked (off by default):
//!    the memory store folder and, when it lives elsewhere, the activity
//!    (audit) log with its rotated copies. Deleted last, and only after
//!    every step above succeeded; otherwise the store is never touched
//!    (7.4). Without the option the audit log is kept too.
//!
//! **A program still in use** (another process, such as an AI tool's
//! `mmhmem stdio`, runs it; Windows only, see `setup::running`): the tools
//! are cleaned up, but the program, its `PATH` entry and the record stay and
//! the results say why and what to do (close the tools, remove again), with
//! exit code 1. Deleting it anyway would rely on a delete deferred to the
//! end of setup that fails silently while the program runs elsewhere.
//!
//! **A tool that cannot be cleaned up** (a file it uses no longer parses)
//! is listed as skipped and left as it is. The other tools are cleaned up,
//! but the program, its `PATH` entry, the record and the memories stay, so
//! the tool keeps working and the user can fix the file and remove again.
//!
//! Backups of every changed file stay in the record folder's `backups/`;
//! the results say where. Cancelling at the review changes nothing. There
//! is no removal without a terminal (the no-terminal run only installs).
//!
//! [`PathValue::without_folder`]: crate::setup::user_path::PathValue::without_folder

use std::io;
use std::path::{Path, PathBuf};

use super::review::Planned;
use super::{
    Confirmed, FileGroup, Flow, MemoriesChoice, Next, Progress, ReviewItem, ReviewView, text,
};
use crate::setup::agents::AgentId;
use crate::setup::plan::{Action, ApplyReport, Change, Outcome, Plan, apply_with};
use crate::setup::record::{Drift, InstallRecord, Probe, Scope, Target, detect_drift};
use text::remove as say;

/// The memory store a removal may delete: the data folder and the activity
/// log (`audit.jsonl`, inside the folder unless configured elsewhere).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Memories {
    /// The store folder (the database and everything mmhmem keeps).
    pub dir: PathBuf,
    /// The audit log; its rotated copies are `<file>.1`, `<file>.2`, ….
    pub log_file: PathBuf,
}

/// The most rotated audit copies looked for.
const MAX_ROTATED: usize = 1000;

impl Memories {
    /// The store of the local profile (`mmhmem stdio`, what the agents run).
    pub fn from_config(config: &crate::config::Config) -> Self {
        Self {
            dir: config.data_dir.clone(),
            log_file: config.log_file.clone(),
        }
    }

    /// Deletes the folder, then a log kept outside it with its copies.
    fn delete(&self) -> Result<(), String> {
        gone(std::fs::remove_dir_all(&self.dir))?;
        if self.log_file.starts_with(&self.dir) {
            return Ok(());
        }
        gone(std::fs::remove_file(&self.log_file))?;
        for n in 1..=MAX_ROTATED {
            let copy = crate::storage::audit_rotate::rotated_path(&self.log_file, n);
            if !copy.exists() {
                break;
            }
            gone(std::fs::remove_file(&copy))?;
        }
        Ok(())
    }
}

/// `Ok` when the delete worked or there was nothing to delete.
fn gone(result: io::Result<()>) -> Result<(), String> {
    match result {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e.kind().to_string()),
        _ => Ok(()),
    }
}

impl Flow {
    /// Plans the removal (see the module docs). Reads only.
    pub(super) fn plan_removal(&mut self) -> Next {
        let Some(record) = self.loaded.record.clone() else {
            return Next::Note(text::CANCELLED);
        };
        let unreadable = self.unreadable_agents(&record);
        let mut plan = Plan::new((self.deps.now)());
        let mut problems = Vec::new();
        for agent in self.registry().agents() {
            let id = agent.id();
            let Some(rec) = record.agents.get(&id) else {
                continue;
            };
            match unreadable.iter().find(|(a, _)| *a == id) {
                Some((_, line)) => problems.push((id, line.clone())),
                None => plan.changes.extend(agent.plan_remove(&self.deps.env, rec)),
            }
        }
        if problems.is_empty() {
            plan.changes.extend(self.path_removal(&record));
            plan.changes.extend(self.program_removal(&record));
        }
        self.planned = Some(Planned {
            plan,
            problems,
            left_out: Vec::new(),
            adoption: false,
            removal: true,
        });
        Next::Review
    }

    /// Switches "Also delete my saved memories" (a removal's review only).
    pub fn toggle_memories(&mut self) {
        if self.planned.as_ref().is_some_and(|p| p.removal) && self.deps.memories.is_some() {
            self.delete_memories = !self.delete_memories;
        }
    }

    /// One line per recorded agent with a managed file that cannot be read
    /// or parsed (it is skipped).
    fn unreadable_agents(&self, record: &InstallRecord) -> Vec<(AgentId, String)> {
        let dirs = super::install::user_path_dirs(&self.deps);
        let probe = Probe {
            user_path: Some(&dirs),
        };
        let mut found: Vec<(AgentId, String)> = Vec::new();
        for report in detect_drift(record, &probe) {
            let (Scope::Agent(id), Drift::Unreadable { reason }) = (report.scope, report.drift)
            else {
                continue;
            };
            if found.iter().all(|(a, _)| *a != id) {
                let file = super::shown_file(self.deps.env.home(), &report.target);
                found.push((id, say::file_unreadable(&file, &self.tidy(&reason))));
            }
        }
        found
    }

    /// The program's folder out of the user's `PATH`, when this install
    /// put it there and it is still there.
    fn path_removal(&self, record: &InstallRecord) -> Option<Change> {
        let entry = record.path_entry.as_ref().filter(|e| e.added)?;
        let paths = &self.deps.paths;
        if !paths.supported() {
            return None;
        }
        let current = paths.read().ok()??;
        if !current.contains(&entry.value) {
            return None;
        }
        let after = current.without_folder(&entry.value, entry.trailing_separator);
        // A value mmhmem created goes again when nothing else is in it.
        let after = (!(entry.created_value && after.is_blank())).then(|| after.encode());
        let shown = self.tidy(&entry.value.display().to_string());
        Some(Change {
            scope: Scope::Install,
            target: Target::PathEntry {
                value: entry.value.clone(),
            },
            file: entry.value.clone(),
            action: Action::Remove,
            before: Some(current.encode()),
            after,
            owner_only: false,
            hide_diff: false,
            missing_parents: None,
            summary: say::change_path(&shown),
        })
    }

    /// The program at its permanent place, when it is there.
    fn program_removal(&self, record: &InstallRecord) -> Option<Change> {
        let path = record
            .binary
            .as_ref()
            .map_or_else(|| self.deps.binary.clone(), |b| b.path.clone());
        let before = std::fs::read(&path).ok()?;
        let running = std::env::current_exe()
            .ok()
            .is_some_and(|exe| crate::setup::ui::same_file(&exe, &path));
        let folders = self.recorded_folders(&path);
        Some(Change {
            scope: Scope::Install,
            target: Target::Binary { path: path.clone() },
            file: path,
            action: Action::Remove,
            before: Some(before),
            after: None,
            owner_only: false,
            hide_diff: false,
            // Empty folders the install created are deleted after it.
            missing_parents: Some(folders),
            summary: if running {
                say::CHANGE_PROGRAM_RUNNING
            } else {
                say::CHANGE_PROGRAM
            }
            .to_owned(),
        })
    }

    /// The review of a removal: every change, then the record and (when
    /// ticked) the memories, and the option itself.
    pub(super) fn removal_review(&self, planned: &Planned) -> ReviewView {
        let mut groups = self.groups(&planned.plan);
        let complete = planned.problems.is_empty();
        let next = planned.plan.changes.len();
        if complete {
            let record = self.deps.store.record_path();
            groups.push(extra_group(self.shown(&record), next, say::RECORD_FILE));
        }
        let memories = self.deps.memories.as_ref().filter(|_| complete);
        if let Some(m) = memories.filter(|_| self.delete_memories) {
            groups.push(extra_group(
                self.shown(&m.dir),
                next + 1,
                say::MEMORIES_FILE,
            ));
        }
        ReviewView {
            intro: vec![say::INTRO.to_owned()],
            groups,
            problems: self.problem_lines(&planned.problems),
            notes: vec![text::REVIEW_BACKUP.to_owned()],
            memories: memories.map(|m| MemoriesChoice {
                on: self.delete_memories,
                label: say::MEMORIES_OPTION,
                help: say::memories_help(&self.shown(&m.dir)),
            }),
        }
    }

    fn shown(&self, path: &Path) -> String {
        self.tidy(&path.display().to_string())
    }

    /// Applies a confirmed removal: the agents first; the program, `PATH`,
    /// the record and the memories only when every agent was cleaned up.
    pub(super) fn confirm_removal(&mut self, planned: &Planned) -> Confirmed {
        let mut record = self.loaded.record.clone().unwrap_or_default();
        let paths = self.deps.paths.as_ref();
        let (agents, install) = split(&planned.plan);
        let applied = apply_with(&agents, &self.deps.store, &mut record, paths);
        let mut lines = self.problem_lines(&planned.problems);
        lines.extend(self.report_lines(&applied));
        let code = if !planned.problems.is_empty() || !all_applied(&applied) {
            self.keep_install(&mut record, &applied, &mut lines, say::KEPT_FOR_SKIPPED)
        } else if self.program_in_use(&install) {
            self.keep_install(&mut record, &applied, &mut lines, say::KEPT_IN_USE)
        } else {
            let done = apply_with(&install, &self.deps.store, &mut record, paths);
            lines.extend(self.report_lines(&done));
            if all_applied(&done) {
                self.finish_removal(&planned.plan, &mut lines)
            } else {
                1
            }
        };
        self.loaded.record = self.deps.store.load().ok().flatten();
        self.progress = Some(Progress {
            lines,
            next_steps: Vec::new(),
            code,
        });
        Confirmed::Done
    }

    /// After a partial removal: the cleaned-up agents leave the record, the
    /// program, `PATH` and the record stay; `why` says so. Exit code 1.
    fn keep_install(
        &self,
        record: &mut InstallRecord,
        applied: &ApplyReport,
        lines: &mut Vec<String>,
        why: &str,
    ) -> i32 {
        drop_cleaned_agents(record, applied);
        if let Err(e) = self.deps.store.save(record) {
            lines.push(self.tidy(&e.to_string()));
        }
        lines.push(why.to_owned());
        1
    }

    /// Whether the program this removal deletes is still run by another
    /// process. Windows cannot delete it then, and the delete deferred to
    /// the end of setup would fail without a word, so it is kept instead.
    fn program_in_use(&self, install: &Plan) -> bool {
        install.changes.iter().any(|c| {
            matches!(c.target, Target::Binary { .. })
                && c.after.is_none()
                && (self.deps.in_use)(&c.file)
        })
    }

    fn report_lines(&self, applied: &ApplyReport) -> Vec<String> {
        applied
            .agents
            .iter()
            .filter_map(|r| self.outcome_line(r.scope, &r.outcome, r.files().len()))
            .collect()
    }

    /// Deletes the record, then (when ticked) the memories; the result lines.
    fn finish_removal(&mut self, plan: &Plan, lines: &mut Vec<String>) -> i32 {
        if let Err(reason) = gone(std::fs::remove_file(self.deps.store.record_path())) {
            lines.push(say::record_failed(&reason));
            return 1;
        }
        lines.push(say::RECORD_DELETED.to_owned());
        let backups = self.deps.store.backup_dir(&plan.run);
        if backups.is_dir() {
            lines.push(say::backups_kept(&self.shown(&backups)));
        }
        let mut code = 0;
        if let Some(m) = &self.deps.memories {
            let dir = self.shown(&m.dir);
            let deleted = self.delete_memories.then(|| m.delete());
            let line = match deleted {
                None => say::memories_kept(&dir),
                Some(Ok(())) => say::MEMORIES_DELETED.to_owned(),
                Some(Err(reason)) => {
                    code = 1;
                    say::memories_failed(&dir, &reason)
                }
            };
            lines.push(line);
        }
        if plan
            .changes
            .iter()
            .any(|c| c.summary == say::CHANGE_PROGRAM_RUNNING)
        {
            lines.push(say::PROGRAM_LATER.to_owned());
        }
        lines.push(String::new());
        lines.push(say::REMOVED.to_owned());
        code
    }
}

/// A review group for a step that is not a planned change (no details).
fn extra_group(file: String, index: usize, summary: &str) -> FileGroup {
    FileGroup {
        file,
        items: vec![ReviewItem {
            index,
            mark: '-',
            summary: summary.to_owned(),
        }],
    }
}

/// The agents' changes and the install-wide changes, as two plans of the
/// same run (one backup folder).
fn split(plan: &Plan) -> (Plan, Plan) {
    let (install, agents): (Vec<Change>, Vec<Change>) = plan
        .changes
        .iter()
        .cloned()
        .partition(|c| c.scope == Scope::Install);
    let of = |changes| Plan {
        run: plan.run.clone(),
        changes,
    };
    (of(agents), of(install))
}

fn all_applied(report: &ApplyReport) -> bool {
    report.agents.iter().all(|a| a.outcome == Outcome::Applied)
}

/// After a partial removal: agents whose removal was applied leave the
/// record, the skipped ones stay.
fn drop_cleaned_agents(record: &mut InstallRecord, applied: &ApplyReport) {
    for report in &applied.agents {
        if let (Scope::Agent(id), Outcome::Applied) = (report.scope, &report.outcome) {
            record.agents.remove(&id);
        }
    }
}

#[cfg(test)]
#[path = "flow_remove_tests.rs"]
mod tests;
