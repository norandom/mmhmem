//! Unit tests for the functional check (task 4.1): reading the registration
//! back from each agent's config shape, step mapping, wording and that no
//! environment value reaches `Display` or `Debug`.

use std::path::PathBuf;

use super::*;
use crate::setup::record::{CheckStep, StepOutcome};

const SECRET: &str = "sk-very-secret-value-0451";

fn outcomes(check: &McpCheck) -> Vec<(CheckStep, StepOutcome)> {
    check.details.iter().map(|d| (d.step, d.outcome)).collect()
}

#[test]
fn claude_json_entry_with_windows_path_and_env() {
    let text = r#"{
  "numStartups": 3,
  "mcpServers": {
    "other": {"command": "npx", "args": ["x"]},
    "mmhmem": {
      "type": "stdio",
      "command": "C:\\Users\\Ann Lee\\AppData\\Local\\mmhmem\\bin\\mmhmem.exe",
      "args": ["stdio"],
      "env": {"WORKER_API_KEY": "sk-very-secret-value-0451", "AGENT_MEM_DATA_DIR": "D:\\data"}
    }
  }
}"#;
    let reg = Registration::from_claude_json(text.as_bytes(), "mmhmem").unwrap();
    assert_eq!(
        reg.program,
        PathBuf::from(r"C:\Users\Ann Lee\AppData\Local\mmhmem\bin\mmhmem.exe")
    );
    assert_eq!(reg.args, ["stdio"]);
    assert_eq!(
        reg.env_keys().collect::<Vec<_>>(),
        ["AGENT_MEM_DATA_DIR", "WORKER_API_KEY"]
    );
    assert_eq!(reg.env()["WORKER_API_KEY"], SECRET);
    assert_eq!(reg.cwd, None);
}

#[test]
fn claude_desktop_entry_without_type_args_or_env() {
    let text = "\u{feff}{\"mcpServers\":{\"mmhmem\":{\"command\":\"/usr/local/bin/mmhmem\"}}}";
    let reg = Registration::from_claude_json(text.as_bytes(), "mmhmem").unwrap();
    assert_eq!(reg.program, PathBuf::from("/usr/local/bin/mmhmem"));
    assert!(reg.args.is_empty());
    assert_eq!(reg.env_keys().count(), 0);
}

