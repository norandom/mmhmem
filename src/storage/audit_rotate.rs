//! Size-bounded audit rotation for the local profile (Req 7.3; design.md
//! "Local audit + rotation").
//!
//! Serve mode keeps the host-logrotate contract (rotation disabled,
//! `audit_max_bytes == 0`). Local mode rotates between writes: the audit
//! writer opens per write (O_APPEND, no persistent handle), so renaming the
//! active file between two appends is trivially safe — the same property
//! host logrotate relies on. Cross-process rotation races are benign and
//! tolerated: a racing writer may append one extra COMPLETE record into a
//! just-renamed file; records are per-line self-verifying (SHA-256 per
//! record), never truncated or rewritten.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Rotates `path` when it has reached `max_bytes`, keeping at most `keep`
/// rotated files (`path.1` newest … `path.<keep>` oldest). `max_bytes == 0`
/// disables rotation. A missing active file is a no-op.
pub fn rotate_if_needed(path: &Path, max_bytes: u64, keep: u32) -> io::Result<()> {
    if max_bytes == 0 || keep == 0 {
        return Ok(());
    }
    let Ok(meta) = fs::metadata(path) else {
        return Ok(());
    };
    if meta.len() < max_bytes {
        return Ok(());
    }
    let keep = keep as usize;
    // Vacate the retention edge and drop any pre-existing overflow beyond
    // `keep` slots (e.g. after shrinking the retention setting): after
    // rotation, at most `keep` rotated files exist.
    cleanup_at_or_beyond(path, keep)?;
    // Shift .(keep-1) → .keep, …, .1 → .2. A NotFound race with another
    // rotating process is benign and skipped; mid-slot destinations were
    // vacated by the cascade order, so no overwrite is needed.
    for i in (1..keep).rev() {
        let from = rotated_path(path, i);
        let to = rotated_path(path, i + 1);
        if from.exists() {
            if let Err(e) = fs::rename(&from, &to) {
                if e.kind() != io::ErrorKind::NotFound {
                    return Err(e);
                }
            }
        }
    }
    // Active file becomes the newest rotated slot; the next append
    // re-creates the active file (O_APPEND + create).
    fs::rename(path, rotated_path(path, 1))
}

/// Removes rotated siblings with slot numbers ≥ `keep` (the edge slot is
/// vacated for the shift; overflow beyond it is dropped entirely).
fn cleanup_at_or_beyond(path: &Path, keep: usize) -> io::Result<()> {
    let name = match path.file_name() {
        Some(n) => n.to_os_string(),
        None => return Ok(()),
    };
    let prefix = {
        let mut p = name.clone();
        p.push(".");
        p
    };
    let parent = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("."),
    };
    for entry in fs::read_dir(&parent)? {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let file_name = entry.file_name();
        let bytes = file_name.as_encoded_bytes();
        let Some(rest) = bytes.strip_prefix(prefix.as_encoded_bytes()) else {
            continue;
        };
        if rest.iter().all(|b| b.is_ascii_digit()) && !rest.is_empty() {
            let slot: usize = match std::str::from_utf8(rest).ok().and_then(|s| s.parse().ok()) {
                Some(n) => n,
                None => continue,
            };
            if slot >= keep {
                if let Err(e) = fs::remove_file(entry.path()) {
                    if e.kind() != io::ErrorKind::NotFound {
                        return Err(e);
                    }
                }
            }
        }
    }
    Ok(())
}

