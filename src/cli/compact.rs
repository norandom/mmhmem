//! `mmhmem compact`: deterministic command-output compaction CLI.
//!
//! - `mmhmem compact` reads stdin, compacts noisy build/test/diff output,
//!   and writes the result to stdout.
//! - `mmhmem compact [--harness <name>] --run <cmd>` runs `<cmd>` via the
//!   system shell, merges stdout and stderr, compacts the output, writes it
//!   to stdout, and exits with `<cmd>`'s exact exit code. When the output
//!   got shorter it records a `command_compact` estimate in the stats store
//!   (best effort: a store that cannot open never changes the output or the
//!   exit code).

use std::io::{Read, Write};
use std::process::{Command, ExitCode, Stdio};

use serde_json::json;

use crate::config::Config;
use crate::hook::compact::{CompactResult, compact};
use crate::services::stats::{self, TokenStatRow, kind};
use crate::storage::db::Db;

/// Harness label recorded when `--harness` is not given.
const DEFAULT_HARNESS: &str = "cli";

/// Entry point for `mmhmem compact`.
pub fn run_compact(args: &[String]) -> ExitCode {
    let (harness, run) = parse_args(args);
    if let Some(cmd) = run {
        return run_wrapped(&cmd, &harness);
    }
    let mut input = String::new();
    if std::io::stdin().read_to_string(&mut input).is_err() {
        return ExitCode::FAILURE;
    }
    let res = compact(&input, None);
    let mut stdout = std::io::stdout().lock();
    let _ = stdout.write_all(res.output.as_bytes());
    let _ = stdout.flush();
    ExitCode::SUCCESS
}

/// `(harness, command to run)` from `[--harness <name>] [--run <cmd>]`.
fn parse_args(args: &[String]) -> (String, Option<String>) {
    let mut harness = DEFAULT_HARNESS.to_owned();
    let mut run = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--harness" if i + 1 < args.len() => {
                harness = args[i + 1].clone();
                i += 2;
            }
            "--run" if i + 1 < args.len() => {
                run = Some(args[i + 1].clone());
                break;
            }
            _ => i += 1,
        }
    }
    (harness, run)
}

fn run_wrapped(cmd: &str, harness: &str) -> ExitCode {
    let output = shell_command(cmd)
        .stdin(Stdio::inherit())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output();
    match output {
        Ok(out) => {
            let mut combined = String::from_utf8_lossy(&out.stdout).into_owned();
            if !out.stderr.is_empty() {
                if !combined.is_empty() && !combined.ends_with('\n') {
                    combined.push('\n');
                }
                combined.push_str(&String::from_utf8_lossy(&out.stderr));
            }
            let res = compact(&combined, Some(cmd));
            {
                let mut stdout = std::io::stdout().lock();
                let _ = stdout.write_all(res.output.as_bytes());
                let _ = stdout.flush();
            }
            record_savings(cmd, harness, &res);
            let code = out.status.code().unwrap_or(1);
            ExitCode::from(u8::try_from(code.clamp(0, 255)).unwrap_or(1))
        }
        Err(e) => {
            eprintln!("mmhmem compact: failed to run command: {e}");
            ExitCode::FAILURE
        }
    }
}

/// The stats row for one compaction, or `None` when nothing was saved.
fn savings_row(
    cmd: &str,
    harness: &str,
    project: &str,
    res: &CompactResult,
) -> Option<TokenStatRow> {
    let avoided = stats::estimate_delegated_read(res.raw_chars, res.kept_chars);
    if avoided == 0 {
        return None;
    }
    // Only the program name: arguments may carry paths or secrets.
    let program = cmd.split_whitespace().next().unwrap_or("");
    Some(TokenStatRow {
        project: project.to_owned(),
        task_id: None,
        harness: harness.to_owned(),
        kind: kind::COMMAND_COMPACT,
        avoided_input_tokens_est: avoided,
        avoided_output_tokens_est: 0,
        worker_prompt_tokens: None,
        worker_completion_tokens: None,
        details: json!({
            "program": program,
            "raw_lines": res.raw_lines,
            "kept_lines": res.kept_lines,
            "raw_chars": res.raw_chars,
            "kept_chars": res.kept_chars,
        }),
    })
}

/// Records the estimate; every failure is ignored (the wrapped command's
/// output and exit code are already settled).
fn record_savings(cmd: &str, harness: &str, res: &CompactResult) {
    let project = std::env::current_dir()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "unknown".to_owned());
    let Some(row) = savings_row(cmd, harness, &project, res) else {
        return;
    };
    let _ = (|| -> anyhow::Result<()> {
        let config = Config::load_local()?;
        let db = Db::open_for_config(&config)?;
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        rt.block_on(stats::record(&db, row))
    })();
}

fn shell_command(cmd: &str) -> Command {
    if cfg!(windows) {
        if let Ok(sh) = which::which("sh") {
            let mut c = Command::new(sh);
            c.args(["-c", cmd]);
            return c;
        }
        let mut c = Command::new("cmd");
        c.args(["/C", cmd]);
        c
    } else {
        let mut c = Command::new("sh");
        c.args(["-c", cmd]);
        c
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn parses_harness_and_run() {
        assert_eq!(parse_args(&args(&[])), ("cli".to_owned(), None));
        assert_eq!(
            parse_args(&args(&["--run", "cargo test"])),
            ("cli".to_owned(), Some("cargo test".to_owned()))
        );
        assert_eq!(
            parse_args(&args(&["--harness", "claude-code", "--run", "git diff"])),
            ("claude-code".to_owned(), Some("git diff".to_owned()))
        );
    }

    #[test]
    fn savings_row_only_when_output_shrank_and_without_arguments() {
        let raw = "   Compiling a v1\n".repeat(200);
        let res = compact(&raw, Some("cargo build"));
        let row = savings_row("cargo build --token sk-secret", "claude-code", "proj", &res)
            .expect("output shrank");
        assert_eq!(row.kind, kind::COMMAND_COMPACT);
        assert_eq!(row.harness, "claude-code");
        assert!(row.avoided_input_tokens_est > 0);
        assert_eq!(row.details["program"], "cargo");
        assert!(!row.details.to_string().contains("sk-secret"));

        let tiny = compact("ok\n", None);
        assert!(savings_row("cargo test", "cli", "proj", &tiny).is_none());
    }
}
