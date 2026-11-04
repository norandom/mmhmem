//! `mmhmem setup` command line (reliable-installers task 6.1).

use std::process::{Command, Output};

fn run_cli(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_mmhmem"))
        .args(args)
        .stdin(std::process::Stdio::null())
        .output()
        .expect("failed to spawn the mmhmem binary")
}

#[test]
fn help_hides_the_internal_setup_arguments() {
    let out = run_cli(&["--help"]);
    let help = String::from_utf8(out.stdout).expect("utf8");
    assert!(help.contains("setup"));
    assert!(!help.contains("--bootstrap"));
    assert!(!help.contains("--release-dir"));
}

#[test]
fn setup_with_an_unknown_argument_exits_two_with_a_plain_message() {
    let out = run_cli(&["setup", "--bogus"]);
    assert_eq!(out.status.code(), Some(2));
    let err = String::from_utf8(out.stderr).expect("utf8");
    assert!(err.contains("Just run: mmhmem setup"), "{err}");
    assert!(!err.contains("--bogus"));
}

/// `mmhmem setup` without a terminal installs for real, so it only runs
/// where every folder it reads can be redirected: on Unix the home folder
/// comes from `HOME`. (On Windows the home folder comes from the system,
/// not the environment, so a test could touch the real user's folders; the
/// no-terminal run is covered by the library tests on temporary homes.)
/// Every home, config and state variable points into a temporary folder
/// and `PATH` is an empty folder, so no agent can be found.
#[cfg(unix)]
mod no_terminal {
    use super::*;

    fn run_isolated(args: &[&str], dir: &std::path::Path) -> Output {
        let path = dir.join("empty-path");
        std::fs::create_dir_all(&path).expect("path dir");
        let home = dir.join("home");
        std::fs::create_dir_all(&home).expect("home");
        Command::new(env!("CARGO_BIN_EXE_mmhmem"))
            .args(args)
            .stdin(std::process::Stdio::null())
            .env_clear()
            .env("HOME", &home)
            .env("XDG_CONFIG_HOME", home.join(".config"))
            .env("XDG_STATE_HOME", home.join(".local").join("state"))
            .env("XDG_DATA_HOME", home.join(".local").join("share"))
            .env("CODEX_HOME", home.join(".codex"))
            .env("CLAUDE_CONFIG_DIR", home.join(".claude"))
            .env("APPDATA", home.join("AppData"))
            .env("LOCALAPPDATA", home.join("LocalAppData"))
            .env("PATH", &path)
            .output()
            .expect("failed to spawn the mmhmem binary")
    }

    fn entries(dir: &std::path::Path) -> Vec<String> {
        let mut found = walk(dir);
        found.sort();
        found
    }

    fn walk(dir: &std::path::Path) -> Vec<String> {
        let mut out = Vec::new();
        for e in std::fs::read_dir(dir).expect("read").flatten() {
            out.push(e.path().display().to_string());
            if e.path().is_dir() {
                out.extend(walk(&e.path()));
            }
        }
        out
    }

    #[test]
    fn setup_finding_no_agent_changes_nothing_and_says_why() {
        let dir = tempfile::tempdir().expect("tempdir");
        let out = run_isolated(&["setup"], dir.path());
        assert_eq!(out.status.code(), Some(1));
        let text = String::from_utf8(out.stdout).expect("utf8");
        assert!(text.contains("No supported AI tool was found"), "{text}");
        // Only the two folders the test made: nothing was written.
        let found = entries(dir.path());
        assert_eq!(found.len(), 2, "{found:?}");
    }

    #[test]
    fn setup_accepts_the_internal_arguments() {
        let dir = tempfile::tempdir().expect("tempdir");
        let out = run_isolated(
            &["setup", "--bootstrap", "v0.0.0", "--release-dir", "x"],
            dir.path(),
        );
        assert_eq!(out.status.code(), Some(1));
        assert_eq!(entries(dir.path()).len(), 2);
    }
}
