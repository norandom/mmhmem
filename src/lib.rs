//! mmhmem — self-hosted cross-agent memory and I/O optimization server.
//!
//! Scaffolding (task 1.1) exposes the crate identity behind the
//! `--version` / `--help` CLI surface; task 1.3 adds env-only configuration
//! (`config`); task 2.1 adds durable storage (`storage`); task 3.1 adds the
//! worker-model client (`services::worker`). The session bus, shunt engine,
//! and the MCP server land in later tasks per design.md "File Structure Plan".

pub mod cli;
pub mod config;
pub mod mcp;
pub mod services;
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
         \x20   {NAME} stats [--json]\n\
         \x20   {NAME} prune [--dry-run]\n\
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
         \x20   stats [--json]    Print token-savings statistics from the local\n\
         \x20                    per-user store: a human table by day, project,\n\
         \x20                    and harness (or JSON with --json)\n\
         \x20   prune [--dry-run]  Remove stale cache rows (missing file or changed\n\
         \x20                    content) from the local per-user store; --dry-run\n\
         \x20                    previews the grouped counts\n\
         \x20   healthcheck      Probe GET /healthz on the local daemon and exit 0 on\n\
         \x20                    HTTP 200 (container healthchecks; no curl in the image)\n\
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
