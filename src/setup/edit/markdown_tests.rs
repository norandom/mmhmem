//! Tests of the shared Markdown section: the golden corpus of
//! `manage_routing.ps1`/`.py` (`tests/fixtures/routing-instructions`), odd
//! files, replacement in place and ambiguous markers.

use std::path::PathBuf;

use serde_json::Value;

use super::*;

fn routing() -> SectionId {
    SectionId::try_from("routing".to_owned()).unwrap()
}

fn corpus() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/routing-instructions")
}

fn read(name: &str) -> Vec<u8> {
    std::fs::read(corpus().join(name)).unwrap()
}

/// Every case of the old helpers' corpus for a shared (not dedicated) file
/// with UTF-8 input gives the same bytes or the same refusal.
#[test]
fn the_golden_corpus_of_the_old_helpers_holds() {
    let cases: Value = serde_json::from_slice(&read("cases.json")).unwrap();
    let canonical = String::from_utf8(read("canonical.md")).unwrap();
    let id = routing();
    let mut ran = 0;
    for case in cases["cases"].as_array().unwrap() {
        let name = case["id"].as_str().unwrap();
        let input = match case["input"].as_str() {
            Some(file) => read(file),
            None => Vec::new(),
        };
        let skip =
            case["dedicated"] == true || (case["input"].is_null() && case["operation"] == "remove");
        let Ok(text) = String::from_utf8(input) else {
            assert_eq!(case["outcome"], "error", "{name}");
            continue;
        };
        if skip {
            continue;
        }
        let got = match case["operation"].as_str().unwrap() {
            "install" => install_section(&text, &id, &canonical),
            _ => remove_section(&text, &id),
        };
        match case["expected"].as_str() {
            Some(file) => assert_eq!(got.unwrap().into_bytes(), read(file), "{name}"),
            None => assert_eq!(got, Err(AmbiguousMarkers), "{name}"),
        }
        ran += 1;
    }
    assert!(ran >= 30, "only {ran} cases ran");
}

#[test]
fn install_then_remove_round_trips_on_odd_files() {
    let id = routing();
    for original in [
        "",
        "# Mine",
        "# Mine\n",
        "# Mine\r\n\r\nText\r\n",
        "a\r\nb",
        "\u{feff}x\n",
        "x\n\n",
    ] {
        let added = install_section(original, &id, "Body\n").unwrap();
        assert!(added.contains("<!-- mmhmem:routing:start -->"));
        let again = install_section(&added, &id, "Body\n").unwrap();
        assert_eq!(again, added, "idempotent");
        assert_eq!(
            remove_section(&added, &id).unwrap(),
            original,
            "{original:?}"
        );
    }
    let crlf = install_section("# Mine\r\n", &id, "a\nb").unwrap();
    assert_eq!(
        crlf,
        "# Mine\r\n<!-- mmhmem:routing:start -->\r\n<!-- mmhmem:routing:prefix=none -->\r\na\r\nb\r\n<!-- mmhmem:routing:end -->\r\n"
    );
    let lf = install_section("# Mine", &id, "a").unwrap();
    assert_eq!(
        lf,
        "# Mine\n<!-- mmhmem:routing:start -->\n<!-- mmhmem:routing:prefix=lf -->\na\n<!-- mmhmem:routing:end -->\n"
    );
    let bom = install_section("\u{feff}", &id, "a").unwrap();
    assert!(bom.starts_with("\u{feff}<!-- mmhmem:routing:start -->\n"));
}

#[test]
fn an_outdated_section_is_replaced_in_place() {
    let id = routing();
    let old = "a\n<!-- mmhmem:routing:start -->\n<!-- mmhmem:routing:prefix=none -->\nold\n<!-- mmhmem:routing:end -->\nb\n";
    let new = install_section(old, &id, "new").unwrap();
    assert_eq!(
        new,
        "a\n<!-- mmhmem:routing:start -->\n<!-- mmhmem:routing:prefix=none -->\nnew\n<!-- mmhmem:routing:end -->\nb\n"
    );
    assert_eq!(remove_section(&new, &id).unwrap(), "a\nb\n");
    assert_eq!(remove_section("a\n", &id).unwrap(), "a\n", "absent");
}

#[test]
fn ambiguous_markers_are_refused_with_a_fixed_reason() {
    let id = routing();
    let start = id.start_marker();
    let end = id.end_marker();
    for text in [
        format!("{start}\n{start}\n"),
        format!("{start}\nno end\n"),
        format!("{end}\n"),
        format!("{end}\n{start}\n"),
        format!("{start}\n{end}\n{start}\n{end}\n"),
        format!("x\n{start}\n<!-- mmhmem:routing:prefix=crlf -->\n{end}\n"),
    ] {
        assert_eq!(install_section(&text, &id, "b"), Err(AmbiguousMarkers));
        assert_eq!(remove_section(&text, &id), Err(AmbiguousMarkers));
    }
    let marked = format!("say {start}");
    assert_eq!(install_section("", &id, &marked), Err(AmbiguousMarkers));
    assert_eq!(AmbiguousMarkers.to_string(), AMBIGUOUS_MARKERS);
}

#[test]
fn another_section_id_is_left_alone() {
    let other = SectionId::try_from("other".to_owned()).unwrap();
    let text = install_section("a\n", &routing(), "b").unwrap();
    assert_eq!(remove_section(&text, &other).unwrap(), text);
}
