//! Audit-writer tests (task 2.2). design.md "File Structure Plan" puts
//! `tests/audit_redaction.rs` here ("secret patterns never reach disk;
//! record hash verifies"); the tasks.md 2.2 test matrix lives in this
//! integration test so `src/storage/audit.rs` stays under the Req 13.3
//! 500-LOC-per-src-file cap. Everything asserted here goes through the
//! crate's public API only (`mmhmem::storage::audit`).

use std::collections::BTreeMap;

use mmhmem::storage::audit::{
    redact_secrets, AuditEventType, AuditWriter, REDACTED_PRIVATE_TAG, REDACTED_SECRET,
};
use regex::Regex;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use time::OffsetDateTime;

/// SHA-256 hex digest, recomputed independently in the tests (the writer's
/// own `sha256_hex` is private).
fn sha256_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest.iter() {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

fn temp_writer() -> (tempfile::TempDir, AuditWriter) {
    let dir = tempfile::tempdir().unwrap();
    let writer = AuditWriter::new(dir.path().join("audit.jsonl"));
    (dir, writer)
}

fn raw_log(writer: &AuditWriter) -> String {
    std::fs::read_to_string(writer.path()).unwrap()
}

/// Parses one log line and verifies `record_sha256` over the sorted-keys
/// record minus the digest field (the documented canonical form).
fn verify_record(line: &str) -> Value {
    let record: Value = serde_json::from_str(line).expect("audit line must parse as JSON");
    let obj = record.as_object().expect("record is a JSON object");
    let digest = obj
        .get("record_sha256")
        .and_then(Value::as_str)
        .expect("record_sha256 present")
        .to_string();
    let sorted: BTreeMap<String, Value> = obj
        .iter()
        .filter(|(k, _)| k.as_str() != "record_sha256")
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let canonical = serde_json::to_string(&sorted).unwrap();
    assert_eq!(
        digest,
        sha256_hex(canonical.as_bytes()),
        "record_sha256 must verify over the sorted-keys record minus the digest"
    );
    record
}

/// Writes one event; returns (raw file bytes, verified parsed record).
fn write_one(writer: &AuditWriter, details: Value) -> (String, Value) {
    writer
        .write_event(
            AuditEventType::Error,
            "test-harness",
            "test-project",
            details,
        )
        .unwrap();
    let raw = raw_log(writer);
    let line = raw.strip_suffix('\n').expect("line ends with newline");
    assert!(!line.contains('\n'), "record must be a single line");
    let record = verify_record(line);
    (raw, record)
}

#[test]
fn redact_secrets_handles_every_key_assignment_form() {
    let s = "sk-XYZ-999";
    let cases = [
        (
            format!("api_key: {s}"),
            format!("api_key: {REDACTED_SECRET}"),
        ),
        (
            format!("api_key = {s}"),
            format!("api_key = {REDACTED_SECRET}"),
        ),
        (
            format!("api_key=\"{s}\""),
            format!("api_key=\"{REDACTED_SECRET}\""),
        ),
        (
            format!("api_key: '{s}'"),
            format!("api_key: '{REDACTED_SECRET}'"),
        ),
        (
            format!("\"api_key\": \"{s}\""),
            format!("\"api_key\": \"{REDACTED_SECRET}\""),
        ),
        (
            format!("Authorization: Bearer {s}"),
            format!("Authorization: {REDACTED_SECRET}"),
        ),
    ];
    for (input, expected) in cases {
        assert_eq!(redact_secrets(&input), expected, "input={input:?}");
    }
}

#[test]
fn redact_secrets_is_case_insensitive_for_key_names() {
    for key in [
        "API_KEY",
        "Secret",
        "TOKEN",
        "Password",
        "MYSQL_PWD",
        "dbPassword",
        "AUTHORIZATION",
    ] {
        let out = redact_secrets(&format!("{key}=topsecret123"));
        assert!(!out.contains("topsecret123"), "key={key}: {out}");
        assert!(out.contains(REDACTED_SECRET), "key={key}: {out}");
    }
}

#[test]
fn redact_secrets_scrubs_slack_tokens() {
    let out = redact_secrets("a xapp-1234567890-AbCdEf_01 b xoxe-abc-12-34 c");
    assert!(!out.contains("xapp-1234567890-AbCdEf_01"), "{out}");
    assert!(!out.contains("xoxe-abc-12-34"), "{out}");
    assert_eq!(out.matches(REDACTED_SECRET).count(), 2, "{out}");
}

#[test]
fn redact_secrets_scrubs_private_spans_dotall() {
    let out = redact_secrets("a <private>line1\nline2 secret</private> b");
    assert_eq!(out, format!("a {REDACTED_PRIVATE_TAG} b"));
}

#[test]
fn key_assigned_secrets_never_reach_log_bytes_for_each_key_and_form() {
    let keys = [
        "api_key",
        "secret",
        "token",
        "password",
        "MYSQL_PWD",
        "dbPassword",
        "authorization",
    ];
    let secret = "s3cr3t-value-XYZ";
    for key in keys {
        for form in [
            format!("{key}: {secret}"),
            format!("{key} = {secret}"),
            format!("{key}=\"{secret}\""),
            format!("{key}: '{secret}'"),
        ] {
            let (_dir, writer) = temp_writer();
            let (raw, record) = write_one(&writer, json!({"note": form}));
            assert!(!raw.contains(secret), "secret leaked for {form:?}: {raw}");
            assert!(
                raw.contains("[REDACTED"),
                "marker missing for {form:?}: {raw}"
            );
            let details = record["details"].to_string();
            assert!(
                details.contains(REDACTED_SECRET),
                "details not redacted: {details}"
            );
        }
    }
}

#[test]
fn bearer_authorization_header_never_reaches_log_bytes() {
    let (_dir, writer) = temp_writer();
    let token = "eyJhbGciOiJIUzI1NiJ9.payload.sig";
    let (raw, _) = write_one(
        &writer,
        json!({"headers": format!("Authorization: Bearer {token}")}),
    );
    assert!(!raw.contains(token), "bearer token leaked: {raw}");
    assert!(raw.contains("[REDACTED"), "marker missing: {raw}");
}

#[test]
fn slack_tokens_never_reach_log_bytes() {
    let (_dir, writer) = temp_writer();
    let xapp = "xapp-1234567890-AbCdEfGhIjKlMn_012";
    let xoxe = "xoxe-abcdef12-3456-7890ab-cdef";
    let (raw, _) = write_one(&writer, json!({"a": xapp, "b": xoxe}));
    assert!(!raw.contains(xapp), "xapp leaked: {raw}");
    assert!(!raw.contains(xoxe), "xoxe leaked: {raw}");
    assert!(raw.contains(REDACTED_SECRET), "marker missing: {raw}");
}

#[test]
fn private_span_never_reaches_log_bytes() {
    let (_dir, writer) = temp_writer();
    let inner = "top-secret multi line content";
    let (raw, record) = write_one(
        &writer,
        json!({"note": format!("<private>{inner}</private> done")}),
    );
    assert!(!raw.contains("top-secret"), "private span leaked: {raw}");
    assert!(raw.contains(REDACTED_PRIVATE_TAG), "marker missing: {raw}");
    assert!(record["details"].to_string().contains(REDACTED_PRIVATE_TAG));
}

#[test]
fn harness_and_project_string_fields_are_redacted_too() {
    let (_dir, writer) = temp_writer();
    writer
        .write_event(
            AuditEventType::AuthFailure,
            "harness token: abc123",
            "proj",
            json!({}),
        )
        .unwrap();
    let raw = raw_log(&writer);
    assert!(
        !raw.contains("abc123"),
        "secret in harness field leaked: {raw}"
    );
    let record = verify_record(raw.lines().last().unwrap());
    assert_eq!(
        record["agent_harness"].as_str().unwrap(),
        format!("harness token: {REDACTED_SECRET}")
    );
}

#[test]
fn nested_details_stay_structured_and_redacted() {
    let (_dir, writer) = temp_writer();
    let secret = "sk-abc-123";
    let (_, record) = write_one(
        &writer,
        json!({"config": {"api_key": secret}, "list": [format!("token: {secret}")]}),
    );
    let raw = raw_log(&writer);
    assert!(!raw.contains(secret), "secret leaked: {raw}");
    assert_eq!(
        record["details"]["config"]["api_key"].as_str().unwrap(),
        REDACTED_SECRET
    );
    assert_eq!(
        record["details"]["list"][0].as_str().unwrap(),
        format!("token: {REDACTED_SECRET}")
    );
}

#[test]
fn record_has_contract_fields_sorted_keys_line_and_matching_timestamps() {
    let (_dir, writer) = temp_writer();
    let before = OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000;
    let (raw, record) = write_one(&writer, json!({"b": 1, "a": 2}));
    let after = OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000;
    for field in [
        "ts",
        "epoch_ms",
        "event_type",
        "agent_harness",
        "project",
        "details",
        "record_sha256",
    ] {
        assert!(record.get(field).is_some(), "missing field {field}");
    }
    assert_eq!(record["event_type"].as_str().unwrap(), "ERROR");
    assert_eq!(record["agent_harness"].as_str().unwrap(), "test-harness");
    assert_eq!(record["project"].as_str().unwrap(), "test-project");
    assert!(
        raw.starts_with("{\"agent_harness\":"),
        "written line uses sorted-keys serialization: {raw}"
    );
    let ts = record["ts"].as_str().unwrap();
    let iso_utc = Regex::new(r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(\.\d+)?Z$").unwrap();
    assert!(iso_utc.is_match(ts), "ts must be ISO-8601 UTC: {ts}");
    let epoch_ms = record["epoch_ms"].as_i64().unwrap();
    assert!(
        (before..=after).contains(&(epoch_ms as i128)),
        "epoch_ms {epoch_ms} not in [{before}, {after}]"
    );
}

#[test]
fn append_per_write_produces_one_verified_line_per_event() {
    let (_dir, writer) = temp_writer();
    writer
        .write_event(AuditEventType::SessionResume, "h1", "p", json!({}))
        .unwrap();
    writer
        .write_event(AuditEventType::SessionCheckpoint, "h1", "p", json!({}))
        .unwrap();
    let raw = raw_log(&writer);
    let lines: Vec<&str> = raw.lines().collect();
    assert_eq!(lines.len(), 2, "one line per write: {raw:?}");
    let first = verify_record(lines[0]);
    let second = verify_record(lines[1]);
    assert_eq!(first["event_type"].as_str().unwrap(), "SESSION_RESUME");
    assert_eq!(second["event_type"].as_str().unwrap(), "SESSION_CHECKPOINT");
}

#[test]
fn rotation_safe_open_per_write_recreates_file_after_rename() {
    let (dir, writer) = temp_writer();
    writer
        .write_event(AuditEventType::IoDirectRead, "h", "p", json!({}))
        .unwrap();
    let rotated = dir.path().join("audit.jsonl.1");
    std::fs::rename(writer.path(), &rotated).unwrap(); // simulate logrotate
    writer
        .write_event(AuditEventType::MemSearchIndex, "h", "p", json!({}))
        .unwrap();
    let raw_new = raw_log(&writer);
    assert_eq!(
        raw_new.lines().count(),
        1,
        "log recreated at original path: {raw_new:?}"
    );
    verify_record(raw_new.lines().next().unwrap());
    let raw_old = std::fs::read_to_string(&rotated).unwrap();
    assert_eq!(raw_old.lines().count(), 1, "rotated file keeps its record");
    verify_record(raw_old.lines().next().unwrap());
}

#[test]
fn parent_directories_are_created_at_first_write() {
    let dir = tempfile::tempdir().unwrap();
    let writer = AuditWriter::new(dir.path().join("nested").join("deep").join("audit.jsonl"));
    writer
        .write_event(AuditEventType::TokenStats, "h", "p", json!({}))
        .unwrap();
    assert!(writer.path().is_file());
    verify_record(raw_log(&writer).lines().next().unwrap());
}

#[test]
fn every_event_type_is_unique_screaming_snake_case_and_written_verbatim() {
    let names: Vec<&'static str> = AuditEventType::ALL.iter().map(|e| e.as_str()).collect();
    // 17 since corpus-warmup-prune added the WARMUP (Req 1.8) and PRUNE
    // (Req 3.5) batch events.
    assert_eq!(names.len(), 17);
    let unique: std::collections::BTreeSet<_> = names.iter().collect();
    assert_eq!(unique.len(), names.len(), "event type names must be unique");
    for name in &names {
        assert!(
            name.chars().all(|c| c.is_ascii_uppercase() || c == '_'),
            "{name}"
        );
        assert!(!name.starts_with('_') && !name.ends_with('_'), "{name}");
    }
    let (_dir, writer) = temp_writer();
    for ev in AuditEventType::ALL {
        writer.write_event(ev, "h", "p", json!({})).unwrap();
        let last = raw_log(&writer).lines().last().unwrap().to_string();
        let record = verify_record(&last);
        assert_eq!(record["event_type"].as_str().unwrap(), ev.as_str());
    }
    assert_eq!(raw_log(&writer).lines().count(), AuditEventType::ALL.len());
}

#[test]
fn from_config_uses_agent_mem_log_file_env_path() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("custom-audit.jsonl");
    let mut env = std::collections::HashMap::new();
    env.insert(mmhmem::config::ENV_TOKEN.to_string(), "tok".to_string());
    env.insert(
        mmhmem::config::ENV_ALLOWED_ROOTS.to_string(),
        "/ws".to_string(),
    );
    env.insert(
        mmhmem::config::ENV_LOG_FILE.to_string(),
        log.to_string_lossy().into_owned(),
    );
    let cfg = mmhmem::config::Config::from_env(&|k| env.get(k).cloned()).unwrap();
    let writer = AuditWriter::from_config(&cfg);
    assert_eq!(writer.path(), log.as_path());
    writer
        .write_event(AuditEventType::AuthFailure, "h", "p", json!({}))
        .unwrap();
    assert!(log.is_file());
}
