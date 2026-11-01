# Research & Design Decisions Log

## Summary
- **Feature**: cross-agent-memory-server (mmhmem)
- **Discovery Scope**: New Feature (greenfield) — full discovery executed
- **Key Findings**:
  - The official Rust MCP SDK **rmcp** (modelcontextprotocol/rust-sdk, v3.x) implements the stable MCP 2026-07-28 spec and provides a Streamable HTTP server transport supporting multiple simultaneous clients — this removes the need to hand-roll MCP framing.
  - **rusqlite** with the `bundled` cargo feature compiles an up-to-date SQLite amalgamation (with FTS5) inside the crate, so the daemon is a single static binary with no system SQLite dependency.
  - **axum 0.8.x** (0.8.9 as of Sep 2026) on **tokio 1.x** is the current stable async HTTP stack; rmcp's Streamable HTTP transport builds on this stack, so axum is used indirectly (auth middleware) rather than as a parallel framework.
  - Language decision locked with the user: **Rust** (Go fallback rejected for this security-grade daemon; Zig rejected for immature server ecosystem).

## Research Log

### Rust MCP server SDK landscape
- **Context**: Requirement 1 demands a remote Streamable HTTP MCP endpoint usable by Claude Code, Antigravity, OpenCode, Claude Desktop.
- **Sources Consulted**: github.com/modelcontextprotocol/rust-sdk, crates.io/crates/rmcp (range 3.0.1), docs.rs/rmcp, rustify.rs 2026 guide.
- **Findings**: rmcp 3.x supports MCP spec 2026-07-28 (compatible with 2025-11-25), stateless Streamable HTTP server mode with multiple simultaneous clients; tool macros for registering tools; supports auth hooks for Bearer validation.
- **Implications**: Adopt rmcp instead of building MCP framing manually. Bearer auth enforced in a middleware/hook before tool dispatch (Req 1.2). No client hooks needed anywhere (Req 1.5).

### Embedded storage with full-text search
- **Context**: Requirements 2, 3, 5, 9 require durable local state, WAL concurrency, FTS index, content-addressed cache.
- **Sources Consulted**: crates.io/crates/rusqlite (0.41.x feature flags), rusqlite GitHub discussions on bundled builds.
- **Findings**: `bundled` feature compiles the latest SQLite amalgamation into the binary — WAL mode, FTS5 virtual tables, and triggers all available with zero system dependencies. Single-binary deployment aligns with the systemd/logrotate operations story (Req 8).
- **Implications**: rusqlite + bundled. Schema: active_sessions, session_breadcrumbs, shunt_file_cache, observations, observations_fts (FTS5 external-content table synced by AFTER INSERT trigger). Connections serialized through a single async mutex or a small pool with WAL (Req 9.1).

### Async HTTP/runtime stack
- **Context**: Requirement 9 requires concurrent multi-harness handling and sub-50 ms local paths.
- **Sources Consulted**: tokio.rs blog (axum 0.8.0 announcement), crates.io axum, encore.dev 2026 framework comparison.
- **Findings**: axum 0.8.x on tokio 1.x is the maintained standard; rmcp's streamable-http-server transport uses axum/hyper internally, so the stack is coherent without double frameworks.
- **Implications**: tokio runtime, rmcp for MCP transport, reqwest for the worker model HTTP client, sha2 + serde_json for audit hashing/records.

### Worker model integration
- **Context**: Requirements 3–6 need an OpenAI-compatible chat-completions endpoint (local vLLM/Ollama or API), temperature 0, strict prompts, error propagation.
- **Sources Consulted**: research doc (Self-Hosted Cross-Agent Memory Solutions.md), OpenAI-compatible API conventions.
- **Findings**: Plain POST to `/v1/chat/completions` with model, temperature, messages; no vendor SDK needed — reqwest + serde is sufficient and keeps the binary lean.
- **Implications**: WorkerClient is a thin typed client; failures return explicit errors and audit events, never stale data (Req 6.3).

## Architecture Pattern Evaluation

| Option | Description | Strengths | Risks / Limitations | Notes |
|--------|-------------|-----------|---------------------|-------|
| Layered single daemon (chosen) | Protocol (MCP) → services → storage, one process | Matches doc reference architecture; one static binary; simple systemd ops; dependency direction enforceable | Single process = single point of failure | Chosen; clustering explicitly out of scope |
| Microservices (MCP gateway + memory svc + audit svc) | Split concerns into processes | Independent scaling | Operational overhead, network trust boundaries between own components | Rejected: overkill for a single-host tool |
| Python FastMCP port of the reference impl | Reuse the doc's reference code directly | Fastest to write | Not compiled (user requirement); interpreter/runtime deps on host | Rejected: user explicitly asked for a compiled language |
| Go implementation | Single binary, fast builds | Great ops ergonomics | Less memory-safety rigor for untrusted input handling | Fallback if team velocity wins; not selected |

