//! End-to-end measurement suite with a real harness (task 13.1; Req 12).
//!
//! GATED: runs only with `E2E_LLM=1` so plain `cargo test` never needs model
//! access (Req 12.3). With the flag set, the suite:
//!
//! 1. builds a fixed fixture repo (read-heavy: one ~500-line Rust module;
//!    boilerplate-heavy: a reference Python module + write task) twice — one
//!    baseline copy and one mmhmem copy, each with an isolated project-local
//!    OpenCode config (baseline: no MCP; mmhmem: remote mmhmem MCP plus the
//!    SKILL.md playbook as instructions),
//! 2. brings the compose stack up (docker-compose.yml plus the e2e override;
//!    workspace mounted read-write so boilerplate_write works),
//! 3. runs scripted `opencode run --standalone --format json` sessions and
//!    collects harness-reported per-session prompt/completion tokens,
//! 4. persists the MEASURED numbers as kind=client_reported in token_stats
//!    and the audit log by calling `session_resume_or_handoff` with
//!    client_usage (the 10.2 wiring),
//! 5. computes per-task/aggregate deltas, asserts the configured input
//!    reduction on the read-heavy task (E2E_THRESHOLD_PCT, default 25), and
//!    emits a human-readable report (stdout + target/e2e/<run>/report.md).
//!
//! Env (beyond E2E_LLM=1): E2E_MODEL (optional, opencode default otherwise),
//! E2E_WORKER_API_URL / E2E_WORKER_MODEL / E2E_WORKER_API_KEY (worker for the
//! mmhmem daemon; URL/MODEL required), E2E_THRESHOLD_PCT (default 25),
//! E2E_TIMEOUT_SECS per session (default 900), E2E_COMPOSE (default
//! `docker compose`; on the Windows dev host use
//! `wsl -d Debian -e docker compose` — path args are translated automatically).
//! Provider credentials come from the ambient environment (CI secrets).

mod e2e;

#[test]
fn e2e_measured_token_savings_with_real_opencode() {
    if std::env::var("E2E_LLM").ok().as_deref() != Some("1") {
        eprintln!("skipping: E2E_LLM=1 not set (deterministic suite only, Req 12.3)");
        return;
    }
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime for the e2e suite");
    rt.block_on(e2e::run_suite());
}
