//! The journaled apply and the per-agent rollback (task 2.5, design.md §3
//! "Atomic writes" and §4 "Rollback"; Requirements 3.2, 3.5 and 5.2).
//!
//! Each change runs [`begin`] (stale re-check, backup, record saved with the
//! entry `applying`), [`write_change`] (atomic write, owner-only write or
//! delete) and the journal finish (record saved with the entry `done`).
//!
//! A rollback undoes applied changes newest first. Each undo is journaled
//! too: the entry is saved `applying` from the written state back to the
//! state before the run, the file is restored from this run's backup (or
//! deleted when it did not exist), and the entry is then put back exactly as
//! the record held it before the run. A change whose write failed is undone
//! in the record only; the atomic write left the file as it was.

use std::io;
use std::path::Path;

use super::check::{check_agent, hashes, read_current};
use super::{AgentReport, Applied, ApplyReport, Change, Outcome, Plan, RollbackError};
use crate::setup::fsx;
use crate::setup::record::{
    InstallRecord, ManagedEntry, PendingChange, RecordStore, Scope, Target, UtcTimestamp,
};

/// A failed change: why, and the applied change when the file was written.
pub(super) struct Failure {
    pub(super) reason: String,
    written: Option<Box<Applied>>,
}

impl Failure {
    fn new(reason: String) -> Self {
        Self {
            reason,
            written: None,
        }
    }
}

/// Apply `plan` agent by agent, in plan order, journaling every change in
/// `record` (saved through `store`). One agent's failure never stops the
/// others; the report says per agent what happened.
pub fn apply(plan: &Plan, store: &RecordStore, record: &mut InstallRecord) -> ApplyReport {
    let agents = plan
        .scopes()
        .into_iter()
        .map(|scope| apply_agent(plan, scope, store, record))
        .collect();
    ApplyReport { agents }
}

/// Restore every file `scope` changed in this run from its backup, newest
/// first, and put its record entries back as they were (for a failed
/// functional check, Requirement 3.5). `reason` is the plain line kept in
/// the report.
pub fn rollback_agent(
    report: &mut ApplyReport,
    scope: Scope,
    store: &RecordStore,
    record: &mut InstallRecord,
    reason: &str,
) -> Result<(), RollbackError> {
    let agent = report
        .agents
        .iter_mut()
        .find(|a| a.scope == scope && a.outcome == Outcome::Applied)
        .ok_or(RollbackError::NotApplied)?;
    let problems = undo_all(agent, store, record);
    agent.outcome = rolled_back(reason.to_owned(), problems.clone());
    if problems.is_empty() {
        Ok(())
    } else {
        Err(RollbackError::Incomplete { problems })
    }
}

fn apply_agent(
    plan: &Plan,
    scope: Scope,
    store: &RecordStore,
    record: &mut InstallRecord,
) -> AgentReport {
    let changes: Vec<&Change> = plan.changes_for(scope).collect();
    let created_agent = match scope {
        Scope::Agent(id) => !record.agents.contains_key(&id),
        Scope::Install => false,
    };
    let mut report = AgentReport {
        scope,
        outcome: Outcome::Applied,
        applied: Vec::new(),
        created_agent,
    };
    if let Err(skip) = check_agent(&changes) {
        report.outcome = Outcome::Skipped(skip);
        return report;
    }
    for change in changes {
        match apply_one(change, &plan.run, store, record) {
            Ok(applied) => report.applied.push(applied),
            Err(failure) => {
                report.applied.extend(failure.written.map(|a| *a));
                let problems = undo_all(&mut report, store, record);
                report.outcome = rolled_back(failure.reason, problems);
                return report;
            }
        }
    }
    report
}

fn rolled_back(reason: String, problems: Vec<String>) -> Outcome {
    if problems.is_empty() {
        Outcome::RolledBack { reason }
    } else {
        Outcome::RollbackIncomplete { reason, problems }
    }
}

fn apply_one(
    change: &Change,
    run: &UtcTimestamp,
    store: &RecordStore,
    record: &mut InstallRecord,
) -> Result<Applied, Failure> {
    let applied = begin(change, run, store, record)?;
    if let Err(reason) = write_change(change) {
        // The atomic write left the file as it was: undo the record only.
        set_entry(record, change.scope, &change.target, applied.prior.clone());
        let _ = store.save(record);
        return Err(Failure::new(reason));
    }
    if let Err(e) = store.journal_finish(record, change.scope, &change.target) {
        return Err(Failure {
            reason: e.to_string(),
            written: Some(Box::new(applied)),
        });
    }
    Ok(applied)
}

