//! Entrypoint of the mmhmem daemon.
//!
//! Scaffolding only (task 1.1): handles `--version` / `--help` and exits 0.
//! Daemon wiring (env config, storage init, MCP server) lands in task 10.4.

use std::process::ExitCode;

fn main() -> ExitCode {
    match std::env::args().nth(1).as_deref() {
        Some("--version") | Some("-V") => mmhmem::print_version(),
        Some("--help") | Some("-h") | None => print!("{}", mmhmem::help_text()),
        Some(other) => {
            eprintln!("unknown argument: {other}");
            eprint!("{}", mmhmem::help_text());
            return ExitCode::FAILURE;
        }
    }
    ExitCode::SUCCESS
}
