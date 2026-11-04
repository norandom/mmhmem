//! Golden tests for JSON/JSONC entry edits (task 2.2): add followed by remove
//! is byte-identical, replacing a value touches only its span, and invalid
//! input is reported by line and column without echoing document text.

use super::*;
use serde_json::json;

const MCP_PATH: &[&str] = &["mcp", "servers", "mmhmem"];

fn entry() -> Value {
    json!({"type": "local", "command": ["/opt/mmhmem/bin/mmhmem", "stdio"]})
}

/// `set` then `remove_and_prune` with the `missing_parents` count taken
/// before `set` must give back the input; with existing parents a plain
/// `remove` must too.
fn assert_round_trip(input: &str, path: &[&str]) -> String {
    assert_eq!(
        get(input, path).unwrap(),
        None,
        "fixture already has the key"
    );
    let created = missing_parents(input, path).unwrap();
    let added = set(input, path, &entry()).unwrap();
    assert_eq!(get(&added, path).unwrap(), Some(entry()));
    let removed = remove_and_prune(&added, path, created).unwrap();
    assert_eq!(removed, input, "round trip changed bytes; added:\n{added}");
    if created == 0 {
        assert_eq!(remove(&added, path).unwrap(), input);
    }
    added
}

/// Replacing the value of `path` must keep everything outside `old`'s span.
fn assert_span_only(input: &str, path: &[&str], old: &str, value: &Value) -> String {
    let start = input.find(old).expect("old value text in fixture");
    let end = start + old.len();
    let out = set(input, path, value).unwrap();
    assert!(out.starts_with(&input[..start]), "prefix changed:\n{out}");
    assert!(out.ends_with(&input[end..]), "suffix changed:\n{out}");
    assert_eq!(get(&out, path).unwrap().as_ref(), Some(value));
    out
}

const COMMENTS: &str = r#"// Claude Desktop config
{
  /* servers the app starts */
  "mcpServers": {
    "other": { "command": "other" } // keep me
  },
  "theme": "dark" /* trailing block */
}
"#;

#[test]
fn round_trip_line_and_block_comments() {
    let added = assert_round_trip(COMMENTS, &["mcpServers", "mmhmem"]);
    assert!(added.contains("// keep me"));
    assert!(added.contains("/* trailing block */"));
    assert_eq!(dialect(&added).unwrap(), Dialect::Jsonc);
}

const ODD_WHITESPACE: &str =
    "{\n\t\"mcpServers\" :{\n\t\t\"a\"  :   1 ,\n\t\t\"b\":2\n\t} ,\n   \"x\":  true\n}";

#[test]
fn round_trip_odd_whitespace_uses_tab_indent() {
    let added = assert_round_trip(ODD_WHITESPACE, &["mcpServers", "mmhmem"]);
    assert!(added.contains("\n\t\t\"mmhmem\": {"), "added:\n{added}");
}

const TRAILING_COMMAS: &str =
    "{\n  \"mcp\": {\n    \"servers\": {\n      \"x\": 1,\n    },\n  },\n}\n";

#[test]
fn round_trip_trailing_commas() {
    let added = assert_round_trip(TRAILING_COMMAS, MCP_PATH);
    assert!(added.contains("\"x\": 1,\n"), "added:\n{added}");
}

const CRLF: &str = "{\r\n  \"mcpServers\": {\r\n    \"a\": 1\r\n  }\r\n}\r\n";

#[test]
fn round_trip_crlf_keeps_crlf_in_new_entry() {
    let added = assert_round_trip(CRLF, &["mcpServers", "mmhmem"]);
    assert!(
        !added.replace("\r\n", "").contains('\n'),
        "bare LF in:\n{added:?}"
    );
}

#[test]
fn round_trip_bom_is_kept() {
    let input = format!("\u{feff}{CRLF}");
    let added = assert_round_trip(&input, &["mcpServers", "mmhmem"]);
    assert!(added.starts_with('\u{feff}'));
    assert_eq!(dialect(&input).unwrap(), Dialect::Json);
}

fn deep_document(levels: usize) -> (String, Vec<String>) {
    let keys: Vec<String> = (0..levels).map(|i| format!("k{i}")).collect();
    let mut text = String::new();
    for (depth, key) in keys.iter().enumerate() {
        let pad = "  ".repeat(depth + 1);
        text.push_str(&format!("{{\n{pad}\"{key}\": "));
    }
    text.push_str("{\n");
    text.push_str(&format!("{}\"leaf\": [1, 2]\n", "  ".repeat(levels + 1)));
    for depth in (0..=levels).rev() {
        text.push_str(&format!("{}}}\n", "  ".repeat(depth)));
    }
    (text.trim_end().to_owned() + "\n", keys)
}