#[test]
fn claude_json_problems_are_named_without_file_text() {
    let cases: [(&str, RegistrationError); 6] = [
        ("{not json", RegistrationError::Unreadable),
        (r#"{"mcpServers":{}}"#, RegistrationError::Missing),
        (
            r#"{"mcpServers":{"mmhmem":{"args":[]}}}"#,
            RegistrationError::NoCommand,
        ),
        (
            r#"{"mcpServers":{"mmhmem":{"type":"http","url":"https://x"}}}"#,
            RegistrationError::NotLocal,
        ),
        (
            r#"{"mcpServers":{"mmhmem":{"command":"m","args":"stdio"}}}"#,
            RegistrationError::BadField("args"),
        ),
        (
            r#"{"mcpServers":{"mmhmem":{"command":"m","env":{"K":1}}}}"#,
            RegistrationError::BadField("env"),
        ),
    ];
    for (text, want) in cases {
        let got = Registration::from_claude_json(text.as_bytes(), "mmhmem").unwrap_err();
        assert_eq!(got, want, "{text}");
        assert!(!got.to_string().contains("stdio") && !got.to_string().contains("http"));
    }
    let not_utf8 = Registration::from_claude_json(&[0xff, 0xfe, 0x00], "mmhmem").unwrap_err();
    assert_eq!(not_utf8, RegistrationError::Unreadable);
}

#[test]
fn codex_table_with_env_cwd_and_windows_paths() {
    let text = "model = \"o3\"\r\n\r\n[mcp_servers.mmhmem]\r\n\
        command = 'C:\\Users\\ann\\.local\\bin\\mmhmem.exe'\r\n\
        args = [\"stdio\"]\r\n\
        cwd = 'C:\\Users\\ann'\r\n\
        startup_timeout_sec = 20\r\n\
        \r\n[mcp_servers.mmhmem.env]\r\nWORKER_API_KEY = \"sk-very-secret-value-0451\"\r\n";
    let reg = Registration::from_codex_toml(text.as_bytes(), "mmhmem").unwrap();
    assert_eq!(
        reg.program,
        PathBuf::from(r"C:\Users\ann\.local\bin\mmhmem.exe")
    );
    assert_eq!(reg.args, ["stdio"]);
    assert_eq!(reg.cwd, Some(PathBuf::from(r"C:\Users\ann")));
    assert_eq!(reg.env_keys().collect::<Vec<_>>(), ["WORKER_API_KEY"]);
}

#[test]
fn codex_table_problems() {
    let cases: [(&str, RegistrationError); 4] = [
        ("[mcp_servers.mmhmem\n", RegistrationError::Unreadable),
        (
            "[mcp_servers.other]\ncommand = \"x\"\n",
            RegistrationError::Missing,
        ),
        (
            "[mcp_servers.mmhmem]\ncommand = \"x\"\nenabled = false\n",
            RegistrationError::Disabled,
        ),
        (
            "[mcp_servers.mmhmem]\ncommand = [\"x\"]\n",
            RegistrationError::BadField("command"),
        ),
    ];
    for (text, want) in cases {
        let got = Registration::from_codex_toml(text.as_bytes(), "mmhmem").unwrap_err();
        assert_eq!(got, want, "{text}");
    }
}

#[test]
fn opencode_entry_from_jsonc_with_environment() {
    let text = r#"{
  // user comment
  "$schema": "https://opencode.ai/config.json",
  "mcp": {
    "servers": {
      "mmhmem": {
        "type": "local",
        "command": ["C:\\Tools\\mmhmem.exe", "stdio"],
        "enabled": true,
        "environment": {"WORKER_API_KEY": "sk-very-secret-value-0451"},
      },
    },
  },
}"#;
    let reg = Registration::from_opencode_json(text.as_bytes(), "mmhmem").unwrap();
    assert_eq!(reg.program, PathBuf::from(r"C:\Tools\mmhmem.exe"));
    assert_eq!(reg.args, ["stdio"]);
    assert_eq!(reg.env_keys().collect::<Vec<_>>(), ["WORKER_API_KEY"]);
}

