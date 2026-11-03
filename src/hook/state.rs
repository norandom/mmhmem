//! Repeat-read window (read-routing-hooks design.md "state
//! (`src/hook/state.rs`)", Req 2.5).
//!
//! A redirected read is remembered per harness, session and file for
//! [`RepeatWindow::DEFAULT_WINDOW_MS`]; a second request inside the window
//! passes. State is a small JSON object in `hook-redirects.json`, mapping
//! the SHA-256 of `harness NUL session NUL path` to the redirect time in
//! epoch milliseconds. Entries older than one hour are pruned on write.
//!
//! Concurrent writers may lose an entry, which only means one more
//! redirect. A missing, empty, corrupt or concurrently replaced file is
//! treated as empty.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

use crate::storage::audit::sha256_hex;

/// State file name inside the directory given to [`RepeatWindow::new`].
pub const STATE_FILE: &str = "hook-redirects.json";
/// Entries older than this are dropped when a redirect is remembered.
pub const PRUNE_AFTER_MS: u64 = 3_600_000;

type Entries = BTreeMap<String, u64>;

/// Repeat-read window backed by a state file in one directory.
#[derive(Debug, Clone)]
pub struct RepeatWindow {
    dir: PathBuf,
    window_ms: u64,
}

impl RepeatWindow {
    /// Default repeat window: 120 seconds.
    pub const DEFAULT_WINDOW_MS: u64 = 120_000;

    /// Window over `dir` (the local data directory) with `window_ms`.
    pub fn new(dir: &Path, window_ms: u64) -> Self {
        Self {
            dir: dir.to_path_buf(),
            window_ms,
        }
    }

    /// Window over `dir` with the default 120 second window.
    pub fn with_default_window(dir: &Path) -> Self {
        Self::new(dir, Self::DEFAULT_WINDOW_MS)
    }

    /// True when a redirect for the same harness, session and path was
    /// remembered within the window. Never errors: any problem is `false`.
    pub fn is_repeat(&self, harness: &str, session: &str, path: &str, now_ms: u64) -> bool {
        let entries = load(&self.dir.join(STATE_FILE));
        match entries.get(&key(harness, session, path)) {
            Some(&at) => now_ms.saturating_sub(at) <= self.window_ms,
            None => false,
        }
    }

    /// Record a redirect at `now_ms`, prune entries older than one hour and
    /// write the file atomically (temp file in the same directory, then
    /// `std::fs::rename`, which replaces an existing file on Windows and
    /// POSIX). Creates the directory when missing. The caller ignores the
    /// error (fail open).
    pub fn remember(
        &self,
        harness: &str,
        session: &str,
        path: &str,
        now_ms: u64,
    ) -> io::Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        let file = self.dir.join(STATE_FILE);
        let mut entries = load(&file);
        entries.retain(|_, at| now_ms.saturating_sub(*at) <= PRUNE_AFTER_MS);
        entries.insert(key(harness, session, path), now_ms);
        let json = serde_json::to_vec(&entries).map_err(io::Error::other)?;
        // Unique per process and call so concurrent writers never share a temp.
        let tmp = self
            .dir
            .join(format!("{STATE_FILE}.{}.{now_ms}.tmp", std::process::id()));
        let result = std::fs::write(&tmp, &json).and_then(|()| std::fs::rename(&tmp, &file));
        if result.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
        result
    }
}

fn key(harness: &str, session: &str, path: &str) -> String {
    sha256_hex(format!("{harness}\0{session}\0{path}").as_bytes())
}