/// Journal steps 1–2 for `change`: re-check the file, back it up and save
/// the record with the entry `applying`. Nothing is written to the file.
pub(super) fn begin(
    change: &Change,
    run: &UtcTimestamp,
    store: &RecordStore,
    record: &mut InstallRecord,
) -> Result<Applied, Failure> {
    let current = read_current(&change.file).map_err(|s| Failure::new(s.to_string()))?;
    if current != change.before {
        let file = change.file.display();
        return Err(Failure::new(format!(
            "{file} changed while setup was running"
        )));
    }
    let (before_hash, after_hash) = hashes(change).map_err(Failure::new)?;
    let backup = store
        .backup(&change.file, run)
        .map_err(|e| Failure::new(e.to_string()))?;
    let prior = record.find(change.scope, &change.target).cloned();
    let pending = PendingChange {
        target: change.target.clone(),
        before: before_hash.clone(),
        after: after_hash.clone(),
        backup: backup.clone(),
        missing_parents: change.missing_parents,
    };
    if let Err(e) = store.journal_begin(record, change.scope, pending) {
        set_entry(record, change.scope, &change.target, prior);
        return Err(Failure::new(e.to_string()));
    }
    Ok(Applied {
        target: change.target.clone(),
        file: change.file.clone(),
        backup,
        prior,
        before_hash,
        after_hash,
        owner_only: change.owner_only,
    })
}

/// Journal step 3: write exactly the planned bytes (or delete the file).
/// Identical bytes of a normal file are not rewritten.
pub(super) fn write_change(change: &Change) -> Result<(), String> {
    let result = match (&change.after, change.owner_only) {
        (Some(bytes), _) if !change.owner_only && change.before.as_ref() == Some(bytes) => Ok(()),
        (Some(bytes), true) => fsx::write_owner_only(&change.file, bytes),
        (Some(bytes), false) => fsx::write_atomic(&change.file, bytes),
        (None, _) => remove_file(&change.file),
    };
    result.map_err(|e| format!("{e:#}"))
}

fn remove_file(file: &Path) -> anyhow::Result<()> {
    match fs_err::remove_file(file) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e.into()),
        _ => Ok(()),
    }
}

/// Undo `report`'s applied changes newest first; one problem line per file
/// that could not be restored. Drops an agent record this run created when
/// it is empty again.
fn undo_all(
    report: &mut AgentReport,
    store: &RecordStore,
    record: &mut InstallRecord,
) -> Vec<String> {
    let mut problems = Vec::new();
    while let Some(applied) = report.applied.pop() {
        if let Err(problem) = undo_one(&applied, report.scope, store, record) {
            problems.push(problem);
        }
    }
    if let (true, Scope::Agent(id)) = (report.created_agent, report.scope)
        && record
            .agents
            .get(&id)
            .is_some_and(|a| *a == Default::default())
    {
        record.agents.remove(&id);
        if let Err(e) = store.save(record) {
            problems.push(e.to_string());
        }
    }
    problems
}

fn undo_one(
    applied: &Applied,
    scope: Scope,
    store: &RecordStore,
    record: &mut InstallRecord,
) -> Result<(), String> {
    let file = applied.file.display();
    let pending = PendingChange {
        target: applied.target.clone(),
        before: applied.after_hash.clone(),
        after: applied.before_hash.clone(),
        backup: applied.backup.clone(),
        missing_parents: applied.prior.as_ref().and_then(|p| p.missing_parents),
    };
    store
        .journal_begin(record, scope, pending)
        .map_err(|e| format!("{file}: {e}"))?;
    if let Err(e) = restore_file(applied, store) {
        // The file still holds what this run wrote; say so in the record.
        let _ = record.undo_change(scope, &applied.target);
        let _ = store.save(record);
        return Err(format!("{file} could not be restored: {e:#}"));
    }
    set_entry(record, scope, &applied.target, applied.prior.clone());
    store.save(record).map_err(|e| format!("{file}: {e}"))
}

/// Put the file back from its backup, or delete it when it did not exist.
fn restore_file(applied: &Applied, store: &RecordStore) -> anyhow::Result<()> {
    let Some(backup) = &applied.backup else {
        return remove_file(&applied.file);
    };
    let bytes = fs_err::read(backup.resolve(store.dir()))?;
    if applied.owner_only {
        fsx::write_owner_only(&applied.file, &bytes)
    } else {
        fsx::write_atomic(&applied.file, &bytes)
    }
}

/// Make the record hold exactly `entry` for `target` (or no entry).
fn set_entry(
    record: &mut InstallRecord,
    scope: Scope,
    target: &Target,
    entry: Option<ManagedEntry>,
) {
    let list = match scope {
        Scope::Install => &mut record.managed,
        Scope::Agent(id) => &mut record.agents.entry(id).or_default().managed,
    };
    let at = list.iter().position(|e| &e.target == target);
    match (at, entry) {
        (Some(i), Some(e)) => list[i] = e,
        (Some(i), None) => {
            list.remove(i);
        }
        (None, Some(e)) => list.push(e),
        (None, None) => {}
    }
}
