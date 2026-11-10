//! The `AuditWriter` component (task 2.2): the redacting, hash-verifying,
//! append-only JSONL audit writer (design.md "storage layer" / "Audit record
//! contract" / "Security Considerations"; requirements.md Req 7.1-7.5,
//! 3.3, 6.3, 6.4, 10.1; research.md "Audit writer is the only write path").
//!
//! [`AuditWriter`] is the ONLY component that writes the audit log; every
//! service goes through it, so redaction + hashing cannot be bypassed.
//!
//! # Record contract (design.md "Audit record contract")
//!
//! One single-line JSON object per event with fields: `ts` (ISO-8601 UTC,
//! RFC 3339, e.g. `2026-11-01T09:15:30.123Z`), `epoch_ms` (i64 ms since the
//! Unix epoch), `event_type` (SCREAMING_SNAKE_CASE, see [`AuditEventType`]),
//! `agent_harness`, `project`, `details` (redacted), `record_sha256`.
//!
//! # Canonical digest form (documented + tested)
//!
//! `record_sha256` is the SHA-256 hex digest over the record WITHOUT the
//! digest field, serialized as compact JSON (no whitespace, UTF-8) of a
//! `BTreeMap<String, Value>` — object keys sorted lexicographically by
//! Unicode scalar value — computed BEFORE `record_sha256` is inserted:
//! `sha256_hex(serde_json::to_string(BTreeMap of fields minus record_sha256))`.
//! The written line is that same sorted-keys serialization with
//! `record_sha256` added, plus a trailing `\n`. Building the `BTreeMap`
//! explicitly keeps canonicalization independent of serde_json's
//! `preserve_order` feature. Verification (used by the tests): parse the
//! line, drop `record_sha256`, re-serialize the rest through a `BTreeMap`,
//! SHA-256 hex must equal the stored digest.
//!
//! # Redaction (Req 7.1/7.2, applied BEFORE serialization)
//!
//! [`redact_secrets`] ports the Python reference (`Self-Hosted Cross-Agent
//! Memory Solutions.md`, claude-mem v15.5.0 rules): `<private>...</private>`
//! spans (DOTALL) -> `[REDACTED_PRIVATE_TAG]`, then key-assigned secrets and
//! Slack tokens -> `[REDACTED_SECRET]`. Port notes: the key separator allows
//! an optional quote (and its JSON escape) on each side
//! (`key\s*\\?["']?\s*[:=]\s*\\?["']?`) so the filter also works on the
//! JSON-stringified `details` (`"api_key": "..."`, `api_key=\"...\"`), and
//! the value group carries an optional `bearer\s+` prefix so
//! `Authorization: Bearer <token>` headers are scrubbed whole instead of
//! leaking the token after the `Bearer` keyword.
//!
//! # Append semantics (Req 7.4)
//!
//! Every write opens the file with `OpenOptions::append(true).create(true)`
//! (O_APPEND) PER WRITE, writes the single line + `\n` in one `write_all`,
//! and closes — no persistent handle, so logrotate can rotate the file
//! mid-run without restarting the daemon. Parent dirs are created at first
//! write. The path comes from config `AGENT_MEM_LOG_FILE` (Req 7.5; default
//! `/var/log/agent-memory/audit.jsonl` — on a Windows dev host set
//! `AGENT_MEM_LOG_FILE` explicitly, e.g. `C:\ProgramData\agent-memory\audit.jsonl`).
//!
//! `write_event` is synchronous on purpose: callers run it inside
//! `spawn_blocking` (like `Db::with_conn`), one short blocking section per
//! event.
//!
//! Tests for the redaction pattern classes, digest verification, and
//! append/rotation semantics live in `tests/audit_redaction.rs` (design.md
//! "File Structure Plan"), keeping this file under the Req 13.3
//! 500-LOC-per-src-file cap.

use std::collections::BTreeMap;
use std::fmt;
use std::fs::OpenOptions;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use regex::{Captures, Regex};
use serde_json::Value;
use sha2::{Digest, Sha256};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::config::Config;

/// Placeholder written in place of any scrubbed secret value (Req 7.2).
pub const REDACTED_SECRET: &str = "[REDACTED_SECRET]";
/// Placeholder written in place of a whole `<private>...</private>` span.
pub const REDACTED_PRIVATE_TAG: &str = "[REDACTED_PRIVATE_TAG]";

/// Audit event types, serialized as SCREAMING_SNAKE_CASE (steering
/// structure.md "Naming Conventions"; design.md "Audit record contract":
/// the SESSION_/IO_/MEM_/TOKEN_STATS events plus error variants).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AuditEventType {
    SessionResume = 0,
    SessionCheckpoint,
    IoTargetedRead,
    IoDirectRead,
    IoShuntCacheHit,
    IoShuntDelegated,
    IoShuntCodeWrite,
    MemSearchIndex,
    MemGetDetails,
    MemRecordObs,
    TokenStats,
    TokenStatsClient,
    WorkerError,
    AuthFailure,
    Error,
    Prune,
    MemSearchBreadcrumbs,
    HookRedirect,
    HookPass,
    StatusCheck,
    ContentCacheEvict,
    CacheMigrated,
}

