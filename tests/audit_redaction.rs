//! Audit-writer tests (task 2.2). design.md "File Structure Plan" puts
//! `tests/audit_redaction.rs` here ("secret patterns never reach disk;
//! record hash verifies"); the tasks.md 2.2 test matrix lives in this
//! integration test so `src/storage/audit.rs` stays under the Req 13.3
//! 500-LOC-per-src-file cap. Everything asserted here goes through the
//! crate's public API only (`mmhmem::storage::audit`).

use std::collections::BTreeMap;

use mmhmem::config::{Config, ENV_ALLOWED_ROOTS, ENV_SHUNT_MIN_LINES, ENV_TOKEN};
use mmhmem::services::session_bus::SessionBus;
use mmhmem::services::shunt::ShuntEngine;
use mmhmem::services::shunt_content::{ContentKind, ContentOutcome, ContentRequest};
use mmhmem::storage::audit::{
    AuditEventType, AuditWriter, REDACTED_PRIVATE_TAG, REDACTED_SECRET, redact_secrets,
};
use mmhmem::storage::db::Db;
use regex::Regex;
use serde_json::{Value, json};
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
    // corpus-warmup-prune added PRUNE (Req 3.5) and MEM_SEARCH_BREADCRUMBS
    // (Req 10); read-routing-hooks added HOOK_REDIRECT, HOOK_PASS;
    // reliable-installers added STATUS_CHECK (Req 6.1);
    // shunt-content-tool added CONTENT_CACHE_EVICT (Req 4.3);
    // shunt-cache-efficiency added CACHE_MIGRATED (task 1.2); the
    // 2026-11-10 restore removed the batch summary event → 22.
    assert_eq!(names.len(), 22);
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

// ---------- shunt-content-tool task 3.2: content-bypass redaction (Req 6.4) ----------
//
// design.md "Testing strategy / Audit": secrets in content never reach
// audit or breadcrumbs. tests/shunt_content_bypass.rs (task 2.1) pinned
// the bypass audit SHAPE; this sweep drives real `shunt_content`
// bypasses — the full Req 3.2 secret corpus plus the hint-path and
// below-threshold rules — through the real writer, store and session
// bus, and proves the raw audit BYTES and breadcrumb rows stay clean.
// Content bypasses are hash-only by construction (Req 3.4), so nothing
// is ever left for the writer's redaction to catch.

/// Engine plus live store and audit log for real content bypass calls.
/// No worker: every case here bypasses before the worker stage
/// (Req 3.1-3.3), so none is ever needed.
struct ContentEngine {
    #[allow(dead_code)]
    data: tempfile::TempDir,
    logs: tempfile::TempDir,
    db: Db,
    eng: ShuntEngine,
}

fn content_engine() -> ContentEngine {
    let data = tempfile::tempdir().unwrap();
    let logs = tempfile::tempdir().unwrap();
    let mut env = std::collections::HashMap::new();
    env.insert(ENV_TOKEN.to_string(), "tok".to_string());
    env.insert(
        ENV_ALLOWED_ROOTS.to_string(),
        data.path().display().to_string(),
    );
    env.insert(ENV_SHUNT_MIN_LINES.to_string(), "150".to_string());
    let cfg = Config::from_env(&|k| env.get(k).cloned()).unwrap();
    let db = Db::open(data.path()).unwrap();
    let audit = AuditWriter::new(logs.path().join("audit.jsonl"));
    let bus = SessionBus::new(db.clone(), audit.clone());
    let eng = ShuntEngine::new(db.clone(), audit, cfg, Some(bus), None);
    ContentEngine {
        data,
        logs,
        db,
        eng,
    }
}

/// A FileRead content request with an optional source hint.
fn content_req<'a>(content: &'a str, hint: Option<&'a str>) -> ContentRequest<'a> {
    ContentRequest {
        project: "redaction",
        harness: "audit-test",
        task: None,
        content,
        kind: ContentKind::FileRead,
        source_hint: hint,
        question: "q",
    }
}

/// `total` filler lines with `inject` replacing the middle line, so the
/// corpus sits above the 150-line threshold unless a rule bypasses it.
fn filler_with(total: usize, inject: &str) -> String {
    let mut lines: Vec<String> = (1..=total)
        .map(|i| format!("plain filler row {i}"))
        .collect();
    lines[total / 2] = inject.to_string();
    lines.join("\n")
}

