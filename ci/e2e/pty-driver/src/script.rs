//! The driver's script: one directive per line, `#` starts a comment line.
//!
//! Set-up (before the first step):
//!
//! ```text
//! program <path>            the program to start (required)
//! arg <value>               one argument, the rest of the line verbatim
//! env <NAME>=<value>        one extra environment variable for the program
//! size <columns> <rows>     the terminal size (default 120 x 45)
//! ```
//!
//! Steps, run in order once the program is started:
//!
//! ```text
//! timeout <seconds>         the limit for each following step (default 120)
//! expect <regex>            wait until the screen matches
//! gone <regex>              wait until the screen no longer matches
//! select <regex>            press ↓ until a screen line matches (at most 40 times)
//! send <key> ...            keys: enter space esc up down tab q d y n m b,
//!                           or text:<literal characters>
//! pause <milliseconds>      wait, reading output
//! exit <code>|nonzero|any   wait for the program to end with that exit code
//! ```
//!
//! The screen is the visible terminal as a user would read it: rows joined
//! by `\n`, trailing spaces removed (a `vt100` model of the output, so a
//! full-screen program's partial redraws do not matter).

use regex::Regex;
use std::time::Duration;

/// What the program's exit code must be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExitWant {
    /// Exactly this code.
    Code(i32),
    /// Anything but 0.
    NonZero,
    /// Any code (the step only waits for the end).
    Any,
}

impl ExitWant {
    /// Whether `code` is what was wanted.
    pub fn accepts(&self, code: i32) -> bool {
        match self {
            Self::Code(want) => *want == code,
            Self::NonZero => code != 0,
            Self::Any => true,
        }
    }
}

/// One step.
#[derive(Debug, Clone)]
pub enum Step {
    /// Limit for the following steps.
    Timeout(Duration),
    /// Wait until the screen matches.
    Expect(Regex),
    /// Wait until the screen no longer matches.
    Gone(Regex),
    /// Press ↓ until a line matches.
    Select(Regex),
    /// Write these bytes to the terminal.
    Send(Vec<u8>),
    /// Wait this long.
    Pause(Duration),
    /// Wait for the end of the program.
    Exit(ExitWant),
}

/// A parsed script.
#[derive(Debug, Clone)]
pub struct Script {
    /// The program.
    pub program: String,
    /// Its arguments.
    pub args: Vec<String>,
    /// Extra environment variables.
    pub env: Vec<(String, String)>,
    /// Columns.
    pub cols: u16,
    /// Rows.
    pub rows: u16,
    /// The steps.
    pub steps: Vec<(usize, String, Step)>,
}

/// The bytes a terminal sends for a key name.
pub fn key_bytes(name: &str) -> Option<Vec<u8>> {
    if let Some(text) = name.strip_prefix("text:") {
        return Some(text.as_bytes().to_vec());
    }
    let bytes: &[u8] = match name {
        "enter" => b"\r",
        "space" => b" ",
        "esc" => b"\x1b",
        "up" => b"\x1b[A",
        "down" => b"\x1b[B",
        "tab" => b"\t",
        "q" | "d" | "y" | "n" | "m" | "b" => name.as_bytes(),
        _ => return None,
    };
    Some(bytes.to_vec())
}

fn regex(text: &str, line: usize) -> Result<Regex, String> {
    Regex::new(text).map_err(|e| format!("line {line}: bad regex: {e}"))
}

fn number<T: std::str::FromStr>(text: &str, line: usize) -> Result<T, String> {
    text.trim()
        .parse()
        .map_err(|_| format!("line {line}: not a number: {text}"))
}

