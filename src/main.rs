//! Entrypoint of the mmhmem daemon (task 10.4 wiring).
//!
//! CLI flags (`--version` / `--help`) are handled before any config load.
//! With no flag: load env-only config fail-fast (task 1.3), build the service
//! graph + telemetry (AppState::build), and serve MCP Streamable HTTP until a
//! shutdown signal (SIGTERM on unix, Ctrl-C elsewhere), then flush OTel and
//! exit cleanly.

use std::process::ExitCode;
use std::sync::Arc;

use mmhmem::config::{Config, ConfigError};
use mmhmem::mcp::server::{self, AppState};
use tokio_util::sync::CancellationToken;
use tracing_subscriber::EnvFilter;

fn main() -> ExitCode {
    match std::env::args().nth(1).as_deref() {
        Some("--version") | Some("-V") => {
            mmhmem::print_version();
            ExitCode::SUCCESS
        }
        Some("--help") | Some("-h") => {
            print!("{}", mmhmem::help_text());
            ExitCode::SUCCESS
        }
        Some("healthcheck") => run_healthcheck(),
        // `serve` is the explicit alias for today's no-argument daemon
        // (Req 1.1); the no-arg path stays byte-identical (Req 9.3).
        Some("serve") => run_daemon(),
        // Local mode: the harness-spawned stdio child (Req 1.2, 2.1).
        Some("stdio") => mmhmem::mcp::stdio::run_stdio(),
        Some(other) => unknown_arg(other),
        None => run_daemon(),
    }
}

/// Unknown CLI flag: usage on stderr, exit 1.
fn unknown_arg(other: &str) -> ExitCode {
    eprintln!("unknown argument: {other}");
    eprint!("{}", mmhmem::help_text());
    ExitCode::FAILURE
}

/// `healthcheck` subcommand (task 12.3): GET the local daemon's `/healthz`
/// (Req 1.1) and exit 0 only on HTTP 200. The Docker runtime image contains
/// ONLY this binary — no shell, no curl — so container healthchecks (Docker
/// HEALTHCHECK / docker-compose) must run through it (Req 8.6).
fn run_healthcheck() -> ExitCode {
    let url = format!("http://127.0.0.1:{}/healthz", env_port());
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build();
    match runtime {
        Ok(rt) => match rt.block_on(probe_healthz(&url)) {
            Ok(true) => ExitCode::SUCCESS,
            Ok(false) => {
                eprintln!("mmhmem healthcheck: {url} answered non-200");
                ExitCode::FAILURE
            }
            Err(e) => {
                eprintln!("mmhmem healthcheck: {url} unreachable: {e}");
                ExitCode::FAILURE
            }
        },
        Err(e) => {
            eprintln!("mmhmem healthcheck: cannot start the tokio runtime: {e}");
            ExitCode::FAILURE
        }
    }
}

/// `AGENT_MEM_PORT` for the probe (trimmed), defaulting to the documented
/// port so the subcommand works with an empty env inside containers.
fn env_port() -> String {
    std::env::var(mmhmem::config::ENV_PORT)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| mmhmem::config::DEFAULT_PORT.to_string())
}

/// One bounded GET; true iff the daemon answered 200.
async fn probe_healthz(url: &str) -> Result<bool, reqwest::Error> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()?;
    let resp = client.get(url).send().await?;
    Ok(resp.status() == reqwest::StatusCode::OK)
}

/// No-flag path: tracing, fail-fast config, multi-thread tokio runtime, serve.
fn run_daemon() -> ExitCode {
    init_tracing();
    match Config::load() {
        Ok(config) => match tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime.block_on(serve(config)),
            Err(e) => {
                eprintln!("mmhmem: cannot start the tokio runtime: {e}");
                ExitCode::FAILURE
            }
        },
        Err(err) => config_error(err),
    }
}

/// Builds the app state and serves until the cancellation token fires.
async fn serve(config: Config) -> ExitCode {
    let state = match AppState::build(config) {
        Ok(state) => Arc::new(state),
        Err(e) => {
            eprintln!("mmhmem: cannot initialize: {e}");
            return ExitCode::FAILURE;
        }
    };
    tracing::info!("effective configuration: {}", state.config.redacted_debug());
    let ct = CancellationToken::new();
    spawn_signal_handler(ct.clone());
    let result = server::run(Arc::clone(&state), ct).await;
    state.telemetry.shutdown();
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("mmhmem: server error: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Cancels the token on the first shutdown signal.
fn spawn_signal_handler(ct: CancellationToken) {
    tokio::spawn(async move {
        shutdown_signal().await;
        tracing::info!("shutdown signal received");
        ct.cancel();
    });
}

/// SIGTERM on unix, Ctrl-C elsewhere (Windows has no SIGTERM).
#[cfg(unix)]
async fn shutdown_signal() {
    use tokio::signal::unix::{signal, SignalKind};
    if let Ok(mut sigterm) = signal(SignalKind::terminate()) {
        sigterm.recv().await;
    }
}

/// SIGTERM on unix, Ctrl-C elsewhere (Windows has no SIGTERM).
#[cfg(not(unix))]
async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}

/// Config load failure: clear error, exit 1, before any socket/DB work.
fn config_error(err: ConfigError) -> ExitCode {
    eprintln!("mmhmem: invalid configuration: {err}");
    eprintln!(
        "mmhmem: required env vars: AGENT_MEM_TOKEN, AGENT_MEM_ALLOWED_ROOTS \
         (comma-separated); refusing to start"
    );
    ExitCode::FAILURE
}

/// INFO-level logs by default; `RUST_LOG` overrides.
fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();
}
