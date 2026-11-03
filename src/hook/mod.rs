//! Read-routing hook decision core (read-routing-hooks design.md
//! "decide (`src/hook/mod.rs`)", Req 2.1-2.4, 3.3).
//!
//! `decide` is a pure function over one read request: it touches only the
//! target file's metadata and a bounded prefix of its bytes, never the
//! network, configuration or secrets (the caller passes [`HookSettings`]).

use std::path::PathBuf;

/// One native read request, as mapped from a harness tool call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadRequest {
    /// Resolved absolute path of the file to read.
    pub path: PathBuf,
    /// `(offset, limit)` when the call asks for an explicit line range.
    pub range: Option<(u64, u64)>,
    /// Harness tool name, kept for the audit record.
    pub tool: String,
}

/// The local settings the decision depends on (loaded by the caller).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HookSettings {
    /// Line threshold from the local profile (server `SHUNT_MIN_LINES`).
    pub shunt_min_lines: u32,
    /// Whether a worker model is configured (env or `.mmhmem.json`).
    pub worker_configured: bool,
}

/// Why a read is let through unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PassReason {
    /// The call asked for an explicit line range (Req 2.2).
    Range,
    /// The file has fewer lines than the threshold (Req 2.3).
    Short,
    /// The path matches the shunt engine's security rule (Req 2.3).
    SecurityCritical,
    /// The file is not text: a NUL byte in its first 8 KiB (Req 2.3).
    NotText,
    /// The path does not exist or is not a regular file (Req 2.3).
    Missing,
    /// No worker model is configured, so no summary can exist (Req 2.4).
    NoWorker,
    /// A repeat of a recently redirected read (Req 2.5; decided by the
    /// repeat-window state, never by [`decide`]).
    Repeat,
    /// The tool call is not a whole-file read (decided by the adapters and
    /// shell recognition, never by [`decide`]).
    NotARead,
}

/// The hook's answer for one read request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// Let the read proceed.
    Pass(PassReason),
    /// Decline the read and point the agent at `smart_read`.
    Redirect {
        /// Line count, capped at `shunt_min_lines`: counting stops at the
        /// threshold, so this means "at least this many", not the exact total.
        lines: u64,
    },
}

/// Bytes inspected for a NUL to classify a file as not text.
const TEXT_PROBE_BYTES: usize = 8 * 1024;
/// Read buffer for the streaming line count.
const READ_CHUNK_BYTES: usize = 64 * 1024;

/// Decides one read request (design.md order: range, missing/not regular,
/// security rule, no worker, not text, below threshold, redirect).
///
/// A file that cannot be opened or read after its metadata check passes as
/// [`PassReason::Missing`], so the hook fails open (Req 3.1).
pub fn decide(req: &ReadRequest, settings: &HookSettings) -> Decision {
    if req.range.is_some() {
        return Decision::Pass(PassReason::Range);
    }
    if !std::fs::metadata(&req.path).is_ok_and(|m| m.is_file()) {
        return Decision::Pass(PassReason::Missing);
    }
    if crate::services::shunt::is_security_critical_path(&req.path) {
        return Decision::Pass(PassReason::SecurityCritical);
    }
    if !settings.worker_configured {
        return Decision::Pass(PassReason::NoWorker);
    }
    let threshold = u64::from(settings.shunt_min_lines);
    match scan(&req.path, threshold) {
        Err(_) => Decision::Pass(PassReason::Missing),
        Ok(Scan::NotText) => Decision::Pass(PassReason::NotText),
        Ok(Scan::Lines(n)) if n < threshold => Decision::Pass(PassReason::Short),
        Ok(Scan::Lines(lines)) => Decision::Redirect { lines },
    }
}

/// Outcome of one streaming pass over a file.
enum Scan {
    NotText,
    /// Line count, capped at the threshold (counting stops there).
    Lines(u64),
}

/// Streams the file once: a NUL or invalid UTF-8 in the first
/// [`TEXT_PROBE_BYTES`] means not text; otherwise counts lines with the shunt engine's `str::lines`
/// semantics (each `\n` ends a line, and a final unterminated line counts),
/// stopping as soon as the count reaches `threshold`.
fn scan(path: &std::path::Path, threshold: u64) -> std::io::Result<Scan> {
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut probe = Vec::with_capacity(TEXT_PROBE_BYTES);
    (&mut file)
        .take(TEXT_PROBE_BYTES as u64)
        .read_to_end(&mut probe)?;
    // smart_read reads with read_to_string, so invalid UTF-8 cannot be
    // summarised; a sequence merely cut at the probe's end is still text.
    let invalid_utf8 = std::str::from_utf8(&probe).is_err_and(|e| e.error_len().is_some());
    if probe.contains(&0) || invalid_utf8 {
        return Ok(Scan::NotText);
    }
    let mut counter = LineCounter::default();
    counter.feed(&probe);
    let mut buf = vec![0u8; READ_CHUNK_BYTES];
    while counter.newlines < threshold {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        counter.feed(&buf[..n]);
    }
    // Capped so the reported count is deterministic ("at least threshold")
    // regardless of how many lines the last buffered chunk happened to hold.
    Ok(Scan::Lines(counter.total().min(threshold)))
}