impl AuditEventType {
    /// Every variant in declaration order (used by the tests).
    pub const ALL: [AuditEventType; 22] = [
        AuditEventType::SessionResume,
        AuditEventType::SessionCheckpoint,
        AuditEventType::IoTargetedRead,
        AuditEventType::IoDirectRead,
        AuditEventType::IoShuntCacheHit,
        AuditEventType::IoShuntDelegated,
        AuditEventType::IoShuntCodeWrite,
        AuditEventType::MemSearchIndex,
        AuditEventType::MemGetDetails,
        AuditEventType::MemRecordObs,
        AuditEventType::TokenStats,
        AuditEventType::TokenStatsClient,
        AuditEventType::WorkerError,
        AuditEventType::AuthFailure,
        AuditEventType::Error,
        AuditEventType::Prune,
        AuditEventType::MemSearchBreadcrumbs,
        AuditEventType::HookRedirect,
        AuditEventType::HookPass,
        AuditEventType::StatusCheck,
        AuditEventType::ContentCacheEvict,
        AuditEventType::CacheMigrated,
    ];

    /// The SCREAMING_SNAKE_CASE name written into the record.
    pub fn as_str(self) -> &'static str {
        const NAMES: [&str; 22] = [
            "SESSION_RESUME",
            "SESSION_CHECKPOINT",
            "IO_TARGETED_READ",
            "IO_DIRECT_READ",
            "IO_SHUNT_CACHE_HIT",
            "IO_SHUNT_DELEGATED",
            "IO_SHUNT_CODE_WRITE",
            "MEM_SEARCH_INDEX",
            "MEM_GET_DETAILS",
            "MEM_RECORD_OBS",
            "TOKEN_STATS",
            "TOKEN_STATS_CLIENT",
            "WORKER_ERROR",
            "AUTH_FAILURE",
            "ERROR",
            "PRUNE",
            "MEM_SEARCH_BREADCRUMBS",
            "HOOK_REDIRECT",
            "HOOK_PASS",
            "STATUS_CHECK",
            "CONTENT_CACHE_EVICT",
            "CACHE_MIGRATED",
        ];
        NAMES[self as usize]
    }
}

impl fmt::Display for AuditEventType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// `<private>...</private>` spans with DOTALL — Python reference
/// `<private>.*?</private>` (case-sensitive there, kept here).
static PRIVATE_TAG_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)<private>.*?</private>").expect("valid regex"));

/// Port of the Python reference `SECRET_REDACT_RE` (claude-mem v15.5.0, see
/// the research doc at the repo root): group 1 is the key + separator, group
/// 2 the secret value (with an optional `bearer\s+` prefix so Bearer headers
/// are scrubbed whole), group 3 the Slack `xapp-`/`xoxe-` tokens. Port notes:
/// the separator allows an optional quote (and its JSON escape `\"`) on each
/// side — `key\s*\\?["']?\s*[:=]\s*\\?["']?` — so the filter works on the
/// JSON-stringified `details` too (`"api_key": "..."`, `api_key=\"...\"`).
static SECRET_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?i)((?:api_key|secret|token|password|mysql_pwd|dbpassword|authorization)\s*\\?["']?\s*[:=]\s*\\?["']?)((?:bearer\s+)?[^'"\s,}\]]+)|(xapp-[0-9]+-[A-Za-z0-9_]+|xoxe-[A-Za-z0-9-]+)"#,
    )
    .expect("valid regex")
});

/// The redaction filter (Req 7.1/7.2), applied to every string before it is
/// persisted. Public so the optional OTel exporter (task 9.1, Req 11.3)
/// reuses the exact same filter.
pub fn redact_secrets(input: &str) -> String {
    let without_private = PRIVATE_TAG_RE.replace_all(input, REDACTED_PRIVATE_TAG);
    SECRET_RE
        .replace_all(&without_private, |caps: &Captures<'_>| {
            format!(
                "{}{REDACTED_SECRET}",
                caps.get(1).map_or("", |m| m.as_str())
            )
        })
        .into_owned()
}

/// The append-only JSONL audit writer — the only write path to the log
/// (research.md decision "Audit writer is the only write path to the log";
/// steering structure.md: audit.rs owns redaction filter + append + hashing).
#[derive(Clone, Debug)]
pub struct AuditWriter {
    path: PathBuf,
    /// Rotation threshold in bytes; 0 disables rotation (serve mode keeps
    /// the host-logrotate contract, Req 7.3/9.5).
    audit_max_bytes: u64,
    /// Rotated files retained when rotation is enabled (Req 7.3).
    audit_rotate_keep: u32,
}

