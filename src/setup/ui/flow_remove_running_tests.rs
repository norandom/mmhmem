//! A removal while another process still runs the installed program
//! (`flow_remove.rs` "A program still in use"): the tools are cleaned up,
//! the program, its `PATH` entry and the record stay, the results say so
//! with exit code 1, and Remove finishes the job once nothing runs it.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use super::{Home, PROGRAM, installed, remove, say};

/// [`installed`] with the program reported as in use; returns the paths
/// the removal asked about.
fn installed_in_use() -> (Home, Arc<Mutex<Vec<PathBuf>>>) {
    let mut home = installed();
    let asked = Arc::new(Mutex::new(Vec::new()));
    let seen = asked.clone();
    home.fx.deps.in_use = Arc::new(move |p: &Path| {
        seen.lock().unwrap().push(p.to_path_buf());
        true
    });
    (home, asked)
}

#[test]
fn a_program_still_running_elsewhere_is_kept_and_the_results_say_so() {
    let (home, asked) = installed_in_use();
    let program = home.fx.deps.binary.clone();
    let flow = remove(&home, true);
    assert_eq!(*asked.lock().unwrap(), std::slice::from_ref(&program));
    let progress = flow.progress().unwrap();
    assert_eq!(progress.code, 1, "{:?}", progress.lines);
    let has = |line: &str| progress.lines.iter().any(|l| l == line);
    assert!(has(say::KEPT_IN_USE), "{:?}", progress.lines);
    assert!(!has(say::REMOVED) && !has(say::PROGRAM_LATER));
    // The tools no longer use mmhmem; the program, PATH, record and the
    // memories stay.
    let codex = home.file(&[".codex", "config.toml"]);
    let text = std::fs::read_to_string(&codex).unwrap();
    assert_eq!(text, home.expected()[&codex]);
    assert_eq!(std::fs::read(&program).unwrap(), PROGRAM);
    assert!(home.path.get().unwrap().contains(program.parent().unwrap()));
    assert!(home.memories().join("mmhmem.db").exists());
    let record = home.fx.deps.store.load().unwrap().expect("record kept");
    assert!(record.agents.is_empty(), "{:?}", record.agents);
    assert!(record.binary.is_some());
}

#[test]
fn remove_finishes_once_nothing_runs_the_program() {
    let (mut home, _) = installed_in_use();
    let program = home.fx.deps.binary.clone();
    remove(&home, false);
    home.fx.deps.in_use = Arc::new(|_| false);
    let flow = remove(&home, false);
    let progress = flow.progress().unwrap();
    assert_eq!(progress.code, 0, "{:?}", progress.lines);
    assert!(!program.exists());
    assert_eq!(home.path.get(), home.original_path);
    assert!(!home.fx.deps.store.record_path().exists());
}
