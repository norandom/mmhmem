//! Human-readable `mmhmem stats` table (local-mode Req 8.1/8.3), drawn in
//! the style of a polars DataFrame: a `shape: (rows, cols)` line, box
//! borders, a `╞═╪═╡` header rule and the total row last, set off by a
//! second rule.
//!
//! By default the table shows what matters: rows of the last 7 UTC days
//! (or, when that is empty, the most recent day with data), no all-zero
//! numeric columns, and a width that fits the terminal. To fit, the project
//! column is first cut to [`PROJECT_MAX`] characters, then numeric columns
//! are dropped by [`Column::priority`] (read counts first, then frontier
//! client-reported tokens, then worker tokens, the avoided estimate last)
//! and one `…` column stands in their place. A short footer says what was
//! hidden; `--all` turns all of this off. The labeling discipline holds in
//! every header (Req 8.3): estimates say `(est.)`, client-reported values
//! say `(client)`, and nothing computed is presented as measured.
//!
//! Pure by contract: width, today and `all` come in through
//! [`TableOptions`]; terminal detection lives in [`super::stats_args`].

use std::cmp::Reverse;

use time::Date;

use super::stats::{DayAggregate, totals};

/// Inputs that make the rendering deterministic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TableOptions {
    /// Maximum line width in characters; `None` means no limit.
    pub width: Option<usize>,
    /// The current UTC day; the row window ends here.
    pub today: Date,
    /// `--all`: every row and column, no width limit.
    pub all: bool,
}

/// Days in the default row window (today inclusive).
pub const WINDOW_DAYS: i64 = 7;

/// Longest project name kept when the table is too wide.
pub const PROJECT_MAX: usize = 24;

/// Hint printed under the table whenever something was hidden.
pub const ALL_HINT: &str = "Run \"mmhmem stats --all\" for everything.";

/// One numeric column: header, cell value and fit priority (0 is kept
/// longest; ties drop the rightmost first).
struct Column {
    header: &'static str,
    value: fn(&DayAggregate) -> i64,
    priority: u8,
}

/// Numeric columns in display order (Req 8.3 labels).
const COLUMNS: [Column; 10] = [
    col("avoided (est.)", 0, |r| r.avoided_est),
    col("frontier in (client)", 2, |r| r.client_prompt),
    col("frontier out (client)", 2, |r| r.client_completion),
    col("worker in", 1, |r| r.worker_prompt),
    col("worker out", 1, |r| r.worker_completion),
    col("summaries", 3, |r| count(r.activity.summary_reads)),
    col("cache hits", 3, |r| count(r.activity.cache_hits)),
    col("slices", 3, |r| count(r.activity.slice_reads)),
    col("direct", 3, |r| count(r.activity.direct_reads)),
    col("redirects", 3, |r| count(r.activity.hook_redirects)),
];

/// Always-shown text columns.
const TEXT_HEADERS: [&str; 3] = ["day", "project", "harness"];

const fn col(header: &'static str, priority: u8, value: fn(&DayAggregate) -> i64) -> Column {
    Column {
        header,
        value,
        priority,
    }
}

fn count(n: u64) -> i64 {
    i64::try_from(n).unwrap_or(i64::MAX)
}

/// One display column: header, alignment and the cells of every body row
/// (data rows, then the total).
struct Shown {
    header: String,
    right: bool,
    cells: Vec<String>,
}

impl Shown {
    fn width(&self) -> usize {
        self.cells
            .iter()
            .map(|c| c.chars().count())
            .chain([self.header.chars().count()])
            .max()
            .unwrap_or(0)
    }
}

/// Renders the table plus, when anything was hidden, a footer.
pub fn render_table(rows: &[DayAggregate], opts: &TableOptions) -> String {
    let (shown, window) = select_rows(rows, opts);
    let mut body = shown.clone();
    body.push(total_row(&shown));
    let numeric = numeric_columns(&body, opts.all);
    let mut hidden_columns: Vec<String> = (0..COLUMNS.len())
        .filter(|i| !numeric.contains(i))
        .map(|i| COLUMNS[i].header.to_string())
        .collect();
    let mut columns = text_columns(&body);
    columns.extend(numeric.iter().map(|&i| numeric_column(&COLUMNS[i], &body)));
    let shape = (shown.len(), columns.len());
    hidden_columns.extend(fit(
        &mut columns,
        &numeric,
        opts.width.filter(|_| !opts.all),
    ));
    let mut out = format!("shape: ({}, {})\n", shape.0, shape.1);
    out.push_str(&draw(&columns, shown.len()));
    let hidden = Hidden {
        rows: rows.len() - shown.len(),
        window,
        columns: hidden_columns,
    };
    out.push_str(&footer(&hidden));
    out
}

