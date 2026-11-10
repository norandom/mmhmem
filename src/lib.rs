//! mmhmem — self-hosted cross-agent memory and I/O optimization server.
//!
//! Scaffolding (task 1.1) exposes the crate identity behind the
//! `--version` / `--help` CLI surface; task 1.3 adds env-only configuration
//! (`config`); task 2.1 adds durable storage (`storage`); task 3.1 adds the
//! worker-model client (`services::worker`). The session bus, shunt engine,
//! and the MCP server land in later tasks per design.md "File Structure Plan".

pub mod cli;
pub mod config;
pub mod hook;
pub mod mcp;
pub mod services;
pub mod setup;
pub mod storage;
pub mod telemetry;

/// Crate name from Cargo.toml.
pub const NAME: &str = env!("CARGO_PKG_NAME");
/// Crate version from Cargo.toml.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Prints `mmhmem <version>` to stdout.
pub fn print_version() {
    println!("{NAME} {VERSION}");
}

/// Usage text printed by `--help` (contains the version line).
pub fn help_text() -> String {
    format!(
        "{NAME} {VERSION} - self-hosted cross-agent memory server (MCP Streamable HTTP)\n\
         \n\
         USAGE:\n\
         \x20   {NAME} [FLAG]\n\
         \x20   {NAME} serve\n\
         \x20   {NAME} stdio\n\
         \x20   {NAME} stats [--all] [--json]\n\
         \x20   {NAME} prune [--dry-run]\n\
         \x20   {NAME} setup\n\
         \x20   {NAME} update\n\
         \x20   {NAME} hook <harness>\n\
         \x20   {NAME} healthcheck\n\
         \n\
         FLAGS:\n\
         \x20   -h, --help       Print this help text and exit\n\
         \x20   -V, --version    Print the version and exit\n\
         \n\
         COMMANDS:\n\
         \x20   serve            Start the daemon (same as no argument)\n\
         \x20   stdio            Local mode: serve MCP over stdin/stdout for\n\
         \x20                    the spawning harness, using per-user paths and\n\
         \x20                    no bearer token (AGENT_MEM_* optional)\n\
         \x20   stats [--all] [--json]\n\
         \x20                    Print token-savings statistics from the local\n\
         \x20                    per-user store: a table by day, project, and\n\
         \x20                    harness of the last 7 days that fits the\n\
         \x20                    terminal (--all: every row and column; --json:\n\
         \x20                    all data as JSON)\n\
         \x20   prune [--dry-run]  Remove stale cache rows (missing file or changed\n\
         \x20                    content) from the local per-user store; --dry-run\n\
         \x20                    previews the grouped counts\n\
         \x20   setup            Install or update mmhmem for your AI tools\n\
         \x20   update           Update mmhmem to the newest release, keeping your\n\
         \x20                    tools and choices as they are\n\
         \x20   hook <harness>   Read-routing pre-tool hook (claude, codex, opencode,\n\
         \x20                    antigravity): reads\n\
         \x20                    one hook event on stdin and declines full reads of\n\
         \x20                    large files in favour of smart_read; always exits 0\n\
         \x20   healthcheck      Probe GET /healthz on the local daemon and exit 0 on\n\
         \x20                    HTTP 200 (container healthchecks; no curl in the image)\n\
         \n\
         MCP TOOLS (same set over serve and stdio; docs/usage.md has details):\n\
         \x20   session_resume_or_handoff  Resume or checkpoint working state\n\
         \x20   smart_read                 Read exact lines or summarize large files\n\
         \x20   boilerplate_write          Generate files through the worker\n\
         \x20   mem_search                 Search the compact memory index\n\
         \x20   mem_get_observations       Fetch selected observation details\n\
         \x20   mem_record_observation     Store a project observation\n\
         \x20   shunt_prune                Remove stale cache entries\n\
         \x20   mem_search_breadcrumbs     Search project action history\n\
         \x20   status                     Report version and store health\n\
         \x20   shunt_content              Summarize content a caller holds as a string\n\
         \x20                              (programmatic consumers; for files use smart_read)\n\
         \n\
         With no flag the daemon loads its env-only configuration (AGENT_MEM_*,\n\
         SHUNT_MIN_LINES, WORKER_*, OTEL_*) fail-fast and exits non-zero when a\n\
         required variable is missing; storage and MCP server wiring lands in a\n\
         later task.\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_is_three_part_numeric() {
        let parts: Vec<&str> = VERSION.split('.').collect();
        assert_eq!(parts.len(), 3);
        assert!(
            parts
                .iter()
                .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
        );
    }

    #[test]
    fn help_mentions_both_flags_and_version() {
        let help = help_text();
        assert!(help.contains("--help"));
        assert!(help.contains("--version"));
        assert!(help.contains(VERSION));
        assert!(help.contains(NAME));
    }
}