fn step(word: &str, rest: &str, line: usize) -> Result<Step, String> {
    Ok(match word {
        "timeout" => Step::Timeout(Duration::from_secs(number(rest, line)?)),
        "expect" => Step::Expect(regex(rest, line)?),
        "gone" => Step::Gone(regex(rest, line)?),
        "select" => Step::Select(regex(rest, line)?),
        "pause" => Step::Pause(Duration::from_millis(number(rest, line)?)),
        "send" => {
            let mut bytes = Vec::new();
            for key in rest.split_whitespace() {
                let b = key_bytes(key).ok_or_else(|| format!("line {line}: unknown key {key}"))?;
                bytes.extend(b);
            }
            if bytes.is_empty() {
                return Err(format!("line {line}: send needs a key"));
            }
            Step::Send(bytes)
        }
        "exit" => Step::Exit(match rest.trim() {
            "nonzero" => ExitWant::NonZero,
            "any" => ExitWant::Any,
            code => ExitWant::Code(number(code, line)?),
        }),
        other => return Err(format!("line {line}: unknown directive {other}")),
    })
}

/// Parses a script.
pub fn parse(text: &str) -> Result<Script, String> {
    let mut script = Script {
        program: String::new(),
        args: Vec::new(),
        env: Vec::new(),
        cols: 120,
        rows: 45,
        steps: Vec::new(),
    };
    for (index, raw) in text.lines().enumerate() {
        let line = index + 1;
        let trimmed = raw.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let (word, rest) = trimmed.split_once(' ').unwrap_or((trimmed, ""));
        let setup = matches!(word, "program" | "arg" | "env" | "size");
        if setup && !script.steps.is_empty() {
            return Err(format!(
                "line {line}: {word} must come before the first step"
            ));
        }
        match word {
            "program" => script.program = rest.to_owned(),
            "arg" => script.args.push(rest.to_owned()),
            "env" => {
                let (name, value) = rest
                    .split_once('=')
                    .ok_or_else(|| format!("line {line}: env needs NAME=value"))?;
                script.env.push((name.to_owned(), value.to_owned()));
            }
            "size" => {
                let mut parts = rest.split_whitespace();
                script.cols = number(parts.next().unwrap_or(""), line)?;
                script.rows = number(parts.next().unwrap_or(""), line)?;
            }
            _ => script
                .steps
                .push((line, trimmed.to_owned(), step(word, rest, line)?)),
        }
    }
    if script.program.is_empty() {
        return Err("the script names no program".to_owned());
    }
    Ok(script)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_set_up_and_steps() {
        let s = parse(
            "# demo\nprogram /bin/sh\narg -c\narg echo 'a b' | cat\nenv TERM=xterm\nsize 100 30\n\
             timeout 5\nexpect Review: this\nsend down down enter text:ab\nselect > Remove\n\
             gone checking\npause 10\nexit nonzero\n",
        )
        .unwrap();
        assert_eq!(s.program, "/bin/sh");
        assert_eq!(s.args, ["-c", "echo 'a b' | cat"]);
        assert_eq!(s.env, [("TERM".to_owned(), "xterm".to_owned())]);
        assert_eq!((s.cols, s.rows), (100, 30));
        assert_eq!(s.steps.len(), 7);
        match &s.steps[2].2 {
            Step::Send(bytes) => assert_eq!(bytes, b"\x1b[B\x1b[B\rab"),
            other => panic!("{other:?}"),
        }
        match &s.steps[6].2 {
            Step::Exit(want) => {
                assert!(want.accepts(2) && !want.accepts(0));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn rejects_bad_scripts() {
        assert!(parse("expect x\n").is_err(), "no program");
        assert!(parse("program p\nsend f12\n").is_err());
        assert!(parse("program p\nexpect (\n").is_err());
        assert!(parse("program p\nexpect x\narg late\n").is_err());
        assert!(parse("program p\nfrobnicate\n").is_err());
        assert!(parse("program p\nexit maybe\n").is_err());
    }

    #[test]
    fn exit_expectations() {
        assert!(ExitWant::Code(0).accepts(0) && !ExitWant::Code(0).accepts(1));
        assert!(ExitWant::Any.accepts(5));
    }
}
