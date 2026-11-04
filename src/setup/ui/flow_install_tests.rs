//! Tests of the program copy, the `PATH` entry and the update (task 7.1):
//! temporary homes, the agent doubles, a fake user `PATH` (never the
//! registry) and a local signed release (`--release-dir`, test keys).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::*;
use crate::setup::agents::Selection;
use crate::setup::record::{EntryState, PathEntryInfo, Sha256Hex};
use crate::setup::release::fixtures::{FakeRelease, TRUSTED};
use crate::setup::release::{LocalRelease, SUMS_NAME, SUMS_SIG_NAME};
use crate::setup::release_keys::test_keys::public_key;
use crate::setup::ui::fixtures::{Fx, tag, with_clis};
use crate::setup::user_path::{FakeUserPath, PathValue, ValueKind};

const NEW_PROGRAM: &[u8] = b"the program the bootstrap downloaded";

/// A user PATH value as the registry stores it (UTF-16LE, one NUL).
fn reg(kind: ValueKind, text: &str) -> PathValue {
    let units: Vec<u16> = text.encode_utf16().chain([0]).collect();
    PathValue {
        kind,
        data: units.iter().flat_map(|u| u.to_le_bytes()).collect(),
    }
}

/// `fx` started by the bootstrap from `temp`, with a fake user PATH.
fn bootstrapped(fx: &mut Fx, temp: &Path, path: &Arc<FakeUserPath>) {
    let program = temp.join("mmhmem");
    std::fs::write(&program, NEW_PROGRAM).expect("temp program");
    fx.deps.install_from = Some(program);
    fx.deps.paths = path.clone();
}

fn install(fx: &Fx) -> Flow {
    let mut flow = fx.flow(Newest::Unknown);
    assert_eq!(flow.begin(&HomeAction::Install), Next::Agents);
    flow.make_plan();
    assert_eq!(flow.confirm(), Confirmed::Done);
    assert_eq!(flow.exit_code(), 0, "{:?}", flow.progress());
    flow
}

/// Every file under `dir` whose text mentions `needle`.
fn files_naming(dir: &Path, needle: &str) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let Ok(read) = std::fs::read_dir(dir) else {
        return found;
    };
    for entry in read.filter_map(Result::ok) {
        let path = entry.path();
        if path.is_dir() {
            found.extend(files_naming(&path, needle));
        } else if String::from_utf8_lossy(&std::fs::read(&path).unwrap_or_default())
            .replace("\\\\", "\\")
            .contains(needle)
        {
            found.push(path);
        }
    }
    found
}

/// The review of a bootstrap install: the program copy first (no diff,
/// its SHA-256), then the PATH change.
fn assert_install_review(fx: &Fx, flow: &Flow) {
    let review = flow.review();
    let binary_file = tidy(fx.home.home(), &fx.deps.binary.display().to_string());
    assert_eq!(review.groups[0].file, binary_file);
    let sha = Sha256Hex::of(NEW_PROGRAM);
    assert_eq!(
        review.groups[0].items[0].summary,
        text::change_binary(false, sha.as_str())
    );
    assert_eq!(flow.diff(0), crate::setup::plan::BINARY_DIFF);
    assert_eq!(review.groups[1].file, text::PATH_SETTING);
}

/// Nothing anywhere names the temporary program; agent settings name the
/// permanent one.
fn assert_permanent_program_only(fx: &Fx, temp: &Path) {
    let temp_text = temp.display().to_string();
    let args: Vec<String> = fx
        .home
        .runner
        .calls()
        .iter()
        .flat_map(|c| c.args.clone())
        .collect();
    assert!(args.iter().all(|a| !a.contains(&temp_text)), "{args:?}");
    let permanent = fx.deps.binary.display().to_string();
    let configs = files_naming(fx.home.dir.path(), &permanent);
    assert!(
        configs.len() > 1,
        "settings naming the program: {configs:?}"
    );
    let temp_named = files_naming(fx.home.dir.path(), &temp_text);
    assert_eq!(temp_named, Vec::<PathBuf>::new());
}