#[tokio::test]
async fn content_bypass_secrets_never_reach_audit_bytes_or_breadcrumbs() {
    let ce = content_engine();
    // The Req 3.2 corpus (PEM header, cloud key id, api-key assignment,
    // bearer token, secret environment line), each embedded mid-content.
    let pem =
        "-----BEGIN RSA PRIVATE KEY-----\nMIIEowIBAAKCAQEAxKEYMAT\n-----END RSA PRIVATE KEY-----";
    let cloud = "the key id is AKIAIOSFODNN7EXAMPLE here";
    let assignment = "api_key = sk-W9Xqz0123456789bCdEf";
    let bearer = "Authorization: Bearer eyJhbGciOiJIUzI1NiJ9.payload.sig";
    let env_line = "GITHUB_TOKEN=ghp_16C7e42F292c6912E7710c838347Ae178B4a";
    let mut cases: Vec<(String, Option<&str>)> = [pem, cloud, assignment, bearer, env_line]
        .iter()
        .map(|p| (filler_with(160, p), None))
        .collect();
    cases.push((
        filler_with(160, "mid benign row"),
        Some("src/auth/token.rs"),
    ));
    cases.push((filler_with(149, "mid benign row"), None));
    for (content, hint) in &cases {
        let out = ce
            .eng
            .shunt_content(&content_req(content, *hint))
            .await
            .unwrap();
        assert!(
            matches!(out, ContentOutcome::Bypassed { .. }),
            "every corpus case must bypass, got {out:?}"
        );
    }

    // The raw audit bytes: no secret payload, no benign content line.
    let raw = std::fs::read_to_string(ce.logs.path().join("audit.jsonl")).unwrap();
    let needles = [
        pem,
        cloud,
        assignment,
        bearer,
        env_line,
        "MIIEowIBAAKCAQEAxKEYMAT",
        "AKIAIOSFODNN7EXAMPLE",
        "sk-W9Xqz0123456789bCdEf",
        "eyJhbGciOiJIUzI1NiJ9.payload.sig",
        "ghp_16C7e42F292c6912E7710c838347Ae178B4a",
        "plain filler row",
        "mid benign row",
    ];
    for needle in needles {
        assert!(
            !raw.contains(needle),
            "leak into audit bytes: {needle} in {raw}"
        );
    }

    // Every bypass line is a verified record with hash-only details
    // (source label + optional hint, kind, sha, lines, reason).
    let allowed: BTreeMap<&str, ()> = [
        "bypass_reason",
        "hint",
        "kind",
        "sha256",
        "source",
        "total_lines",
    ]
    .into_iter()
    .map(|k| (k, ()))
    .collect();
    let mut bypass_events = 0usize;
    for line in raw.lines() {
        let record = verify_record(line);
        if record["event_type"].as_str() != Some("IO_DIRECT_READ") {
            continue;
        }
        bypass_events += 1;
        let d = &record["details"];
        assert_eq!(d["source"], "content", "content bypass label: {line}");
        assert_eq!(d["kind"], "file_read", "kind carried: {line}");
        let sha = d["sha256"].as_str().unwrap();
        assert_eq!(sha.len(), 64, "content hash carried: {line}");
        assert!(
            d.as_object()
                .unwrap()
                .keys()
                .all(|k| allowed.contains_key(k.as_str())),
            "hash-only details (never content): {line}"
        );
    }
    assert_eq!(
        bypass_events, 7,
        "exactly one audit event per bypass: {raw}"
    );

    // The breadcrumb rows the bypasses appended stay clean too.
    let crumbs: Vec<(String, String)> = ce
        .db
        .with_conn(|c| {
            let mut s = c
                .prepare("SELECT target_resource, action_summary FROM session_breadcrumbs")
                .unwrap();
            let rows = s
                .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap();
            Ok(rows)
        })
        .await
        .unwrap();
    assert_eq!(crumbs.len(), 7, "one crumb per bypass");
    for (resource, summary) in &crumbs {
        for needle in needles {
            assert!(
                !resource.contains(needle),
                "leak into crumb resource: {resource}"
            );
            assert!(
                !summary.contains(needle),
                "leak into crumb summary: {summary}"
            );
        }
    }
}
