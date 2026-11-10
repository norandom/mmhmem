//! Tests for [`super`]: the range type, the capped count from a start line,
//! and the span rule per harness (shunt-cache-efficiency tasks 3.1, 3.2).

use super::*;
use crate::config::DEFAULT_SHUNT_MIN_LINES;
use crate::hook::adapters::{self, Harness};
use crate::hook::{Decision, HookSettings, PassReason, ReadRequest, decide};
use crate::services::shunt::is_security_critical_path;
use serde_json::{Value, json};
use std::path::PathBuf;

/// The shipped threshold, so the tests follow the default.
const T: u64 = DEFAULT_SHUNT_MIN_LINES as u64;

fn settings() -> HookSettings {
    HookSettings {
        shunt_min_lines: DEFAULT_SHUNT_MIN_LINES,
        worker_configured: true,
    }
}

/// A temp dir whose own path never trips the security rule.
fn plain_dir() -> tempfile::TempDir {
    loop {
        let dir = tempfile::Builder::new()
            .prefix("mmhmem-range-")
            .tempdir()
            .expect("tempdir");
        if !is_security_critical_path(dir.path()) {
            return dir;
        }
    }
}

fn text(n: u64) -> String {
    (1..=n).map(|i| format!("line {i}\n")).collect()
}

fn write(dir: &Path, name: &str, lines: u64) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, text(lines)).expect("write");
    path
}

/// `base` with the keys of the object `extra` added.
fn with(mut base: Value, extra: Value) -> Value {
    if let (Some(base), Value::Object(extra)) = (base.as_object_mut(), extra) {
        base.extend(extra);
    }
    base
}

/// A native read event of `harness` for `file` with its range keys.
fn native(harness: Harness, dir: &Path, file: &Path, range: Value) -> Value {
    match harness {
        Harness::Claude => json!({"session_id": "s", "cwd": dir, "tool_name": "Read",
            "tool_input": with(json!({"file_path": file}), range)}),
        Harness::OpenCode => json!({"tool": "read", "cwd": dir, "sessionID": "s",
            "input": with(json!({"path": file}), range)}),
        Harness::Antigravity => json!({"conversationId": "c", "workspacePaths": [dir],
            "toolCall": {"name": "read_file", "args": with(json!({"Path": file}), range)}}),
        Harness::Codex => unreachable!("Codex has no native read tool"),
    }
}

/// A shell read event of `harness` running `command` in `dir`.
fn shell(harness: Harness, dir: &Path, command: &str) -> Value {
    match harness {
        Harness::Claude | Harness::Codex => json!({"session_id": "s", "cwd": dir,
            "tool_name": "Bash", "tool_input": {"command": command}}),
        Harness::OpenCode => json!({"tool": "shell", "cwd": dir, "sessionID": "s",
            "input": {"command": command}}),
        Harness::Antigravity => unreachable!("Antigravity's shell is not a read"),
    }
}

/// The decision for one raw event, as `mmhmem hook` reaches it.
fn decide_event(harness: Harness, event: &Value) -> Decision {
    let call = adapters::parse(harness, event.to_string().as_bytes());
    let req = call
        .read
        .unwrap_or_else(|| panic!("{harness:?}: not a read: {event}"));
    decide(&req, &settings(), Path::new(&call.cwd))
}

const REDIRECT: Decision = Decision::Redirect { lines: T };
const RANGE: Decision = Decision::Pass(PassReason::Range);