## Design Decisions

### Decision: Rust as the implementation language
- **Context**: User asked for a compiled-language recommendation; the reference implementation is Python.
- **Alternatives Considered**: 1. Go 2. C++ 3. V (vlang) 4. Zig 5. Stay with Python.
- **Selected Approach**: Rust — axum/tokio stack + rmcp + rusqlite(bundled) + reqwest + sha2.
- **Rationale**: Network-facing, multi-tenant, security-grade daemon (path handling on untrusted input, secret redaction, concurrent sessions): ownership/borrow checker eliminates whole bug classes; single static binary fits the self-hosted systemd deployment; ai-memory precedent proves the pattern in Rust.
- **Trade-offs**: Slower compile times and steeper onboarding vs. memory safety and deployment simplicity.
- **Follow-up**: Pin toolchain in rust-toolchain.toml; CI with clippy -D warnings.

### Decision: Rejected — Go, C++, V, Zig
- **Go**: viable fallback (fast builds, pure-Go SQLite via modernc, official MCP Go SDK), but weaker compile-time guarantees on the security-critical paths (ignored `error` values, runtime-only race detection). Chosen only if team velocity outweighs safety rigor.
- **C++**: viable in principle — SQLite amalgamation + `SQLITE_ENABLE_FTS5` is rock solid, mature HTTP servers (Drogon, Crow, Boost.Beast), single static binary — but rejected for this project: no official MCP SDK (official set is TS/Python/C#/Go/Java/Rust/Ruby/PHP/Swift/Kotlin; only unofficial community ones like cpp-mcp/gopher-mcp, and MCP impls are security-sensitive with 382 NVD CVEs), manual memory management on a daemon that parses agent-supplied paths and text (classic CVE surface), no compile-time race safety (TSan is runtime-only), slow builds. No native-interop or perf-control need here that would justify it over Rust.
- **V (vlang)**: attractive compile speed and tiny binaries, but rejected — no MCP SDK exists (would hand-roll the 2026-07-28 Streamable HTTP contract while MCP already has 382 NVD CVEs, i.e. treated as security-critical), `db.sqlite` still needs system libsqlite3 on Linux (open issue #26830) with FTS5 dependent on build flags, pre-1.0 (0.5) language with breaking changes, no race detector, tiny security-library ecosystem. Good fit for small internal CLIs, not for a multi-harness protocol server.
- **Zig**: the language itself is excellent (C interop means the SQLite amalgamation + FTS5 bundles cleanly, fast builds, tiny static binaries, explicit allocators), but rejected for production here in 2026: still pre-1.0 (0.16.0 shipped Apr 2026, 0.17 in dev) with stdlib breaking changes roughly every release and community reports of instability ("Why Zig Isn't 1.0 Yet", JetBrains 2026); HTTP server story is low-level `std.http` plus young community frameworks — a 2026 blog literally titled "I Tried to Write a HTTP Service in Zig and Failed" used http.zig + SQLite, i.e. exactly our stack; MCP has no official/tiered SDK (only a single hobbyist lib, mcp.zig); plus 2026 governance turbulence (left GitHub, foundation, no-AI policy). Fine for systems tooling; not for a multi-harness protocol daemon that must track MCP spec revisions.

### Decision: Adopt rmcp for MCP instead of hand-rolling
- **Context**: Req 1 needs Streamable HTTP MCP with Bearer auth.
- **Alternatives Considered**: Hand-rolled JSON-RPC over axum; rust-mcp-sdk community crate.
- **Selected Approach**: Official rmcp 3.x with streamable-http-server transport; Bearer check in an auth hook/middleware.
- **Rationale**: Spec-compliant (2026-07-28), maintained by the MCP project, multi-client stateless HTTP built in.
- **Trade-offs**: Less control over transport internals; mitigated by keeping tool handlers thin.
- **Follow-up**: Verify auth-hook API surface during implementation.

### Decision: Server-side shunt policy in a dedicated engine, not in tool handlers
- **Context**: Reqs 3–4 (deterministic routing, security bypass, cache).
- **Alternatives Considered**: Policy inline in each tool handler.
- **Selected Approach**: ShuntEngine owns the four-stage evaluation (targeted read → threshold/security bypass → cache → worker), tool handlers stay thin.
- **Rationale**: Single place for policy, unit-testable without HTTP; matches steering rule that handlers are thin.
- **Trade-offs**: None material.
- **Follow-up**: Security-critical regex kept in one constant with tests per pattern class.

### Decision: Add workspace path confinement (beyond the reference impl)
- **Context**: smart_read/boilerplate_write take file paths from agents; the Python reference resolves paths but does not confine them.
- **Alternatives Considered**: Trust the caller (reference behavior); hard-coded root.
- **Selected Approach**: Configurable list of allowed workspace root directories (env `AGENT_MEM_ALLOWED_ROOTS`); every file path resolved and canonicalized, rejected if outside all roots; rejection audited.
- **Rationale**: Security-grade daemon must not become an arbitrary-file-read/write primitive; cheap to enforce, testable.
- **Trade-offs**: Operators must configure roots; default = deny outside configured roots.
- **Follow-up**: Cover with dedicated security tests (traversal attempts).

### Decision: Audit writer is the only write path to the log
- **Context**: Reqs 7.x (tamper-evident, redacted, logrotate-safe).
- **Alternatives Considered**: Scattered logging at call sites.
- **Selected Approach**: Single AuditWriter with redaction filter + SHA-256 record digest; opens file with O_APPEND per write; all services call it.
- **Rationale**: Guarantees redaction/hash on every record; append-per-write keeps logrotate working without restart (Req 7.4).
- **Trade-offs**: Slight write latency; negligible at this volume.
- **Follow-up**: Redaction regex ported from claude-mem v15.5.0 patterns plus `<private>` tags.

### Decision: Minimal dependency set ("adopt, don't build") to keep complexity low
- **Context**: Greenfield daemon; complexity budget matters more than flexibility.
- **Selected Approach**: ~12 well-known crates, each replacing a hand-rolled component:
  - `rmcp` 3.x (official tier-1 MCP SDK, Streamable HTTP server transport) — replaces hand-rolled JSON-RPC + MCP 2026-07-28 framing
  - `tokio` 1.x — runtime only; no actor frameworks
  - `rusqlite` with `bundled` — SQLite amalgamation (WAL + FTS5) compiled into the binary; replaces external DB, ORM, and migration framework (schema = `CREATE TABLE IF NOT EXISTS` + `PRAGMA user_version`, like the reference impl)
  - `serde`/`serde_json` — all JSON (audit records, worker payloads, MCP handled by rmcp)
  - `sha2` — content hashes + audit record digests; `subtle` — constant-time bearer-token compare
  - `regex` — secret-redaction filter and security-critical path pattern
  - `reqwest` (rustls, json, per-request timeout) — the only egress: worker model endpoint; E2E tests also use it to POST MCP JSON-RPC directly (no test client SDK needed)
  - `time` or `chrono` — ISO-8601 UTC timestamps in audit records
  - `anyhow` internally, mapped to plain tool-error strings at the MCP boundary — no elaborate error taxonomy
  - `tracing` + `tracing-subscriber` for stderr diagnostics (the JSONL audit remains the system of record)
  - dev-only: `tempfile`, `tokio::test`
- **Simplification rules**: single process; one SQLite connection behind a `tokio::sync::Mutex` (WAL, no pool config); fully synchronous tool logic — the worker LLM call is the only await; env-only config (~30 lines, no config framework); bearer auth = one tiny axum middleware layer, no OAuth/JWT; TLS terminated at a reverse proxy, not in the binary; no background queues/workers; logrotate does rotation, daemon just appends per write; no web UI (already out of scope); `Cargo.lock` committed; clippy `-D warnings`; no `unsafe`.
- **Build order**: port the Python reference implementation 1:1 as v1 (it is a complete spec), get one tool E2E first (vertical slice: `session_resume_or_handoff`), then the rest; the single deliberate hardening addition on top is workspace path confinement (`AGENT_MEM_ALLOWED_ROOTS`).
- **Rationale**: every avoided component (MCP framing, DB ops, pools, migrations, OAuth, TLS, rotation) is a class of bugs and ops burden we don't need; the reference architecture is already proven in Python, so a straight port keeps review easy.
- **Trade-offs**: less flexibility (single token, no TLS in-process, no clustering) — all acceptable per boundary commitments.
- **Follow-up**: verify bundled SQLite exposes FTS5 with a `SELECT fts5(?)` smoke query in the first integration test.

### Decision: Measurable token budget without a tokenizer dependency
- **Context**: Req 2.5 caps working state at 1,500 tokens; pulling in a model tokenizer would break the minimal-dependency budget and couple the cap to a specific model.
- **Selected Approach**: approximate 4 chars/token → a 6,000-character budget, truncated at the last complete line before the limit; documented as a SessionBus invariant and unit-tested.
- **Rationale**: keeps the acceptance criterion testable while bounding handoff payload size, which is the actual goal.
- **Trade-offs**: approximate, not model-exact — acceptable because the requirement bounds size, not exact token count.

### Decision: SQLite connection serviced from a blocking thread
- **Context**: Req 9.1/9.2 require safe concurrency and sub-50 ms local paths; rusqlite calls are blocking and must not run on tokio's async worker threads.
- **Selected Approach**: one `Connection` behind a `tokio::sync::Mutex`, only touched inside `spawn_blocking`; the lock is never held across an `.await` (a shunt cache miss = lookup, worker call, store as three separate locked sections).
- **Rationale**: keeps single-writer simplicity (no locking bugs) while protecting latency under concurrent harnesses.
- **Trade-offs**: slightly more plumbing than a naively shared connection.

### Decision: Docker Compose as a first-class deployment path (in addition to systemd)
- **Context**: Req 8 was systemd-only; operators increasingly run single-host services via compose.
- **Selected Approach**: multi-stage Dockerfile — rust builder stage, runtime stage with a minimal non-root base containing **only** the dedicated `mmhmem` binary; `docker-compose.yml` with env_file, published MCP port, volumes for the data dir, the audit log dir (host-side logrotate rotates the bind-mounted file — no container restart, Req 8.7), and allowed roots mounted read-only; `GET /healthz` (no auth) for the compose healthcheck.
- **Rationale**: same single-binary ops story as systemd, reproducible env config, no extra daemons inside the container.
- **Trade-offs**: compose needs the log volume writable by the daemon uid and readable by host logrotate; documented in the compose file comments.

### Decision: Token-savings statistics = server-computed estimates + optional client-reported deltas
- **Context**: Req 10 wants per-harness / per-task token-savings stats over time. The server never sees the frontier model's API, so true frontier token counts cannot be known server-side.
- **Selected Approach**: (a) server computes honest estimates at each shunt/write event — avoided frontier input ≈ (raw chars − returned chars)/4 for delegated reads, raw chars/4 for cache hits, reference+generated chars/4 for boilerplate — plus the worker API's own `usage` prompt/completion tokens; stored in `token_stats` (kind=shunt_delegated|shunt_cache_hit|code_write) and audited as TOKEN_STATS; (b) the harness may include its real cumulative frontier usage in the `session_resume_or_handoff` checkpoint (`client_usage`), stored as kind=client_reported and audited as TOKEN_STATS_CLIENT (Req 10.4). Estimates are labelled estimates; client values are labelled by source; nothing is invented (10.5).
- **Rationale**: the server knows exactly how many raw bytes it kept out of the frontier context and what it returned instead — that delta is the savings mechanism and is exactly measurable; the client delta closes the loop with ground truth from the frontier API.
- **Trade-offs**: estimates use the ÷4 chars/token heuristic (same as the 2.5 cap); per-session attribution uses workspace_task_id + harness.
- **Follow-up**: OTel counters mirror the same numbers when enabled (11.3); no extra MCP tool is added to read stats (kept out of breath.toml) — stats are read from token_stats / audit / OTel.

### Decision: Optional OTel via a feature-gated exporter, JSONL audit stays the system of record
- **Context**: Operator has a self-hosted OpenObserve and wants traces/metrics/logs for MCP calls and server behavior on both ends.
- **Selected Approach**: `opentelemetry` + `opentelemetry-otlp` behind cargo feature `otel`, compiled only into the Docker image; inert unless `OTEL_EXPORTER_OTLP_ENDPOINT` is set; spans around every tool call, histograms/counters for latency, cache hit rate, tokens avoided, worker tokens; every payload passes the audit redaction filter; egress limited to the configured self-hosted endpoint. Agent-side tracing is configured in the harness itself (e.g. OpenCode's own OTLP export to OpenObserve) and documented in deploy/examples — mmhmem ships no client code (Req 11.5).
- **Rationale**: observability without changing the security posture — off by default, no SaaS, redacted, and the tamper-evident JSONL remains authoritative for forensics.
- **Trade-offs**: ~4 extra crates in the Docker image only; plain release binary stays at the minimal dependency set.

### Decision: E2E measurement with a real harness (OpenCode + Mistral) instead of claimed savings
- **Context**: Req 12 — token savings must be measured, not claimed.
- **Selected Approach**: `tests/e2e/` brings up the compose stack, runs scripted OpenCode sessions (default provider/model in this environment: Mistral via vibe-oidc) on a fixed fixture repo in baseline mode (mmhmem disconnected) and mmhmem mode (MCP + SKILL.md), harvests harness-reported per-session prompt/completion tokens, asserts the configured input-token reduction threshold on read-heavy tasks, and persists measured numbers as `client_reported` stats. Gated by `E2E_LLM=1`; plain `cargo test` runs only deterministic tests.
- **Rationale**: the only honest way to prove savings is to diff real harness usage with/without the server on identical tasks.
- **Trade-offs**: needs model access and is slower/flakier than unit tests — hence gated and threshold-based, not exact-equality.

### Decision: breath.toml as the scope guard; prompt-injection scanning explicitly rejected
- **Context**: Discussion surfaced candidate features (prompt-injection detection/classification at the server, a stats query tool, client SDKs) that would grow scope.
- **Selected Approach**: a `breath.toml` feature map in the spec directory lists everything in scope and everything explicitly out, with reasons; editing it is the only way to change scope.
- **Rationale**: prompt-injection *detection* is mostly blind server-side because the main injection channels (native reads, web fetches, bash output) never transit the server; the hash-verified audit trail remains the forensic record. Keep the project focused.
- **Trade-offs**: none — forensics are not lost, only active detection is out of scope.

### Decision: McCabe-style complexity caps to prevent codebase explosion
- **Context**: Greenfield daemon ported from a reference implementation, with stats/OTel/docker/e2e added on top — real risk of creeping complexity while implementing.
- **Selected Approach**: hard caps enforced by tooling, not by review alone: clippy `cyclomatic_complexity` ≤ 10 and `cognitive_complexity` ≤ 15 per function via `clippy.toml` thresholds + `-D warnings` in CI (no `#[allow]` without written justification); ≤ 500 LOC per `src/` file, ≤ 80 LOC per function, ≤ 800 LOC per test file; per-task budget ≤ 400 added non-test lines (task generation splits larger work); CI `tokei` LOC report as informational. Codified as Req 13, copied into every task's acceptance criteria (13.6).
- **Rationale**: McCabe's original ≤ 10 keeps each function auditable on one screen — decisive for the security-critical paths (path confinement, redaction, bearer compare); LOC caps force the layered module split already in the design instead of god-modules. "Port the reference 1:1" stays a port, not a rewrite.
- **Trade-offs**: occasional early refactor to split a function or module; acceptable and cheaper than review-driven discovery later.

## Risks & Mitigations
- **Risk 1**: rusqlite bundled compile time slows CI. — Mitigation: cache cargo registry/target in CI; use release profile only for artifacts.
- **Risk 2**: rmcp 3.x API churn between minor versions. — Mitigation: pin exact version in Cargo.toml; upgrade deliberately with integration tests.
- **Risk 3**: Worker model latency (10–30 s) on cache misses stalls agent turns. — Mitigation: per-request timeout (30 s read / 45 s write), latency audited, cache makes repeats instant (Req 3.6).
- **Risk 4**: FTS5 query injection via mem_search query. — Mitigation: double-quote-escape query terms and wrap in quoted phrase syntax, as in the reference implementation.
- **Risk 5**: Path traversal via smart_read/boilerplate_write — including the `boilerplate_write` *reference* path, whose contents are sent to the worker endpoint (arbitrary-file exfiltration if unconfined). — Mitigation: canonicalization + allowed-roots confinement on every agent-supplied path (read target, write target, write reference), with dedicated security tests (see Design Decisions).
- **Risk 6**: E2E measurement variance — frontier models are non-deterministic, so baseline vs mmhmem token counts vary run to run. — Mitigation: fixed fixture tasks, threshold-based assertions (not exact equality), temperature-0 worker, generous CI retry budget, and measured numbers persisted per run for trend analysis rather than a single pass/fail gate.

## References
- [modelcontextprotocol/rust-sdk (rmcp)](https://github.com/modelcontextprotocol/rust-sdk) — official Rust MCP SDK, v3.x, MCP 2026-07-28 spec
- [crates.io: rmcp](https://crates.io/crates/rmcp) — v3.0.1 range, Streamable HTTP server transport
- [crates.io: rusqlite](https://crates.io/crates/rusqlite) — `bundled` feature compiles SQLite amalgamation incl. FTS5
- [axum (tokio project)](https://github.com/tokio-rs/axum) — axum 0.8.x, tokio 1.x
- Internal research doc `Self-Hosted Cross-Agent Memory Solutions.md` (repo root) — reference architecture, schema, shunt policy, audit patterns (claude-mem v15.5.0 redaction rules, Spotify shunt benchmarks)
