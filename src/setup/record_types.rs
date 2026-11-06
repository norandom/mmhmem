//! Validated value types of the install record (task 2.4, design.md §3).
//!
//! Every string the record can hold goes through one of these types, so the
//! record can only contain hashes, release tags, timestamps, config key
//! names, section ids and paths (Requirement 5.7). Their parse errors are
//! fixed messages that never repeat the rejected text.

use std::fmt;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Fixed error for a value that does not have the expected shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidValue(&'static str);

impl fmt::Display for InvalidValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

impl std::error::Error for InvalidValue {}

/// A lowercase hex SHA-256 digest (64 characters).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Sha256Hex(String);

impl Sha256Hex {
    /// The digest of `bytes`.
    pub fn of(bytes: &[u8]) -> Self {
        Self(crate::storage::audit::sha256_hex(bytes))
    }

    /// The digest as 64 lowercase hex characters.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for Sha256Hex {
    type Error = InvalidValue;
    fn try_from(s: String) -> Result<Self, InvalidValue> {
        let ok = s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
        ok.then_some(Self(s))
            .ok_or(InvalidValue("not a SHA-256 hash"))
    }
}

impl From<Sha256Hex> for String {
    fn from(v: Sha256Hex) -> Self {
        v.0
    }
}

impl fmt::Display for Sha256Hex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A release tag such as `v0.5.0` or `v0.5.0-rc.1`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ReleaseTag(String);

impl ReleaseTag {
    /// The tag text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for ReleaseTag {
    type Error = InvalidValue;
    fn try_from(s: String) -> Result<Self, InvalidValue> {
        let rest = s.strip_prefix('v').unwrap_or("");
        let ok = s.len() <= 64
            && rest.starts_with(|c: char| c.is_ascii_digit())
            && rest
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'+'));
        ok.then_some(Self(s))
            .ok_or(InvalidValue("not a release tag"))
    }
}

impl From<ReleaseTag> for String {
    fn from(v: ReleaseTag) -> Self {
        v.0
    }
}

/// A UTC time in the form `2026-11-04T09:31:12Z`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct UtcTimestamp(String);

impl UtcTimestamp {
    /// The current time.
    pub fn now() -> Self {
        Self::from_datetime(time::OffsetDateTime::now_utc())
    }

    /// The given time, converted to UTC and truncated to whole seconds.
    pub fn from_datetime(at: time::OffsetDateTime) -> Self {
        let at = at.to_offset(time::UtcOffset::UTC);
        Self(format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
            at.year(),
            u8::from(at.month()),
            at.day(),
            at.hour(),
            at.minute(),
            at.second()
        ))
    }

    /// The timestamp text.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The same time usable as a folder name on every OS (`:` becomes `-`),
    /// e.g. `2026-11-04T09-30-00Z`.
    pub fn folder_name(&self) -> String {
        self.0.replace(':', "-")
    }
}

impl TryFrom<String> for UtcTimestamp {
    type Error = InvalidValue;
    fn try_from(s: String) -> Result<Self, InvalidValue> {
        let b = s.as_bytes();
        let shape = b"0000-00-00T00:00:00Z";
        let ok = b.len() == shape.len()
            && b.iter().zip(shape).all(|(c, want)| match want {
                b'0' => c.is_ascii_digit(),
                other => c == other,
            });
        ok.then_some(Self(s))
            .ok_or(InvalidValue("not a UTC timestamp"))
    }
}

impl From<UtcTimestamp> for String {
    fn from(v: UtcTimestamp) -> Self {
        v.0
    }
}

/// One key name in a JSON pointer or TOML key path (e.g. `mcpServers`).
///
/// At most 128 characters and no control characters. It names a key; the
/// record never stores the value under that key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct KeySegment(String);

impl KeySegment {
    /// The key name.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for KeySegment {
    type Error = InvalidValue;
    fn try_from(s: String) -> Result<Self, InvalidValue> {
        let ok = !s.is_empty() && s.chars().count() <= 128 && !s.chars().any(char::is_control);
        ok.then_some(Self(s))
            .ok_or(InvalidValue("not a config key name"))
    }
}

impl TryFrom<&str> for KeySegment {
    type Error = InvalidValue;
    fn try_from(s: &str) -> Result<Self, InvalidValue> {
        Self::try_from(s.to_owned())
    }
}

impl From<KeySegment> for String {
    fn from(v: KeySegment) -> Self {
        v.0
    }
}

/// Build a key path from string literals (fails on an invalid segment).
pub fn key_path(segments: &[&str]) -> Result<Vec<KeySegment>, InvalidValue> {
    segments.iter().map(|s| KeySegment::try_from(*s)).collect()
}

