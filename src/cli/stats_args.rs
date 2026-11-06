//! `mmhmem stats [--all] [--json]` flags and the table width source
//! (local-mode Req 1.3, 8.1, 8.4).
//!
//! Flags come in any order; anything else is a usage error (exit 2 in
//! `main.rs`). The width is the `COLUMNS` variable when it is a positive
//! integer, else the terminal size when stdout is a terminal, else no limit
//! (pipes and files get the whole table).

use std::io::IsTerminal;

/// Printed on stderr for an unknown `stats` argument.
pub const STATS_USAGE: &str = "usage: mmhmem stats [--all] [--json]";

/// Parsed `stats` flags.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StatsArgs {
    /// `--json`: the machine-readable report (all rows, all fields).
    pub json: bool,
    /// `--all`: every row and column in the table, no width limit.
    pub all: bool,
}

impl StatsArgs {
    /// Parses the arguments after `stats`; the error is the first unknown one.
    pub fn parse<S: AsRef<str>>(args: &[S]) -> Result<Self, String> {
        let mut parsed = Self::default();
        for arg in args {
            match arg.as_ref() {
                "--json" => parsed.json = true,
                "--all" => parsed.all = true,
                other => return Err(other.to_string()),
            }
        }
        Ok(parsed)
    }
}

/// The table width for this process: `COLUMNS`, else the terminal size.
pub fn table_width() -> Option<usize> {
    width_from(std::env::var("COLUMNS").ok().as_deref(), || {
        if !std::io::stdout().is_terminal() {
            return None;
        }
        crossterm::terminal::size()
            .ok()
            .map(|(cols, _)| usize::from(cols))
    })
}

/// `columns` wins when it is a positive integer; else `terminal` decides.
pub fn width_from(
    columns: Option<&str>,
    terminal: impl FnOnce() -> Option<usize>,
) -> Option<usize> {
    match columns.and_then(|c| c.trim().parse::<usize>().ok()) {
        Some(n) if n > 0 => Some(n),
        _ => terminal().filter(|n| *n > 0),
    }
}

#[cfg(test)]
#[path = "stats_args_tests.rs"]
mod tests;