/// Which period the shown rows cover, for the footer.
enum Window {
    All,
    LastDays(String),
    MostRecent(String),
}

/// The rows to show: everything with `all`, else the last [`WINDOW_DAYS`]
/// UTC days, else the most recent day with data.
fn select_rows(rows: &[DayAggregate], opts: &TableOptions) -> (Vec<DayAggregate>, Window) {
    if opts.all {
        return (rows.to_vec(), Window::All);
    }
    let start = iso_day(opts.today - time::Duration::days(WINDOW_DAYS - 1));
    let recent = keep_rows(rows, |day| day >= start.as_str());
    match rows.iter().map(|r| r.day.as_str()).max() {
        Some(latest) if recent.is_empty() => (
            keep_rows(rows, |day| day == latest),
            Window::MostRecent(latest.to_string()),
        ),
        _ => (recent, Window::LastDays(start)),
    }
}

fn keep_rows(rows: &[DayAggregate], keep: impl Fn(&str) -> bool) -> Vec<DayAggregate> {
    rows.iter().filter(|r| keep(&r.day)).cloned().collect()
}

/// `YYYY-MM-DD`, the same form as the `day` column.
fn iso_day(d: Date) -> String {
    format!("{:04}-{:02}-{:02}", d.year(), u8::from(d.month()), d.day())
}

/// The total row: sums of the SHOWN rows, labeled `total`.
fn total_row(rows: &[DayAggregate]) -> DayAggregate {
    let t = totals(rows);
    DayAggregate {
        day: "total".into(),
        project: String::new(),
        harness: String::new(),
        avoided_est: t.avoided_est,
        client_prompt: t.client_prompt,
        client_completion: t.client_completion,
        client_sources: Vec::new(),
        worker_prompt: t.worker_prompt,
        worker_completion: t.worker_completion,
        activity: t.activity,
    }
}

/// Indexes into [`COLUMNS`] to show: all with `all`, else those with a
/// non-zero value; when every one is zero, the avoided estimate stays so the
/// table still says something.
fn numeric_columns(body: &[DayAggregate], all: bool) -> Vec<usize> {
    let keep: Vec<usize> = (0..COLUMNS.len())
        .filter(|&i| all || body.iter().any(|r| (COLUMNS[i].value)(r) != 0))
        .collect();
    if keep.is_empty() { vec![0] } else { keep }
}

fn text_columns(body: &[DayAggregate]) -> Vec<Shown> {
    let fields: [fn(&DayAggregate) -> &str; 3] = [|r| &r.day, |r| &r.project, |r| &r.harness];
    TEXT_HEADERS
        .iter()
        .zip(fields)
        .map(|(header, field)| Shown {
            header: (*header).to_string(),
            right: false,
            cells: body.iter().map(|r| field(r).to_string()).collect(),
        })
        .collect()
}

fn numeric_column(c: &Column, body: &[DayAggregate]) -> Shown {
    Shown {
        header: c.header.to_string(),
        right: true,
        cells: body.iter().map(|r| thousands((c.value)(r))).collect(),
    }
}

/// Line width of a table with these column widths:
/// `│ a ┆ b │` is the cell widths plus three per column plus one.
fn line_width(columns: &[Shown]) -> usize {
    columns.iter().map(|c| c.width() + 3).sum::<usize>() + 1
}

/// Fits the table into `width`: truncate the project column, then drop
/// numeric columns by priority and insert one `…` column where the leftmost
/// dropped one was. Returns the dropped headers in display order.
fn fit(columns: &mut Vec<Shown>, numeric: &[usize], width: Option<usize>) -> Vec<String> {
    let Some(width) = width else {
        return Vec::new();
    };
    if line_width(columns) > width {
        truncate_cells(&mut columns[1], PROJECT_MAX);
    }
    let mut order: Vec<usize> = (0..numeric.len()).collect();
    order.sort_by_key(|&k| (Reverse(COLUMNS[numeric[k]].priority), Reverse(k)));
    let mut drop = vec![false; numeric.len()];
    for k in order {
        if fitted_width(columns, &drop) <= width {
            break;
        }
        drop[k] = true;
    }
    apply_drops(columns, &drop)
}

/// Width after dropping the marked numeric columns (plus a `…` column).
fn fitted_width(columns: &[Shown], drop: &[bool]) -> usize {
    let text = TEXT_HEADERS.len();
    let kept: usize = columns[..text]
        .iter()
        .chain(
            columns[text..]
                .iter()
                .zip(drop)
                .filter(|(_, d)| !**d)
                .map(|(c, _)| c),
        )
        .map(|c| c.width() + 3)
        .sum();
    let ellipsis = if drop.contains(&true) { 4 } else { 0 };
    kept + ellipsis + 1
}