/// Read the state file; any problem yields an empty map.
fn load(file: &Path) -> Entries {
    std::fs::read(file)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: u64 = 10_000_000;

    fn window(dir: &Path) -> RepeatWindow {
        RepeatWindow::with_default_window(dir)
    }

    #[test]
    fn nothing_remembered_is_not_a_repeat() {
        let d = tempfile::tempdir().unwrap();
        assert!(!window(d.path()).is_repeat("claude", "s1", "/a", T0));
    }

    #[test]
    fn repeat_inside_window_is_true() {
        let d = tempfile::tempdir().unwrap();
        let w = window(d.path());
        w.remember("claude", "s1", "/a", T0).unwrap();
        assert!(w.is_repeat("claude", "s1", "/a", T0 + 119_999));
        assert!(w.is_repeat("claude", "s1", "/a", T0 + 120_000));
    }

    #[test]
    fn repeat_outside_window_is_false() {
        let d = tempfile::tempdir().unwrap();
        let w = window(d.path());
        w.remember("claude", "s1", "/a", T0).unwrap();
        assert!(!w.is_repeat("claude", "s1", "/a", T0 + 120_001));
    }

    #[test]
    fn different_session_harness_or_path_is_false() {
        let d = tempfile::tempdir().unwrap();
        let w = window(d.path());
        w.remember("claude", "s1", "/a", T0).unwrap();
        assert!(!w.is_repeat("claude", "s2", "/a", T0 + 1));
        assert!(!w.is_repeat("codex", "s1", "/a", T0 + 1));
        assert!(!w.is_repeat("claude", "s1", "/b", T0 + 1));
    }

    #[test]
    fn field_boundaries_do_not_collide() {
        let d = tempfile::tempdir().unwrap();
        let w = window(d.path());
        w.remember("ab", "c", "/a", T0).unwrap();
        assert!(!w.is_repeat("a", "bc", "/a", T0 + 1));
    }

    #[test]
    fn remember_prunes_entries_older_than_an_hour() {
        let d = tempfile::tempdir().unwrap();
        let w = window(d.path());
        w.remember("claude", "s1", "/old", T0).unwrap();
        w.remember("claude", "s1", "/new", T0 + PRUNE_AFTER_MS + 1)
            .unwrap();
        let raw = std::fs::read_to_string(d.path().join(STATE_FILE)).unwrap();
        let map: Entries = serde_json::from_str(&raw).unwrap();
        assert_eq!(map.len(), 1);
        assert!(map.contains_key(&key("claude", "s1", "/new")));
    }

    #[test]
    fn corrupt_file_is_empty_and_remember_rewrites_it() {
        let d = tempfile::tempdir().unwrap();
        let file = d.path().join(STATE_FILE);
        std::fs::write(&file, b"{not json\x00").unwrap();
        let w = window(d.path());
        assert!(!w.is_repeat("claude", "s1", "/a", T0));
        w.remember("claude", "s1", "/a", T0).unwrap();
        assert!(w.is_repeat("claude", "s1", "/a", T0 + 1));
        let map: Entries = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(map.len(), 1);
    }

    #[test]
    fn empty_file_is_empty() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join(STATE_FILE), b"").unwrap();
        assert!(!window(d.path()).is_repeat("claude", "s1", "/a", T0));
    }

    #[test]
    fn missing_directory_is_created_by_remember() {
        let d = tempfile::tempdir().unwrap();
        let sub = d.path().join("nested").join("data");
        let w = window(&sub);
        assert!(!w.is_repeat("claude", "s1", "/a", T0));
        w.remember("claude", "s1", "/a", T0).unwrap();
        assert!(w.is_repeat("claude", "s1", "/a", T0 + 1));
    }

    #[test]
    fn remember_replaces_existing_file_and_leaves_no_temp_files() {
        let d = tempfile::tempdir().unwrap();
        let w = window(d.path());
        w.remember("claude", "s1", "/a", T0).unwrap();
        w.remember("claude", "s1", "/b", T0 + 1).unwrap();
        assert!(w.is_repeat("claude", "s1", "/a", T0 + 2));
        assert!(w.is_repeat("claude", "s1", "/b", T0 + 2));
        let names: Vec<_> = std::fs::read_dir(d.path()).unwrap().collect();
        assert_eq!(names.len(), 1);
    }
}