/// The record holds the copied program and the added PATH entry.
fn assert_install_record(fx: &Fx, flow: &Flow) {
    let record = flow.record().expect("record");
    let binary = record.binary.as_ref().expect("binary");
    let sha = Sha256Hex::of(NEW_PROGRAM);
    assert_eq!((&binary.path, &binary.sha256), (&fx.deps.binary, &sha));
    let value = fx.deps.binary.parent().expect("folder").to_path_buf();
    let entry = PathEntryInfo {
        added: true,
        value,
        created_value: false,
        trailing_separator: false,
    };
    assert_eq!(record.path_entry, Some(entry));
    assert_eq!(record.managed.len(), 2);
    assert!(record.managed.iter().all(|e| e.state == EntryState::Done));
}

#[test]
fn a_bootstrap_install_copies_the_program_and_agents_launch_its_permanent_place() {
    let mut fx = with_clis();
    let temp = tempfile::tempdir().expect("temp");
    let original = reg(ValueKind::Plain, r"C:\Tools;%USERPROFILE%\bin");
    let path = Arc::new(FakeUserPath::with(Some(original.clone())));
    bootstrapped(&mut fx, temp.path(), &path);
    let mut flow = fx.flow(Newest::Unknown);
    flow.begin(&HomeAction::Install);
    flow.make_plan();
    assert_install_review(&fx, &flow);
    assert_eq!(flow.confirm(), Confirmed::Done);
    assert_eq!(flow.exit_code(), 0, "{:?}", flow.progress());

    assert_eq!(std::fs::read(&fx.deps.binary).expect("copied"), NEW_PROGRAM);
    let added = original.with_folder(fx.deps.binary.parent().expect("folder"));
    assert_eq!(path.get(), Some(added.clone()));
    assert_permanent_program_only(&fx, temp.path());
    assert_install_record(&fx, &flow);

    // Second run of the same release: nothing to change, nothing written.
    let before = fx.digest();
    let home = fx.flow(Newest::Unknown).home();
    assert!(
        home.notes.contains(&text::REVIEW_NOTHING.to_owned()),
        "{home:?}"
    );
    assert!(!home.actions.contains(&HomeAction::Install));
    assert_eq!(fx.digest(), before);
    assert_eq!(path.get(), Some(added));
}

#[test]
fn a_folder_already_on_path_is_kept_and_recorded_as_not_added() {
    let mut fx = with_clis();
    let temp = tempfile::tempdir().expect("temp");
    let folder = fx.deps.binary.parent().expect("folder").to_path_buf();
    let text = format!(r"C:\Tools;{}", folder.display());
    let original = reg(ValueKind::Expand, &text);
    let path = Arc::new(FakeUserPath::with(Some(original.clone())));
    bootstrapped(&mut fx, temp.path(), &path);
    let flow = install(&fx);
    assert_eq!(path.get(), Some(original));
    let record = flow.record().expect("record");
    assert_eq!(
        record.path_entry,
        Some(PathEntryInfo {
            added: false,
            value: folder,
            created_value: false,
            trailing_separator: false,
        })
    );
    assert!(
        flow.progress()
            .expect("results")
            .next_steps
            .iter()
            .all(|l| !l.contains("PATH"))
    );
}

#[test]
fn without_a_path_store_nothing_is_edited_and_the_results_say_how_to_add_it() {
    let fx = with_clis();
    let folder = fx.deps.binary.parent().expect("folder").to_path_buf();
    let flow = install(&fx);
    let steps = &flow.progress().expect("results").next_steps;
    let hint = text::path_hint("$HOME/.local/bin");
    assert_eq!(steps.last(), Some(&hint), "{steps:?}");
    assert_eq!(
        flow.record().expect("record").path_entry,
        Some(PathEntryInfo {
            added: false,
            value: folder.clone(),
            created_value: false,
            trailing_separator: false,
        })
    );

    let mut on_path = with_clis();
    let mut dirs = on_path.deps.env.path_dirs().to_vec();
    dirs.push(on_path.deps.binary.parent().unwrap().to_path_buf());
    on_path.deps.env.dirs.user_path = Some(dirs);
    let flow = install(&on_path);
    assert!(!flow.progress().expect("results").next_steps.contains(&hint));
}

