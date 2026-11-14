//! `mmhmem compact`: deterministic command-output compaction CLI.
//!
//! - `mmhmem compact` reads stdin, compacts noisy build/test/diff output,
//!   and writes the result to stdout.
//! - `mmhmem compact --run <cmd>` runs `<cmd>` via the system shell, merges
//!   stdout and stderr, compacts the output, writes it to stdout, and exits
//!   with `<cmd>`'s exact exit code.

use std::io::{Read, Write};
use std::process::{Command, ExitCode, Stdio};

use crate::hook::compact::compact;

/// Entry point for `mmhmem compact`.
pub fn run_compact(args: &[String]) -> ExitCode {
    if args.len() >= 2 && args[0] == "--run" {
        return run_wrapped(&args[1]);
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

fn run_wrapped(cmd: &str) -> ExitCode {
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
            let mut stdout = std::io::stdout().lock();
            let _ = stdout.write_all(res.output.as_bytes());
            let _ = stdout.flush();
            let code = out.status.code().unwrap_or(1);
            ExitCode::from(u8::try_from(code.clamp(0, 255)).unwrap_or(1))
        }
        Err(e) => {
            eprintln!("mmhmem compact: failed to run command: {e}");
            ExitCode::FAILURE
        }
    }
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
