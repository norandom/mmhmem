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

/// (day, project, harness, avoided, client in/out, worker in/out, reads:
/// summaries, cache hits, slices, direct, redirects, followed redirects).
type Seed = (
    &'static str,
    &'static str,
    &'static str,
    i64,
    (i64, i64),
    (i64, i64),
    [u64; 6],
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
            followed_redirects: reads[5],
        },
    }
}

/// Day DESC, as `aggregate` returns them. Three rows fall in the window;
/// `slices` is non-zero only in an older row. Rates: QOpenBB 2 hits of 6
/// (33%), 1 of 1 redirect followed (100%); mmhmem 0 of 3 (0%), 1 of 2
/// (50%); the long project has neither (`-`).
fn fixture() -> Vec<DayAggregate> {
    [
        (
            "2026-11-06",
            "QOpenBB",
            "codex",
            8_574,
            (0, 0),
            (1_200, 300),
            [4, 2, 0, 0, 1, 1],
        ),
        (
            "2026-11-05",
            "mmhmem",
            "claude-code",
            30_101,
            (120_000, 4_500),
            (2_000, 500),
            [3, 0, 0, 1, 2, 1],
        ),
        (
            "2026-10-31",
            "a-really-long-client-project-name",
            "opencode",
            12_000,
            (0, 0),
            (900, 100),
            [0, 0, 0, 5, 0, 0],
        ),
        (
            "2026-10-30",
            "old",
            "codex",
            1_000,
            (0, 0),
            (0, 0),
            [0, 0, 7, 0, 0, 0],
        ),
        (
            "2026-09-01",
            "older",
            "codex",
            2_000,
            (10, 1),
            (0, 0),
            [0; 6],
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
fn rows_without_savings_are_hidden_by_default() {
    let mut rows = fixture();
    rows.insert(
        1,
        agg((
            "2026-11-06",
            "reads-only",
            "codex",
            0,
            (0, 0),
            (0, 0),
            [0, 0, 9, 0, 0, 0],
        )),
    );
    rows.insert(
        2,
        agg((
            "2026-11-05",
            "nothing",
            "opencode",
            0,
            (5, 1),
            (0, 0),
            [0; 6],
        )),
    );
    let table = render_table(&rows, &opts(None, false));
    assert!(table.starts_with("shape: (3, "), "{table}");
    assert!(
        !table.contains("reads-only") && !table.contains("nothing"),
        "{table}"
    );
    assert!(
        table.contains(
            "2 rows without savings hidden; 2 older rows hidden (showing since 2026-10-31); "
        ),
        "{table}"
    );
    assert!(table.contains("The total covers the shown rows only."));
    // Their columns are not shown either: only hidden rows had slices.
    assert!(!header(&table).contains("slices"), "{table}");
    let all = render_table(&rows, &opts(None, true));
    assert!(all.starts_with("shape: (7, "), "{all}");
    assert!(
        all.contains("reads-only") && all.contains("nothing"),
        "{all}"
    );
}

#[test]
fn without_any_savings_every_row_is_shown() {
    let rows = vec![
        agg((
            "2026-11-06",
            "a",
            "codex",
            0,
            (0, 0),
            (0, 0),
            [0, 0, 3, 0, 0, 0],
        )),
        agg((
            "2026-11-05",
            "b",
            "codex",
            0,
            (0, 0),
            (0, 0),
            [0, 0, 0, 1, 0, 0],
        )),
    ];
    let table = render_table(&rows, &opts(None, false));
    assert!(table.starts_with("shape: (2, "), "{table}");
    assert!(!table.contains("without savings"), "{table}");
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
    let rows = vec![agg(("2026-11-06", "p", "h", 0, (0, 0), (0, 0), [0; 6]))];
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
        !head.contains("(rate"),
        "rates go with the read counts: {head}"
    );
    assert!(
        table.contains("10 columns hidden (slices, frontier in (client), …)."),
        "{table}"
    );
}

#[test]
fn the_rates_are_dropped_first_when_the_table_is_too_wide() {
    let table = render_table(&fixture(), &opts(Some(200), false));
    let head = header(&table);
    assert!(head.chars().count() <= 200, "{head}");
    assert!(
        !head.contains("follow-through"),
        "rightmost rate first: {head}"
    );
    assert!(head.contains("avoided (est.)") && head.contains("worker in"));
    assert!(table.contains("columns hidden (slices, "), "{table}");
}

#[test]
fn a_wide_terminal_needs_no_fitting() {
    let wide = render_table(&fixture(), &opts(Some(260), false));
    let free = render_table(&fixture(), &opts(None, false));
    assert_eq!(wide, free);
    assert!(wide.contains("a-really-long-client-project-name"));
}

#[test]
fn all_ignores_the_width_limit() {
    let table = render_table(&fixture(), &opts(Some(40), true));
    assert!(header(&table).chars().count() > 40);
    assert!(table.contains("a-really-long-client-project-name"));
    assert!(table.starts_with("shape: (5, 15)"), "{table}");
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
        "hit rate (rate)",
        "follow-through (rate, 10 min)",
    ] {
        assert!(head.contains(label), "missing {label}: {head}");
    }
    assert!(!head.contains("avoided (client)") && !head.contains("measured"));
}

/// Cells of one table line, by header.
fn cells_by_header(table: &str, line: &str) -> std::collections::HashMap<String, String> {
    let cells = |l: &str| -> Vec<String> {
        l.split(['│', '┆'])
            .skip(1)
            .map(|c| c.trim().to_string())
            .collect()
    };
    cells(header(table)).into_iter().zip(cells(line)).collect()
}

fn row_line<'a>(table: &'a str, project: &str) -> &'a str {
    table.lines().find(|l| l.contains(project)).unwrap()
}

