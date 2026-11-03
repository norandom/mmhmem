//! Read-activity counts for `mmhmem stats` (read-routing-hooks Req 1.1-1.5;
//! design.md "stats_activity").
//!
//! Counts, per day (UTC) x project x harness, how reads were served: worker
//! summaries, cache hits, exact slices, direct reads of short or
//! security-critical files, and hook redirects. They come from the audit
//! JSONL log only (the active file plus its rotated siblings), opened
//! read-only; no store, no network. Counts are plain event counts, never
//! token estimates, and they exist for days with no savings row at all.

use std::collections::BTreeMap;
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
}

impl ActivityCounts {
    /// Adds another set of counts to this one.
    pub fn add(&mut self, other: &ActivityCounts) {
        self.summary_reads += other.summary_reads;
        self.cache_hits += other.cache_hits;
        self.slice_reads += other.slice_reads;
        self.direct_reads += other.direct_reads;
        self.hook_redirects += other.hook_redirects;
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
    let mut map = ActivityMap::new();
    scan_file(log_file, &mut map)?;
    for n in 1..=keep as usize {
        scan_file(&rotated_path(log_file, n), &mut map)?;
    }
    Ok(map)
}

/// Counts one file, opened read-only; a missing file contributes nothing.
fn scan_file(path: &Path, map: &mut ActivityMap) -> anyhow::Result<()> {
    let file = match File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e).with_context(|| read_failure(path)),
    };
    for line in BufReader::new(file).split(b'\n') {
        let line = line.with_context(|| read_failure(path))?;
        if let Some((key, route)) = parse_line(&line) {
            map.entry(key).or_default().bump(route);
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

/// Key and route of one audit line; `None` for other events and for lines
/// that are blank, garbled or lack a day, project or harness.
fn parse_line(line: &[u8]) -> Option<(ActivityKey, Route)> {
    let record: Value = serde_json::from_slice(line).ok()?;
    let route = route_of(record.get("event_type")?.as_str()?)?;
    let day = utc_day(record.get("epoch_ms")?.as_i64()?)?;
    let project = record.get("project")?.as_str()?.to_string();
    let harness = record.get("agent_harness")?.as_str()?.to_string();
    Some(((day, project, harness), route))
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
