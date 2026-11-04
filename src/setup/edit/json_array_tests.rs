//! Golden tests for element-level array edits (task 2.2): our Claude Code
//! hook group is appended to and removed from `hooks.PreToolUse` while the
//! user's own groups stay byte-identical.

use super::*;
use serde_json::json;

const HOOKS: &[&str] = &["hooks", "PreToolUse"];

fn ours() -> Value {
    json!({
        "matcher": "Read",
        "hooks": [{"type": "command", "command": "/opt/mmhmem/bin/mmhmem hook read"}]
    })
}

fn is_ours(value: &Value) -> bool {
    value.get("matcher") == Some(&json!("Read")) && value.to_string().contains("mmhmem hook read")
}

/// Two user hook groups with odd formatting and comments-free strict JSON.
const SETTINGS: &str = "{\r\n  \"model\": \"opus\",\r\n  \"hooks\": {\r\n    \"PreToolUse\": [\r\n      { \"matcher\":\"Bash\",  \"hooks\": [ {\"type\":\"command\",\"command\":\"audit.sh\"} ] },\r\n      {\r\n\t\"matcher\": \"Write\",\r\n\t\"hooks\": [{ \"type\": \"command\", \"command\": \"lint.sh\" }]\r\n      }\r\n    ]\r\n  },\r\n  \"theme\": \"dark\"\r\n}\r\n";

#[test]
fn append_then_remove_ours_is_byte_identical() {
    let added = array_append(SETTINGS, HOOKS, &ours()).unwrap();
    assert!(added.starts_with(&SETTINGS[..SETTINGS.find("\r\n      }").unwrap()]));
    assert_eq!(dialect(&added).unwrap(), Dialect::Json);
    assert!(
        !added.replace("\r\n", "").contains('\n'),
        "bare LF:\n{added:?}"
    );
    let found = array_find(&added, HOOKS, is_ours).unwrap();
    assert_eq!(found, vec![2]);
    assert_eq!(array_remove(&added, HOOKS, found[0]).unwrap(), SETTINGS);
}

#[test]
fn append_to_absent_array_then_prune_is_byte_identical() {
    let inputs = [
        "{\n  \"model\": \"opus\"\n}\n".to_owned(),
        "{\n  // user settings\n  \"hooks\": {}\n}\n".to_owned(),
        "\u{feff}{\r\n  \"hooks\": {\r\n    \"Stop\": []\r\n  }\r\n}\r\n".to_owned(),
        "{}".to_owned(),
    ];
    for input in &inputs {
        let created = missing_keys(input, HOOKS).unwrap();
        let added = array_append(input, HOOKS, &ours()).unwrap();
        assert_eq!(array_find(&added, HOOKS, is_ours).unwrap(), vec![0]);
        let removed = array_remove_and_prune(&added, HOOKS, 0, created).unwrap();
        assert_eq!(&removed, input, "added:\n{added}");
    }
}

#[test]
fn remove_first_middle_and_last_keeps_commas_valid() {
    let input = "{\n  \"a\": [\n    1, // one\n    2,\n    3\n  ]\n}\n";
    let cases = [
        (0, "{\n  \"a\": [\n    2,\n    3\n  ]\n}\n"),
        (1, "{\n  \"a\": [\n    1, // one\n    3\n  ]\n}\n"),
        (2, "{\n  \"a\": [\n    1, // one\n    2\n  ]\n}\n"),
    ];
    for (index, expected) in cases {
        let out = array_remove(input, &["a"], index).unwrap();
        assert_eq!(out, expected, "index {index}");
    }
    let inline = "{\"a\": [1, 2, 3]}";
    assert_eq!(array_remove(inline, &["a"], 0).unwrap(), "{\"a\": [2, 3]}");
    assert_eq!(array_remove(inline, &["a"], 1).unwrap(), "{\"a\": [1, 3]}");
    assert_eq!(array_remove(inline, &["a"], 2).unwrap(), "{\"a\": [1, 2]}");
    let trailing = "{\n  \"a\": [\n    1,\n    2,\n  ],\n}\n";
    let out = array_remove(trailing, &["a"], 1).unwrap();
    assert_eq!(out, "{\n  \"a\": [\n    1,\n  ],\n}\n");
}

#[test]
fn append_follows_trailing_comma_style() {
    let input = "{\n  \"a\": [\n    1,\n  ],\n}\n";
    let out = array_append(input, &["a"], &json!(2)).unwrap();
    assert_eq!(out, "{\n  \"a\": [\n    1,\n    2,\n  ],\n}\n");
    assert_eq!(array_remove(&out, &["a"], 1).unwrap(), input);
}

#[test]
fn out_of_range_or_absent_is_a_no_op() {
    assert_eq!(array_remove(SETTINGS, HOOKS, 9).unwrap(), SETTINGS);
    assert_eq!(
        array_remove(SETTINGS, &["hooks", "Stop"], 0).unwrap(),
        SETTINGS
    );
    assert_eq!(
        array_find(SETTINGS, &["hooks", "Stop"], |_| true).unwrap(),
        Vec::<usize>::new()
    );
    assert_eq!(
        array_find(SETTINGS, HOOKS, is_ours).unwrap(),
        Vec::<usize>::new()
    );
    assert_eq!(array_find(SETTINGS, HOOKS, |_| true).unwrap(), vec![0, 1]);
}

#[test]
fn non_array_and_empty_path_are_errors() {
    assert_eq!(
        array_append(SETTINGS, &["theme"], &ours()),
        Err(JsonEditError::NotAnArray)
    );
    assert_eq!(
        array_remove(SETTINGS, &["hooks"], 0),
        Err(JsonEditError::NotAnArray)
    );
    assert_eq!(
        array_find(SETTINGS, &[], |_| true),
        Err(JsonEditError::EmptyPath)
    );
    assert_eq!(missing_keys(SETTINGS, HOOKS).unwrap(), 0);
    assert_eq!(missing_keys("{}", HOOKS).unwrap(), 2);
}

#[test]
fn invalid_input_reports_position_without_secrets() {
    let input =
        "{\n  \"env\": {\"TOKEN\": \"sk-SECRET123\"},\n  \"hooks\": {\"PreToolUse\": [1 2]}\n}";
    for err in [
        array_append(input, HOOKS, &ours()).unwrap_err(),
        array_remove(input, HOOKS, 0).unwrap_err(),
        array_find(input, HOOKS, |_| true).unwrap_err(),
    ] {
        assert!(
            matches!(err, JsonEditError::Parse { line: 3, .. }),
            "{err:?}"
        );
        assert!(!format!("{err} {err:?}").contains("SECRET"));
    }
}
