//! mmhmem — self-hosted cross-agent memory and I/O optimization server.
//!
//! Scaffolding (task 1.1): exposes the crate identity behind the
//! `--version` / `--help` CLI surface. Module wiring for config, storage,
//! services, and the MCP server lands in later tasks per design.md
//! "File Structure Plan".

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
         \n\
         FLAGS:\n\
         \x20   -h, --help       Print this help text and exit\n\
         \x20   -V, --version    Print the version and exit\n\
         \n\
         The daemon wiring (env config, storage, MCP endpoint) is added in a later\n\
         task; this build only provides the CLI surface.\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_is_three_part_numeric() {
        let parts: Vec<&str> = VERSION.split('.').collect();
        assert_eq!(parts.len(), 3);
        assert!(parts
            .iter()
            .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit())));
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
