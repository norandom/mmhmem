//! Hook command integration tests (read-routing-hooks Req 2.4, 2.5, 3.1-3.4,
//! 5.2-5.4; design.md "hook command (`src/cli/hook.rs`)"): run the REAL
//! binary as a harness would, with an isolated per-user environment, and
//! check stdout, the exit code and the audit trail.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use mmhmem::services::shunt::is_security_critical_path;
use serde_json::{Value, json};

const SECRET_KEY: &str = "sk-hooktest-0123456789abcdef";
const MARKER: &str = "UNIQUE_FILE_CONTENT_MARKER";

/// Isolated environment: project dir (the harness cwd), per-user home and
/// data dir, all under one temp dir whose path never trips the security rule.
struct Env {
    _root: tempfile::TempDir,
    project: PathBuf,
    home: PathBuf,
    data: PathBuf,
}

impl Env {
    fn new() -> Self {
        let root = loop {
            let dir = tempfile::Builder::new()
                .prefix("mmhmem-hookcli-")
                .tempdir()
                .expect("tempdir");
            if !is_security_critical_path(dir.path()) {
                break dir;
            }
        };
        let project = root.path().join("proj");
        let home = root.path().join("home");
        let data = root.path().join("data");
        for d in [&project, &home, &data] {
            std::fs::create_dir_all(d).expect("mkdir");
        }
        Self {
            _root: root,
            project,
            home,
            data,
        }
    }

    fn file(&self, rel: &str, lines: usize) -> PathBuf {
        let path = self.project.join(rel);
        let body: String = (0..lines).map(|i| format!("{MARKER} {i}\n")).collect();
        std::fs::write(&path, body).expect("write");
        path
    }

    fn audit(&self) -> Vec<Value> {
        let text = std::fs::read_to_string(self.data.join("audit.jsonl")).unwrap_or_default();
        text.lines()
            .map(|l| serde_json::from_str(l).expect("audit line is JSON"))
            .collect()
    }

    fn command(&self, harness: &str, worker: bool) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_mmhmem"));
        cmd.args(["hook", harness])
            .current_dir(&self.project)
            .env("AGENT_MEM_DATA_DIR", &self.data)
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .env("APPDATA", &self.home)
            .env("XDG_DATA_HOME", &self.home)
            .env("XDG_CONFIG_HOME", &self.home)
            .env_remove("AGENT_MEM_LOG_FILE")
            .env_remove("SHUNT_MIN_LINES")
            .env_remove("AGENT_MEM_AUDIT_MAX_BYTES")
            .env_remove("AGENT_MEM_AUDIT_ROTATE_KEEP")
            .env("RUST_LOG", "trace")
            // Blank, not removed: blank wins over any .mmhmem.json lookup.
            .env("WORKER_MODEL", if worker { "test-model" } else { "" })
            .env("WORKER_API_URL", "")
            .env("WORKER_API_KEY", SECRET_KEY)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        cmd
    }

    fn run_raw(&self, harness: &str, worker: bool, stdin: &[u8]) -> Output {
        let mut child = self.command(harness, worker).spawn().expect("spawn");
        let mut pipe = child.stdin.take().expect("stdin");
        // The hook may stop reading early (oversized input); ignore EPIPE.
        let _ = pipe.write_all(stdin);
        drop(pipe);
        child.wait_with_output().expect("wait")
    }

    fn run(&self, worker: bool, tool: &str, input: Value) -> Output {
        let event = json!({"session_id": "sess-1", "cwd": self.project,
            "hook_event_name": "PreToolUse", "tool_name": tool, "tool_input": input});
        self.run_raw("claude", worker, event.to_string().as_bytes())
    }

    fn read(&self, worker: bool, path: &Path) -> Output {
        self.run(worker, "Read", json!({"file_path": path}))
    }
}

fn assert_pass(out: &Output) {
    assert!(out.status.success(), "exit: {:?}", out.status);
    assert!(
        out.stdout.is_empty(),
        "stdout: {}",
        String::from_utf8_lossy(&out.stdout)
    );
}

fn assert_no_secret(out: &Output) {
    for stream in [&out.stdout, &out.stderr] {
        assert!(!String::from_utf8_lossy(stream).contains(SECRET_KEY));
    }
}

