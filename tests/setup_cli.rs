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

#[test]
fn setup_without_a_terminal_exits_zero() {
    let out = run_cli(&["setup"]);
    assert_eq!(out.status.code(), Some(0));
    let text = String::from_utf8(out.stdout).expect("utf8");
    assert!(text.contains("Nothing was changed."));
}

#[test]
fn setup_accepts_the_internal_arguments() {
    let out = run_cli(&["setup", "--bootstrap", "v0.0.0", "--release-dir", "x"]);
    assert_eq!(out.status.code(), Some(0));
}
