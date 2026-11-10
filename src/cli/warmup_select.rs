//! Candidate files for `mmhmem warmup` (shunt-cache-efficiency task 6.1;
//! design.md "Warmup CLI"; Req 6.1, 6.2, 6.4).
//!
//! The root's files come from git when the root is inside a work tree
//! (`git ls-files` of the tracked and the untracked, not ignored files, so
//! an ignored file is never listed), else from a depth-bounded walk that
//! never enters `.git`, `target` or `node_modules`. Of those regular files
//! a candidate is text with at least the threshold of lines (the hook's
//! capped scan), not security-critical by the part of its path inside the
//! root, and inside the allowed roots. Selection stops at `--max-files`
//! when another candidate remains. git runs as a process; without git the
//! walk is used.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::hook::text_lines_capped;
use crate::services::paths::{AllowedRoots, PathError};
use crate::services::security_path::is_security_critical_path;

/// Default `--max-files`.
pub const DEFAULT_MAX_FILES: usize = 200;

/// Deepest folder level the walk enters below the root.
pub const MAX_DEPTH: usize = 16;

/// Folders the walk never enters.
const SKIP_DIRS: [&str; 3] = [".git", "target", "node_modules"];

/// What makes a file a candidate.
pub struct Rules<'a> {
    /// Minimum line count (`SHUNT_MIN_LINES`).
    pub threshold: u64,
    /// The configured confinement (unconfined in local mode by default).
    pub roots: &'a AllowedRoots,
    /// `--max-files`: candidates per run.
    pub max_files: usize,
}

/// One file to warm.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    /// Path below the root, as listed.
    pub rel: PathBuf,
    /// The root joined with `rel`.
    pub abs: PathBuf,
}

/// Files examined but not selected, by reason.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Skipped {
    /// Fewer lines than the threshold.
    pub below_threshold: u32,
    /// A NUL or invalid UTF-8 in the first 8 KiB.
    pub not_text: u32,
    /// Matches the security rule inside the root.
    pub security: u32,
    /// Outside `AGENT_MEM_ALLOWED_ROOTS`.
    pub outside_roots: u32,
    /// Could not be read or resolved.
    pub unreadable: u32,
}

/// Where the file list came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Listing {
    /// `git ls-files` in a work tree.
    Git,
    /// The bounded walk.
    Walk,
}

/// The candidates of one root.
#[derive(Debug)]
pub struct Selection {
    pub listing: Listing,
    /// Candidates in listing order (sorted by path).
    pub files: Vec<Candidate>,
    /// Regular files looked at (the candidates included).
    pub examined: u32,
    pub skipped: Skipped,
    /// Another candidate remained beyond `max_files`.
    pub capped: bool,
}

/// The line printed when the cap stopped the selection.
pub fn cap_message(max_files: usize) -> String {
    format!("stopped at {max_files} files; raise --max-files to continue")
}

/// The candidates under `root` by `rules`.
pub fn select(root: &Path, rules: &Rules<'_>) -> Selection {
    let (listing, listed) = list_files(root);
    let mut selection = Selection {
        listing,
        files: Vec::new(),
        examined: 0,
        skipped: Skipped::default(),
        capped: false,
    };
    for rel in listed {
        let abs = root.join(&rel);
        let Some(verdict) = judge(&rel, &abs, rules) else {
            continue;
        };
        match verdict {
            Verdict::Take if selection.files.len() >= rules.max_files => {
                selection.capped = true;
                break;
            }
            Verdict::Take => selection.files.push(Candidate { rel, abs }),
            Verdict::Skip(reason) => selection.skipped.count(reason),
        }
        selection.examined += 1;
    }
    selection
}

enum Verdict {
    Take,
    Skip(Reason),
}

#[derive(Clone, Copy)]
enum Reason {
    BelowThreshold,
    NotText,
    Security,
    OutsideRoots,
    Unreadable,
}

impl Skipped {
    fn count(&mut self, reason: Reason) {
        let n = match reason {
            Reason::BelowThreshold => &mut self.below_threshold,
            Reason::NotText => &mut self.not_text,
            Reason::Security => &mut self.security,
            Reason::OutsideRoots => &mut self.outside_roots,
            Reason::Unreadable => &mut self.unreadable,
        };
        *n += 1;
    }
}

/// The verdict on one listed path; `None` for anything but a regular file
/// (folders, links, submodules, tracked files deleted on disk).
fn judge(rel: &Path, abs: &Path, rules: &Rules<'_>) -> Option<Verdict> {
    let meta = std::fs::symlink_metadata(abs).ok()?;
    if !meta.file_type().is_file() {
        return None;
    }
    if is_security_critical_path(rel) {
        return Some(Verdict::Skip(Reason::Security));
    }
    Some(match text_lines_capped(abs, rules.threshold) {
        Err(_) => Verdict::Skip(Reason::Unreadable),
        Ok(None) => Verdict::Skip(Reason::NotText),
        Ok(Some(lines)) if lines < rules.threshold => Verdict::Skip(Reason::BelowThreshold),
        Ok(Some(_)) => confine(abs, rules.roots),
    })
}

fn confine(abs: &Path, roots: &AllowedRoots) -> Verdict {
    match roots.confine(&abs.to_string_lossy()) {
        Ok(_) => Verdict::Take,
        Err(PathError::OutsideRoots(_)) => Verdict::Skip(Reason::OutsideRoots),
        Err(PathError::NotFound(_)) => Verdict::Skip(Reason::Unreadable),
    }
}

/// The root's files relative to it, sorted: from git inside a work tree,
/// else from the walk.
pub fn list_files(root: &Path) -> (Listing, Vec<PathBuf>) {
    match git_files(root) {
        Some(files) => (Listing::Git, files),
        None => (Listing::Walk, walk_files(root)),
    }
}

/// Tracked plus untracked, not ignored files below `root`; `None` when the
/// root is not in a work tree or git is missing.
fn git_files(root: &Path) -> Option<Vec<PathBuf>> {
    let inside = git(root, &["rev-parse", "--is-inside-work-tree"])?;
    if inside.trim_ascii() != b"true" {
        return None;
    }
    let listed = git(
        root,
        &[
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
        ],
    )?;
    let mut files: Vec<PathBuf> = listed
        .split(|&b| b == 0)
        .filter(|name| !name.is_empty())
        .map(|name| PathBuf::from(String::from_utf8_lossy(name).into_owned()))
        .collect();
    files.sort();
    files.dedup();
    Some(files)
}

/// The output of a successful `git <args>` in `dir`.
fn git(dir: &Path, args: &[&str]) -> Option<Vec<u8>> {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .stdin(Stdio::null())
        .output()
        .ok()?;
    out.status.success().then_some(out.stdout)
}

/// Regular files below `root`, sorted by path, at most [`MAX_DEPTH`]
/// folders deep, never inside [`SKIP_DIRS`] and never through a link.
pub fn walk_files(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    walk(root, Path::new(""), 0, &mut files);
    files
}

fn walk(root: &Path, rel: &Path, depth: usize, files: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(root.join(rel)) else {
        return;
    };
    let mut entries: Vec<std::fs::DirEntry> = entries.filter_map(Result::ok).collect();
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        let name = entry.file_name();
        let child = rel.join(&name);
        if kind.is_file() {
            files.push(child);
        } else if kind.is_dir() && depth < MAX_DEPTH && !SKIP_DIRS.iter().any(|d| name == *d) {
            walk(root, &child, depth + 1, files);
        }
    }
}

#[cfg(test)]
#[path = "warmup_select_tests.rs"]
mod tests;
