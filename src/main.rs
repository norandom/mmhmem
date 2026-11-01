//! Entrypoint of the mmhmem daemon.
//!
//! CLI flags (`--version` / `--help`) are handled before any config load.
//! With no flag the daemon loads the env-only config fail-fast (task 1.3):
//! missing/invalid required config prints a clear error to stderr and exits 1
//! before any socket or database work; on success the redacted effective
//! config is logged via tracing. Storage + MCP server wiring lands in 10.4.

use std::process::ExitCode;
use tracing_subscriber::EnvFilter;

fn main() -> ExitCode {
    match std::env::args().nth(1).as_deref() {
        Some("--version") | Some("-V") => {
            mmhmem::print_version();
            return ExitCode::SUCCESS;
        }
        Some("--help") | Some("-h") => {
            print!("{}", mmhmem::help_text());
            return ExitCode::SUCCESS;
        }
        Some(other) => {
            eprintln!("unknown argument: {other}");
            eprint!("{}", mmhmem::help_text());
            return ExitCode::FAILURE;
        }
        None => {}
    }
    init_tracing();
    match mmhmem::config::Config::load() {
        Ok(config) => {
            tracing::info!("effective configuration: {}", config.redacted_debug());
            tracing::info!("storage + MCP server wiring lands in task 10.4; exiting");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("mmhmem: invalid configuration: {err}");
            eprintln!(
                "mmhmem: required env vars: AGENT_MEM_TOKEN, AGENT_MEM_ALLOWED_ROOTS \
                 (comma-separated); refusing to start"
            );
            ExitCode::FAILURE
        }
    }
}

/// INFO-level logs by default; `RUST_LOG` overrides.
fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();
}
