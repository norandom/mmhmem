//! The `McpServer` component (task 10.1; design.md "mcp layer" /
//! "Components and Interfaces" McpServer row; requirements.md Req 1.1-1.3,
//! 8.1).
//!
//! Remote MCP endpoint over Streamable HTTP (rmcp 3.x
//! `StreamableHttpService`) at a configurable path (default `/mcp`) and port
//! (default 8443, Req 1.1). Every request except `GET /healthz` must carry
//! `Authorization: Bearer <AGENT_MEM_TOKEN>`, compared in constant time
//! (`subtle`); failures return HTTP 401 before any tool dispatch and are
//! audited as AUTH_FAILURE (Req 1.2). Graceful shutdown on the cancellation
//! token (SIGTERM in task 10.4). Tools are registered in tasks 10.2/10.3.

use std::sync::Arc;

use anyhow::Context as _;
use axum::Router;
use axum::body::Body;
use axum::extract::State;
use axum::http::header::AUTHORIZATION;
use axum::http::{Request, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use subtle::ConstantTimeEq;
use tokio_util::sync::CancellationToken;

use crate::config::Config;
use crate::services::boilerplate::BoilerplateWriter;
use crate::services::breadcrumbs::BreadcrumbService;
use crate::services::memory::MemoryService;
use crate::services::prune::PruneService;
use crate::services::session_bus::SessionBus;
use crate::services::shunt::ShuntEngine;
use crate::services::stats::{OtelSink, StatsCollector};
use crate::services::worker::WorkerClient;
use crate::storage::audit::{AuditEventType, AuditWriter};
use crate::storage::db::Db;
use crate::telemetry::{self, Telemetry};

/// Built service graph shared by all MCP sessions.
#[derive(Clone)]
pub struct Services {
    /// Live cross-agent session bus.
    pub bus: SessionBus,
    /// Progressive-disclosure memory.
    pub memory: MemoryService,
    /// Token-savings stats collector (OTel sink wired when enabled).
    pub stats: StatsCollector,
    /// Read shunt engine (always present; worker may be absent, Req 9.4).
    pub shunt: ShuntEngine,
    /// Boilerplate writer (always present; worker may be unconfigured, Req 9.4 —
    /// confinement still runs before the worker check).
    pub boilerplate: BoilerplateWriter,
    /// Read-only breadcrumb history search (corpus-warmup-prune Req 10).
    pub breadcrumbs: BreadcrumbService,
    /// Stale-cache pruning (corpus-warmup-prune Req 3, 4).
    pub prune: PruneService,
}

/// Shared application state handed to the MCP service factory and middleware.
#[derive(Clone)]
pub struct AppState {
    /// Effective env-only config.
    pub config: Config,
    /// Audit writer (the only write path to the audit log).
    pub audit: AuditWriter,
    /// Optional OTel telemetry (task 9.1; no-op unless enabled).
    pub telemetry: Arc<Telemetry>,
    /// Whether a worker model is configured (Req 8 notice gate).
    pub worker_configured: bool,
    /// Service graph.
    pub services: Services,
}

impl AppState {
    /// Builds the full state from env-only config: opens storage, audit,
    /// telemetry (inert without OTEL_EXPORTER_OTLP_ENDPOINT), and services.
    pub fn build(config: Config) -> anyhow::Result<Self> {
        let db = Db::open_for_config(&config)?;
        let audit = AuditWriter::from_config(&config);
        let telemetry = Arc::new(telemetry::init_from_config(&config));
        let bus = SessionBus::new(db.clone(), audit.clone());
        let memory = MemoryService::new(db.clone(), audit.clone(), Some(bus.clone()));
        let stats = StatsCollector::with_sink(
            db.clone(),
            audit.clone(),
            Arc::clone(&telemetry) as Arc<dyn OtelSink>,
        );
        let worker = WorkerClient::from_config(&config).ok();
        let worker_configured = worker.is_some();
        let breadcrumbs = BreadcrumbService::new(db.clone(), audit.clone(), Some(bus.clone()));
        let prune = PruneService::new(db.clone(), audit.clone());
        let shunt = ShuntEngine::new(
            db.clone(),
            audit.clone(),
            config.clone(),
            Some(bus.clone()),
            worker.clone(),
        );
        let boilerplate = BoilerplateWriter::new(
            db.clone(),
            audit.clone(),
            &config,
            Some(bus.clone()),
            worker,
        );
        Ok(Self {
            config,
            audit,
            telemetry,
            worker_configured,
            services: Services {
                bus,
                memory,
                stats,
                shunt,
                boilerplate,
                breadcrumbs,
                prune,
            },
        })
    }
}

/// The MCP service (one per session via the factory closure). Tools are
/// registered in tasks 10.2/10.3; 10.1 ships the protocol surface + auth.
#[derive(Clone)]
pub struct McpServer {
    /// Shared app state; tool handlers (tasks 10.2/10.3) read services from it.
    pub state: Arc<AppState>,
}

// rmcp 3.x pattern: the tool router AND the ServerHandler impl (with
// get_info) live together in tools.rs — the generated `tool_router` fn is
// module-private in rmcp 3.x.

/// Constant-time bearer-token comparison (design.md Security Considerations).
fn token_matches(presented: &str, expected: &str) -> bool {
    presented.as_bytes().ct_eq(expected.as_bytes()).into()
}

/// Bearer-auth middleware: 401 before tool dispatch when the header is missing
/// or wrong; audited as AUTH_FAILURE. `GET /healthz` is exempt (container
/// healthcheck).
async fn bearer_auth(
    State(state): State<Arc<AppState>>,
    req: Request<Body>,
    next: Next,
) -> Response {
    if req.uri().path() == "/healthz" {
        return next.run(req).await;
    }
    let presented = req
        .headers()
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));
    let ok = presented
        .map(|t| token_matches(t, &state.config.token))
        .unwrap_or(false);
    if !ok {
        let _ = state.audit.write_event(
            AuditEventType::AuthFailure,
            "unknown",
            "unknown",
            serde_json::json!({
                "path": req.uri().path(),
                "reason": if presented.is_some() { "invalid_token" } else { "missing_token" },
            }),
        );
        return StatusCode::UNAUTHORIZED.into_response();
    }
    next.run(req).await
}