#[test]
fn opencode_entry_problems() {
    let entry = |body: &str| format!(r#"{{"mcp":{{"servers":{{"mmhmem":{body}}}}}}}"#);
    let cases: [(String, RegistrationError); 5] = [
        (
            entry(r#"{"type":"remote","url":"x"}"#),
            RegistrationError::NotLocal,
        ),
        (
            entry(r#"{"type":"local","command":[]}"#),
            RegistrationError::NoCommand,
        ),
        (entry(r#"{"type":"local"}"#), RegistrationError::NoCommand),
        (
            entry(r#"{"type":"local","command":"mmhmem stdio"}"#),
            RegistrationError::BadField("command"),
        ),
        (
            entry(r#"{"type":"local","command":["m"],"enabled":false}"#),
            RegistrationError::Disabled,
        ),
    ];
    for (text, want) in cases {
        let got = Registration::from_opencode_json(text.as_bytes(), "mmhmem").unwrap_err();
        assert_eq!(got, want, "{text}");
    }
}

#[test]
fn no_env_value_in_display_or_debug() {
    let reg =
        Registration::new("/nowhere/mmhmem-missing", &["stdio"]).with_env("WORKER_API_KEY", SECRET);
    assert!(!format!("{reg:?}").contains(SECRET));
    assert!(format!("{reg:?}").contains("WORKER_API_KEY"));
    let check = check_mcp(&reg, &EXPECTED_TOOLS, "0.0.0", DEFAULT_CHECK_TIMEOUT);
    for shown in [
        format!("{check}"),
        format!("{check:?}"),
        check.summary(),
        check.line(AgentId::Codex),
        check.what_you_can_do().unwrap_or_default(),
        format!("{:?}", check.failure),
    ] {
        assert!(!shown.contains(SECRET), "{shown}");
    }
    let mut with_output = McpCheck::new(PathBuf::from("x"), None);
    with_output.diagnostics = format!("token {SECRET} here");
    assert!(!format!("{with_output:?}").contains(SECRET));
    assert!(!format!("{with_output}").contains(SECRET));
}

#[test]
fn missing_absolute_program_fails_the_registration_step() {
    let program = std::env::temp_dir()
        .join("mmhmem-check-missing")
        .join("mmhmem.exe");
    let check = check_mcp(
        &Registration::new(&program, &["stdio"]),
        &EXPECTED_TOOLS,
        "1.0.0",
        DEFAULT_CHECK_TIMEOUT,
    );
    assert_eq!(
        check.failure,
        Some(CheckFailure::ProgramMissing {
            program: program.clone()
        })
    );
    assert_eq!(
        outcomes(&check),
        [
            (CheckStep::Registration, StepOutcome::Fail),
            (CheckStep::Handshake, StepOutcome::Skipped),
            (CheckStep::ToolsList, StepOutcome::Skipped),
            (CheckStep::StatusTool, StepOutcome::Skipped),
        ]
    );
    assert_eq!(
        check.summary(),
        format!(
            "mmhmem did not start: the program at \"{}\" does not exist. \
             What you can do: run the installer again.",
            program.display()
        )
    );
    assert!(check.line(AgentId::OpenCode).starts_with("✘ OpenCode — "));
}

#[test]
fn steps_before_the_failure_pass_and_later_ones_are_skipped() {
    let check = McpCheck::new(
        PathBuf::from("mmhmem"),
        Some(CheckFailure::MissingTools {
            names: vec!["status".into()],
        }),
    );
    assert_eq!(
        outcomes(&check),
        [
            (CheckStep::Registration, StepOutcome::Pass),
            (CheckStep::Handshake, StepOutcome::Pass),
            (CheckStep::ToolsList, StepOutcome::Fail),
            (CheckStep::StatusTool, StepOutcome::Skipped),
        ]
    );
    assert_eq!(check.result(), crate::setup::record::CheckResult::Fail);
    let pass = McpCheck::new(PathBuf::from("mmhmem"), None);
    assert!(pass.details.iter().all(|d| d.outcome == StepOutcome::Pass));
    assert_eq!(pass.result(), crate::setup::record::CheckResult::Pass);
}

#[test]
fn registration_error_maps_to_a_failed_registration_step() {
    let check = McpCheck::registration_failed(RegistrationError::Disabled);
    assert_eq!(check.details[0].outcome, StepOutcome::Fail);
    assert_eq!(
        check.summary(),
        "mmhmem did not start: mmhmem is switched off in the agent's settings. \
         What you can do: switch mmhmem on in the agent's settings, or run the installer again."
    );
}

#[test]
fn every_failure_has_plain_wording() {
    let failures = [
        CheckFailure::FolderMissing {
            cwd: PathBuf::from("/x"),
        },
        CheckFailure::CannotStart {
            program: PathBuf::from("/x"),
            kind: std::io::ErrorKind::PermissionDenied,
        },
        CheckFailure::ExitedEarly {
            step: CheckStep::Handshake,
        },
        CheckFailure::NoAnswer {
            step: CheckStep::Handshake,
            seconds: 20,
        },
        CheckFailure::NotMcp {
            step: CheckStep::ToolsList,
        },
        CheckFailure::MissingTools {
            names: vec!["status".into()],
        },
        CheckFailure::StatusFailed,
        CheckFailure::StoreNotReady,
        CheckFailure::WrongVersion {
            expected: "0.5.0".into(),
        },
        CheckFailure::Internal,
    ];
    for failure in failures {
        let check = McpCheck::new(PathBuf::from("/bin/mmhmem"), Some(failure));
        let summary = check.summary();
        for jargon in [
            "rmcp",
            "JSON-RPC",
            "tools/list",
            "initialize",
            "stderr",
            "MCP",
        ] {
            assert!(!summary.contains(jargon), "{jargon} in {summary}");
        }
        assert!(summary.contains(". What you can do: "), "{summary}");
    }
}

#[test]
fn expected_tools_are_distinct_and_include_status() {
    let mut names = EXPECTED_TOOLS.to_vec();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), EXPECTED_TOOLS.len());
    assert!(names.contains(&"status"));
}
