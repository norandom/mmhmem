//! Tests of the user `PATH` value (task 7.1): append once, keep the value
//! type and every other byte, the Linux/macOS hint, and a `PATH` change
//! applied and rolled back through the plan engine with [`FakeUserPath`]
//! (never the real registry).

use std::path::{Path, PathBuf};

use super::*;
use crate::setup::plan::{Action, Change, Outcome, Plan, apply_with};
use crate::setup::record::{InstallRecord, RecordStore, Scope, Target, UtcTimestamp};

fn value(kind: ValueKind, text: &str, nuls: usize) -> PathValue {
    let mut units: Vec<u16> = text.encode_utf16().collect();
    units.extend(std::iter::repeat_n(0, nuls));
    PathValue {
        kind,
        data: units.iter().flat_map(|u| u.to_le_bytes()).collect(),
    }
}

fn folder() -> PathBuf {
    PathBuf::from(r"C:\Users\u\AppData\Local\Programs\mmhmem")
}

#[test]
fn the_folder_is_appended_once_with_type_and_other_bytes_kept() {
    let before = value(ValueKind::Plain, r"C:\Tools;%USERPROFILE%\bin", 2);
    let after = before.with_folder(&folder());
    assert_eq!(after.kind, ValueKind::Plain);
    assert_eq!(
        after,
        value(
            ValueKind::Plain,
            r"C:\Tools;%USERPROFILE%\bin;C:\Users\u\AppData\Local\Programs\mmhmem",
            2
        )
    );
    assert!(
        after
            .data
            .starts_with(&before.data[..before.data.len() - 4])
    );
    assert_eq!(after.with_folder(&folder()), after, "added only once");
    let trailing = value(ValueKind::Expand, r"C:\Tools;", 1).with_folder(&folder());
    assert_eq!(trailing.entries(), [PathBuf::from(r"C:\Tools"), folder()]);
}

#[test]
fn a_folder_already_there_is_found_ignoring_trailing_separators() {
    let v = value(
        ValueKind::Expand,
        r"C:\Users\u\AppData\Local\Programs\mmhmem\;C:\x",
        1,
    );
    assert!(v.contains(&folder()));
    assert_eq!(v.with_folder(&folder()), v);
    let empty = value(ValueKind::Expand, "", 1).with_folder(&folder());
    assert_eq!(empty.entries(), [folder()]);
    assert_eq!(PathValue::new_with(&folder()).entries(), [folder()]);
}

#[test]
fn encoding_round_trips_and_refuses_odd_data() {
    let v = value(ValueKind::Plain, "a;b", 1);
    assert_eq!(PathValue::decode(&v.encode()), Ok(v));
    assert!(PathValue::decode(b"Sabc").is_err());
    assert!(PathValue::decode(b"Xab").is_err());
    assert!(PathValue::decode(b"").is_err());
}

#[test]
fn the_unix_hint_appears_only_when_the_folder_is_missing() {
    let home = Path::new("/home/u");
    let bin = home.join(".local").join("bin");
    let hint = path_hint(&[PathBuf::from("/usr/bin")], &bin, home).expect("hint");
    assert!(
        hint.contains("export PATH=\"$HOME/.local/bin:$PATH\""),
        "{hint}"
    );
    assert!(path_hint(&[PathBuf::from("/usr/bin"), bin.clone()], &bin, home).is_none());
    let other = Path::new("/opt/mmhmem");
    let hint = path_hint(&[], other, home).expect("hint");
    assert!(hint.contains("/opt/mmhmem:$PATH"), "{hint}");
}

#[test]
fn no_store_refuses_and_says_so() {
    assert!(!NoUserPath.supported());
    assert!(NoUserPath.read().is_err());
    assert!(NoUserPath.write(None).is_err());
}

fn path_change(before: Option<&PathValue>) -> Change {
    let after = before.map_or_else(
        || PathValue::new_with(&folder()),
        |b| b.with_folder(&folder()),
    );
    Change {
        scope: Scope::Install,
        target: Target::PathEntry { value: folder() },
        file: folder(),
        action: Action::Add,
        before: before.map(PathValue::encode),
        after: Some(after.encode()),
        owner_only: false,
        hide_diff: false,
        missing_parents: None,
        summary: "add".to_owned(),
    }
}