impl AuditWriter {
    /// Writer targeting `path` (normally `config.log_file`, Req 7.5) with
    /// rotation disabled.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            audit_max_bytes: 0,
            audit_rotate_keep: 0,
        }
    }

    /// Writer with size-bounded rotation enabled (local profile, Req 7.3).
    pub fn with_rotation(path: impl Into<PathBuf>, max_bytes: u64, keep: u32) -> Self {
        Self {
            path: path.into(),
            audit_max_bytes: max_bytes,
            audit_rotate_keep: keep,
        }
    }

    /// Writer targeting `AGENT_MEM_LOG_FILE` from the config (Req 7.5);
    /// rotation knobs come from the profile's settings (Req 7.3).
    pub fn from_config(config: &Config) -> Self {
        Self {
            path: config.log_file.clone(),
            audit_max_bytes: config.audit_max_bytes,
            audit_rotate_keep: config.audit_rotate_keep,
        }
    }

    /// The configured log file path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Appends one redacted, hash-verified single-line record (Req 7.1-7.4).
    ///
    /// Synchronous on purpose: callers run this inside `spawn_blocking`
    /// (like `Db::with_conn`), one short blocking section per event.
    pub fn write_event(
        &self,
        event_type: AuditEventType,
        harness: &str,
        project: &str,
        details: Value,
    ) -> io::Result<()> {
        let now = OffsetDateTime::now_utc();
        let mut record = base_record(now, event_type, harness, project, &details)?;
        let canonical = serde_json::to_string(&record)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        record.insert(
            "record_sha256".to_string(),
            Value::String(sha256_hex(canonical.as_bytes())),
        );
        let mut line = serde_json::to_string(&record)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        line.push('\n');
        self.append_line(&line)
    }

    /// O_APPEND open-per-write (Req 7.4): one open + one single-line write +
    /// close per record; no persistent handle, so logrotate rotation works
    /// without a daemon restart. Parent dirs are created at first write.
    /// When size-bounded rotation is enabled (local profile, Req 7.3), the
    /// active file is rotated between writes first; a rotation failure is
    /// logged and never blocks the audit append itself.
    fn append_line(&self, line: &str) -> io::Result<()> {
        if let Some(parent) = self.path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        if self.audit_max_bytes > 0
            && let Err(e) = super::audit_rotate::rotate_if_needed(
                &self.path,
                self.audit_max_bytes,
                self.audit_rotate_keep,
            )
        {
            tracing::warn!("audit rotation failed (appending anyway): {e}");
        }
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        file.write_all(line.as_bytes())
    }
}

/// Record fields minus `record_sha256`, every string redacted (Req 7.1).
fn base_record(
    now: OffsetDateTime,
    event_type: AuditEventType,
    harness: &str,
    project: &str,
    details: &Value,
) -> io::Result<BTreeMap<String, Value>> {
    let ts = now
        .format(&Rfc3339)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    let mut record = BTreeMap::new();
    record.insert("ts".to_string(), Value::String(ts));
    record.insert(
        "epoch_ms".to_string(),
        Value::from((now.unix_timestamp_nanos() / 1_000_000) as i64),
    );
    record.insert(
        "event_type".to_string(),
        Value::String(event_type.as_str().to_string()),
    );
    record.insert(
        "agent_harness".to_string(),
        Value::String(redact_secrets(harness)),
    );
    record.insert(
        "project".to_string(),
        Value::String(redact_secrets(project)),
    );
    record.insert("details".to_string(), redact_details(details));
    Ok(record)
}

/// Redacts the stringified `details` exactly like the Python reference
/// (`redact_secrets(json.dumps(details))`) and parses the result back into a
/// `Value`. If redaction ever broke the JSON structure, the redacted STRING
/// is kept — unredacted bytes can never reach disk (fail-safe, Req 7.1).
fn redact_details(details: &Value) -> Value {
    let raw = serde_json::to_string(details).unwrap_or_else(|_| "null".to_string());
    let redacted = redact_secrets(&raw);
    serde_json::from_str(&redacted).unwrap_or(Value::String(redacted))
}

/// SHA-256 hex digest of `bytes` (pub: reused by the shunt engine's content
/// addressing and by the stats collector).
pub fn sha256_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest.iter() {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::AuditEventType;

    #[test]
    fn hook_events_are_listed_with_stable_names() {
        assert_eq!(AuditEventType::HookRedirect.as_str(), "HOOK_REDIRECT");
        assert_eq!(AuditEventType::HookPass.as_str(), "HOOK_PASS");
        assert!(AuditEventType::ALL.contains(&AuditEventType::HookRedirect));
        assert!(AuditEventType::ALL.contains(&AuditEventType::HookPass));
        // shunt-content-tool added CONTENT_CACHE_EVICT (Req 4.3);
        // shunt-cache-efficiency added CACHE_MIGRATED (task 1.2); the
        // 2026-11-10 restore removed the batch summary event.
        assert_eq!(AuditEventType::ALL.len(), 22);
        assert_eq!(AuditEventType::CacheMigrated.as_str(), "CACHE_MIGRATED");
        assert_eq!(AuditEventType::StatusCheck.as_str(), "STATUS_CHECK");
    }
}
