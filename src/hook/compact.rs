//! Deterministic, zero-latency command output compaction (`mmhmem compact`).
//!
//! Strips ANSI escape sequences, collapses repetitive build/test/lockfile
//! spam while preserving errors, warnings, and summaries, and bounds
//! oversized output without touching the user's interactive shell.

/// Result of compacting one command's output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactResult {
    pub output: String,
    pub raw_lines: usize,
    pub kept_lines: usize,
    pub raw_chars: usize,
    pub kept_chars: usize,
}

const MAX_OUTPUT_LINES: usize = 120;
const HEAD_KEEP_LINES: usize = 40;
const TAIL_KEEP_LINES: usize = 60;

/// Compacts `raw` command output deterministically.
pub fn compact(raw: &str, _command: Option<&str>) -> CompactResult {
    let raw_chars = raw.len();
    let cleaned = strip_ansi(raw);
    let raw_lines: Vec<&str> = cleaned.lines().collect();
    let raw_line_count = raw_lines.len();

    let collapsed = collapse_noise(&raw_lines);
    let deduped = dedupe_consecutive(&collapsed);
    let bounded = bound_lines(&deduped);

    let mut output = bounded.join("\n");
    if raw.ends_with('\n') && !output.is_empty() {
        output.push('\n');
    }
    let kept_lines = bounded.len();
    let kept_chars = output.len();
    CompactResult {
        output,
        raw_lines: raw_line_count,
        kept_lines,
        raw_chars,
        kept_chars,
    }
}

/// Returns true when `cmd` is a known noisy command whose output benefits from
/// compaction and is not already bounded or piped to `mmhmem compact`.
pub fn should_compact_command(cmd: &str) -> bool {
    let trimmed = cmd.trim();
    if trimmed.is_empty() || trimmed.contains("mmhmem compact") {
        return false;
    }
    let lower = trimmed.to_ascii_lowercase();
    if is_already_bounded(&lower) {
        return false;
    }
    const NOISY_PREFIXES: &[&str] = &[
        "cargo test",
        "cargo build",
        "cargo check",
        "cargo clippy",
        "npm test",
        "pnpm test",
        "yarn test",
        "pytest",
        "jest",
        "vitest",
        "git diff",
        "git log",
        "docker logs",
        "kubectl logs",
        "make",
        "gradle",
        "mvn",
    ];
    NOISY_PREFIXES.iter().any(|p| {
        lower == *p
            || lower.starts_with(&format!("{p} "))
            || lower.contains(&format!("&& {p}"))
            || lower.contains(&format!("; {p}"))
    })
}

fn is_already_bounded(lower: &str) -> bool {
    if lower.starts_with("git log")
        && (lower.contains(" -n")
            || lower.contains("--max-count")
            || lower.contains("--oneline")
            || lower.contains("--stat"))
    {
        return true;
    }
    if lower.starts_with("cargo ") && (lower.contains(" -q") || lower.contains(" --quiet")) {
        return true;
    }
    if (lower.starts_with("docker logs") || lower.starts_with("kubectl logs"))
        && lower.contains("--tail")
    {
        return true;
    }
    false
}

/// Strips ANSI CSI (`\x1b[...<letter>`) and OSC (`\x1b]...(\x07|\x1b\\)`) sequences.
fn strip_ansi(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == 0x1b {
            if i + 1 < bytes.len() && bytes[i + 1] == b'[' {
                i += 2;
                while i < bytes.len() && !(0x40..=0x7e).contains(&bytes[i]) {
                    i += 1;
                }
                if i < bytes.len() {
                    i += 1;
                }
                continue;
            } else if i + 1 < bytes.len() && bytes[i + 1] == b']' {
                i += 2;
                while i < bytes.len() {
                    if bytes[i] == 0x07 {
                        i += 1;
                        break;
                    }
                    if bytes[i] == 0x1b && i + 1 < bytes.len() && bytes[i + 1] == b'\\' {
                        i += 2;
                        break;
                    }
                    i += 1;
                }
                continue;
            }
        }
        let ch = input[i..].chars().next().unwrap();
        if ch != '\r' {
            out.push(ch);
        }
        i += ch.len_utf8();
    }
    out
}

fn is_important_line(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    lower.contains("error")
        || lower.contains("failed")
        || lower.contains("failures:")
        || lower.contains("panic")
        || lower.contains("warning:")
        || lower.contains("exception")
        || lower.contains("traceback")
        || lower.starts_with("test result:")
}

fn is_build_progress(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with("Compiling ")
        || t.starts_with("Downloading ")
        || t.starts_with("Downloaded ")
        || t.starts_with("Fetching ")
        || t.starts_with("Updating ")
        || t.starts_with("Fresh ")
        || t.starts_with("Checking ")
}

fn is_passing_test_line(line: &str) -> bool {
    let t = line.trim_start();
    (t.starts_with("test ") && t.ends_with("... ok"))
        || t.starts_with("PASS ")
        || t.starts_with("✓ ")
}

fn is_lockfile_diff_header(line: &str) -> bool {
    if !line.starts_with("diff --git ") {
        return false;
    }
    const LOCKFILES: &[&str] = &[
        "Cargo.lock",
        "package-lock.json",
        "pnpm-lock.yaml",
        "yarn.lock",
        "uv.lock",
        "poetry.lock",
    ];
    LOCKFILES.iter().any(|lf| line.contains(lf))
}