/// A release folder with `fake` as `--release-dir` would hold it.
fn release_dir(fake: &FakeRelease) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("release dir");
    std::fs::write(dir.path().join(fake.archive_name()), &fake.archive).unwrap();
    std::fs::write(dir.path().join(SUMS_NAME), &fake.sums).unwrap();
    std::fs::write(dir.path().join(SUMS_SIG_NAME), &fake.sig).unwrap();
    dir
}

/// An installed home whose program holds `OLD`, updating from `dir` with
/// `keys`.
fn installed_old(dir: &Path, keys: Vec<[u8; 32]>) -> Fx {
    let mut fx = with_clis();
    std::fs::create_dir_all(fx.deps.binary.parent().unwrap()).unwrap();
    std::fs::write(&fx.deps.binary, b"OLD").unwrap();
    install(&fx);
    let server = LocalRelease::serve(dir).expect("serve");
    fx.deps.releases = Releases {
        keys,
        ..Releases::local(server)
    };
    fx
}

fn new_release() -> FakeRelease {
    let target = crate::setup::release::Target::current().expect("platform");
    FakeRelease::good(target, b"NEW PROGRAM")
}

#[test]
fn an_update_replaces_the_program_keeps_the_choices_and_then_is_current() {
    let fake = new_release();
    let dir = release_dir(&fake);
    let fx = installed_old(dir.path(), vec![public_key(TRUSTED)]);
    let choices = Selection::from_record(&fx.deps.store.load().unwrap().unwrap());
    let mut flow = fx.flow(Newest::Known(fake.tag.clone()));
    let home = flow.home();
    assert_eq!(home.actions[0], HomeAction::UpdateTo(tag("v9.9.9")));
    assert_eq!(flow.begin(&home.actions[0]), Next::Review);
    assert_eq!(flow.review().intro, [text::updated("v9.9.9")]);
    assert_eq!(flow.selection(), &choices);
    assert_eq!(flow.confirm(), Confirmed::Done);
    assert_eq!(flow.exit_code(), 0, "{:?}", flow.progress());

    assert_eq!(std::fs::read(&fx.deps.binary).unwrap(), b"NEW PROGRAM");
    let stored = fx.deps.store.load().unwrap().unwrap();
    assert_eq!(stored.release, Some(tag("v9.9.9")));
    assert_eq!(
        stored.binary.as_ref().map(|b| b.sha256.clone()),
        Some(Sha256Hex::of(b"NEW PROGRAM"))
    );
    assert_eq!(Selection::from_record(&stored), choices);

    let before = fx.digest();
    let again = fx.flow(Newest::Known(fake.tag.clone()));
    let home = again.home();
    assert!(
        home.notes.contains(&text::REVIEW_NOTHING.to_owned()),
        "{home:?}"
    );
    assert!(
        !home
            .actions
            .iter()
            .any(|a| matches!(a, HomeAction::UpdateTo(_)))
    );
    assert!(!home.notes.contains(&text::BINARY_CHANGED.to_owned()));
    assert_eq!(fx.digest(), before);
}

#[test]
fn without_a_release_key_an_update_is_refused_and_nothing_changes() {
    let fake = new_release();
    let dir = release_dir(&fake);
    let fx = installed_old(dir.path(), Vec::new());
    let before = fx.digest();
    let mut flow = fx.flow(Newest::Known(fake.tag.clone()));
    assert_eq!(
        flow.begin(&HomeAction::UpdateTo(fake.tag.clone())),
        Next::Note(text::UPDATE_NO_KEY)
    );
    assert_eq!(flow.confirm(), Confirmed::Done);
    assert_eq!(fx.digest(), before);
    assert_eq!(std::fs::read(&fx.deps.binary).unwrap(), b"OLD");
}

#[test]
fn an_update_signed_by_an_unknown_key_shows_why_and_changes_nothing() {
    let fake = new_release();
    let dir = release_dir(&fake);
    let fx = installed_old(dir.path(), vec![public_key(7)]);
    let before = fx.digest();
    let mut flow = fx.flow(Newest::Known(fake.tag.clone()));
    assert_eq!(
        flow.begin(&HomeAction::UpdateTo(fake.tag.clone())),
        Next::Note(text::UPDATE_FAILED)
    );
    let notes = flow.home().notes;
    assert!(
        notes.iter().any(|n| n.ends_with("Nothing was changed.")),
        "{notes:?}"
    );
    assert_eq!(fx.digest(), before);
    assert_eq!(std::fs::read(&fx.deps.binary).unwrap(), b"OLD");
}