#[test]
fn rates_render_as_percentages_or_a_dash_without_a_denominator() {
    let table = render_table(&fixture(), &opts(None, false));
    let (hit, follow) = ("hit rate (rate)", "follow-through (rate, 10 min)");
    for (project, want_hit, want_follow) in [
        ("QOpenBB", "33%", "100%"),
        ("mmhmem", "0%", "50%"),
        ("a-really-long-client-project-name", "-", "-"),
    ] {
        let cells = cells_by_header(&table, row_line(&table, project));
        assert_eq!(cells[hit], want_hit, "{project}: {table}");
        assert_eq!(cells[follow], want_follow, "{project}: {table}");
    }
    let rates = cells_by_header(&table, row_line(&table, "QOpenBB"));
    assert_eq!(rates["cache hits"], "2", "the counts stay: {table}");
}

#[test]
fn rate_totals_are_recomputed_from_the_summed_counts() {
    let table = render_table(&fixture(), &opts(None, false));
    let total = cells_by_header(&table, total_line(&table));
    // 2 hits of 9 requests and 2 of 3 redirects followed — not the mean of
    // the row rates (17% and 75%).
    assert_eq!(total["hit rate (rate)"], "22%", "{table}");
    assert_eq!(total["follow-through (rate, 10 min)"], "67%", "{table}");
}

#[test]
fn rates_without_any_denominator_are_hidden_like_all_zero_columns() {
    let rows = vec![agg((
        "2026-11-06",
        "p",
        "h",
        10,
        (0, 0),
        (0, 0),
        [0, 0, 3, 0, 0, 0],
    ))];
    let table = render_table(&rows, &opts(None, false));
    assert!(!header(&table).contains("(rate"), "{table}");
    assert!(
        table.contains("10 columns hidden"),
        "counted in the footer: {table}"
    );
    let all = render_table(&rows, &opts(None, true));
    let cells = cells_by_header(&all, row_line(&all, "│ 2026-11-06"));
    assert_eq!(cells["hit rate (rate)"], "-", "{all}");
    assert_eq!(cells["follow-through (rate, 10 min)"], "-", "{all}");
}

#[test]
fn a_zero_rate_is_shown_not_hidden() {
    let rows = vec![agg((
        "2026-11-06",
        "p",
        "h",
        10,
        (0, 0),
        (0, 0),
        [5, 0, 0, 0, 4, 0],
    ))];
    let table = render_table(&rows, &opts(None, false));
    let cells = cells_by_header(&table, total_line(&table));
    assert_eq!(cells["hit rate (rate)"], "0%", "{table}");
    assert_eq!(cells["follow-through (rate, 10 min)"], "0%", "{table}");
}

#[test]
fn rate_rounds_to_whole_percent() {
    assert_eq!(rate(0, 0), "-");
    assert_eq!(rate(0, 7), "0%");
    assert_eq!(rate(1, 3), "33%");
    assert_eq!(rate(2, 3), "67%");
    assert_eq!(rate(1, 200), "1%");
    assert_eq!(rate(5, 5), "100%");
    assert_eq!(rate(u64::MAX, u64::MAX), "100%");
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
