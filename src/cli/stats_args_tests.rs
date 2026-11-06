//! Unit tests for `stats_args.rs` (flag parsing and the width source).
use super::*;

#[test]
fn flags_parse_in_any_order() {
    let both = StatsArgs {
        json: true,
        all: true,
    };
    assert_eq!(StatsArgs::parse(&["--all", "--json"]), Ok(both));
    assert_eq!(StatsArgs::parse(&["--json", "--all"]), Ok(both));
    assert_eq!(
        StatsArgs::parse(&["--all"]),
        Ok(StatsArgs {
            json: false,
            all: true
        })
    );
    assert_eq!(StatsArgs::parse::<&str>(&[]), Ok(StatsArgs::default()));
}

#[test]
fn an_unknown_argument_is_an_error_naming_it() {
    assert_eq!(
        StatsArgs::parse(&["--json", "--wide"]),
        Err("--wide".into())
    );
    assert_eq!(StatsArgs::parse(&["json"]), Err("json".into()));
    assert!(STATS_USAGE.contains("[--all] [--json]"));
}

#[test]
fn columns_wins_when_it_is_a_positive_integer() {
    assert_eq!(width_from(Some("100"), || Some(80)), Some(100));
    assert_eq!(width_from(Some(" 90 "), || None), Some(90));
}

#[test]
fn a_bad_columns_value_falls_back_to_the_terminal() {
    for bad in ["0", "-5", "wide", ""] {
        assert_eq!(width_from(Some(bad), || Some(80)), Some(80), "{bad}");
    }
    assert_eq!(width_from(None, || Some(120)), Some(120));
}

#[test]
fn no_columns_and_no_terminal_means_no_limit() {
    assert_eq!(width_from(None, || None), None);
    assert_eq!(width_from(Some("x"), || Some(0)), None);
}
