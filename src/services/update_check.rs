//! Update notice for local mode (the session-start "a newer mmhmem is
//! available" line). Only a note: nothing installs itself, and the update
//! itself stays `mmhmem update`, which checks signature and checksum.
//!
//! Contract:
//! - The tool call never waits on the network. A small cache file
//!   (`update-check.json` in the local data dir: `checked_at` in Unix
//!   seconds and the newest stable tag seen) is the only thing the resume
//!   path reads. A missing, unreadable or corrupt cache means "no notice".
//! - [`refresh`] asks the injected lookup at most once per
//!   [`REFRESH_EVERY`] and replaces the cache atomically (same-folder temp
//!   file + rename). A failed lookup keeps the last known tag and still
//!   records the attempt, so an offline machine retries once a day, not on
//!   every agent start. `mmhmem stdio` runs it on a background thread with a
//!   short timeout; errors are ignored.
//! - Local profile only; the serve profile never notes. Opt-out with
//!   `MMHMEM_NO_UPDATE_CHECK=1` or `"update_check": false` in `.mmhmem.json`.
//! - Only stable `vMAJOR.MINOR.PATCH` tags count; pre-releases are ignored.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::config::profile::Profile;

/// Environment opt-out: `MMHMEM_NO_UPDATE_CHECK=1` turns the check off.
pub const ENV_NO_UPDATE_CHECK: &str = "MMHMEM_NO_UPDATE_CHECK";
/// Cache file name inside the local data dir.
pub const CACHE_FILE: &str = "update-check.json";
/// How long a cached answer stays fresh.
pub const REFRESH_EVERY: Duration = Duration::from_secs(24 * 60 * 60);

/// The cache file's content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateCache {
    /// When the last lookup was attempted (Unix seconds).
    pub checked_at: i64,
    /// The newest stable tag known, e.g. `v0.5.2` (None: never found one).
    pub newest_tag: Option<String>,
}

/// Whether the update notice is on: local profile, no env opt-out, and
/// the `.mmhmem.json` key `update_check` not set to `false`.
pub fn enabled(
    profile: Profile,
    get: &dyn Fn(&str) -> Option<String>,
    file_setting: Option<bool>,
) -> bool {
    let env_off = get(ENV_NO_UPDATE_CHECK).is_some_and(|v| {
        let v = v.trim();
        !v.is_empty() && v != "0" && !v.eq_ignore_ascii_case("false")
    });
    profile == Profile::Local && !env_off && file_setting != Some(false)
}

/// The cache path in `data_dir`.
pub fn cache_path(data_dir: &Path) -> PathBuf {
    data_dir.join(CACHE_FILE)
}

/// Reads the cache; missing, unreadable or corrupt → None.
pub fn read_cache(path: &Path) -> Option<UpdateCache> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

/// True when the cache is missing, from the future (clock change), or
/// older than [`REFRESH_EVERY`].
pub fn needs_refresh(cache: Option<&UpdateCache>, now: i64) -> bool {
    let max_age = REFRESH_EVERY.as_secs() as i64;
    match cache {
        None => true,
        Some(c) => c.checked_at > now || now - c.checked_at >= max_age,
    }
}

/// Refreshes the cache at `path` when it is due: runs `lookup` (the newest
/// stable tag, or None on any failure) and writes the result atomically.
/// A failed lookup keeps the previously known tag. Returns whether a
/// lookup ran. Write errors are ignored (the notice is best effort).
pub fn refresh(path: &Path, now: i64, lookup: impl FnOnce() -> Option<String>) -> bool {
    let old = read_cache(path);
    if !needs_refresh(old.as_ref(), now) {
        return false;
    }
    let newest_tag = lookup()
        .filter(|t| stable(t).is_some())
        .or_else(|| old.and_then(|c| c.newest_tag));
    let cache = UpdateCache {
        checked_at: now,
        newest_tag,
    };
    let _ = write_atomic(path, &cache);
    true
}

fn write_atomic(path: &Path, cache: &UpdateCache) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(dir)?;
    let bytes = serde_json::to_vec(cache).map_err(std::io::Error::other)?;
    let mut temp = tempfile::NamedTempFile::new_in(dir)?;
    std::io::Write::write_all(&mut temp, &bytes)?;
    temp.persist(path).map(drop).map_err(|e| e.error)
}

/// The notice line when `cache` names a stable release newer than
/// `installed` (this program's version, e.g. `0.5.1`); otherwise None.
pub fn notice(installed: &str, cache: Option<&UpdateCache>) -> Option<String> {
    let tag = cache?.newest_tag.as_deref()?;
    let newest = stable(tag)?;
    let (core, pre) = match installed.split_once('-') {
        Some((core, _)) => (core, true),
        None => (installed, false),
    };
    let own = stable(&format!("v{core}"))?;
    let newer = newest > own || (newest == own && pre);
    newer.then(|| {
        format!(
            "mmhmem {tag} is available (installed: v{installed}). \
             To update, run in a terminal: mmhmem update"
        )
    })
}

/// `vMAJOR.MINOR.PATCH` as numbers; anything else (pre-release, build
/// suffix, leading zeros) → None.
fn stable(tag: &str) -> Option<(u64, u64, u64)> {
    let parts: Vec<&str> = tag.strip_prefix('v')?.split('.').collect();
    let number = |p: &str| {
        let ok = !p.is_empty()
            && p.bytes().all(|b| b.is_ascii_digit())
            && (p == "0" || !p.starts_with('0'));
        ok.then(|| p.parse::<u64>().ok()).flatten()
    };
    match parts.as_slice() {
        [a, b, c] => Some((number(a)?, number(b)?, number(c)?)),
        _ => None,
    }
}

/// Runs [`refresh`] on a background thread (errors ignored; a thread that
/// cannot start just means no refresh this time). The caller never waits.
pub fn spawn_refresh(path: PathBuf, lookup: impl FnOnce() -> Option<String> + Send + 'static) {
    let _ = std::thread::Builder::new()
        .name("mmhmem-update-check".into())
        .spawn(move || {
            refresh(&path, now_secs(), lookup);
        });
}

/// Current Unix time in seconds (0 if the clock is before 1970).
pub fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// The update notice wired into a local-mode server: reads only the cache.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateNotice {
    /// The cache file to read.
    pub cache_file: PathBuf,
}

impl UpdateNotice {
    /// The notice line for this program's version, if a newer release is
    /// cached. Never touches the network.
    pub fn line(&self) -> Option<String> {
        notice(
            env!("CARGO_PKG_VERSION"),
            read_cache(&self.cache_file).as_ref(),
        )
    }
}

#[cfg(test)]
#[path = "update_check_tests.rs"]
mod tests;
