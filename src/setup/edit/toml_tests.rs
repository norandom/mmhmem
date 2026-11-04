use super::*;

const P: &[&str] = &["mcp_servers", "mmhmem"];

fn entry() -> McpServerEntry {
    McpServerEntry {
        command: r"C:\Users\me\AppData\Local\mmhmem\mmhmem.exe".into(),
        args: vec!["serve".into(), "--stdio".into()],
        env: vec![("RUST_LOG".into(), "info".into())],
        cwd: Some(r"C:\work dir".into()),
        startup_timeout_sec: Some(30),
        enabled: Some(true),
    }
}

fn fixtures() -> Vec<String> {
    vec![
        String::new(),
        "model = \"x\"\n".into(),
        "# top comment\n\nmodel = \"x\"\n\n\n# trailing\n".into(),
        "a = 1\r\nb = 2\r\n\r\n[other]\r\nk = \"v\"\r\n".into(),
        "[mcp_servers.alpha]\ncommand = \"a\"\n\n[mcp_servers.zeta]\ncommand = \"z\"\n".into(),
        "[mcp_servers.alpha]\ncommand = \"a\"\nargs = [\"x\"]\n".into(),
        "x.y = 1\nx.z = { a = 1, b = [1, 2] }\n[t]\ninline = { k = \"v\" }\n".into(),
        "[mcp_servers]\n# note\n\n[mcp_servers.alpha]\ncommand = \"a\" # c\n\n".into(),
        "\n".into(),
        "s = \"\"\"\n[mcp_servers.mmhmem]\n\"\"\"\n".into(),
    ]
}

#[test]
fn add_then_remove_is_byte_identical() {
    for input in fixtures() {
        let added = set_table(&input, P, &entry()).unwrap();
        assert_ne!(added, input);
        assert_eq!(remove_table(&added, P).unwrap(), input, "fixture {input:?}");
    }
}

#[test]
fn set_round_trips_via_get_and_keeps_windows_paths() {
    let out = set_table("a = 1\n", P, &entry()).unwrap();
    assert_eq!(get_table(&out, P).unwrap(), Some(entry()));
    assert!(out.starts_with("a = 1\n\n[mcp_servers.mmhmem]\n"));
}

#[test]
fn crlf_input_gets_crlf_block() {
    let out = set_table("a = 1\r\n", P, &entry()).unwrap();
    assert!(!out.replace("\r\n", "").contains('\n'));
    assert_eq!(get_table(&out, P).unwrap(), Some(entry()));
}

#[test]
fn replace_keeps_position_and_other_content() {
    let input = "# head\n[a]\nk = 1\n\n[mcp_servers.mmhmem]\ncommand = \"old\"\n[mcp_servers.mmhmem.env]\nX = \"1\"\n\n# tail\n[z]\nq = 2\n";
    let out = set_table(input, P, &entry()).unwrap();
    assert!(out.starts_with("# head\n[a]\nk = 1\n\n[mcp_servers.mmhmem]\ncommand = "));
    assert!(out.ends_with("\n# tail\n[z]\nq = 2\n"));
    let mut want = entry();
    want.env.insert(0, ("X".into(), "1".into()));
    assert_eq!(get_table(&out, P).unwrap(), Some(want));
    assert_eq!(out.matches("[mcp_servers.mmhmem]").count(), 1);
}

#[test]
fn remove_only_the_target() {
    let input = "[mcp_servers.a]\ncommand = \"a\"\n\n[mcp_servers.mmhmem]\ncommand = \"m\"\n\n[mcp_servers.z]\ncommand = \"z\"\n";
    let out = remove_table(input, P).unwrap();
    assert_eq!(
        out,
        "[mcp_servers.a]\ncommand = \"a\"\n\n[mcp_servers.z]\ncommand = \"z\"\n"
    );
}

#[test]
fn remove_absent_is_unchanged() {
    for input in fixtures() {
        assert_eq!(remove_table(&input, P).unwrap(), input);
    }
}

#[test]
fn get_absent_is_none_and_wrong_type_errors() {
    assert_eq!(get_table("a = 1\n", P).unwrap(), None);
    let e = get_table("[mcp_servers.mmhmem]\ncommand = 3\n", P).unwrap_err();
    assert_eq!(e, TomlEditError::UnexpectedType { field: "command" });
}

