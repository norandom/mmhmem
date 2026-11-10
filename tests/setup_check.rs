//! The installer's MCP functional check against the real binary
//! (reliable-installers task 4.1; Requirements 6.1 and 6.5): a pass, and
//! plain failures for a wrong path, a server that exits, a server that never
//! answers (killed at the timeout), a missing tool and a wrong version.

use std::path::Path;
use std::time::{Duration, Instant};

use mmhmem::setup::agents::AgentId;
use mmhmem::setup::check::{
    CheckFailure, DEFAULT_CHECK_TIMEOUT, EXPECTED_TOOLS, McpCheck, Registration, check_mcp,
};
use mmhmem::setup::record::{CheckStep, StepOutcome};

const BIN: &str = env!("CARGO_BIN_EXE_mmhmem");
const VERSION: &str = env!("CARGO_PKG_VERSION");
const SECRET: &str = "check-test-secret-6b1e";

/// `mmhmem <args>` with a private store and the worker settings blank (a
/// removed variable would fall back to the machine's `.mmhmem.json`; see
/// `tests/local_stdio.rs`).
fn registration(data: &Path, args: &[&str]) -> Registration {
    Registration::new(BIN, args)
        .with_env("AGENT_MEM_DATA_DIR", data.to_string_lossy())
        .with_env("MMHMEM_NO_UPDATE_CHECK", "1")
        .with_env("WORKER_API_URL", "")
        .with_env("WORKER_MODEL", "")
        .with_env("WORKER_API_KEY", "")
        .with_env("MMHMEM_CHECK_TEST_SECRET", SECRET)
}

fn outcome(check: &McpCheck, step: CheckStep) -> StepOutcome {
    check
        .details
        .iter()
        .find(|d| d.step == step)
        .map(|d| d.outcome)
        .expect("every MCP step has an outcome")
}

fn assert_no_env_values(check: &McpCheck, data: &Path) {
    let data = data.to_string_lossy();
    for shown in [format!("{check}"), format!("{check:?}")] {
        assert!(!shown.contains(SECRET), "{shown}");
        assert!(!shown.contains(data.as_ref()), "{shown}");
    }
    assert!(!check.diagnostics().contains(SECRET));
    assert!(!check.diagnostics().contains(data.as_ref()));
}

#[cfg(windows)]
fn is_running(pid: u32) -> bool {
    let out = std::process::Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/NH", "/FO", "CSV"])
        .output()
        .expect("tasklist runs");
    String::from_utf8_lossy(&out.stdout).contains(&format!("\"{pid}\""))
}

/// Linux (and any Unix with procfs): `/proc/<pid>/stat` is gone once the
/// process is reaped, and its state field is `Z` while it is a zombie. No
/// `ps` is needed, so this works in minimal containers (the musl CI image
/// has none).
#[cfg(unix)]
fn is_running(pid: u32) -> bool {
    if Path::new("/proc/self/stat").exists() {
        return match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
            // The state follows the command name, which may itself hold
            // spaces or parentheses: read it after the last `)`.
            Ok(stat) => stat
                .rsplit_once(')')
                .and_then(|(_, rest)| rest.split_whitespace().next())
                .is_some_and(|state| state != "Z" && state != "X"),
            Err(_) => false,
        };
    }
    is_running_ps(pid)
}

/// Where there is no procfs (macOS), `ps` is part of the base system.
#[cfg(unix)]
fn is_running_ps(pid: u32) -> bool {
    let out = std::process::Command::new("ps")
        .args(["-o", "stat=", "-p", &pid.to_string()])
        .output()
        .expect("ps runs");
    let stat = String::from_utf8_lossy(&out.stdout);
    let stat = stat.trim();
    !stat.is_empty() && !stat.starts_with('Z')
}

