//! Unit tests for `stats_table.rs` (local-mode Req 8.1/8.3): the polars-style
//! box, the 7-day row window and its fallback, all-zero column hiding,
//! width fitting, `--all`, and totals over the shown rows. Today is fixed at
//! 2026-11-06 so the snapshots are stable.
use super::*;
use crate::cli::stats_activity::ActivityCounts;
use time::Month;

fn today() -> Date {
    Date::from_calendar_date(2026, Month::November, 6).unwrap()
}

fn opts(width: Option<usize>, all: bool) -> TableOptions {
    TableOptions {
        width,
        today: today(),
        all,
    }
}

/// (day, project, harness, avoided, client in/out, worker in/out, reads).
type Seed = (
    &'static str,
    &'static str,
    &'static str,
    i64,
    (i64, i64),
    (i64, i64),
    [u64; 5],
);

fn agg(s: Seed) -> DayAggregate {
    let (day, project, harness, avoided, client, worker, reads) = s;
    DayAggregate {
        day: day.into(),
        project: project.into(),
        harness: harness.into(),
        avoided_est: avoided,
        client_prompt: client.0,
        client_completion: client.1,
        client_sources: Vec::new(),
        worker_prompt: worker.0,
        worker_completion: worker.1,
        activity: ActivityCounts {
            summary_reads: reads[0],
            cache_hits: reads[1],
            slice_reads: reads[2],
            direct_reads: reads[3],
            hook_redirects: reads[4],
        },
    }
}

/// Day DESC, as `aggregate` returns them. Three rows fall in the window;
/// `slices` is non-zero only in an older row.
fn fixture() -> Vec<DayAggregate> {
    [
        (
            "2026-11-06",
            "QOpenBB",
            "codex",
            8_574,
            (0, 0),
            (1_200, 300),
            [4, 2, 0, 0, 1],
        ),
        (
            "2026-11-05",
            "mmhmem",
            "claude-code",
            30_101,
            (120_000, 4_500),
            (2_000, 500),
            [3, 0, 0, 1, 2],
        ),
        (
            "2026-10-31",
            "a-really-long-client-project-name",
            "opencode",
            12_000,
            (0, 0),
            (900, 100),
            [0, 0, 0, 5, 0],
        ),
        (
            "2026-10-30",
            "old",
            "codex",
            1_000,
            (0, 0),
            (0, 0),
            [0, 0, 7, 0, 0],
        ),
        (
            "2026-09-01",
            "older",
            "codex",
            2_000,
            (10, 1),
            (0, 0),
            [0, 0, 0, 0, 0],
        ),
    ]
    .into_iter()
    .map(agg)
    .collect()
}

fn header(table: &str) -> &str {
    table.lines().nth(2).unwrap()
}

fn total_line(table: &str) -> &str {
    table.lines().find(|l| l.starts_with("│ total")).unwrap()
}

#[test]
fn default_table_at_full_width() {
    insta::assert_snapshot!(
        "default_unlimited",
        render_table(&fixture(), &opts(None, false))
    );
}

#[test]
fn default_table_at_width_100() {
    let table = render_table(&fixture(), &opts(Some(100), false));
    insta::assert_snapshot!("default_width_100", table);
}

#[test]
fn default_table_at_width_200() {
    let table = render_table(&fixture(), &opts(Some(200), false));
    insta::assert_snapshot!("default_width_200", table);
}

#[test]
fn all_table_shows_every_row_and_column() {
    insta::assert_snapshot!("all", render_table(&fixture(), &opts(Some(60), true)));
}

#[test]
fn the_window_keeps_the_last_seven_days_and_says_so() {
    let table = render_table(&fixture(), &opts(None, false));
    assert!(table.starts_with("shape: (3, "), "{table}");
    assert!(table.contains("2026-10-31") && !table.contains("2026-10-30"));
    assert!(table.contains("2 older rows hidden (showing since 2026-10-31)"));
    assert!(table.contains("The total covers the shown rows only."));
    assert!(table.contains(ALL_HINT));
}

#[test]
fn an_empty_window_falls_back_to_the_most_recent_day() {
    let rows: Vec<DayAggregate> = fixture().into_iter().skip(3).collect();
    let table = render_table(&rows, &opts(None, false));
    assert!(table.starts_with("shape: (1, "), "{table}");
    assert!(table.contains("2026-10-30") && !table.contains("2026-09-01"));
    assert!(table.contains("1 older row hidden (showing the most recent day, 2026-10-30)"));
}

