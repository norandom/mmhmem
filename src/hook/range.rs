//! Line ranges of native reads and the span rule (shunt-cache-efficiency
//! design.md "LineRange and span rule (`hook`)", "Hook range rule"; Req
//! 3.1-3.3, 3.5).
//!
//! Every adapter turns its harness's own range fields into one
//! [`LineRange`]; [`super::decide`] sizes it with [`span_at_or_above`]: a
//! range of at least the threshold, or an open-ended one with at least that
//! many lines left, is judged like a whole-file read; a smaller one passes.

use std::io::Read;
use std::path::Path;

use super::{LineCounter, READ_CHUNK_BYTES};

/// The lines one read asks for, 1-based and inclusive; `end: None` reads to
/// the end of the file.
///
/// `start: 0` is otherwise invalid and marks a tail read: `end` then holds
/// the number of last lines (`tail -n N`, [`LineRange::from_tail`]), whose
/// position is unknown without reading the file but whose size is not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineRange {
    /// First line (1-based), or 0 for a tail read.
    pub start: u64,
    /// Last line (inclusive), `None` for "to the end of the file"; for a
    /// tail read, the number of last lines.
    pub end: Option<u64>,
}

impl LineRange {
    /// Lines `start..=end` (`end: None`: to the end); a start of 0 counts
    /// as 1, so this never builds a tail read.
    pub fn new(start: u64, end: Option<u64>) -> Self {
        Self {
            start: start.max(1),
            end,
        }
    }

    /// The range of `offset`/`limit` as Claude Code's `Read` and OpenCode's
    /// `read` take them; `None` when both are absent. `start = max(offset,
    /// 1)`; a positive `limit` ends the range at `start + limit - 1`, an
    /// absent or zero one reads to the end.
    pub fn from_offset_limit(offset: Option<u64>, limit: Option<u64>) -> Option<Self> {
        if offset.is_none() && limit.is_none() {
            return None;
        }
        let start = offset.unwrap_or(1).max(1);
        let end = limit
            .filter(|&n| n > 0)
            .map(|n| start.saturating_add(n - 1));
        Some(Self { start, end })
    }

    /// The last `n` lines of the file (`tail -n n`).
    pub fn from_tail(n: u64) -> Self {
        Self {
            start: 0,
            end: Some(n),
        }
    }

    /// Whether this is a [`LineRange::from_tail`] range.
    pub fn is_tail(&self) -> bool {
        self.start == 0 && self.end.is_some()
    }

    /// The number of lines asked for when it is known without reading the
    /// file: `end - start + 1` (0 when `end` is before `start`), `n` for a
    /// tail read, `None` for an open-ended range.
    pub fn span_lines(&self) -> Option<u64> {
        let end = self.end?;
        if self.start == 0 {
            return Some(end);
        }
        Some(
            end.checked_sub(self.start)
                .map_or(0, |d| d.saturating_add(1)),
        )
    }
}

/// Whether `range` asks for at least `threshold` lines of `path`: a known
/// span is compared as is; an open-ended range counts the lines left from
/// its start, reading no further than `start - 1 + threshold` lines. An I/O
/// error is returned so the caller can fail open.
pub fn span_at_or_above(range: &LineRange, path: &Path, threshold: u64) -> std::io::Result<bool> {
    let span = match range.span_lines() {
        Some(span) => span,
        None => scan_from(std::fs::File::open(path)?, range.start, threshold)?,
    };
    Ok(span >= threshold)
}

/// Lines from the 1-based `start` to the end of `file`, as `str::lines`
/// counts them, capped at `threshold` (counting stops there).
fn scan_from(mut file: impl Read, start: u64, threshold: u64) -> std::io::Result<u64> {
    let skipped = start.saturating_sub(1);
    let cap = skipped.saturating_add(threshold);
    let mut counter = LineCounter::default();
    let mut buf = vec![0u8; READ_CHUNK_BYTES];
    while counter.newlines < cap {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        counter.feed(&buf[..n]);
    }
    Ok(counter.total().saturating_sub(skipped).min(threshold))
}

#[cfg(test)]
#[path = "range_tests.rs"]
mod tests;