fn wait_until_gone(pid: u32) -> bool {
    assert!(is_running(std::process::id()), "the liveness probe works");
    let until = Instant::now() + Duration::from_secs(10);
    while Instant::now() < until {
        if !is_running(pid) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

#[test]
fn real_server_passes_and_lists_exactly_this_releases_tools() {
    let data = tempfile::tempdir().unwrap();
    let reg = registration(data.path(), &["stdio"]).with_env("RUST_LOG", "info");
    let check = check_mcp(&reg, &EXPECTED_TOOLS, VERSION, DEFAULT_CHECK_TIMEOUT);
    assert!(check.passed(), "{check:?}: {}", check.summary());
    assert!(check.details.iter().all(|d| d.outcome == StepOutcome::Pass));
    let mut expected: Vec<String> = EXPECTED_TOOLS.iter().map(|s| (*s).to_owned()).collect();
    expected.sort();
    assert_eq!(
        check.tools, expected,
        "EXPECTED_TOOLS must match the server"
    );
    assert_eq!(
        check.line(AgentId::Codex),
        // Ten tools: shunt-content-tool Req 6.1 added shunt_content; the
        // 2026-11-10 restore removed the batch summary tool.
        "✔ Codex — mmhmem connected, 10 tools"
    );
    // A leading `v` (a release tag) is accepted as well.
    let tagged = check_mcp(
        &reg,
        &EXPECTED_TOOLS,
        &format!("v{VERSION}"),
        DEFAULT_CHECK_TIMEOUT,
    );
    assert!(tagged.passed(), "{}", tagged.summary());
    assert_no_env_values(&check, data.path());
    // The server logs its data folder; the copy kept for a log blanks it.
    assert!(
        check.diagnostics().contains("[hidden]"),
        "{}",
        check.diagnostics()
    );
    assert!(!check.diagnostics().contains('\u{1b}'));
    let pid = check.pid.expect("the child had a pid");
    assert!(
        wait_until_gone(pid),
        "the server must be stopped after the check"
    );
}

#[test]
fn wrong_path_says_the_program_does_not_exist() {
    let data = tempfile::tempdir().unwrap();
    let missing = data.path().join("bin").join("mmhmem.exe");
    let reg = Registration::new(&missing, &["stdio"]).with_env("MMHMEM_CHECK_TEST_SECRET", SECRET);
    let check = check_mcp(&reg, &EXPECTED_TOOLS, VERSION, DEFAULT_CHECK_TIMEOUT);
    assert_eq!(
        check.failure,
        Some(CheckFailure::ProgramMissing {
            program: missing.clone()
        })
    );
    assert_eq!(outcome(&check, CheckStep::Registration), StepOutcome::Fail);
    assert_eq!(outcome(&check, CheckStep::Handshake), StepOutcome::Skipped);
    let summary = check.summary();
    assert!(
        summary.contains(&format!(
            "the program at \"{}\" does not exist",
            missing.display()
        )),
        "{summary}"
    );
    assert!(
        summary.ends_with("What you can do: run the installer again."),
        "{summary}"
    );
    assert!(check.pid.is_none());
}

#[test]
fn server_that_exits_at_once_is_reported_as_stopped() {
    let data = tempfile::tempdir().unwrap();
    let reg = registration(data.path(), &["--no-such-option"]);
    let started = Instant::now();
    let check = check_mcp(&reg, &EXPECTED_TOOLS, VERSION, DEFAULT_CHECK_TIMEOUT);
    assert!(
        started.elapsed() < Duration::from_secs(15),
        "no wait for the timeout"
    );
    assert_eq!(
        check.failure,
        Some(CheckFailure::ExitedEarly {
            step: CheckStep::Handshake
        }),
        "{}",
        check.summary()
    );
    assert_eq!(outcome(&check, CheckStep::Registration), StepOutcome::Pass);
    assert_eq!(outcome(&check, CheckStep::Handshake), StepOutcome::Fail);
    assert_eq!(outcome(&check, CheckStep::ToolsList), StepOutcome::Skipped);
    assert!(
        check
            .summary()
            .starts_with("mmhmem did not start: mmhmem stopped right after it started."),
        "{}",
        check.summary()
    );
    assert!(check.summary().contains(" stdio to see the error"));
    assert_no_env_values(&check, data.path());
}

#[test]
fn server_that_never_answers_times_out_and_is_killed() {
    let data = tempfile::tempdir().unwrap();
    // `mmhmem hook claude` reads its input until the end and the check
    // never closes it, so the program waits and never answers.
    let reg = registration(data.path(), &["hook", "claude"]);
    let limit = Duration::from_secs(3);
    let started = Instant::now();
    let check = check_mcp(&reg, &EXPECTED_TOOLS, VERSION, limit);
    let took = started.elapsed();
    assert!(took >= limit, "took {took:?}");
    assert!(took < limit + Duration::from_secs(5), "took {took:?}");
    assert_eq!(
        check.failure,
        Some(CheckFailure::NoAnswer {
            step: CheckStep::Handshake,
            seconds: 3
        })
    );
    assert_eq!(
        check.reason().as_deref(),
        Some("mmhmem did not answer within 3 seconds")
    );
    let pid = check.pid.expect("the child had a pid");
    assert!(
        wait_until_gone(pid),
        "the child must be killed at the timeout"
    );
}

#[test]
fn missing_tool_fails_tools_list_with_a_version_hint() {
    let data = tempfile::tempdir().unwrap();
    let reg = registration(data.path(), &["stdio"]);
    let mut expected = EXPECTED_TOOLS.to_vec();
    expected.push("mem_teleport");
    let check = check_mcp(&reg, &expected, VERSION, DEFAULT_CHECK_TIMEOUT);
    assert_eq!(
        check.failure,
        Some(CheckFailure::MissingTools {
            names: vec!["mem_teleport".into()]
        })
    );
    assert_eq!(outcome(&check, CheckStep::Handshake), StepOutcome::Pass);
    assert_eq!(outcome(&check, CheckStep::ToolsList), StepOutcome::Fail);
    assert_eq!(outcome(&check, CheckStep::StatusTool), StepOutcome::Skipped);
    let summary = check.summary();
    assert!(summary.contains("does not offer mem_teleport"), "{summary}");
    assert!(summary.contains("different version"), "{summary}");
    assert!(summary.contains("starts the mmhmem version this installer brings"));
}

#[test]
fn wrong_version_fails_the_status_step() {
    let data = tempfile::tempdir().unwrap();
    let reg = registration(data.path(), &["stdio"]);
    let check = check_mcp(&reg, &EXPECTED_TOOLS, "99.0.0", DEFAULT_CHECK_TIMEOUT);
    assert_eq!(
        check.failure,
        Some(CheckFailure::WrongVersion {
            expected: "99.0.0".into()
        })
    );
    assert_eq!(outcome(&check, CheckStep::ToolsList), StepOutcome::Pass);
    assert_eq!(outcome(&check, CheckStep::StatusTool), StepOutcome::Fail);
    assert_eq!(
        check.reason().as_deref(),
        Some("mmhmem answered, but it is not version 99.0.0")
    );
    assert_no_env_values(&check, data.path());
}

#[test]
fn registration_read_from_written_config_starts_the_real_server() {
    let data = tempfile::tempdir().unwrap();
    let config = serde_json::json!({"mcpServers": {"mmhmem": {
        "command": BIN,
        "args": ["stdio"],
        "env": {
            "AGENT_MEM_DATA_DIR": data.path().to_string_lossy(),
            "WORKER_API_URL": "", "WORKER_MODEL": "", "WORKER_API_KEY": "",
            "MMHMEM_NO_UPDATE_CHECK": "1"
        }
    }}});
    let bytes = serde_json::to_vec_pretty(&config).unwrap();
    let reg = Registration::from_claude_json(&bytes, "mmhmem").unwrap();
    let check = check_mcp(&reg, &EXPECTED_TOOLS, VERSION, DEFAULT_CHECK_TIMEOUT);
    assert!(check.passed(), "{}", check.summary());
}