/// The id of a delimited Markdown section: `[a-z0-9-]{1,32}`, as in
/// `<!-- mmhmem:routing:start -->` (id `routing`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct SectionId(String);

impl SectionId {
    /// The id text.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The line that opens the section.
    pub fn start_marker(&self) -> String {
        format!("<!-- mmhmem:{}:start -->", self.0)
    }

    /// The line that closes the section.
    pub fn end_marker(&self) -> String {
        format!("<!-- mmhmem:{}:end -->", self.0)
    }
}

impl TryFrom<String> for SectionId {
    type Error = InvalidValue;
    fn try_from(s: String) -> Result<Self, InvalidValue> {
        let ok = (1..=32).contains(&s.len())
            && s.bytes()
                .all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'-'));
        ok.then_some(Self(s))
            .ok_or(InvalidValue("not a section id"))
    }
}

impl From<SectionId> for String {
    fn from(v: SectionId) -> Self {
        v.0
    }
}

/// A backup path relative to the record folder, `/`-separated and inside
/// `backups/` (e.g. `backups/2026-11-04T09-30-00Z/config.toml`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct BackupPath(String);

impl BackupPath {
    /// The relative form of `backup` (an absolute path returned by
    /// `fsx::backup`) when it lies under `record_dir/backups/`.
    pub fn from_absolute(record_dir: &Path, backup: &Path) -> Result<Self, InvalidValue> {
        let rel = backup
            .strip_prefix(record_dir)
            .map_err(|_| InvalidValue("the backup is not inside the record folder"))?;
        let mut parts = Vec::new();
        for c in rel.components() {
            match c {
                Component::Normal(p) => parts.push(
                    p.to_str()
                        .ok_or(InvalidValue("the backup path is not valid text"))?
                        .to_owned(),
                ),
                _ => return Err(InvalidValue("not a backup path")),
            }
        }
        Self::try_from(parts.join("/"))
    }

    /// The relative path text.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The absolute path of the backup under `record_dir`.
    pub fn resolve(&self, record_dir: &Path) -> PathBuf {
        self.0
            .split('/')
            .fold(record_dir.to_path_buf(), |p, s| p.join(s))
    }
}

impl TryFrom<String> for BackupPath {
    type Error = InvalidValue;
    fn try_from(s: String) -> Result<Self, InvalidValue> {
        let mut parts = s.split('/');
        let ok = parts.next() == Some("backups")
            && s.split('/').count() >= 3
            && parts
                .all(|p| !p.is_empty() && p != "." && p != ".." && !p.contains(['\\', ':', '\0']));
        ok.then_some(Self(s))
            .ok_or(InvalidValue("not a backup path"))
    }
}

impl From<BackupPath> for String {
    fn from(v: BackupPath) -> Self {
        v.0
    }
}

/// The `<agent>` argument of `mmhmem hook <agent>` (see `src/cli/hook.rs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HookHarness {
    /// `hook claude`.
    Claude,
    /// `hook codex`.
    Codex,
    /// `hook opencode`.
    Opencode,
}

impl HookHarness {
    /// The argument text.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Opencode => "opencode",
        }
    }
}

/// How mmhmem's entry in a shared JSON array is recognised. A closed set:
/// the record stores which rule applies, never the element's text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ElementMatcher {
    /// mmhmem's read-routing hook HANDLER inside a hook group (an object
    /// with a `hooks` array). mmhmem owns the handler, never the group: the
    /// old installers could leave it in a group that also holds the user's
    /// handlers (see `record_hook.rs`). A handler is mmhmem's when it runs
    /// the mmhmem binary (file name `mmhmem` or `mmhmem.exe`) with
    /// `hook <harness>`, either as `command` = binary path plus
    /// `args = ["hook", "<harness>"]` (Claude Code form), or as a `command`
    /// string `[& ]<binary path, bare or quoted> hook <harness>` (Codex
    /// shell form).
    MmhmemHook(HookHarness),
}

impl ElementMatcher {
    /// Whether the group `element` holds at least one of mmhmem's handlers.
    pub fn matches(self, element: &serde_json::Value) -> bool {
        !self.own_handlers(element).is_empty()
    }

    /// The indices of mmhmem's handlers in the `hooks` array of `element`.
    pub fn own_handlers(self, element: &serde_json::Value) -> Vec<usize> {
        let Self::MmhmemHook(harness) = self;
        let hooks = element.get("hooks").and_then(serde_json::Value::as_array);
        hooks.map_or_else(Vec::new, |hooks| {
            (0..hooks.len())
                .filter(|i| super::hook::is_own_handler(&hooks[*i], harness))
                .collect()
        })
    }
}