#[test]
fn round_trip_deep_nesting_beyond_twelve_levels() {
    let (input, keys) = deep_document(14);
    let mut path: Vec<&str> = keys.iter().map(String::as_str).collect();
    path.push("mmhmem");
    let added = assert_round_trip(&input, &path);
    assert!(added.contains("\"leaf\": [1, 2]"));
    let mut leaf = path.clone();
    leaf.pop();
    leaf.push("leaf");
    assert_eq!(get(&input, &leaf).unwrap(), Some(json!([1, 2])));
}

#[test]
fn round_trip_empty_object_document() {
    assert_round_trip("{}", &["mmhmem"]);
    assert_round_trip("{}\n", MCP_PATH);
}

#[test]
fn empty_file_gets_an_object() {
    let added = set("", MCP_PATH, &entry()).unwrap();
    assert_eq!(get(&added, MCP_PATH).unwrap(), Some(entry()));
    assert_eq!(missing_parents("", MCP_PATH).unwrap(), 2);
    assert_eq!(remove_and_prune(&added, MCP_PATH, 2).unwrap(), "{}\n");
    assert_eq!(get("", MCP_PATH).unwrap(), None);
    assert_eq!(remove("", MCP_PATH).unwrap(), "");
}

const EMPTY_PARENT: &str =
    "{\n  \"$schema\": \"https://opencode.ai/config.json\",\n  \"mcp\": {}\n}\n";

#[test]
fn round_trip_empty_parent_object() {
    assert_round_trip(EMPTY_PARENT, MCP_PATH);
    assert_eq!(missing_parents(EMPTY_PARENT, MCP_PATH).unwrap(), 1);
    let added = set(EMPTY_PARENT, MCP_PATH, &entry()).unwrap();
    // Plain remove keeps the created "servers" parent.
    assert_eq!(
        get(&remove(&added, MCP_PATH).unwrap(), &["mcp", "servers"]).unwrap(),
        Some(json!({}))
    );
}

const OPENCODE: &str = r#"{
  "$schema": "https://opencode.ai/config.json",
  // MCP servers OpenCode starts
  "mcp": {
    "servers": {
      // the memory server
      "mmhmem": { "type": "local", "command": ["old", "stdio"] }, // installed 2025
      /* another server */
      "github": { "type": "remote", "url": "https://example.test" },
    },
  },
}
"#;

#[test]
fn replace_existing_entry_changes_only_its_span() {
    let old = r#"{ "type": "local", "command": ["old", "stdio"] }"#;
    let out = assert_span_only(OPENCODE, MCP_PATH, old, &entry());
    assert!(out.contains("// the memory server\n      \"mmhmem\": {"));
    assert!(out.contains("}, // installed 2025\n"));
}

#[test]
fn replace_scalar_changes_only_its_span() {
    assert_span_only(COMMENTS, &["theme"], "\"dark\"", &json!("light"));
}

#[test]
fn set_equal_value_is_a_no_op() {
    let current = json!({"type": "local", "command": ["old", "stdio"]});
    assert_eq!(set(OPENCODE, MCP_PATH, &current).unwrap(), OPENCODE);
}

#[test]
fn remove_existing_entry_keeps_neighbours_and_comments() {
    let out = remove(OPENCODE, MCP_PATH).unwrap();
    assert_eq!(get(&out, MCP_PATH).unwrap(), None);
    assert!(out.contains("// MCP servers OpenCode starts"));
    assert!(out.contains("/* another server */"));
    assert!(!out.contains("installed 2025"));
    assert!(get(&out, &["mcp", "servers", "github"]).unwrap().is_some());
}

#[test]
fn remove_absent_key_returns_input() {
    for input in [COMMENTS, ODD_WHITESPACE, TRAILING_COMMAS, CRLF, "{}", ""] {
        assert_eq!(remove(input, MCP_PATH).unwrap(), input);
        assert_eq!(remove_and_prune(input, MCP_PATH, 2).unwrap(), input);
    }
    // A non-object parent means the key is absent.
    assert_eq!(remove(COMMENTS, &["theme", "x"]).unwrap(), COMMENTS);
}

#[test]
fn get_reads_values_through_comments() {
    assert_eq!(
        get(OPENCODE, &["mcp", "servers", "github", "type"]).unwrap(),
        Some(json!("remote"))
    );
    assert_eq!(get(COMMENTS, &["missing", "x"]).unwrap(), None);
    assert_eq!(get(COMMENTS, &["theme", "x"]).unwrap(), None);
}

#[test]
fn dialect_detects_jsonc_features() {
    assert_eq!(dialect(CRLF).unwrap(), Dialect::Json);
    assert_eq!(dialect(EMPTY_PARENT).unwrap(), Dialect::Json);
    assert_eq!(dialect(COMMENTS).unwrap(), Dialect::Jsonc);
    assert_eq!(dialect(TRAILING_COMMAS).unwrap(), Dialect::Jsonc);
}