#[test]
fn the_total_sums_the_shown_rows_only() {
    let table = render_table(&fixture(), &opts(None, false));
    let total = total_line(&table);
    assert!(total.contains("50,675"), "8,574 + 30,101 + 12,000: {total}");
    let all = render_table(&fixture(), &opts(None, true));
    assert!(total_line(&all).contains("53,675"), "{all}");
}

#[test]
fn the_total_row_is_last_and_set_off_by_a_rule() {
    let table = render_table(&fixture(), &opts(None, false));
    let lines: Vec<&str> = table.lines().collect();
    let at = lines.iter().position(|l| l.starts_with("│ total")).unwrap();
    assert!(lines[at - 1].starts_with('╞'), "{table}");
    assert!(lines[at + 1].starts_with('└'), "{table}");
}

#[test]
fn all_zero_columns_are_hidden_by_default() {
    let table = render_table(&fixture(), &opts(None, false));
    assert!(!header(&table).contains("slices"), "{table}");
    assert!(header(&table).contains("cache hits"));
    assert!(table.contains("; 1 column hidden (slices)."), "{table}");
    let all = render_table(&fixture(), &opts(None, true));
    assert!(header(&all).contains("slices"));
}

#[test]
fn every_column_zero_still_shows_the_estimate() {
    let rows = vec![agg(("2026-11-06", "p", "h", 0, (0, 0), (0, 0), [0; 5]))];
    let table = render_table(&rows, &opts(None, false));
    assert!(header(&table).contains("avoided (est.)"), "{table}");
    assert!(table.starts_with("shape: (1, 4)"), "{table}");
}

#[test]
fn a_narrow_table_cuts_the_project_and_drops_columns_by_priority() {
    let table = render_table(&fixture(), &opts(Some(100), false));
    let width = |l: &str| l.chars().count();
    let box_lines = table
        .lines()
        .filter(|l| l.starts_with(['┌', '│', '╞', '└']));
    for line in box_lines {
        assert!(width(line) <= 100, "{} chars: {line}", width(line));
    }
    assert!(table.contains("a-really-long-client-pr…"), "{table}");
    let head = header(&table);
    assert!(head.contains("avoided (est.)") && head.contains("worker in"));
    assert!(head.contains(" … "), "ellipsis column: {head}");
    assert!(!head.contains("redirects"), "read counts go first: {head}");
    assert!(
        table.contains("8 columns hidden (slices, frontier in (client), …)."),
        "{table}"
    );
}

#[test]
fn a_wide_terminal_needs_no_fitting() {
    let wide = render_table(&fixture(), &opts(Some(200), false));
    let free = render_table(&fixture(), &opts(None, false));
    assert_eq!(wide, free);
    assert!(wide.contains("a-really-long-client-project-name"));
}

#[test]
fn all_ignores_the_width_limit() {
    let table = render_table(&fixture(), &opts(Some(40), true));
    assert!(header(&table).chars().count() > 40);
    assert!(table.contains("a-really-long-client-project-name"));
    assert!(table.starts_with("shape: (5, 13)"), "{table}");
    assert!(!table.contains(ALL_HINT), "nothing hidden, no footer");
}

#[test]
fn headers_keep_the_labeling_discipline() {
    let table = render_table(&fixture(), &opts(None, true));
    let head = header(&table);
    for label in [
        "avoided (est.)",
        "frontier in (client)",
        "frontier out (client)",
        "worker in",
        "worker out",
        "summaries",
        "cache hits",
        "slices",
        "direct",
        "redirects",
    ] {
        assert!(head.contains(label), "missing {label}: {head}");
    }
    assert!(!head.contains("avoided (client)") && !head.contains("measured"));
}

#[test]
fn numbers_align_right_and_text_left() {
    let table = render_table(&fixture(), &opts(None, false));
    let row = table.lines().find(|l| l.contains("QOpenBB")).unwrap();
    assert!(row.starts_with("│ 2026-11-06 ┆ QOpenBB "), "{row}");
    assert!(row.contains("┆          8,574 ┆"), "{row}");
}

#[test]
fn empty_input_renders_a_zero_total() {
    let table = render_table(&[], &opts(None, false));
    assert!(table.starts_with("shape: (0, 4)"), "{table}");
    assert!(total_line(&table).contains(" 0 "), "{table}");
}

#[test]
fn thousands_groups_digits() {
    assert_eq!(thousands(0), "0");
    assert_eq!(thousands(1_400), "1,400");
    assert_eq!(thousands(-1_234_567), "-1,234,567");
}