fn deny_reason(out: &Output) -> String {
    assert!(out.status.success(), "exit: {:?}", out.status);
    let v: Value = serde_json::from_slice(&out.stdout).expect("stdout is one JSON object");
    let hso = &v["hookSpecificOutput"];
    assert_eq!(hso["hookEventName"], "PreToolUse");
    assert_eq!(hso["permissionDecision"], "deny");
    hso["permissionDecisionReason"]
        .as_str()
        .expect("reason")
        .to_string()
}

#[test]
fn large_full_read_is_redirected_once_then_passes_on_repeat() {
    let env = Env::new();
    let big = env.file("big.rs", 400);
    let start = Instant::now();
    let first = env.read(true, &big);
    let took = start.elapsed();
    let reason = deny_reason(&first);
    assert!(reason.contains("smart_read"), "{reason}");
    assert!(!reason.contains(MARKER));
    assert_no_secret(&first);
    eprintln!("hook wall time (redirect, incl. spawn): {took:?}");
    assert!(took < Duration::from_secs(1), "hook took {took:?}");
    let second = env.read(true, &big);
    assert_pass(&second);
    let audit = env.audit();
    assert_eq!(audit.len(), 2, "{audit:?}");
    assert_eq!(audit[0]["event_type"], "HOOK_REDIRECT");
    assert_eq!(audit[1]["event_type"], "HOOK_PASS");
    assert_eq!(audit[1]["details"]["reason"], "repeat");
}

/// Harness, project, exactly the five detail keys, no content, no secret.
fn assert_record_shape(rec: &Value, project: &str) {
    // Task 4.1 / 5.1 review: the smart_read agent_harness, not the CLI name.
    assert_eq!(rec["agent_harness"], "claude-code");
    assert_eq!(rec["project"], project);
    let details = rec["details"].as_object().expect("details object");
    let mut keys: Vec<&str> = details.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(keys, ["decision", "file_path", "lines", "reason", "tool"]);
    assert_eq!(details["tool"], "Read");
    let line = rec.to_string();
    assert!(!line.contains(MARKER), "content leaked: {line}");
    assert!(!line.contains(SECRET_KEY), "secret leaked: {line}");
}

#[test]
fn audit_records_carry_the_decision_fields_but_no_content() {
    let env = Env::new();
    let big = env.file("big.rs", 400);
    let small = env.file("small.rs", 10);
    let _ = env.read(true, &big);
    let _ = env.read(true, &small);
    let audit = env.audit();
    assert_eq!(audit.len(), 2, "{audit:?}");
    let project = env.project.file_name().unwrap().to_str().unwrap();
    for rec in &audit {
        assert_record_shape(rec, project);
    }
    let redirect = &audit[0]["details"];
    assert_eq!(audit[0]["event_type"], "HOOK_REDIRECT");
    assert_eq!(redirect["decision"], "redirect");
    assert_eq!(redirect["lines"], 150);
    assert!(redirect["file_path"].as_str().unwrap().ends_with("big.rs"));
    let pass = &audit[1]["details"];
    assert_eq!(audit[1]["event_type"], "HOOK_PASS");
    assert_eq!(pass["decision"], "pass");
    assert_eq!(pass["reason"], "short");
    assert!(pass["file_path"].as_str().unwrap().ends_with("small.rs"));
}

#[test]
fn ranged_short_and_no_worker_reads_pass() {
    let env = Env::new();
    let big = env.file("big.rs", 400);
    let small = env.file("small.rs", 10);
    let ranged = env.run(
        true,
        "Read",
        json!({"file_path": big, "offset": 1, "limit": 50}),
    );
    assert_pass(&ranged);
    assert_pass(&env.read(true, &small));
    let no_worker = env.read(false, &big);
    assert_pass(&no_worker);
    assert_no_secret(&no_worker);
    let reasons: Vec<Value> = env
        .audit()
        .iter()
        .map(|r| r["details"]["reason"].clone())
        .collect();
    assert_eq!(
        reasons,
        [json!("range"), json!("short"), json!("no_worker")]
    );
}

