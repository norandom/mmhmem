//! Unit tests for `warmup_select.rs` (shunt-cache-efficiency task 6.1;
//! Req 6.1, 6.2, 6.4): candidates of a temp repository, the walk without
//! git, the allowed roots and the cap.
use super::*;
use crate::config::{Config, ENV_ALLOWED_ROOTS, ENV_DATA_DIR};
use std::fs;

/// A small threshold keeps the fixtures small; the rule is the same.
const THRESHOLD: u64 = 20;

fn lines(n: u64, tag: &str) -> String {
    (0..n).map(|i| format!("{tag} line {i}\n")).collect()
}

fn write(root: &Path, rel: &str, body: &str) {
    let path = root.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, body).unwrap();
}

/// Allowed roots from a local-profile config (`None`: unconfined).
fn roots(allowed: Option<&Path>) -> AllowedRoots {
    let data = std::env::temp_dir().join("mmhmem-warmup-select-tests");
    let allowed = allowed.map(|p| p.display().to_string());
    let config = Config::from_env_local(&|key| match key {
        ENV_DATA_DIR => Some(data.display().to_string()),
        ENV_ALLOWED_ROOTS => allowed.clone(),
        _ => None,
    })
    .unwrap();
    AllowedRoots::from_config(&config)
}

fn rules(roots: &AllowedRoots, max_files: usize) -> Rules<'_> {
    Rules {
        threshold: THRESHOLD,
        roots,
        max_files,
    }
}

/// Runs git in `dir`; false when git is missing or the command fails.
fn git(dir: &Path, args: &[&str]) -> bool {
    std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .is_ok_and(|o| o.status.success())
}

fn rels(selection: &Selection) -> Vec<String> {
    selection
        .files
        .iter()
        .map(|c| c.rel.to_string_lossy().replace('\\', "/"))
        .collect()
}

/// Large tracked and untracked files, a small one, an ignored large one, a
/// security-critical large one and a binary one.
fn repository() -> Option<tempfile::TempDir> {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    if !git(root, &["init", "-q"]) {
        eprintln!("git is not available; skipping the repository case");
        return None;
    }
    write(root, ".gitignore", "ignored/\n");
    write(root, "big_tracked.rs", &lines(THRESHOLD, "tracked"));
    write(
        root,
        "src/big_untracked.rs",
        &lines(THRESHOLD + 5, "untracked"),
    );
    write(root, "small.rs", &lines(THRESHOLD - 1, "small"));
    write(
        root,
        "ignored/big_ignored.rs",
        &lines(THRESHOLD * 2, "ignored"),
    );
    write(root, "src/secret_store.rs", &lines(THRESHOLD * 2, "secret"));
    let mut binary = lines(THRESHOLD * 2, "binary").into_bytes();
    binary[3] = 0;
    fs::write(root.join("blob.dat"), binary).unwrap();
    assert!(git(root, &["add", "big_tracked.rs"]), "git add");
    Some(dir)
}

#[test]
fn a_repository_yields_its_large_non_ignored_non_secret_text_files() {
    let Some(repo) = repository() else { return };
    let roots = roots(None);
    let got = select(repo.path(), &rules(&roots, DEFAULT_MAX_FILES));
    assert_eq!(got.listing, Listing::Git);
    assert_eq!(rels(&got), ["big_tracked.rs", "src/big_untracked.rs"]);
    // .gitignore, blob.dat, the two big ones, small.rs, the secret one.
    assert_eq!(got.examined, 6, "{got:?}");
    let want = Skipped {
        below_threshold: 2,
        not_text: 1,
        security: 1,
        outside_roots: 0,
        unreadable: 0,
    };
    assert_eq!(got.skipped, want, "{got:?}");
    assert!(!got.capped);
    for c in &got.files {
        assert_eq!(c.abs, repo.path().join(&c.rel));
    }
}

#[test]
fn files_outside_the_allowed_roots_are_dropped() {
    let Some(repo) = repository() else { return };
    let elsewhere = tempfile::tempdir().unwrap();
    let outside = roots(Some(elsewhere.path()));
    let got = select(repo.path(), &rules(&outside, DEFAULT_MAX_FILES));
    assert!(got.files.is_empty(), "{got:?}");
    assert_eq!(got.skipped.outside_roots, 2, "{got:?}");
    let inside = roots(Some(repo.path()));
    let got = select(repo.path(), &rules(&inside, DEFAULT_MAX_FILES));
    assert_eq!(got.files.len(), 2, "{got:?}");
}

#[test]
fn the_cap_stops_at_max_files_only_when_more_candidates_remain() {
    let dir = tempfile::tempdir().unwrap();
    for name in ["a.rs", "b.rs", "c.rs"] {
        write(dir.path(), name, &lines(THRESHOLD, name));
    }
    write(dir.path(), "d.rs", &lines(1, "d"));
    let roots = roots(None);
    let capped = select(dir.path(), &rules(&roots, 2));
    assert_eq!(rels(&capped), ["a.rs", "b.rs"]);
    assert!(capped.capped, "a third candidate remained");
    assert_eq!(capped.examined, 2, "the stop is not counted as examined");
    let exact = select(dir.path(), &rules(&roots, 3));
    assert_eq!(exact.files.len(), 3);
    assert!(!exact.capped, "only d.rs (short) remained: {exact:?}");
    assert_eq!(
        cap_message(2),
        "stopped at 2 files; raise --max-files to continue"
    );
}

#[test]
fn the_walk_skips_vcs_build_and_dependency_folders_and_stays_bounded() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(root, "a.rs", "a\n");
    write(root, "src/b.rs", "b\n");
    write(root, ".git/config", "x\n");
    write(root, "target/debug/c.rs", "c\n");
    write(root, "node_modules/pkg/d.js", "d\n");
    let deep: Vec<String> = (0..=MAX_DEPTH).map(|i| format!("d{i}")).collect();
    write(root, &format!("{}/too_deep.rs", deep.join("/")), "e\n");
    let shallow = deep[..MAX_DEPTH].join("/");
    write(root, &format!("{shallow}/deep_enough.rs"), "f\n");
    let got: Vec<String> = walk_files(root)
        .iter()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .collect();
    assert_eq!(
        got,
        [
            "a.rs".to_string(),
            format!("{shallow}/deep_enough.rs"),
            "src/b.rs".to_string(),
        ]
    );
}

#[test]
fn outside_a_repository_the_walk_lists_the_candidates() {
    let dir = tempfile::tempdir().unwrap();
    // Only meaningful when the temp folder is not inside a work tree.
    if git(dir.path(), &["rev-parse", "--is-inside-work-tree"]) {
        eprintln!("the temp folder is inside a git work tree; skipping");
        return;
    }
    write(dir.path(), "big.rs", &lines(THRESHOLD, "big"));
    write(dir.path(), "target/big.rs", &lines(THRESHOLD, "built"));
    let roots = roots(None);
    let got = select(dir.path(), &rules(&roots, DEFAULT_MAX_FILES));
    assert_eq!(got.listing, Listing::Walk);
    assert_eq!(rels(&got), ["big.rs"]);
}