/// Running `\n` count plus whether the last byte seen ended a line.
#[derive(Default)]
struct LineCounter {
    newlines: u64,
    open_line: bool,
}

impl LineCounter {
    fn feed(&mut self, bytes: &[u8]) {
        if let Some(&last) = bytes.last() {
            self.newlines += count_newlines(bytes);
            self.open_line = last != b'\n';
        }
    }

    /// Lines as `str::lines` counts them for the bytes fed so far.
    fn total(&self) -> u64 {
        self.newlines + u64::from(self.open_line)
    }
}

/// Counts `\n` bytes eight at a time (SWAR zero-byte count over `word ^
/// 0x0a..`), keeping the 10 MiB worst case fast even in debug builds.
fn count_newlines(bytes: &[u8]) -> u64 {
    const LO7: u64 = 0x7f7f_7f7f_7f7f_7f7f;
    const NL: u64 = 0x0a0a_0a0a_0a0a_0a0a;
    let (words, tail) = bytes.as_chunks::<8>();
    let mut count = 0u64;
    for word in words {
        let x = u64::from_ne_bytes(*word) ^ NL;
        let nonzero_high = ((x & LO7) + LO7) | x;
        count += u64::from((!nonzero_high & !LO7).count_ones());
    }
    count + tail.iter().filter(|&&b| b == b'\n').count() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::shunt::is_security_critical_path;
    use std::path::Path;
    use std::time::{Duration, Instant};

    const THRESHOLD: u32 = 150;

    fn settings(worker: bool) -> HookSettings {
        HookSettings {
            shunt_min_lines: THRESHOLD,
            worker_configured: worker,
        }
    }

    /// A temp dir whose own path never trips the security rule (tempfile's
    /// random suffix could otherwise spell e.g. `jwt` and flake the tests).
    fn plain_dir() -> tempfile::TempDir {
        loop {
            let dir = tempfile::Builder::new()
                .prefix("mmhmem-hook-")
                .tempdir()
                .expect("tempdir");
            if !is_security_critical_path(dir.path()) {
                return dir;
            }
        }
    }

    fn write(dir: &Path, rel: &str, bytes: &[u8]) -> PathBuf {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("mkdir");
        }
        std::fs::write(&path, bytes).expect("write");
        path
    }

    fn text_lines(n: usize) -> Vec<u8> {
        (0..n)
            .map(|i| format!("line {i}\n"))
            .collect::<String>()
            .into_bytes()
    }

    fn run(path: &Path, range: Option<(u64, u64)>, worker: bool) -> Decision {
        let req = ReadRequest {
            path: path.to_path_buf(),
            range,
            tool: "Read".to_string(),
        };
        decide(&req, &settings(worker))
    }

    #[test]
    fn decision_matrix_covers_every_reason_and_redirect() {
        let dir = plain_dir();
        let d = dir.path();
        let big = write(d, "src/big.rs", &text_lines(400));
        let short = write(d, "src/short.rs", &text_lines(10));
        let mut bin = vec![b'x'; 100];
        bin.push(0);
        bin.extend(text_lines(400));
        let bin = write(d, "assets/blob.bin", &bin);
        let secret = write(d, "auth/secret_store.rs", &text_lines(5000));
        let missing = d.join("nope.rs");
        let p = Decision::Pass;
        assert_eq!(run(&big, Some((1, 20)), true), p(PassReason::Range));
        assert_eq!(run(&missing, Some((1, 20)), true), p(PassReason::Range));
        assert_eq!(run(&missing, None, true), p(PassReason::Missing));
        assert_eq!(run(d, None, true), p(PassReason::Missing));
        assert_eq!(run(&secret, None, true), p(PassReason::SecurityCritical));
        assert_eq!(run(&secret, None, false), p(PassReason::SecurityCritical));
        assert_eq!(run(&big, None, false), p(PassReason::NoWorker));
        assert_eq!(run(&bin, None, false), p(PassReason::NoWorker));
        assert_eq!(run(&bin, None, true), p(PassReason::NotText));
        assert_eq!(run(&short, None, true), p(PassReason::Short));
        let lines = u64::from(THRESHOLD);
        assert_eq!(run(&big, None, true), Decision::Redirect { lines });
    }

    #[test]
    fn threshold_boundary_matches_server_line_semantics() {
        let dir = plain_dir();
        let d = dir.path();
        let n = THRESHOLD as usize;
        let below = write(d, "below.txt", &text_lines(n - 1));
        let at = write(d, "at.txt", &text_lines(n));
        assert_eq!(run(&below, None, true), Decision::Pass(PassReason::Short));
        assert!(matches!(run(&at, None, true), Decision::Redirect { .. }));
        // A final line without a trailing newline counts, as `str::lines`
        // does in the shunt engine.
        let mut open = text_lines(n - 1);
        open.extend_from_slice(b"last line, no newline");
        let open = write(d, "open.txt", &open);
        assert!(matches!(run(&open, None, true), Decision::Redirect { .. }));
        // CRLF endings count once per line, like `str::lines`.
        let crlf: Vec<u8> = (0..n).flat_map(|_| b"x\r\n".to_vec()).collect();
        let crlf = write(d, "crlf.txt", &crlf);
        assert!(matches!(run(&crlf, None, true), Decision::Redirect { .. }));
        let crlf_short: Vec<u8> = (0..n - 1).flat_map(|_| b"x\r\n".to_vec()).collect();
        let crlf_short = write(d, "crlf_short.txt", &crlf_short);
        let short = Decision::Pass(PassReason::Short);
        assert_eq!(run(&crlf_short, None, true), short);
        let empty = write(d, "empty.txt", b"");
        assert_eq!(run(&empty, None, true), short);
    }

    #[test]
    fn nul_only_in_the_first_8_kib_marks_not_text() {
        let dir = plain_dir();
        let mut late = text_lines(2000);
        assert!(late.len() > 8 * 1024);
        late.push(0);
        let late = write(dir.path(), "late_nul.txt", &late);
        assert!(matches!(run(&late, None, true), Decision::Redirect { .. }));
        let mut early = vec![b'a'; 8 * 1024 - 1];
        early.push(0);
        early.extend(text_lines(400));
        let early = write(dir.path(), "early_nul.txt", &early);
        let not_text = Decision::Pass(PassReason::NotText);
        assert_eq!(run(&early, None, true), not_text);
    }

    #[test]
    fn invalid_utf8_in_the_probe_is_not_text_but_a_split_char_is() {
        // smart_read reads with read_to_string: a Latin-1 file would fail there,
        // so it must not be redirected (design Key Decision 4).
        let dir = plain_dir();
        let mut latin1 = text_lines(400);
        latin1.extend_from_slice(b"caf\xe9\n");
        let latin1 = write(dir.path(), "latin1.txt", &latin1);
        assert_eq!(
            run(&latin1, None, true),
            Decision::Pass(PassReason::NotText)
        );
        // A multi-byte character cut by the 8 KiB probe edge is still text.
        let mut split = vec![b'a'; 8 * 1024 - 1];
        split.extend_from_slice("é\n".as_bytes());
        split.extend(text_lines(400));
        let split = write(dir.path(), "split.txt", &split);
        assert!(matches!(run(&split, None, true), Decision::Redirect { .. }));
    }

    #[test]
    fn swar_newline_count_matches_a_naive_count() {
        let bytes: Vec<u8> = (0..4099u32).map(|i| (i * 37 % 256) as u8).collect();
        for start in 0..9 {
            let slice = &bytes[start..];
            let naive = slice.iter().filter(|&&b| b == b'\n').count() as u64;
            assert_eq!(count_newlines(slice), naive);
        }
        let all = [b'\n'; 17];
        assert_eq!(count_newlines(&all), 17);
        let near = [0x0b, 0x8a, 0x0a, 0xff, 0x00, 0x0a, 0x09, 0x0a, 0x8a];
        assert_eq!(count_newlines(&near), 3);
    }

    /// Req 3.3: a 10 MiB file is decided in < 200 ms. Two shapes: many short
    /// lines (early stop at the threshold) and a few very long lines (a full
    /// scan, the worst case). The bound is asserted in debug builds too.
    #[test]
    fn ten_mib_file_is_decided_under_200_ms() {
        let dir = plain_dir();
        let size = 10 * 1024 * 1024;
        let mut many = text_lines(size / 9);
        many.resize(size, b'y');
        let many = write(dir.path(), "many.txt", &many);
        let mut few = Vec::with_capacity(size);
        for _ in 0..100 {
            few.resize(few.len() + size / 100 - 1, b'z');
            few.push(b'\n');
        }
        let few = write(dir.path(), "few.txt", &few);
        for (path, redirect) in [(many, true), (few, false)] {
            let start = Instant::now();
            let decision = run(&path, None, true);
            let took = start.elapsed();
            assert_eq!(matches!(decision, Decision::Redirect { .. }), redirect);
            assert!(took < Duration::from_millis(200), "{path:?} took {took:?}");
        }
    }
}