#[test]
fn dotted_layout_is_unsupported_not_corrupted() {
    let input = "[mcp_servers]\nmmhmem = { command = \"x\" }\n";
    assert_eq!(
        set_table(input, P, &entry()),
        Err(TomlEditError::UnsupportedLayout)
    );
    assert_eq!(
        remove_table(input, P),
        Err(TomlEditError::UnsupportedLayout)
    );
    assert!(get_table(input, P).unwrap().is_some());
}

fn marked(begin: &str) -> String {
    format!(
        "a = 1\n\n{begin}\n[mcp_servers.mmhmem]\ncommand = \"m\"\nargs = []\n# END mmhmem\n\n[z]\nq = 2\n"
    )
}

#[test]
fn marker_block_converts_for_both_variants() {
    for begin in [MARKER_BEGIN_PS1, MARKER_BEGIN_SH] {
        let input = marked(begin);
        assert!(has_marker_block(&input));
        let out = convert_marker_block(&input, P).unwrap();
        assert_eq!(
            out,
            "a = 1\n\n[mcp_servers.mmhmem]\ncommand = \"m\"\nargs = []\n\n[z]\nq = 2\n"
        );
        assert!(!has_marker_block(&out));
    }
}

#[test]
fn marker_block_converts_with_crlf() {
    let input = marked(MARKER_BEGIN_PS1).replace('\n', "\r\n");
    let out = convert_marker_block(&input, P).unwrap();
    assert!(!out.contains("mmhmem (managed"));
    assert!(out.contains("[mcp_servers.mmhmem]\r\ncommand"));
}

#[test]
fn no_markers_is_unchanged() {
    let input = "a = 1\n";
    assert_eq!(convert_marker_block(input, P).unwrap(), input);
}

#[test]
fn bad_markers_error() {
    let table = "[mcp_servers.mmhmem]\ncommand = \"m\"\n";
    let nested = format!("{MARKER_BEGIN_PS1}\n{MARKER_BEGIN_SH}\n{table}# END mmhmem\n");
    let no_end = format!("{MARKER_BEGIN_SH}\n{table}");
    let no_begin = format!("{table}# END mmhmem\n");
    let empty = format!("{MARKER_BEGIN_SH}\n# END mmhmem\n");
    assert_eq!(
        convert_marker_block(&nested, P),
        Err(TomlEditError::MarkerBlock(MarkerProblem::Nested(2)))
    );
    assert!(matches!(
        convert_marker_block(&no_end, P),
        Err(TomlEditError::MarkerBlock(MarkerProblem::Unmatched(1)))
    ));
    assert!(matches!(
        convert_marker_block(&no_begin, P),
        Err(TomlEditError::MarkerBlock(MarkerProblem::Unmatched(3)))
    ));
    assert_eq!(
        convert_marker_block(&empty, P),
        Err(TomlEditError::MarkerBlock(MarkerProblem::NoTable))
    );
}

#[test]
fn duplicate_table_is_detected() {
    let input = "[mcp_servers.mmhmem]\ncommand = \"a\"\n\n[mcp_servers.mmhmem]\ncommand = \"b\"\n";
    for r in [
        get_table(input, P).map(|_| ()),
        set_table(input, P, &entry()).map(|_| ()),
        remove_table(input, P).map(|_| ()),
    ] {
        assert_eq!(
            r,
            Err(TomlEditError::DuplicateTable {
                line: 4,
                column: 14,
                is_mmhmem: true
            })
        );
    }
    let other = "[x]\na = 1\n[x]\n";
    assert!(matches!(
        validate(other),
        Err(TomlEditError::DuplicateTable {
            is_mmhmem: false,
            ..
        })
    ));
}

#[test]
fn invalid_toml_reports_line_and_column_without_echo() {
    let input = "ok = 1\ntoken = sk-SECRET123 oops\n";
    let err = validate(input).unwrap_err();
    let TomlEditError::Syntax { line, column } = err else {
        panic!("expected syntax error, got {err:?}");
    };
    assert_eq!(line, 2);
    assert!(column >= 1);
    for r in [
        set_table(input, P, &entry()).map(|_| ()),
        remove_table(input, P).map(|_| ()),
        convert_marker_block(&format!("{MARKER_BEGIN_SH}\n{input}# END mmhmem\n"), P).map(|_| ()),
    ] {
        let e = r.unwrap_err();
        assert!(!e.to_string().contains("SECRET"), "{e}");
        assert!(!format!("{e:?}").contains("SECRET"), "{e:?}");
    }
}

