//! Local-mode stdio serving entry (local-mode Req 2.1-2.6; design.md
//! "Stdio serving entry").
//!
//! `mmhmem stdio` is spawned by a harness as a child process and serves
//! the FULL existing tool surface (same `AppState`/`McpServer` as serve
//! mode — parity is structural, Req 4.1-4.3) over the rmcp stdio
//! transport. Contract:
//! - stdout carries ONLY MCP frames; all diagnostics go to stderr (Req
//!   2.2) — `tracing_subscriber::fmt()` defaults to stdout, so this entry
//!   installs a stderr writer explicitly.
//! - stdin EOF (parent closed the pipe or exited) ends the session; the
//!   child flushes telemetry and exits 0 (Req 2.3) — including EOF that
//!   arrives before the handshake completes (the parent going away is a
//!   clean shutdown, not an error).
//! - No network listener exists on this path (Req 12.1): no axum router,
//!   no bearer-auth middleware — those are serve-mode-only concerns.
//! - The per-user store is created on first use (Req 2.6) by
//!   `AppState::build`, exactly like serve mode.
//! - Local mode only: a background thread refreshes the update-notice cache
//!   at most once a day (`services::update_check`); the tool path never
//!   waits on the network.

use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;

use crate::config::Config;
use crate::config::mmhmem_file::MmhmemFile;
use crate::mcp::server::{AppState, McpServer};
use crate::services::update_check::{self, UpdateNotice};
use crate::setup::release::{ReleaseSource, Target, latest_release_within};
use rmcp::ServiceExt;

/// `mmhmem stdio` entrypoint: initialize, serve until EOF, exit 0.
pub fn run_stdio() -> ExitCode {
    init_tracing_stderr();
    let config = match Config::load_local() {
        Ok(config) => config,
        Err(e) => {
            eprintln!("mmhmem stdio: {e}");
            return ExitCode::FAILURE;
        }
    };
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("mmhmem stdio: cannot start async runtime: {e}");
            return ExitCode::FAILURE;
        }
    };
    match runtime.block_on(serve_stdio(config)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("mmhmem stdio: {e:#}");
            ExitCode::FAILURE
        }
    }
}

/// Diagnostics strictly to stderr; stdout stays reserved for MCP frames
/// (Req 2.2). Mirrors the daemon's EnvFilter behavior (default `info`,
/// `RUST_LOG` override).
fn init_tracing_stderr() {
    use tracing_subscriber::EnvFilter;
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .init();
}

/// Builds the shared application state and serves it over stdio until the
/// stream ends.
async fn serve_stdio(config: Config) -> anyhow::Result<()> {
    let mut state = AppState::build(config)
        .context("initializing local-mode state (store, services, audit)")?;
    state.update_notice = start_update_check(&state.config);
    let state = Arc::new(state);
    // The Config Debug impl redacts secrets (Req 12.4); the profile is
    // logged so operators can confirm which mode a child runs in.
    tracing::info!(config = ?state.config, "mmhmem stdio ready (local mode)");
    let service = McpServer {
        state: Arc::clone(&state),
    };
    // `stdio_pipes` must be constructed INSIDE the runtime (it panics
    // off-runtime on Linux; research.md "rmcp 3.8.0 stdio transport API").
    let (reader, writer) = rmcp::transport::stdio_pipes();
    run_session(service, reader, writer).await?;
    state.telemetry.shutdown();
    Ok(())
}

/// Short per-request timeout for the background release lookup.
const UPDATE_LOOKUP_TIMEOUT: Duration = Duration::from_secs(10);

/// Wires the session-start update notice (local mode only, opt-out via
/// `MMHMEM_NO_UPDATE_CHECK=1` or `"update_check": false` in `.mmhmem.json`)
/// and starts the at-most-daily cache refresh on a background thread. The
/// tool path only ever reads the cache (`services::update_check`).
fn start_update_check(config: &Config) -> Option<UpdateNotice> {
    let env = |key: &str| std::env::var(key).ok();
    let file_setting = MmhmemFile::load_default_locations()
        .ok()
        .and_then(|f| f.update_check);
    if !update_check::enabled(config.profile, &env, file_setting) {
        return None;
    }
    let cache_file = update_check::cache_path(&config.data_dir);
    update_check::spawn_refresh(cache_file.clone(), newest_release_tag);
    Some(UpdateNotice { cache_file })
}

/// The newest stable release tag on GitHub, or None on any failure.
fn newest_release_tag() -> Option<String> {
    let target = Target::current().ok()?;
    latest_release_within(&ReleaseSource::github(), target, UPDATE_LOOKUP_TIMEOUT)
        .ok()
        .map(|latest| latest.tag.as_str().to_string())
}

/// Serves until stdin EOF. A parent that goes away before or during the
/// handshake is a CLEAN end (Req 2.3 — a spawned child's job is to end
/// quietly); any other handshake failure is a real error.
async fn run_session(
    service: McpServer,
    reader: rmcp::transport::io::StdioReader,
    writer: rmcp::transport::io::StdioWriter,
) -> anyhow::Result<()> {
    match service.serve((reader, writer)).await {
        Ok(running) => end_session(running).await,
        Err(e) => handle_handshake_error(e),
    }
}

/// A parent that goes away before/during the handshake is a CLEAN end
/// (Req 2.3 — a spawned child's job is to end quietly); any other
/// handshake failure is a real error.
fn handle_handshake_error(e: rmcp::service::ServerInitializeError) -> anyhow::Result<()> {
    if let rmcp::service::ServerInitializeError::ConnectionClosed(stage) = e {
        tracing::info!("stdio input closed at {stage}; exiting cleanly");
        return Ok(());
    }
    Err(anyhow::Error::new(e)).context("stdio MCP handshake failed")
}

/// Waits for the session task to finish (stdin EOF ends serving, Req 2.3).
/// A failed join is logged, not fatal: the session already ended.
async fn end_session(
    running: rmcp::service::RunningService<rmcp::RoleServer, McpServer>,
) -> anyhow::Result<()> {
    let outcome = match running.waiting().await {
        Ok(reason) => format!("stdio session ended: {reason:?}"),
        Err(e) => format!("stdio session task failed: {e}"),
    };
    tracing::info!("{outcome}");
    Ok(())
}
