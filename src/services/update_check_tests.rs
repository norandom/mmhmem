//! Tests for the update notice cache and version compare. No network: the
//! lookup is always an injected closure.

use super::*;
use std::cell::Cell;
use std::collections::HashMap;

const DAY: i64 = 24 * 60 * 60;
const NOW: i64 = 1_800_000_000;

fn cache(tag: Option<&str>, checked_at: i64) -> UpdateCache {
    UpdateCache {
        checked_at,
        newest_tag: tag.map(str::to_string),
    }
}

fn env(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

#[test]
fn missing_cache_refreshes_and_writes_the_tag() {
    let dir = tempfile::tempdir().unwrap();
    let path = cache_path(dir.path());
    assert!(refresh(&path, NOW, || Some("v0.5.2".into())));
    assert_eq!(read_cache(&path), Some(cache(Some("v0.5.2"), NOW)));
}

#[test]
fn fresh_cache_skips_the_lookup() {
    let dir = tempfile::tempdir().unwrap();
    let path = cache_path(dir.path());
    refresh(&path, NOW, || Some("v0.5.2".into()));
    let called = Cell::new(false);
    let ran = refresh(&path, NOW + DAY - 1, || {
        called.set(true);
        Some("v9.9.9".into())
    });
    assert!(!ran && !called.get());
    assert_eq!(
        read_cache(&path).unwrap().newest_tag.as_deref(),
        Some("v0.5.2")
    );
}

#[test]
fn stale_cache_refreshes() {
    let dir = tempfile::tempdir().unwrap();
    let path = cache_path(dir.path());
    refresh(&path, NOW, || Some("v0.5.2".into()));
    assert!(refresh(&path, NOW + DAY, || Some("v0.6.0".into())));
    assert_eq!(read_cache(&path), Some(cache(Some("v0.6.0"), NOW + DAY)));
}

#[test]
fn cache_from_the_future_counts_as_stale() {
    assert!(needs_refresh(Some(&cache(None, NOW + 10)), NOW));
}

#[test]
fn corrupt_cache_reads_as_missing_and_is_replaced() {
    let dir = tempfile::tempdir().unwrap();
    let path = cache_path(dir.path());
    std::fs::write(&path, "{not json").unwrap();
    assert_eq!(read_cache(&path), None);
    assert_eq!(notice("0.5.1", read_cache(&path).as_ref()), None);
    assert!(refresh(&path, NOW, || Some("v0.5.2".into())));
    assert_eq!(read_cache(&path), Some(cache(Some("v0.5.2"), NOW)));
}

#[test]
fn failed_lookup_keeps_the_known_tag_and_records_the_attempt() {
    let dir = tempfile::tempdir().unwrap();
    let path = cache_path(dir.path());
    refresh(&path, NOW, || Some("v0.5.2".into()));
    assert!(refresh(&path, NOW + DAY, || None));
    assert_eq!(read_cache(&path), Some(cache(Some("v0.5.2"), NOW + DAY)));
}

#[test]
fn offline_first_run_writes_no_tag_and_no_notice() {
    let dir = tempfile::tempdir().unwrap();
    let path = cache_path(dir.path());
    assert!(refresh(&path, NOW, || None));
    assert_eq!(read_cache(&path), Some(cache(None, NOW)));
    assert_eq!(notice("0.5.1", read_cache(&path).as_ref()), None);
}

#[test]
fn lookup_returning_a_prerelease_is_not_stored() {
    let dir = tempfile::tempdir().unwrap();
    let path = cache_path(dir.path());
    refresh(&path, NOW, || Some("v0.6.0-rc.1".into()));
    assert_eq!(read_cache(&path), Some(cache(None, NOW)));
}

#[test]
fn notice_only_for_a_newer_stable_release() {
    let line = notice("0.5.1", Some(&cache(Some("v0.5.2"), NOW))).unwrap();
    assert_eq!(
        line,
        "mmhmem v0.5.2 is available (installed: v0.5.1). To update, run in a terminal: mmhmem update"
    );
    assert!(notice("0.5.1", Some(&cache(Some("v1.0.0"), NOW))).is_some());
    assert!(notice("0.5.1", Some(&cache(Some("v0.10.0"), NOW))).is_some());
    // Equal and older: nothing.
    assert_eq!(notice("0.5.1", Some(&cache(Some("v0.5.1"), NOW))), None);
    assert_eq!(notice("0.5.1", Some(&cache(Some("v0.5.0"), NOW))), None);
    assert_eq!(notice("0.5.1", Some(&cache(Some("v0.4.9"), NOW))), None);
}

#[test]
fn prereleases_and_malformed_tags_are_ignored() {
    for tag in [
        "v0.6.0-rc.1",
        "v0.6.0+build",
        "0.6.0",
        "v0.6",
        "v01.0.0",
        "vx.y.z",
        "",
    ] {
        assert_eq!(notice("0.5.1", Some(&cache(Some(tag), NOW))), None, "{tag}");
    }
    assert_eq!(notice("0.5.1", Some(&cache(None, NOW))), None);
    assert_eq!(notice("0.5.1", None), None);
}

#[test]
fn prerelease_build_sees_its_own_stable_release() {
    assert!(notice("0.6.0-dev", Some(&cache(Some("v0.6.0"), NOW))).is_some());
    assert_eq!(notice("0.6.0-dev", Some(&cache(Some("v0.5.9"), NOW))), None);
}

#[test]
fn enabled_only_in_local_mode_without_opt_out() {
    let none = env(&[]);
    let get = |k: &str| none.get(k).cloned();
    assert!(enabled(Profile::Local, &get, None));
    assert!(enabled(Profile::Local, &get, Some(true)));
    assert!(!enabled(Profile::Local, &get, Some(false)));
    assert!(!enabled(Profile::Serve, &get, None));
    assert!(!enabled(Profile::Serve, &get, Some(true)));
}

#[test]
fn env_opt_out_turns_the_check_off() {
    for value in ["1", "true", "yes"] {
        let map = env(&[(ENV_NO_UPDATE_CHECK, value)]);
        let get = |k: &str| map.get(k).cloned();
        assert!(!enabled(Profile::Local, &get, None), "{value}");
    }
    for value in ["", "0", "false"] {
        let map = env(&[(ENV_NO_UPDATE_CHECK, value)]);
        let get = |k: &str| map.get(k).cloned();
        assert!(enabled(Profile::Local, &get, None), "{value:?}");
    }
}

#[test]
fn update_notice_reads_only_the_cache() {
    let dir = tempfile::tempdir().unwrap();
    let n = UpdateNotice {
        cache_file: cache_path(dir.path()),
    };
    assert_eq!(n.line(), None);
    std::fs::write(&n.cache_file, r#"{"checked_at":1,"newest_tag":"v999.0.0"}"#).unwrap();
    assert!(
        n.line()
            .unwrap()
            .starts_with("mmhmem v999.0.0 is available")
    );
}