#[test]
fn empty_path_is_rejected() {
    assert_eq!(get_table("", &[]), Err(TomlEditError::EmptyPath));
}

#[test]
fn array_of_tables_is_unsupported() {
    let input = "[[mcp_servers.mmhmem]]\ncommand = \"x\"\n";
    assert_eq!(
        remove_table(input, P),
        Err(TomlEditError::UnsupportedLayout)
    );
}

const USER_TABLE: &str = "# keep\n[mcp_servers.mmhmem]\ncommand = \"old\" # why\n# user note\ntool_timeout_sec = 9\nenabled_tools = [\"a\"]\n\nargs = [\"serve\", \"--stdio\"]\n\n[mcp_servers.mmhmem.env]\n# mine\nTOKEN = \"x\"\n\n[z]\nq = 2\n";

#[test]
fn set_keeps_user_keys_and_comments() {
    let e = McpServerEntry {
        command: "new".into(),
        args: vec!["serve".into(), "--stdio".into()],
        ..Default::default()
    };
    let out = set_table(USER_TABLE, P, &e).unwrap();
    assert_eq!(out, USER_TABLE.replace("\"old\" # why", "\"new\" # why"));
}

#[test]
fn set_with_identical_values_is_byte_identical() {
    let e = McpServerEntry {
        command: "old".into(),
        args: vec!["serve".into(), "--stdio".into()],
        ..Default::default()
    };
    assert_eq!(set_table(USER_TABLE, P, &e).unwrap(), USER_TABLE);
    let once = set_table("a = 1\n", P, &entry()).unwrap();
    assert_eq!(set_table(&once, P, &entry()).unwrap(), once);
}

#[test]
fn empty_env_keeps_user_env_and_none_fields_keep_values() {
    let e = McpServerEntry {
        command: "old".into(),
        args: vec!["serve".into(), "--stdio".into()],
        ..Default::default()
    };
    let out = set_table(USER_TABLE, P, &e).unwrap();
    assert!(out.contains("# mine\nTOKEN = \"x\"\n"));
    let with_env = McpServerEntry {
        env: vec![("RUST_LOG".into(), "info".into())],
        ..e
    };
    let out = set_table(USER_TABLE, P, &with_env).unwrap();
    assert!(out.contains("TOKEN = \"x\"") && out.contains("RUST_LOG = \"info\""));
}

#[test]
fn crlf_existing_table_update_keeps_crlf_and_mixed_is_refused() {
    let crlf = USER_TABLE.replace('\n', "\r\n");
    let e = McpServerEntry {
        command: "new".into(),
        args: vec!["serve".into(), "--stdio".into()],
        ..Default::default()
    };
    let out = set_table(&crlf, P, &e).unwrap();
    assert_eq!(out, crlf.replace("\"old\" # why", "\"new\" # why"));
    let mixed = format!("{crlf}x = 1\n");
    assert_eq!(
        set_table(&mixed, P, &e),
        Err(TomlEditError::MixedLineEndings)
    );
}

#[test]
fn get_item_json_returns_every_key_and_typed_values() {
    let doc = "[mcp_servers.mmhmem]\ncommand = \"mmhmem\" # c\ntool_timeout_sec = 1.5\nenv = { K = \"v\" }\nwhen = 1979-05-27\n";
    let value = get_item_json(doc, P).unwrap().unwrap();
    assert_eq!(
        value,
        serde_json::json!({
            "command": "mmhmem",
            "tool_timeout_sec": {"$float": "1.5"},
            "env": {"K": "v"},
            "when": {"$datetime": "1979-05-27"}
        })
    );
    let dotted = "mcp_servers.mmhmem.command = \"mmhmem\"\n";
    assert_eq!(
        get_item_json(dotted, P).unwrap(),
        Some(serde_json::json!({"command": "mmhmem"}))
    );
    assert_eq!(get_item_json("x = 1\n", P).unwrap(), None);
    assert_eq!(get_item_json("x = 1\n", &[]), Err(TomlEditError::EmptyPath));
    assert!(matches!(
        get_item_json("[[[", P),
        Err(TomlEditError::Syntax { .. })
    ));
}