fn plan(changes: Vec<Change>) -> Plan {
    let run = UtcTimestamp::try_from("2026-11-04T09:30:00Z".to_owned()).unwrap();
    Plan { run, changes }
}

#[test]
fn a_path_change_is_applied_recorded_and_shown_as_folders() {
    let dir = tempfile::tempdir().unwrap();
    let store = RecordStore::new(dir.path().join("state"));
    let original = value(ValueKind::Expand, r"C:\Tools", 1);
    let paths = FakeUserPath::with(Some(original.clone()));
    let change = path_change(Some(&original));
    assert!(change.diff().contains(&format!("+{}", folder().display())));
    let mut record = InstallRecord::new();
    let report = apply_with(&plan(vec![change.clone()]), &store, &mut record, &paths);
    assert_eq!(report.agents[0].outcome, Outcome::Applied);
    assert_eq!(paths.get(), Some(original.with_folder(&folder())));
    let entry = record
        .find(Scope::Install, &change.target)
        .expect("recorded");
    assert!(entry.backup.is_none());
    assert_eq!(store.load().unwrap(), Some(record.clone()));
    // Planned again from the new value: the stale guard sees no change.
    let again = apply_with(&plan(vec![change]), &store, &mut record, &paths);
    assert!(matches!(again.agents[0].outcome, Outcome::Skipped(_)));
}

#[test]
fn a_failing_install_step_restores_the_path_value_byte_for_byte() {
    let dir = tempfile::tempdir().unwrap();
    let store = RecordStore::new(dir.path().join("state"));
    let original = value(ValueKind::Plain, r"C:\Tools;;%X%", 3);
    let paths = FakeUserPath::with(Some(original.clone()));
    // A program copy into a folder that is a file: its write fails after
    // the PATH change was written.
    let blocker = dir.path().join("blocker");
    std::fs::write(&blocker, b"x").unwrap();
    let binary = blocker.join("mmhmem.exe");
    let copy = Change {
        target: Target::Binary {
            path: binary.clone(),
        },
        file: binary,
        before: None,
        after: Some(b"program".to_vec()),
        ..path_change(None)
    };
    let mut record = InstallRecord::new();
    let changes = vec![path_change(Some(&original)), copy];
    let report = apply_with(&plan(changes), &store, &mut record, &paths);
    assert!(
        matches!(report.agents[0].outcome, Outcome::RolledBack { .. }),
        "{:?}",
        report.agents[0].outcome
    );
    assert_eq!(paths.get(), Some(original), "byte-identical after rollback");
    assert!(record.managed.is_empty());
}

#[test]
fn a_missing_path_value_is_deleted_again_on_rollback() {
    let dir = tempfile::tempdir().unwrap();
    let store = RecordStore::new(dir.path().join("state"));
    let paths = FakeUserPath::with(None);
    let blocker = dir.path().join("blocker");
    std::fs::write(&blocker, b"x").unwrap();
    let binary = blocker.join("mmhmem");
    let copy = Change {
        target: Target::Binary {
            path: binary.clone(),
        },
        file: binary,
        after: Some(b"program".to_vec()),
        ..path_change(None)
    };
    let mut record = InstallRecord::new();
    let report = apply_with(
        &plan(vec![path_change(None), copy]),
        &store,
        &mut record,
        &paths,
    );
    assert!(matches!(
        report.agents[0].outcome,
        Outcome::RolledBack { .. }
    ));
    assert_eq!(paths.get(), None);
}

#[test]
fn a_refused_path_write_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let store = RecordStore::new(dir.path().join("state"));
    let original = value(ValueKind::Expand, r"C:\Tools", 1);
    let paths = FakeUserPath::with(Some(original.clone()));
    paths
        .fail_writes
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let mut record = InstallRecord::new();
    let report = apply_with(
        &plan(vec![path_change(Some(&original))]),
        &store,
        &mut record,
        &paths,
    );
    assert!(matches!(
        report.agents[0].outcome,
        Outcome::RolledBack { .. }
    ));
    assert_eq!(paths.get(), Some(original));
    assert!(record.managed.is_empty());
}
