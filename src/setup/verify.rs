//! After `plan::apply`: prove each agent works, store the result, and roll
//! back an agent whose check failed (reliable-installers task 4.2,
//! `.kiro/specs/reliable-installers/design.md` §4 "Rollback" and §6;
//! Requirements 3.5, 5.5 and 6.1–6.6).
//!
//! [`Checker::run`] checks every agent of the selection whose part of the
//! plan was applied (or had nothing to change), in display order. Agents
//! whose apply was skipped or already rolled back are not checked: nothing
//! of theirs changed.
//!
//! Per agent, the check runs on the record entry as it will be stored: the
//! selected features and the agent's pending user steps for this selection.
//! Then:
//!
//! - **Passed, or passed with user steps pending:** the entry is stored with
//!   `last_check` (time, result, per-step details), features and pending
//!   steps, and the record is saved. Pending steps never roll anything back.
//! - **Failed after this run changed files:** [`rollback_agent`] restores
//!   every file of that agent from this run's backups, newest first, with
//!   the reason "Restored your original <agent> settings."; the agent's
//!   record entry is put back exactly as it was before the run (a new agent
//!   leaves no entry), because what is installed again is the old setup
//!   this check did not test. The lines say so.
//! - **Failed with no file changed in this run** (a re-check): the entry is
//!   stored with the failed `last_check`.

use crate::setup::agents::{Agent, AgentId, CheckReport, Env, Registry, Selection};
use crate::setup::check::agent::restored_line;
use crate::setup::plan::{ApplyReport, Outcome, RollbackError, rollback_agent};
use crate::setup::record::{CheckResult, InstallRecord, RecordStore, Scope, UtcTimestamp};

/// What happened to an agent's files after its check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rollback {
    /// The check did not fail; nothing was restored.
    NotNeeded,
    /// The check failed, but this run changed no file of the agent.
    NothingToRestore,
    /// Every file of the agent was restored and its record entry put back.
    Restored,
    /// Some files could not be restored (one line per file, no contents).
    Incomplete {
        /// The files that could not be restored.
        problems: Vec<String>,
    },
}

/// One agent's check and what followed from it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentVerdict {
    /// The check report.
    pub report: CheckReport,
    /// Whether the agent's files were restored.
    pub rollback: Rollback,
}

impl AgentVerdict {
    /// The agent.
    pub fn agent(&self) -> AgentId {
        self.report.agent
    }

    /// The lines to show: the result line, the pending-step lines, then
    /// whether the original settings were restored.
    pub fn lines(&self) -> Vec<String> {
        let mut lines = self.report.lines();
        let label = self.agent().label();
        match &self.rollback {
            Rollback::Restored => lines.push(restored_line(self.agent())),
            Rollback::Incomplete { problems } => lines.push(format!(
                "Some of your original {label} settings could not be restored: {}.",
                problems.join("; ")
            )),
            Rollback::NotNeeded | Rollback::NothingToRestore => {}
        }
        lines
    }
}

/// The result of [`Checker::run`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VerifyReport {
    /// One verdict per checked agent, in display order.
    pub agents: Vec<AgentVerdict>,
    /// Why the record could not be saved, if it could not (no contents).
    pub record_errors: Vec<String>,
}

impl VerifyReport {
    /// The verdict for `agent`, if it was checked.
    pub fn agent(&self, agent: AgentId) -> Option<&AgentVerdict> {
        self.agents.iter().find(|v| v.agent() == agent)
    }

    /// Every line, agent by agent.
    pub fn lines(&self) -> Vec<String> {
        self.agents.iter().flat_map(AgentVerdict::lines).collect()
    }

    /// Every checked agent passed (pending user steps count as passed).
    pub fn all_passed(&self) -> bool {
        self.agents
            .iter()
            .all(|v| v.report.result != CheckResult::Fail)
    }
}

/// Runs the agent checks of one setup run.
#[derive(Debug, Clone, Copy)]
pub struct Checker<'a> {
    /// The agents.
    pub registry: &'a Registry,
    /// Their environment.
    pub env: &'a Env,
    /// Where the record is saved.
    pub store: &'a RecordStore,
}

impl Checker<'_> {
    /// Checks every agent of `want` that `applied` did not skip or roll
    /// back, stores the results in `record` (saved through the store), and
    /// rolls back each agent whose check failed after this run changed its
    /// files (see the module comment). `applied` is updated with the
    /// rollbacks.
    pub fn run(
        &self,
        want: &Selection,
        applied: &mut ApplyReport,
        record: &mut InstallRecord,
    ) -> VerifyReport {
        let mut report = VerifyReport::default();
        for id in want.agents() {
            let Some(agent) = self.registry.get(id) else {
                continue;
            };
            let scope = Scope::Agent(id);
            if applied
                .agent(scope)
                .is_some_and(|a| a.outcome != Outcome::Applied)
            {
                continue;
            }
            let verdict = self.check_one(agent, want, applied, record, &mut report);
            report.agents.push(verdict);
        }
        report
    }

    fn check_one(
        &self,
        agent: &dyn Agent,
        want: &Selection,
        applied: &mut ApplyReport,
        record: &mut InstallRecord,
        report: &mut VerifyReport,
    ) -> AgentVerdict {
        let id = agent.id();
        let scope = Scope::Agent(id);
        let mut entry = record.agents.get(&id).cloned().unwrap_or_default();
        entry.features = agent
            .features()
            .iter()
            .map(|f| (*f, want.wants(id, *f)))
            .collect();
        entry.pending_user_steps = agent.pending_steps(want);
        let check = agent.check(self.env, &entry);
        let changed = applied.agent(scope).is_some_and(|a| !a.files().is_empty());
        if check.result == CheckResult::Fail && changed {
            let rollback = self.roll_back(id, applied, record);
            return AgentVerdict {
                report: check,
                rollback,
            };
        }
        entry.last_check = Some(check.last_check(UtcTimestamp::now()));
        record.agents.insert(id, entry);
        if let Err(e) = self.store.save(record) {
            report.record_errors.push(e.to_string());
        }
        let rollback = if check.result == CheckResult::Fail {
            Rollback::NothingToRestore
        } else {
            Rollback::NotNeeded
        };
        AgentVerdict {
            report: check,
            rollback,
        }
    }

    fn roll_back(
        &self,
        id: AgentId,
        applied: &mut ApplyReport,
        record: &mut InstallRecord,
    ) -> Rollback {
        let reason = restored_line(id);
        match rollback_agent(applied, Scope::Agent(id), self.store, record, &reason) {
            Ok(()) => Rollback::Restored,
            Err(RollbackError::Incomplete { problems }) => Rollback::Incomplete { problems },
            Err(RollbackError::NotApplied) => Rollback::NothingToRestore,
        }
    }
}

#[cfg(test)]
#[path = "verify_tests.rs"]
mod tests;