/// Records every program start instead of starting it; answers `code`.
fn recording_exec(code: i32) -> (Exec, Arc<std::sync::Mutex<Vec<Relaunch>>>) {
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let log = seen.clone();
    let exec: Exec = Arc::new(move |r: &Relaunch| {
        log.lock().unwrap().push(r.clone());
        Ok(code)
    });
    (exec, seen)
}

#[test]
fn after_an_update_the_new_program_is_started_and_reviews_with_its_own_files() {
    let fake = new_release();
    let dir = release_dir(&fake);
    let mut fx = installed_old(dir.path(), vec![public_key(TRUSTED)]);
    let (exec, seen) = recording_exec(7);
    fx.deps.restart = Restart {
        exec: Some(exec),
        args: vec!["--release-dir".into(), "R".into()],
        resumed: None,
    };
    let mut flow = fx.flow(Newest::Known(fake.tag.clone()));
    assert_eq!(
        flow.begin(&HomeAction::UpdateTo(tag("v9.9.9"))),
        Next::Restart
    );
    assert_eq!(std::fs::read(&fx.deps.binary).unwrap(), b"NEW PROGRAM");
    let want = Relaunch {
        program: fx.deps.binary.clone(),
        args: ["setup", "--release-dir", "R", "--restarted", "v9.9.9"]
            .map(String::from)
            .to_vec(),
    };
    assert_eq!(flow.relaunch(), Some(want.clone()));
    assert_eq!(flow.restart_now(), Some(Ok(7)));
    assert_eq!(*seen.lock().unwrap(), [want]);
    assert_eq!(flow.exit_code(), EXIT_CANCELLED, "nothing confirmed here");

    // The restarted program: straight to the review, never restarting again.
    fx.deps.restart = Restart {
        exec: None,
        args: Vec::new(),
        resumed: Some(tag("v9.9.9")),
    };
    let app = crate::setup::ui::app::App::new(fx.flow(Newest::Known(tag("v9.9.9"))));
    assert_eq!(app.screen(), crate::setup::ui::app::Screen::Review);
    assert_eq!(app.flow().review().intro, [text::updated("v9.9.9")]);
    assert_eq!(app.flow().restart_now(), None);
}

#[test]
fn an_update_to_this_programs_own_release_does_not_restart() {
    let fake = new_release();
    let dir = release_dir(&fake);
    let mut fx = installed_old(dir.path(), vec![public_key(TRUSTED)]);
    let (exec, seen) = recording_exec(0);
    fx.deps.restart.exec = Some(exec);
    fx.deps.version = tag("v9.9.9");
    let mut flow = fx.flow(Newest::Known(fake.tag.clone()));
    assert_eq!(
        flow.begin(&HomeAction::UpdateTo(tag("v9.9.9"))),
        Next::Review
    );
    assert_eq!(flow.restart_now(), None);
    assert!(seen.lock().unwrap().is_empty());
}

#[test]
fn the_numbered_fallback_ends_for_the_restart_and_the_restarted_run_opens_the_review() {
    let fake = new_release();
    let dir = release_dir(&fake);
    let mut fx = installed_old(dir.path(), vec![public_key(TRUSTED)]);
    let (exec, _seen) = recording_exec(0);
    fx.deps.restart.exec = Some(exec);
    let mut flow = fx.flow(Newest::Known(fake.tag.clone()));
    let code = crate::setup::ui::fallback::run(&mut flow, &mut "1\n".as_bytes(), &mut Vec::new());
    assert_eq!(code, 0);
    assert!(flow.relaunch().is_some());

    fx.deps.restart = Restart {
        resumed: Some(tag("v9.9.9")),
        ..Restart::default()
    };
    let mut flow = fx.flow(Newest::Unknown);
    let mut out = Vec::new();
    let code = crate::setup::ui::fallback::run(&mut flow, &mut "q\n".as_bytes(), &mut out);
    assert_eq!(code, EXIT_CANCELLED);
    let out = String::from_utf8(out).unwrap();
    assert!(out.contains(&text::updated("v9.9.9")), "{out}");
}
