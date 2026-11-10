//! Read-activity counts for `mmhmem stats` (read-routing-hooks Req 1.1-1.5;
//! design.md "stats_activity").
//!
//! Counts, per day (UTC) x project x harness, how reads were served: worker
//! summaries, cache hits, exact slices, direct reads of short or
//! security-critical files, and hook redirects. They come from the audit
//! JSONL log only (the active file plus its rotated siblings), opened
//! read-only; no store, no network. Counts are plain event counts, never
//! token estimates, and they exist for days with no savings row at all.
//!
//! A redirect counts as followed (shunt-cache-efficiency Req 5.4) when any
//! `smart_read` record for the same normalised file path — summary, cache
//! hit, slice or direct read, by any agent — comes within
//! [`FOLLOW_WINDOW_MS`] after it. The rates themselves are derived at render
//! time ([`super::stats_table`]); only counts are collected.

use std::collections::{BTreeMap, HashMap};
use std::fs::File;
use std::io::{BufRead, BufReader, ErrorKind};
use std::path::Path;

use anyhow::Context;
use serde::Serialize;
use serde_json::Value;
use time::OffsetDateTime;

use crate::storage::audit_rotate::rotated_path;

/// (day, project, harness) key shared with the savings rows.
pub type ActivityKey = (String, String, String);

/// Activity per key.
pub type ActivityMap = BTreeMap<ActivityKey, ActivityCounts>;

/// Read counts of one day x project x harness. The serde names are the
/// stable `--json` contract (Req 1.3).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct ActivityCounts {
    /// Worker summaries produced (`IO_SHUNT_DELEGATED`).
    pub summary_reads: u64,
    /// Cached summaries served (`IO_SHUNT_CACHE_HIT`).
    pub cache_hits: u64,
    /// Exact line slices (`IO_TARGETED_READ`).
    pub slice_reads: u64,
    /// Whole-file reads of short or security-critical files (`IO_DIRECT_READ`).
    pub direct_reads: u64,
    /// Native reads the hook redirected to `smart_read` (`HOOK_REDIRECT`).
    pub hook_redirects: u64,
    /// Redirects followed within [`FOLLOW_WINDOW_MS`] by a `smart_read`
    /// record for the same file (shunt-cache-efficiency Req 5.4).
    pub followed_redirects: u64,
}

/// How long after a redirect a `smart_read` of the same file still counts
/// as following it: 10 minutes (design.md "Stats rates").
pub const FOLLOW_WINDOW_MS: i64 = 10 * 60 * 1000;

impl ActivityCounts {
    /// Adds another set of counts to this one.
    pub fn add(&mut self, other: &ActivityCounts) {
        self.summary_reads += other.summary_reads;
        self.cache_hits += other.cache_hits;
        self.slice_reads += other.slice_reads;
        self.direct_reads += other.direct_reads;
        self.hook_redirects += other.hook_redirects;
        self.followed_redirects += other.followed_redirects;
    }

    /// Sum of two sets of counts.
    pub fn plus(&self, other: &ActivityCounts) -> ActivityCounts {
        let mut sum = *self;
        sum.add(other);
        sum
    }

    fn bump(&mut self, route: Route) {
        match route {
            Route::Summary => self.summary_reads += 1,
            Route::CacheHit => self.cache_hits += 1,
            Route::Slice => self.slice_reads += 1,
            Route::Direct => self.direct_reads += 1,
            Route::Redirect => self.hook_redirects += 1,
        }
    }
}

/// Reads the audit log at `log_file` and its rotated siblings
/// (`.1` ... `.<keep>`) and counts the read events. Missing files are
/// skipped; unparsable lines are ignored.
pub fn collect(log_file: &Path, keep: u32) -> anyhow::Result<ActivityMap> {
    let mut collector = Collector::default();
    scan_file(log_file, &mut collector)?;
    for n in 1..=keep as usize {
        scan_file(&rotated_path(log_file, n), &mut collector)?;
    }
    Ok(collector.finish())
}

/// Counts plus what the follow-through needs: every redirect with its key,
/// time and path, and the times of the `smart_read` records per path. A
/// redirect and its read may sit in different (rotated) files, so the
/// matching waits until every file is read.
#[derive(Default)]
struct Collector {
    map: ActivityMap,
    redirects: Vec<(ActivityKey, i64, String)>,
    reads: HashMap<String, Vec<i64>>,
}

impl Collector {
    fn add(&mut self, event: Event) {
        let Event {
            key,
            route,
            epoch_ms,
            file,
        } = event;
        if let Some(file) = file {
            match route {
                Route::Redirect => self.redirects.push((key.clone(), epoch_ms, file)),
                _ => self.reads.entry(file).or_default().push(epoch_ms),
            }
        }
        self.map.entry(key).or_default().bump(route);
    }