#[test]
fn shell_whole_file_read_is_redirected() {
    let env = Env::new();
    let _ = env.file("big.rs", 400);
    let out = env.run(true, "Bash", json!({"command": "cat big.rs"}));
    deny_reason(&out);
    let audit = env.audit();
    assert_eq!(audit.len(), 1);
    assert_eq!(audit[0]["details"]["tool"], "Bash");
}

/// Task 4.1 (Req 6.2): the Codex event shape observed from codex-cli
/// 0.166.0 is redirected once, then passes on repeat; ranged and
/// non-shell calls pass; records carry the `codex` harness.
#[test]
fn codex_shell_read_is_redirected_once_then_passes() {
    let env = Env::new();
    let _ = env.file("big.rs", 400);
    let codex = |tool: &str, command: Value| {
        let event = json!({"session_id": "cx-1", "turn_id": "t", "cwd": env.project,
            "hook_event_name": "PreToolUse", "model": "m", "tool_name": tool,
            "tool_input": {"command": command}, "tool_use_id": "exec-1"});
        env.run_raw("codex", true, event.to_string().as_bytes())
    };
    let first = codex("Bash", json!("Get-Content big.rs"));
    let reason = deny_reason(&first);
    assert!(reason.contains("agent_harness \"codex\""), "{reason}");
    assert!(first.stdout.is_ascii());
    assert_no_secret(&first);
    assert_pass(&codex("Bash", json!(["bash", "-lc", "cat big.rs"])));
    assert_pass(&codex("Bash", json!("Get-Content big.rs -TotalCount 40")));
    assert_pass(&codex("apply_patch", json!("cat big.rs")));
    let audit = env.audit();
    assert_eq!(audit.len(), 2, "{audit:?}");
    assert_eq!(audit[0]["event_type"], "HOOK_REDIRECT");
    assert_eq!(audit[1]["details"]["reason"], "repeat");
    assert!(audit.iter().all(|r| r["agent_harness"] == "codex"));
}

#[test]
fn non_read_tools_pass_without_an_audit_record() {
    let env = Env::new();
    let big = env.file("big.rs", 400);
    let edit = json!({"file_path": big, "old_string": "a", "new_string": "b"});
    assert_pass(&env.run(true, "Edit", edit));
    assert_pass(&env.run(true, "Bash", json!({"command": "cargo test"})));
    assert!(env.audit().is_empty(), "{:?}", env.audit());
}

#[test]
fn garbage_empty_and_oversized_stdin_pass_with_exit_0() {
    let env = Env::new();
    let big = env.file("big.rs", 400);
    assert_pass(&env.run_raw("claude", true, b"\x00\xff not json {{{"));
    assert_pass(&env.run_raw("claude", true, b""));
    // A valid redirect candidate padded past the 1 MiB input cap.
    let pad = "x".repeat(1024 * 1024 + 16);
    let event = json!({"session_id": "s", "cwd": env.project, "tool_name": "Read",
        "tool_input": {"file_path": big}, "pad": pad});
    assert_pass(&env.run_raw("claude", true, event.to_string().as_bytes()));
}

#[test]
fn unknown_harness_passes_with_exit_0() {
    let env = Env::new();
    let big = env.file("big.rs", 400);
    let event = json!({"session_id": "s", "cwd": env.project, "tool_name": "Read",
        "tool_input": {"file_path": big}});
    assert_pass(&env.run_raw("nosuchharness", true, event.to_string().as_bytes()));
    // No harness argument at all.
    let mut bare = Command::new(env!("CARGO_BIN_EXE_mmhmem"));
    bare.arg("hook").stdin(Stdio::null());
    let out = bare.output().expect("spawn");
    assert_pass(&out);
}

#[test]
fn unloadable_config_passes() {
    let env = Env::new();
    let big = env.file("big.rs", 400);
    // An invalid project .mmhmem.json makes the local profile fail to load.
    std::fs::write(env.project.join(".mmhmem.json"), "{ not json").expect("write");
    assert_pass(&env.read(true, &big));
}

#[test]
fn help_lists_the_hook_command() {
    let out = Command::new(env!("CARGO_BIN_EXE_mmhmem"))
        .arg("--help")
        .output()
        .expect("spawn");
    assert!(String::from_utf8_lossy(&out.stdout).contains("hook <harness>"));
}