#[test]
fn native_ranges_at_the_threshold_redirect_and_one_line_less_passes() {
    let dir = plain_dir();
    let d = dir.path();
    let long = write(d, "long.txt", 2 * T);
    let cases = [
        (Harness::Claude, json!({"offset": 1, "limit": T}), REDIRECT),
        (Harness::Claude, json!({"offset": 5, "limit": T - 1}), RANGE),
        (Harness::Claude, json!({"limit": 2000}), REDIRECT),
        (Harness::Claude, json!({"offset": 2}), REDIRECT),
        (Harness::OpenCode, json!({"limit": T}), REDIRECT),
        (
            Harness::OpenCode,
            json!({"offset": 10, "limit": T - 1}),
            RANGE,
        ),
        (Harness::OpenCode, json!({"offset": 2}), REDIRECT),
        (
            Harness::Antigravity,
            json!({"StartLine": 1, "EndLine": T}),
            REDIRECT,
        ),
        (
            Harness::Antigravity,
            json!({"StartLine": 2, "EndLine": T}),
            RANGE,
        ),
        (Harness::Antigravity, json!({"StartLine": 2}), REDIRECT),
        (Harness::Antigravity, json!({"EndLine": T - 1}), RANGE),
    ];
    for (harness, range, want) in cases {
        let event = native(harness, d, &long, range);
        assert_eq!(decide_event(harness, &event), want, "{harness:?}: {event}");
    }
}

#[test]
fn shell_ranges_at_the_threshold_redirect_and_short_tails_pass() {
    let dir = plain_dir();
    let d = dir.path();
    let _ = write(d, "long.txt", 2 * T);
    let cases = [
        (format!("sed -n 1,{T}p long.txt"), REDIRECT),
        (format!("sed -n '2,{T}p' long.txt"), RANGE),
        (format!("head -n {T} long.txt"), REDIRECT),
        (format!("head -{} long.txt", T - 1), RANGE),
        ("tail -n +2 long.txt".to_string(), REDIRECT),
        ("tail -n 50 long.txt".to_string(), RANGE),
        (format!("tail -n {T} long.txt"), REDIRECT),
        ("cat long.txt".to_string(), REDIRECT),
    ];
    for harness in [Harness::Claude, Harness::Codex, Harness::OpenCode] {
        for (command, want) in &cases {
            let event = shell(harness, d, command);
            assert_eq!(
                decide_event(harness, &event),
                *want,
                "{harness:?}: {command}"
            );
        }
    }
}

fn request(path: &Path, range: LineRange) -> ReadRequest {
    ReadRequest {
        path: path.to_path_buf(),
        range: Some(range),
        tool: "Read".to_string(),
    }
}

#[test]
fn open_ended_ranges_count_the_lines_left_from_their_start() {
    let dir = plain_dir();
    let long = write(dir.path(), "long.txt", 2 * T);
    let run = |start: u64| {
        decide(
            &request(&long, LineRange::new(start, None)),
            &settings(),
            dir.path(),
        )
    };
    assert_eq!(run(1), REDIRECT);
    assert_eq!(run(T + 1), REDIRECT, "exactly T lines left");
    assert_eq!(run(T + 2), RANGE, "T - 1 lines left");
    assert_eq!(run(10 * T), RANGE, "start past the end");
    // A small file read to the end passes as a range, not as short.
    let short = write(dir.path(), "short.txt", 5);
    let open = request(&short, LineRange::new(1, None));
    assert_eq!(decide(&open, &settings(), dir.path()), RANGE);
}

#[test]
fn large_ranges_follow_the_whole_file_rules_and_errors_fail_open() {
    let dir = plain_dir();
    let d = dir.path();
    let missing = d.join("nope.txt");
    let short = write(d, "short.txt", 5);
    let secret = write(d, "auth_secret.txt", 2 * T);
    let run = |path: &Path, range: LineRange| decide(&request(path, range), &settings(), d);
    let big = LineRange::new(1, Some(T));
    assert_eq!(run(&missing, LineRange::new(1, Some(20))), RANGE);
    assert_eq!(run(&missing, big), Decision::Pass(PassReason::Missing));
    assert_eq!(
        run(&missing, LineRange::new(2, None)),
        Decision::Pass(PassReason::Missing)
    );
    assert_eq!(
        run(d, LineRange::new(2, None)),
        Decision::Pass(PassReason::Missing)
    );
    assert_eq!(run(&short, big), Decision::Pass(PassReason::Short));
    assert_eq!(
        run(&short, LineRange::from_tail(T)),
        Decision::Pass(PassReason::Short)
    );
    let secret_pass = Decision::Pass(PassReason::SecurityCritical);
    assert_eq!(run(&secret, big), secret_pass);
    let no_worker = HookSettings {
        worker_configured: false,
        ..settings()
    };
    let long = write(d, "long.txt", 2 * T);
    let req = request(&long, LineRange::new(1, None));
    assert_eq!(
        decide(&req, &no_worker, d),
        Decision::Pass(PassReason::NoWorker)
    );
}

