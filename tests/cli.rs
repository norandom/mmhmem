//! CLI smoke tests for the scaffolding binary (task 1.1) and the
//! `healthcheck` subcommand (task 12.3).

mod common;

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

#[test]
fn healthcheck_fails_when_no_daemon_listens() {
    use std::net::TcpListener;

    // Bind then drop an ephemeral listener: the port is (practically)
    // guaranteed refused right after, so the probe must fail.
    let port = {
        let l = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral");
        l.local_addr().expect("addr").port()
    };
    let out = Command::new(env!("CARGO_BIN_EXE_mmhmem"))
        .arg("healthcheck")
        .env(mmhmem::config::ENV_PORT, port.to_string())
        .output()
        .expect("failed to spawn the mmhmem binary");
    assert!(!out.status.success(), "must exit non-zero without a daemon");
}

#[tokio::test]
async fn healthcheck_succeeds_against_live_daemon() {
    let (_roots, _data, _logs, state) = common::test_state("healthcheck-token");
    let (port, _handle) = common::spawn_server(state).await;
    let out = tokio::process::Command::new(env!("CARGO_BIN_EXE_mmhmem"))
        .arg("healthcheck")
        .env(mmhmem::config::ENV_PORT, port.to_string())
        .output()
        .await
        .expect("failed to spawn the mmhmem binary");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "must exit 0 against a live daemon; stderr: {stderr}"
    );
}
