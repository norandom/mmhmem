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
//!
//! A `PATH` entry change (install-wide) reads and writes the user's `PATH`
//! value through the [`UserPathStore`] given to [`apply_with`]; it has no
//! backup file, and its rollback writes back the exact value read before
//! the change. The program ([`Target::Binary`]) is written executable; a
//! removal that deletes the running program defers the delete to the end
//! of the process on Windows (`delete_program`).

use std::io;
use std::path::Path;

use super::check::{check_agent, hashes, read_current};
use super::{AgentReport, Applied, ApplyReport, Change, Outcome, Plan, RollbackError};
use crate::setup::fsx;
use crate::setup::record::{
    InstallRecord, ManagedEntry, PendingChange, RecordStore, Scope, Target, UtcTimestamp,
};
use crate::setup::user_path::{NoUserPath, PathValue, UserPathStore};

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
/// others; the report says per agent what happened. A plan with a `PATH`
/// entry needs [`apply_with`].
pub fn apply(plan: &Plan, store: &RecordStore, record: &mut InstallRecord) -> ApplyReport {
    apply_with(plan, store, record, &NoUserPath)
}

/// [`apply`] with the user's `PATH` value reached through `paths`.
pub fn apply_with(
    plan: &Plan,
    store: &RecordStore,
    record: &mut InstallRecord,
    paths: &dyn UserPathStore,
) -> ApplyReport {
    let agents = plan
        .scopes()
        .into_iter()
        .map(|scope| apply_agent(plan, scope, store, record, paths))
        .collect();
    ApplyReport { agents }
}

/// Restore every file `scope` changed in this run from its backup, newest
/// first, and put its record entries back as they were (for a failed
/// functional check, Requirement 3.5). `reason` is the plain line kept in
/// the report. (Agents never change the `PATH`; a `PATH` entry is restored
/// only by [`apply_with`]'s own rollback.)
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
    let problems = undo_all(agent, store, record, &NoUserPath);
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
    paths: &dyn UserPathStore,
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
    if let Err(skip) = check_agent(&changes, paths) {
        report.outcome = Outcome::Skipped(skip);
        return report;
    }
    for change in changes {
        match apply_one(change, &plan.run, store, record, paths) {
            Ok(applied) => report.applied.push(applied),
            Err(failure) => {
                report.applied.extend(failure.written.map(|a| *a));
                let problems = undo_all(&mut report, store, record, paths);
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
    paths: &dyn UserPathStore,
) -> Result<Applied, Failure> {
    let applied = begin(change, run, store, record, paths)?;
    if let Err(reason) = write_change(change, paths) {
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
    paths: &dyn UserPathStore,
) -> Result<Applied, Failure> {
    let current = read_current(change, paths).map_err(|s| Failure::new(s.to_string()))?;
    if current != change.before {
        let file = change.file.display();
        return Err(Failure::new(format!(
            "{file} changed while setup was running"
        )));
    }
    let (before_hash, after_hash) = hashes(change).map_err(Failure::new)?;
    let path_restore = match change.target {
        Target::PathEntry { .. } => Some(decode_value(change.before.as_deref())?),
        _ => None,
    };
    let backup = match path_restore {
        Some(_) => None,
        None => store
            .backup(&change.file, run)
            .map_err(|e| Failure::new(e.to_string()))?,
    };
    let prior = record.find(change.scope, &change.target).cloned();
    let new_file = change.before.is_none() && path_restore.is_none();
    let pending = PendingChange {
        target: change.target.clone(),
        before: before_hash.clone(),
        after: after_hash.clone(),
        backup: backup.clone(),
        missing_parents: change.missing_parents,
        created_file: new_file || prior.as_ref().is_some_and(|p| p.created_file),
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
        path_restore,
    })
}

/// A planned `PATH` value from its change bytes.
fn decode_value(bytes: Option<&[u8]>) -> Result<Option<PathValue>, Failure> {
    bytes
        .map(PathValue::decode)
        .transpose()
        .map_err(Failure::new)
}

/// Journal step 3: write exactly the planned bytes (or delete the file).
/// Identical bytes of a normal file are not rewritten.
pub(super) fn write_change(change: &Change, paths: &dyn UserPathStore) -> Result<(), String> {
    if let Target::PathEntry { .. } = change.target {
        let value = decode_value(change.after.as_deref()).map_err(|f| f.reason)?;
        return paths.write(value.as_ref());
    }
    let result = match (&change.after, change.owner_only) {
        (Some(bytes), _) if !change.owner_only && change.before.as_ref() == Some(bytes) => Ok(()),
        (Some(bytes), true) => fsx::write_owner_only(&change.file, bytes),
        (Some(bytes), false) => fsx::write_atomic(&change.file, bytes),
        (None, _) if matches!(change.target, Target::Binary { .. }) => delete_program(&change.file)
            .map(|()| prune_folders(&change.file, change.missing_parents.unwrap_or(0))),
        (None, _) => remove_file(&change.file),
    };
    let result = result.and_then(|()| match (&change.target, &change.after) {
        (Target::Binary { path }, Some(_)) => make_executable(path),
        _ => Ok(()),
    });
    result.map_err(|e| format!("{e:#}"))
}

/// Unix: mode `0755` for the installed program. Windows: nothing to do.
fn make_executable(path: &Path) -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs_err::set_permissions(path, std::fs::Permissions::from_mode(0o755))?;
    }
    let _ = path;
    Ok(())
}