/// Unauthenticated container healthcheck (compose `healthcheck`, task 12.3).
async fn healthz() -> &'static str {
    "ok"
}

/// Builds the axum router: `/healthz` open, MCP path behind bearer auth.
pub fn router(state: Arc<AppState>) -> Router {
    let session_manager = Arc::new(LocalSessionManager::default());
    let mcp_path = state.config.mcp_path.clone();
    let service = StreamableHttpService::new(
        {
            let state = Arc::clone(&state);
            move || {
                Ok(McpServer {
                    state: Arc::clone(&state),
                })
            }
        },
        session_manager,
        StreamableHttpServerConfig::default(),
    );
    Router::new()
        .route("/healthz", get(healthz))
        .nest_service(&mcp_path, service)
        .layer(axum::middleware::from_fn_with_state(state, bearer_auth))
}

/// Runs the server until `ct` is cancelled (SIGTERM wiring is task 10.4).
pub async fn run(state: Arc<AppState>, ct: CancellationToken) -> anyhow::Result<()> {
    let app = router(Arc::clone(&state));
    let addr = (state.config.host.as_str(), state.config.port);
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("cannot bind {}:{}", state.config.host, state.config.port))?;
    tracing::info!(port = state.config.port, path = %state.config.mcp_path, "mmhmem MCP server listening");
    axum::serve(listener, app)
        .with_graceful_shutdown(async move { ct.cancelled().await })
        .await?;
    Ok(())
}