fn apply_drops(columns: &mut Vec<Shown>, drop: &[bool]) -> Vec<String> {
    let Some(first) = drop.iter().position(|d| *d) else {
        return Vec::new();
    };
    let rows = columns[0].cells.len();
    let numeric: Vec<Shown> = columns.drain(TEXT_HEADERS.len()..).collect();
    let mut dropped = Vec::new();
    for (i, (c, &d)) in numeric.into_iter().zip(drop).enumerate() {
        if i == first {
            columns.push(ellipsis_column(rows));
        }
        if d {
            dropped.push(c.header);
        } else {
            columns.push(c);
        }
    }
    dropped
}

fn ellipsis_column(rows: usize) -> Shown {
    Shown {
        header: "…".into(),
        right: false,
        cells: vec!["…".into(); rows],
    }
}

/// Cuts every cell longer than `max` to `max - 1` characters plus `…`.
fn truncate_cells(column: &mut Shown, max: usize) {
    for cell in &mut column.cells {
        if cell.chars().count() > max {
            let cut: String = cell.chars().take(max - 1).collect();
            *cell = format!("{cut}…");
        }
    }
}

/// Draws the box: top, header, `╞═╪═╡`, data rows, `╞═╪═╡`, total, bottom.
fn draw(columns: &[Shown], data_rows: usize) -> String {
    let widths: Vec<usize> = columns.iter().map(Shown::width).collect();
    let rule = |l: &str, fill: &str, mid: &str, r: &str| {
        let parts: Vec<String> = widths.iter().map(|w| fill.repeat(w + 2)).collect();
        format!("{l}{}{r}\n", parts.join(mid))
    };
    let line = |cells: Vec<(&str, bool)>| {
        let parts: Vec<String> = cells
            .iter()
            .zip(&widths)
            .map(|((cell, right), w)| format!(" {} ", pad(cell, *w, *right)))
            .collect();
        format!("│{}│\n", parts.join("┆"))
    };
    let mut out = rule("┌", "─", "┬", "┐");
    out.push_str(&line(
        columns.iter().map(|c| (c.header.as_str(), false)).collect(),
    ));
    out.push_str(&rule("╞", "═", "╪", "╡"));
    for row in 0..=data_rows {
        if row == data_rows {
            out.push_str(&rule("╞", "═", "╪", "╡"));
        }
        out.push_str(&line(
            columns
                .iter()
                .map(|c| (c.cells[row].as_str(), c.right))
                .collect(),
        ));
    }
    out.push_str(&rule("└", "─", "┴", "┘"));
    out
}

fn pad(cell: &str, width: usize, right: bool) -> String {
    let fill = " ".repeat(width.saturating_sub(cell.chars().count()));
    if right {
        format!("{fill}{cell}")
    } else {
        format!("{cell}{fill}")
    }
}

/// What the default view left out.
struct Hidden {
    rows: usize,
    window: Window,
    /// All-zero columns first, then those dropped to fit.
    columns: Vec<String>,
}

/// Column names the footer lists before it says `…`.
const FOOTER_NAMES: usize = 2;

/// Up to two short lines under the table; empty when nothing was hidden.
fn footer(h: &Hidden) -> String {
    let mut parts = Vec::new();
    if h.rows > 0 {
        let period = match &h.window {
            Window::LastDays(start) => format!("showing since {start}"),
            Window::MostRecent(day) => format!("showing the most recent day, {day}"),
            Window::All => String::new(),
        };
        let rows = plural(h.rows, "row");
        parts.push(format!("{} older {rows} hidden ({period})", h.rows));
    }
    if !h.columns.is_empty() {
        let n = h.columns.len();
        let mut names = h.columns[..n.min(FOOTER_NAMES)].join(", ");
        if n > FOOTER_NAMES {
            names.push_str(", …");
        }
        parts.push(format!("{n} {} hidden ({names})", plural(n, "column")));
    }
    if parts.is_empty() {
        return String::new();
    }
    let total = if h.rows > 0 {
        "The total covers the shown rows only. "
    } else {
        ""
    };
    format!("{}.\n{total}{ALL_HINT}\n", parts.join("; "))
}

fn plural(n: usize, word: &str) -> String {
    if n == 1 {
        word.to_string()
    } else {
        format!("{word}s")
    }
}

/// `1400` → `1,400` (human-readable table numbers).
pub fn thousands(n: i64) -> String {
    let digits = n.unsigned_abs().to_string();
    let len = digits.len();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (len - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    if n < 0 { format!("-{out}") } else { out }
}

#[cfg(test)]
#[path = "stats_table_tests.rs"]
mod tests;