    /// The counts with each redirect's follow-through matched.
    fn finish(mut self) -> ActivityMap {
        for times in self.reads.values_mut() {
            times.sort_unstable();
        }
        for (key, at, file) in self.redirects {
            if self.reads.get(&file).is_some_and(|t| read_within(t, at)) {
                self.map.entry(key).or_default().followed_redirects += 1;
            }
        }
        self.map
    }
}

/// Whether a read in the sorted `times` falls in `[at, at + window]`.
fn read_within(times: &[i64], at: i64) -> bool {
    let first = times.partition_point(|&t| t < at);
    times
        .get(first)
        .is_some_and(|&t| t <= at.saturating_add(FOLLOW_WINDOW_MS))
}

/// Counts one file, opened read-only; a missing file contributes nothing.
fn scan_file(path: &Path, collector: &mut Collector) -> anyhow::Result<()> {
    let file = match File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e).with_context(|| read_failure(path)),
    };
    for line in BufReader::new(file).split(b'\n') {
        let line = line.with_context(|| read_failure(path))?;
        if let Some(event) = parse_line(&line) {
            collector.add(event);
        }
    }
    Ok(())
}

fn read_failure(path: &Path) -> String {
    format!("reading the audit log {}", path.display())
}

/// The read routes the audit log records.
#[derive(Clone, Copy)]
enum Route {
    Summary,
    CacheHit,
    Slice,
    Direct,
    Redirect,
}

fn route_of(event_type: &str) -> Option<Route> {
    match event_type {
        "IO_SHUNT_DELEGATED" => Some(Route::Summary),
        "IO_SHUNT_CACHE_HIT" => Some(Route::CacheHit),
        "IO_TARGETED_READ" => Some(Route::Slice),
        "IO_DIRECT_READ" => Some(Route::Direct),
        "HOOK_REDIRECT" => Some(Route::Redirect),
        _ => None,
    }
}

/// One read event of the audit log.
struct Event {
    key: ActivityKey,
    route: Route,
    epoch_ms: i64,
    /// Normalised file path: the hook's `file_path`, the engine's `file`
    /// (absent for content the caller passed in, `shunt_content`).
    file: Option<String>,
}

/// The event of one audit line; `None` for other events and for lines that
/// are blank, garbled or lack a day, project or harness.
fn parse_line(line: &[u8]) -> Option<Event> {
    let record: Value = serde_json::from_slice(line).ok()?;
    let route = route_of(record.get("event_type")?.as_str()?)?;
    let epoch_ms = record.get("epoch_ms")?.as_i64()?;
    let day = utc_day(epoch_ms)?;
    let project = record.get("project")?.as_str()?.to_string();
    let harness = record.get("agent_harness")?.as_str()?.to_string();
    let field = match route {
        Route::Redirect => "file_path",
        _ => "file",
    };
    let file = record
        .get("details")
        .and_then(|d| d.get(field))
        .and_then(Value::as_str)
        .map(normalise_path);
    Some(Event {
        key: (day, project, harness),
        route,
        epoch_ms,
        file,
    })
}

/// The key a redirect and a later read of the same file share: the hook
/// records the working-folder-joined path, the engine the canonical one
/// (`\\?\` on Windows). The verbatim prefix goes, separators become `/`,
/// `.` and `..` resolve lexically, and case folds (Windows and macOS
/// ignore it; a rare false match elsewhere only nudges a heuristic rate).
fn normalise_path(raw: &str) -> String {
    let plain = match raw.strip_prefix(r"\\?\UNC\") {
        Some(unc) => format!("//{unc}"),
        None => raw.strip_prefix(r"\\?\").unwrap_or(raw).to_string(),
    };
    let slashed = plain.replace('\\', "/").to_lowercase();
    let mut parts: Vec<&str> = Vec::new();
    for part in slashed.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            name => parts.push(name),
        }
    }
    let joined = parts.join("/");
    if slashed.starts_with('/') {
        format!("/{joined}")
    } else {
        joined
    }
}

/// `YYYY-MM-DD` (UTC) of an epoch-millisecond timestamp: the same day the
/// savings rows use.
fn utc_day(epoch_ms: i64) -> Option<String> {
    let date = OffsetDateTime::from_unix_timestamp(epoch_ms.div_euclid(1000))
        .ok()?
        .date();
    Some(format!(
        "{:04}-{:02}-{:02}",
        date.year(),
        u8::from(date.month()),
        date.day()
    ))
}

#[cfg(test)]
#[path = "stats_activity_tests.rs"]
mod tests;