/// The Nth rotated sibling of `path` (`audit.jsonl.1`, …).
fn rotated_path(path: &Path, n: usize) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(format!(".{n}"));
    PathBuf::from(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::audit::{AuditEventType, AuditWriter};

    fn dir() -> std::path::PathBuf {
        tempfile::tempdir().unwrap().keep()
    }

    fn write_file(path: &Path, bytes: &[u8]) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, bytes).unwrap();
    }

    #[test]
    fn missing_active_file_and_disabled_rotation_are_noops() {
        let d = dir();
        let p = d.join("audit.jsonl");
        rotate_if_needed(&p, 0, 5).unwrap();
        rotate_if_needed(&p, 1024, 5).unwrap();
        assert!(!p.exists());
    }

    #[test]
    fn below_threshold_does_not_rotate() {
        let d = dir();
        let p = d.join("audit.jsonl");
        write_file(&p, b"small");
        rotate_if_needed(&p, 4096, 5).unwrap();
        assert!(p.exists(), "active file stays in place below threshold");
        assert!(!rotated_path(&p, 1).exists());
    }

    #[test]
    fn at_threshold_active_moves_to_slot_one() {
        let d = dir();
        let p = d.join("audit.jsonl");
        write_file(&p, b"0123456789"); // 10 bytes
        rotate_if_needed(&p, 10, 5).unwrap();
        assert!(!p.exists(), "the oversized active file is renamed away");
        let first = rotated_path(&p, 1);
        assert_eq!(std::fs::read(&first).unwrap().as_slice(), b"0123456789");
    }

    #[test]
    fn retention_count_is_respected() {
        let d = dir();
        let p = d.join("audit.jsonl");
        write_file(&rotated_path(&p, 1), b"one");
        write_file(&rotated_path(&p, 2), b"two");
        write_file(&rotated_path(&p, 3), b"three");
        write_file(&p, b"0123456789");
        rotate_if_needed(&p, 10, 2).unwrap();
        assert!(
            !rotated_path(&p, 3).exists(),
            "oldest slot dropped (keep=2)"
        );
        assert_eq!(
            std::fs::read(rotated_path(&p, 2)).unwrap().as_slice(),
            b"one"
        );
        assert_eq!(
            std::fs::read(rotated_path(&p, 1)).unwrap().as_slice(),
            b"0123456789"
        );
    }

    #[test]
    fn keep_one_rotates_to_a_single_slot() {
        let d = dir();
        let p = d.join("audit.jsonl");
        write_file(&rotated_path(&p, 1), b"old");
        write_file(&p, b"0123456789");
        rotate_if_needed(&p, 10, 1).unwrap();
        assert!(!rotated_path(&p, 2).exists());
        assert_eq!(
            std::fs::read(rotated_path(&p, 1)).unwrap().as_slice(),
            b"0123456789"
        );
    }

    #[test]
    fn writer_rotates_between_appends_and_records_stay_verifiable() {
        // Writer-level: tiny threshold, keep large enough that nothing is
        // deleted yet — proving rotation loses NO records. Retention-driven
        // deletion is covered by the unit tests above.
        let d = dir();
        let p = d.join("audit.jsonl");
        let writer = AuditWriter::with_rotation(p.clone(), 200, 30);
        for i in 0..30 {
            writer
                .write_event(
                    AuditEventType::SessionCheckpoint,
                    "opencode",
                    "proj",
                    serde_json::json!({ "i": i, "api_key": "sk-secret-value" }),
                )
                .unwrap();
        }
        assert!(rotated_path(&p, 1).exists(), "rotation must have happened");
        // Every line in the active file and all rotated files parses, is
        // redacted, and carries a digest; all 30 records survive.
        let mut files = vec![p.clone()];
        files.extend(
            (1..=30)
                .filter(|i| rotated_path(&p, *i).exists())
                .map(|i| rotated_path(&p, i)),
        );
        let mut records = 0;
        let mut seen_i = std::collections::BTreeSet::new();
        for f in &files {
            if !f.exists() {
                continue;
            }
            for line in std::fs::read_to_string(f).unwrap().lines() {
                let v: serde_json::Value = serde_json::from_str(line).unwrap();
                assert!(v.get("record_sha256").is_some(), "digest present");
                assert!(line.contains("[REDACTED_SECRET]"), "redaction intact");
                seen_i.insert(v["details"]["i"].as_i64().unwrap());
                records += 1;
            }
        }
        assert_eq!(records, 30, "no record lost or duplicated");
        assert_eq!(
            seen_i.iter().copied().collect::<Vec<_>>(),
            (0..30).collect::<Vec<_>>(),
            "every event id present exactly once"
        );
    }
}
