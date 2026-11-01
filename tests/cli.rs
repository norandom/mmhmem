//! CLI smoke tests for the scaffolding binary (task 1.1).

use std::process::{Command, Output};

fn run_cli(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_mmhmem"))
        .args(args)
        .output()
        .expect("failed to spawn the mmhmem binary")
}

#[test]
fn version_flag_prints_version_and_exits_zero() {
    let out = run_cli(&["--version"]);
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).expect("utf8 stdout");
    assert!(stdout.contains("mmhmem"));
    assert!(stdout.contains(env!("CARGO_PKG_VERSION")));
}

#[test]
fn help_flag_prints_usage_and_exits_zero() {
    let out = run_cli(&["--help"]);
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).expect("utf8 stdout");
    assert!(stdout.contains("--version"));
    assert!(stdout.contains("--help"));
    assert!(stdout.contains(env!("CARGO_PKG_VERSION")));
}

#[test]
fn unknown_flag_fails_with_usage_on_stderr() {
    let out = run_cli(&["--bogus"]);
    assert!(!out.status.success());
    let stderr = String::from_utf8(out.stderr).expect("utf8 stderr");
    assert!(stderr.contains("--version"));
}