#[test]
fn offset_and_limit_become_an_inclusive_range() {
    let r = |offset, limit| LineRange::from_offset_limit(offset, limit);
    assert_eq!(r(None, None), None);
    assert_eq!(r(Some(10), Some(20)), Some(LineRange::new(10, Some(29))));
    assert_eq!(r(Some(5), None), Some(LineRange::new(5, None)));
    assert_eq!(r(None, Some(7)), Some(LineRange::new(1, Some(7))));
    assert_eq!(r(Some(0), Some(3)), Some(LineRange::new(1, Some(3))));
    assert_eq!(r(Some(4), Some(0)), Some(LineRange::new(4, None)));
    let max = Some(u64::MAX);
    assert_eq!(r(max, max), Some(LineRange::new(u64::MAX, max)));
}

#[test]
fn spans_are_known_for_closed_and_tail_ranges_only() {
    assert_eq!(
        LineRange::new(0, Some(5)).start,
        1,
        "new never builds a tail"
    );
    assert_eq!(LineRange::new(1, Some(T)).span_lines(), Some(T));
    assert_eq!(LineRange::new(5, Some(5)).span_lines(), Some(1));
    assert_eq!(LineRange::new(10, Some(9)).span_lines(), Some(0));
    assert_eq!(LineRange::new(10, None).span_lines(), None);
    let tail = LineRange::from_tail(50);
    assert!(tail.is_tail() && !LineRange::new(1, Some(50)).is_tail());
    assert_eq!(
        (tail.start, tail.end, tail.span_lines()),
        (0, Some(50), Some(50))
    );
}

#[test]
fn scan_from_counts_the_lines_left_capped_at_the_threshold() {
    let ten = text(10).into_bytes();
    let count = |bytes: &[u8], start, cap| scan_from(bytes, start, cap).expect("scan");
    assert_eq!(count(&ten, 1, 100), 10);
    assert_eq!(count(&ten, 0, 100), 10, "start 0 counts from the top");
    assert_eq!(count(&ten, 4, 100), 7);
    assert_eq!(count(&ten, 10, 100), 1);
    assert_eq!(count(&ten, 11, 100), 0);
    assert_eq!(count(&ten, 1, 5), 5);
    assert_eq!(count(&ten, 8, 5), 3);
    assert_eq!(
        count(b"a\nb\nlast", 3, 100),
        1,
        "an unterminated last line counts"
    );
    assert_eq!(count(b"", 1, 100), 0);
    assert_eq!(count(b"x\r\ny\r\n", 1, 100), 2);
}

#[test]
fn span_at_or_above_reads_only_for_open_ended_ranges() {
    let dir = plain_dir();
    let missing = dir.path().join("nope.txt");
    // Known spans need no file at all.
    assert!(span_at_or_above(&LineRange::new(1, Some(T)), &missing, T).expect("known"));
    assert!(!span_at_or_above(&LineRange::from_tail(T - 1), &missing, T).expect("tail"));
    assert!(span_at_or_above(&LineRange::new(2, None), &missing, T).is_err());
    let long = write(dir.path(), "long.txt", T + 1);
    assert!(span_at_or_above(&LineRange::new(2, None), &long, T).expect("open"));
    assert!(!span_at_or_above(&LineRange::new(3, None), &long, T).expect("open"));
}