#[test]
fn strict_json_stays_strict_after_edits() {
    let added = set(EMPTY_PARENT, MCP_PATH, &entry()).unwrap();
    assert_eq!(dialect(&added).unwrap(), Dialect::Json);
    let added = set(CRLF, &["mcpServers", "mmhmem"], &entry()).unwrap();
    assert_eq!(dialect(&added).unwrap(), Dialect::Json);
}

#[test]
fn structural_errors() {
    assert_eq!(set("{}", &[], &entry()), Err(JsonEditError::EmptyPath));
    assert_eq!(remove("{}", &[]), Err(JsonEditError::EmptyPath));
    assert_eq!(
        set("[1]", MCP_PATH, &entry()),
        Err(JsonEditError::RootNotObject)
    );
    assert_eq!(
        set(COMMENTS, &["theme", "x"], &entry()),
        Err(JsonEditError::NotAnObject { index: 0 })
    );
}

#[test]
fn invalid_input_reports_line_and_column() {
    let input = "{\n  \"a\": 1,\n  \"b\" 2\n}\n";
    let err = set(input, MCP_PATH, &entry()).unwrap_err();
    let JsonEditError::Parse { line, column, .. } = &err else {
        panic!("expected a parse error, got {err:?}");
    };
    assert_eq!((*line, *column), (3, 7));
    assert!(err.to_string().contains("line 3, column 7"));
    assert!(matches!(
        get(input, MCP_PATH),
        Err(JsonEditError::Parse { .. })
    ));
    assert!(matches!(
        remove(input, MCP_PATH),
        Err(JsonEditError::Parse { .. })
    ));
    assert!(matches!(dialect(input), Err(JsonEditError::Parse { .. })));
}

#[test]
fn errors_never_echo_secrets() {
    let secret = "sk-ant-SECRET0123456789";
    let inputs = [
        format!("{{\n  \"token\": \"{secret}\" \"x\"\n}}"),
        format!("{{\n  \"token\": \"{secret}\\u{{zzzz}}\"\n}}"),
        format!("{{\n  \"token\": \"{secret}"),
        format!("{{ {secret}: 1 }} {secret}"),
    ];
    for input in &inputs {
        let err = set(input, MCP_PATH, &json!(secret)).unwrap_err();
        assert!(!err.to_string().contains("SECRET"), "{err}");
        assert!(!format!("{err:?}").contains("SECRET"), "{err:?}");
    }
}

#[test]
fn prune_keeps_parents_with_comments_or_other_keys() {
    let input = "{\n  \"mcp\": {\n    \"servers\": {\n      // note\n      \"mmhmem\": 1\n    },\n    \"x\": 2\n  }\n}\n";
    let out = remove_and_prune(input, MCP_PATH, 2).unwrap();
    assert!(out.contains("// note"), "out:\n{out}");
    assert_eq!(get(&out, &["mcp", "x"]).unwrap(), Some(json!(2)));
    let input =
        "{\n  \"mcp\": {\n    \"servers\": {\n      \"mmhmem\": 1\n    },\n    \"x\": 2\n  }\n}\n";
    let out = remove_and_prune(input, MCP_PATH, 2).unwrap();
    assert_eq!(out, "{\n  \"mcp\": {\n    \"x\": 2\n  }\n}\n");
}

#[test]
fn golden_new_entry_follows_document_style() {
    let added = set(COMMENTS, &["mcpServers", "mmhmem"], &entry()).unwrap();
    let expected = r#"// Claude Desktop config
{
  /* servers the app starts */
  "mcpServers": {
    "other": { "command": "other" }, // keep me
    "mmhmem": {
      "command": ["/opt/mmhmem/bin/mmhmem", "stdio"],
      "type": "local"
    }
  },
  "theme": "dark" /* trailing block */
}
"#;
    assert_eq!(added, expected);
    let added = set(TRAILING_COMMAS, MCP_PATH, &json!(1)).unwrap();
    assert!(
        added.contains("\"x\": 1,\n      \"mmhmem\": 1,\n    },"),
        "{added}"
    );
}

#[test]
fn one_line_objects_and_parser_strictness() {
    let input = "{\"a\": 1, \"b\": 2}";
    assert_eq!(remove(input, &["a"]).unwrap(), "{\"b\": 2}");
    assert_eq!(remove(input, &["b"]).unwrap(), "{\"a\": 1}");
    // Missing commas and JSON5 literals are rejected, as the agents would.
    for bad in [
        "{\"a\": 1 \"b\": 2}",
        "{a: 1}",
        "{\"a\": 'x'}",
        "{\"a\": 0x1}",
    ] {
        assert!(
            matches!(get(bad, &["a"]), Err(JsonEditError::Parse { .. })),
            "{bad}"
        );
    }
}