/// Delete the installed program. When it is the program running now (a
/// removal started from it), `self_replace::self_delete_outside_path` does
/// it: Unix removes the file at once; Windows cannot delete a running
/// program, so the file is deleted as soon as setup exits (by a short-lived
/// helper copy placed outside the program's folder).
fn delete_program(path: &Path) -> anyhow::Result<()> {
    let running = std::env::current_exe()
        .ok()
        .is_some_and(|exe| same_file(&exe, path));
    if !running {
        return remove_file(path);
    }
    let folder = path.parent().unwrap_or(path);
    self_replace::self_delete_outside_path(folder)
        .map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))
}

/// After the program was deleted: delete up to `count` folders above it
/// that the install created, innermost first, stopping at the first one
/// that is not empty (or not deletable). A folder with anything in it is
/// never deleted.
fn prune_folders(program: &Path, count: u32) {
    let mut dir = program.parent();
    for _ in 0..count {
        let Some(folder) = dir else { return };
        if std::fs::remove_dir(folder).is_err() {
            return;
        }
        dir = folder.parent();
    }
}

/// Whether two paths name the same existing file (also through links).
fn same_file(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
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
    paths: &dyn UserPathStore,
) -> Vec<String> {
    let mut problems = Vec::new();
    while let Some(applied) = report.applied.pop() {
        if let Err(problem) = undo_one(&applied, report.scope, store, record, paths) {
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
    paths: &dyn UserPathStore,
) -> Result<(), String> {
    let file = applied.file.display();
    let pending = PendingChange {
        target: applied.target.clone(),
        before: applied.after_hash.clone(),
        after: applied.before_hash.clone(),
        backup: applied.backup.clone(),
        missing_parents: applied.prior.as_ref().and_then(|p| p.missing_parents),
        created_file: applied.prior.as_ref().is_some_and(|p| p.created_file),
    };
    store
        .journal_begin(record, scope, pending)
        .map_err(|e| format!("{file}: {e}"))?;
    if let Err(e) = restore_file(applied, store, paths) {
        // The file still holds what this run wrote; say so in the record.
        let _ = record.undo_change(scope, &applied.target);
        let _ = store.save(record);
        return Err(format!("{file} could not be restored: {e:#}"));
    }
    set_entry(record, scope, &applied.target, applied.prior.clone());
    store.save(record).map_err(|e| format!("{file}: {e}"))
}

/// Put the file back from its backup, or delete it when it did not exist;
/// a `PATH` value is written back exactly as it was read.
fn restore_file(
    applied: &Applied,
    store: &RecordStore,
    paths: &dyn UserPathStore,
) -> anyhow::Result<()> {
    if let Some(value) = &applied.path_restore {
        return paths.write(value.as_ref()).map_err(anyhow::Error::msg);
    }
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