fn collapse_noise(lines: &[&str]) -> Vec<String> {
    let mut out = Vec::with_capacity(lines.len());
    let mut build_count = 0usize;
    let mut pass_count = 0usize;
    let mut in_lockfile = false;
    let mut lockfile_lines = 0usize;

    let flush_build = |out: &mut Vec<String>, count: &mut usize| {
        if *count > 0 {
            out.push(format!("[compacted: {} build/fetch lines]", *count));
            *count = 0;
        }
    };
    let flush_pass = |out: &mut Vec<String>, count: &mut usize| {
        if *count > 0 {
            out.push(format!("[compacted: {} passing test lines]", *count));
            *count = 0;
        }
    };
    let flush_lockfile = |out: &mut Vec<String>, in_lf: &mut bool, count: &mut usize| {
        if *in_lf {
            out.push(format!("[compacted: {} lockfile diff lines]", *count));
            *in_lf = false;
            *count = 0;
        }
    };

    for &line in lines {
        if line.starts_with("diff --git ") {
            flush_lockfile(&mut out, &mut in_lockfile, &mut lockfile_lines);
            if is_lockfile_diff_header(line) {
                flush_build(&mut out, &mut build_count);
                flush_pass(&mut out, &mut pass_count);
                out.push(line.to_owned());
                in_lockfile = true;
                continue;
            }
        }
        if in_lockfile {
            lockfile_lines += 1;
            continue;
        }
        if !is_important_line(line) && is_build_progress(line) {
            flush_pass(&mut out, &mut pass_count);
            build_count += 1;
            continue;
        }
        if !is_important_line(line) && is_passing_test_line(line) {
            flush_build(&mut out, &mut build_count);
            pass_count += 1;
            continue;
        }
        flush_build(&mut out, &mut build_count);
        flush_pass(&mut out, &mut pass_count);
        out.push(line.to_owned());
    }
    flush_lockfile(&mut out, &mut in_lockfile, &mut lockfile_lines);
    flush_build(&mut out, &mut build_count);
    flush_pass(&mut out, &mut pass_count);
    out
}

fn dedupe_consecutive(lines: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    let mut prev: Option<&str> = None;
    let mut count = 0usize;
    let flush = |out: &mut Vec<String>, prev: Option<&str>, count: usize| {
        if let Some(p) = prev {
            if count > 1 && !p.trim().is_empty() {
                out.push(format!("{p} (x{count})"));
            } else {
                for _ in 0..count {
                    out.push(p.to_owned());
                }
            }
        }
    };
    for line in lines {
        if prev == Some(line.as_str()) {
            count += 1;
        } else {
            flush(&mut out, prev, count);
            prev = Some(line.as_str());
            count = 1;
        }
    }
    flush(&mut out, prev, count);
    out
}

fn bound_lines(lines: &[String]) -> Vec<String> {
    if lines.len() <= MAX_OUTPUT_LINES {
        return lines.to_vec();
    }
    let mut out = Vec::with_capacity(HEAD_KEEP_LINES + TAIL_KEEP_LINES + 16);
    out.extend_from_slice(&lines[..HEAD_KEEP_LINES]);
    let mid_end = lines.len() - TAIL_KEEP_LINES;
    let mut omitted = 0usize;
    for line in &lines[HEAD_KEEP_LINES..mid_end] {
        if is_important_line(line) {
            if omitted > 0 {
                out.push(format!("[... {omitted} lines omitted ...]"));
                omitted = 0;
            }
            out.push(line.clone());
        } else {
            omitted += 1;
        }
    }
    if omitted > 0 {
        out.push(format!("[... {omitted} lines omitted ...]"));
    }
    out.extend_from_slice(&lines[mid_end..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_ansi_and_collapses_build_and_passing_tests() {
        let raw = "\x1b[32m   Compiling\x1b[0m foo v0.1.0\n\
                   \x1b[32m   Compiling\x1b[0m bar v0.2.0\n\
                   test a::one ... ok\n\
                   test a::two ... ok\n\
                   test a::fail ... FAILED\n\
                   test result: FAILED. 2 passed; 1 failed\n";
        let res = compact(raw, Some("cargo test"));
        assert!(res.output.contains("[compacted: 2 build/fetch lines]"));
        assert!(res.output.contains("[compacted: 2 passing test lines]"));
        assert!(res.output.contains("test a::fail ... FAILED"));
        assert!(res.output.contains("test result: FAILED"));
        assert!(!res.output.contains("\x1b["));
    }

    #[test]
    fn collapses_lockfile_diffs_while_keeping_code_diffs() {
        let raw = "diff --git a/src/lib.rs b/src/lib.rs\n+pub fn hi() {}\n\
                   diff --git a/Cargo.lock b/Cargo.lock\n+name = \"a\"\n+version = \"1\"\n\
                   diff --git a/src/main.rs b/src/main.rs\n+fn main() {}\n";
        let res = compact(raw, Some("git diff"));
        assert!(res.output.contains("+pub fn hi() {}"));
        assert!(res.output.contains("[compacted: 2 lockfile diff lines]"));
        assert!(res.output.contains("+fn main() {}"));
        assert!(!res.output.contains("name = \"a\""));
    }

    #[test]
    fn bounds_large_output_while_preserving_errors() {
        let mut lines = Vec::new();
        for i in 0..200 {
            if i == 100 {
                lines.push("error[E0425]: cannot find value `x`".to_owned());
            } else {
                lines.push(format!("line {i}"));
            }
        }
        let raw = lines.join("\n");
        let res = compact(&raw, None);
        assert!(res.output.contains("error[E0425]: cannot find value `x`"));
        assert!(res.output.contains("lines omitted"));
        assert!(res.kept_lines < 120);
    }

    #[test]
    fn detects_noisy_commands_and_skips_already_bounded() {
        assert!(should_compact_command("cargo test"));
        assert!(should_compact_command("git diff HEAD~1"));
        assert!(!should_compact_command("git log -n 10 --oneline"));
        assert!(!should_compact_command("cargo test -q"));
        assert!(!should_compact_command("cargo test | mmhmem compact"));
    }
}
