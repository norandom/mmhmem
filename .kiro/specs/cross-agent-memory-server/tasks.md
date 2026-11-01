# Implementation Plan: cross-agent-memory-server (mmhmem)

Scope guard: `breath.toml` in this directory is the feature map — only listed features are implemented; anything in `[features.out]` stays out.

Every implementation task carries the Req 13 complexity caps in its acceptance criteria (13.6): non-test diff ≤ 400 lines (13.4); cyclomatic complexity ≤ 10 and cognitive complexity ≤ 15 per function (13.1–13.2, enforced via `clippy.toml` + `-D warnings`); ≤ 500 LOC per `src/` file and ≤ 80 LOC per function (13.3). Test files are capped at 800 LOC.

- [ ] 1. Foundation: build scaffolding, lint gate, and env-only configuration
- [x] 1.1 Create the Rust crate manifest and pinned toolchain
  - Pin all runtime dependencies (rmcp with Streamable HTTP server transport, rusqlite `bundled`, tokio, reqwest, serde/serde_json, sha2, subtle, regex, time, anyhow, tracing; dev-only tempfile) with an optional `otel` cargo feature held off by default; pin the stable toolchain; commit `Cargo.lock`; provide a minimal entrypoint
  - Observable: `cargo build --release` succeeds and the binary runs; the plain build contains no OTel code path
  - Complexity caps (Req 13): non-test diff ≤ 400 lines (13.4); CC ≤ 10 / cognitive ≤ 15 per function (13.1–13.2); ≤ 500 LOC per src file, ≤ 80 LOC per function (13.3)
  - _Requirements: 8.1, 11.2, 13.1, 13.2, 13.3, 13.4, 13.5_
- [x] 1.2 Add the blocking lint/metrics gate
  - `clippy.toml` with `cyclomatic-complexity-threshold = 10` and `cognitive-complexity-threshold = 15`; CI step running `cargo clippy --all-targets -- -D warnings`; informational `tokei` per-file LOC report (dev tool only, never a runtime dependency)
  - Observable: a deliberately branchy function fails clippy with the configured thresholds; the skeleton passes
  - _Requirements: 13.1, 13.2, 13.5, 13.6_
- [x] 1.3 Implement env-only configuration with fail-fast validation
  - All env vars with documented defaults: data dir, audit log file, allowed roots, bearer token, MCP port/path, shunt threshold (default 150 lines), worker URL/model/API key, `OTEL_EXPORTER_OTLP_ENDPOINT` / `OTEL_EXPORTER_OTLP_HEADERS` / `OTEL_SERVICE_NAME`; startup refuses to run without the bearer token and allowed roots; unit tests for defaults and validation errors
  - Observable: the binary exits non-zero with a clear message when the bearer token is unset; the startup log prints the effective config with the token redacted
  - Complexity caps (Req 13): ≤ 400 non-test LOC (13.4); CC ≤ 10 / cognitive ≤ 15 (13.1–13.2); ≤ 500 LOC per file (13.3)
  - _Requirements: 3.9, 6.1, 7.5, 8.1, 11.1, 13.3_

- [ ] 2. Foundation: durable storage (schema, WAL connection, audit writer)
- [x] 2.1 Implement the SQLite schema and concurrency-safe connection
  - All tables: active_sessions, session_breadcrumbs + index, observations + FTS5 external-content table + AFTER INSERT trigger, shunt_file_cache, token_stats + index; WAL + synchronous NORMAL; idempotent startup migrations via `PRAGMA user_version`; single connection behind a mutex serviced via `spawn_blocking`; the lock is never held across an `.await`
  - Observable: a fresh data dir initializes all tables; the `SELECT fts5(?)` smoke query works; an inserted observation is FTS-searchable immediately
  - Complexity caps (Req 13): ≤ 400 non-test LOC (13.4); CC ≤ 10 / cognitive ≤ 15 (13.1–13.2); ≤ 500 LOC per file, ≤ 80 LOC per function (13.3)
  - _Requirements: 2.1, 2.2, 5.5, 9.1, 9.3, 10.3, 13.3_
- [x] 2.2 Implement the redacting, hash-verifying, append-only audit writer
  - Redaction of key-assigned secrets (`api_key`, `secret`, `token`, `password`, `MYSQL_PWD`, `dbPassword`, `authorization`), `Authorization: Bearer` headers, Slack `xapp-`/`xoxe-` tokens, and `<private>...</private>` spans; single-line JSON records (ISO-8601 UTC `ts`, `epoch_ms`, `event_type`, `agent_harness`, `project`, redacted `details`, `record_sha256` over sorted keys); O_APPEND open-per-write; env-configurable path; all event types incl. TOKEN_STATS, TOKEN_STATS_CLIENT, WORKER_ERROR, and auth failures
  - Unit tests: every secret pattern class is absent from the raw log bytes; every line's digest verifies
  - Observable: a record containing a fake `api_key` lands on disk as `[REDACTED…]` with a verifiable `record_sha256`; the log can be rotated mid-run without restarting the daemon
  - Complexity caps (Req 13): ≤ 400 non-test LOC (13.4); CC ≤ 10 / cognitive ≤ 15 (13.1–13.2); ≤ 500 LOC per file, ≤ 80 LOC per function (13.3)
  - _Requirements: 7.1, 7.2, 7.3, 7.4, 7.5, 3.3, 6.3, 6.4, 10.1, 13.3_

- [ ] 3. Core: worker model client
- [x] 3.1 Build the typed OpenAI-compatible worker client
  - Configurable URL/model/API key, temperature 0, 30 s read / 45 s write timeouts, explicit errors (never stale-or-empty-as-fresh), WORKER_ERROR audit event plus worker model name and round-trip latency recorded, returns the worker `usage` prompt/completion token counts; lazy — never touched on cache-hit, direct-read, or session paths
  - Tests against a local stub endpoint: success returns summary + usage; dead endpoint returns a clear error
  - Observable: with no worker configured, all non-shunt paths work; with a stub, a delegated call returns content plus usage counts
  - _Boundary: WorkerClient_
  - _Depends: 1.3, 2.2_
  - Complexity caps (Req 13): ≤ 400 non-test LOC (13.4); CC ≤ 10 / cognitive ≤ 15 (13.1–13.2); ≤ 500 LOC per file (13.3)
  - _Requirements: 6.1, 6.2, 6.3, 6.4, 3.5, 4.1, 9.4, 10.2, 13.3_

- [ ] 4. Core: live cross-agent session bus
- [x] 4.1 (P) Implement session resume, checkpoint, and automatic breadcrumbs
  - Resume returns working state + last active harness + objective + the 15 most recent breadcrumbs; checkpoint upserts by project/task; a new task ID is initialized when no session exists; breadcrumb API for every tool call; working state capped at 6,000 chars (1,500 tokens at ~4 chars/token) truncated at the last complete line; handoff performs zero LLM calls; the checkpoint accepts the optional client-reported frontier usage payload and returns it in the ack — SessionBus does NOT call StatsCollector and does NOT write token_stats; forwarding into StatsCollector (8.1) happens only in the 10.2 wiring
  - Unit tests for each behavior incl. the truncation boundary
  - Observable: checkpoint from harness A, then resume from harness B returns A's state and breadcrumbs with no worker call; a > 6,000-char state is truncated at a line boundary
  - _Boundary: SessionBus_
  - _Depends: 2.1, 2.2_
  - Complexity caps (Req 13): ≤ 400 non-test LOC (13.4); CC ≤ 10 / cognitive ≤ 15 (13.1–13.2); ≤ 500 LOC per file (13.3)
  - _Requirements: 2.1, 2.2, 2.3, 2.4, 2.5, 2.6, 10.4, 13.3_

- [ ] 5. Core: deterministic read shunting (smart_read)
- [ ] 5.1 (P) Implement the four-stage read evaluation with path confinement
  - Targeted offset/limit slice with exact `L<n>:` numbering (never delegated); below-threshold direct read; security-critical path regex bypass (case-insensitive) with the bypass reason audited; canonicalize + allowed-roots confinement (reject + audit); missing-file error; unit tests per stage plus traversal attempts
  - Observable: an offset/limit read returns exactly the requested lines; an auth-path file returns full content with `bypass_reason` in the audit; a path outside the allowed roots is rejected and audited
  - _Boundary: ShuntEngine_
  - _Depends: 1.3, 2.1, 2.2, 3.1_
  - Complexity caps (Req 13): ≤ 400 non-test LOC (13.4); CC ≤ 10 / cognitive ≤ 15 (13.1–13.2); ≤ 500 LOC per file (13.3)
  - _Requirements: 3.1, 3.2, 3.3, 3.8, 3.9, 13.3_
- [ ] 5.2 Implement the content-addressed shunt cache and worker delegation
  - SHA-256 of the file + 16-char normalized-question fingerprint; cache key (path, hash, fingerprint); hit → cached summary < 50 ms with zero worker calls; miss → line-numbered file + question to the worker with the strict bullet prompt (every bullet begins with an exact symbol name and `[L<start>-L<end>]` range) → store; content change invalidates; the avoided-token estimate is forwarded to StatsCollector (persisted in 8.1, wired in 10.3)
  - Integration tests with a stub worker: repeat read = identical summary with worker call count 0; after editing the file, cache miss
  - Observable: a repeat read of an unchanged 600-line file returns the identical cached summary in < 50 ms with zero worker invocations; editing one line forces re-delegation
  - _Boundary: ShuntEngine_
  - _Depends: 2.1, 2.2, 3.1, 5.1_
  - Complexity caps (Req 13): ≤ 400 non-test LOC (13.4); CC ≤ 10 / cognitive ≤ 15 (13.1–13.2); ≤ 500 LOC per file (13.3)
  - _Requirements: 3.4, 3.5, 3.6, 3.7, 9.2, 10.1, 13.3_

- [ ] 6. Core: boilerplate write shunting
- [ ] 6.1 (P) Implement boilerplate generation written straight to disk
  - Reference + spec sent to the worker ("follow the reference file's conventions, output only raw source code"); strip markdown fences; create parent dirs; write to the target; one-line ack with output SHA-256, line count, worker model, latency; target AND reference canonicalized + confined to allowed roots (reference outside roots → reject + audit with zero bytes sent to the worker); missing reference → error, no write; audit + breadcrumb on success; avoided input/output estimate forwarded to StatsCollector (8.1/10.3)
  - Unit/integration tests incl. fence stripping and reference-outside-roots rejection
  - Observable: a generated stub lands at the confined target path with a one-line checksum ack; a traversal reference path is rejected with nothing sent to the worker
  - _Boundary: BoilerplateWriter_
  - _Depends: 1.3, 2.1, 2.2, 3.1_
  - Complexity caps (Req 13): ≤ 400 non-test LOC (13.4); CC ≤ 10 / cognitive ≤ 15 (13.1–13.2); ≤ 500 LOC per file (13.3)
  - _Requirements: 4.1, 4.2, 4.3, 4.4, 10.1, 13.3_

- [ ] 7. Core: progressive-disclosure long-term memory
- [ ] 7.1 (P) Implement memory search, batch detail fetch, and recording
  - `mem_search`: double-quote-escaped quoted-phrase FTS5 query, compact index rows (ID, type, harness, title, files touched), type filter, clear no-match message, < 15 ms; `mem_get_observations`: full narrative/facts/files for validated IDs only; `mem_record_observation`: durable insert immediately searchable via the trigger, redacted before persist; FTS-injection unit test
  - Observable: record → immediate search hit; search returns index rows only; get-by-ID returns the full narrative; quote injection does not break the query
  - _Boundary: MemoryService_
  - _Depends: 2.1, 2.2_
  - Complexity caps (Req 13): ≤ 400 non-test LOC (13.4); CC ≤ 10 / cognitive ≤ 15 (13.1–13.2); ≤ 500 LOC per file (13.3)
  - _Requirements: 5.1, 5.2, 5.3, 5.4, 5.5, 5.6, 7.1, 13.3_

- [ ] 8. Core: token-savings statistics
- [ ] 8.1 (P) Implement the stats collector
  - Per-event estimates (delegated read: (raw chars − returned chars) ÷ 4; cache hit: raw chars ÷ 4; code write: (reference + generated chars) ÷ 4); captures worker usage; writes `token_stats` rows keyed (project, task, harness, ts, kind) plus TOKEN_STATS / TOKEN_STATS_CLIENT audit events; persists `client_reported` rows forwarded from the session bus; estimates labelled as estimates, client values labelled by source, nothing invented; OTel counters go through an injected sink that defaults to a no-op and is wired to the real Telemetry (9.1) in 10.4 — no direct dependency on 9.1 here
  - Unit tests for the arithmetic per kind and the labelling
  - Observable: one delegated read produces a `shunt_delegated` row with `avoided_input_tokens_est > 0` and worker token counts; a checkpoint carrying client usage produces a `client_reported` row
  - _Boundary: StatsCollector_
  - _Depends: 2.1, 2.2, 3.1_
  - Complexity caps (Req 13): ≤ 400 non-test LOC (13.4); CC ≤ 10 / cognitive ≤ 15 (13.1–13.2); ≤ 500 LOC per file (13.3)
  - _Requirements: 10.1, 10.2, 10.3, 10.4, 10.5, 11.3, 13.3_

- [ ] 9. Core: optional OTel telemetry (cargo feature `otel`)
- [ ] 9.1 (P) Implement the feature-gated OTLP exporter
  - Inert unless `OTEL_EXPORTER_OTLP_ENDPOINT` is set; spans around each MCP tool call (tool, harness, project, outcome, latency); metrics (tool latency histogram, shunt cache hit rate, tokens-avoided counters, worker token counters); redacted audit log records exported; every payload passes the audit redaction filter; no other egress; the Docker image builds with `--features otel`, the plain release does not
  - Tests: with the endpoint unset, a local listener confirms zero OTLP traffic; with a local OTLP stub, one tool call produces a redacted span
  - Observable: the no-feature release binary makes no OTLP calls; the Docker build with the feature exports spans to the stub
  - _Boundary: Telemetry_
  - _Depends: 1.1, 1.3, 2.2_
  - Complexity caps (Req 13): ≤ 400 non-test LOC (13.4); CC ≤ 10 / cognitive ≤ 15 (13.1–13.2); ≤ 500 LOC per file (13.3)
  - _Requirements: 11.1, 11.2, 11.3, 8.5, 13.3_

- [ ] 10. Integration: MCP server, bearer auth, six tool handlers, entrypoint
- [ ] 10.1 Implement the Streamable HTTP MCP server with bearer auth
  - Configurable path (default `/mcp`) and port (default 8443); every request requires `Authorization: Bearer` compared in constant time; failures → HTTP 401 before tool dispatch and audited; unauthenticated `GET /healthz` → 200 for container healthchecks; graceful shutdown on SIGTERM; HTTP integration tests (no/wrong token → 401; valid token → MCP initialize succeeds)
  - Observable: `curl` without a token returns 401; with a token the MCP handshake completes; `/healthz` returns 200 unauthenticated
  - _Depends: 1.3, 2.2_
  - Complexity caps (Req 13): ≤ 400 non-test LOC (13.4); CC ≤ 10 / cognitive ≤ 15 (13.1–13.2); ≤ 500 LOC per file (13.3)
  - _Requirements: 1.1, 1.2, 1.3, 8.1, 13.3_
- [ ] 10.2 Register the session and memory tools with thin handlers
  - Thin handlers for `session_resume_or_handoff`, `mem_search`, `mem_get_observations`, `mem_record_observation`: parse args, call services, format markdown, map errors to actionable MCP tool errors; every call appends a breadcrumb and an audit event; forwards the optional `client_usage` payload from the checkpoint into StatsCollector (the only place this wiring happens); no client hooks or proxies anywhere
  - Observable: over HTTP, harness A checkpoints and harness B resumes and sees the state + breadcrumbs; each call appears as a breadcrumb and an audit line
  - _Depends: 4.1, 7.1, 8.1_
  - Complexity caps (Req 13): ≤ 400 non-test LOC (13.4); CC ≤ 10 / cognitive ≤ 15 (13.1–13.2); ≤ 500 LOC per file (13.3)
  - _Requirements: 1.4, 1.5, 2.4, 10.4, 13.3_
- [ ] 10.3 Register the I/O tools (smart_read, boilerplate_write) with thin handlers
  - Thin handlers for `smart_read` and `boilerplate_write` with the same breadcrumb/audit wrapper; wires the shunt and boilerplate avoided-token estimates into StatsCollector; error mapping per 3.8 / 4.3 / 6.3
  - Observable: over HTTP, `smart_read` returns the four outcomes and each produces the correct audit event type; `boilerplate_write` returns the one-line ack
  - _Depends: 3.1, 5.2, 6.1, 8.1_
  - Complexity caps (Req 13): ≤ 400 non-test LOC (13.4); CC ≤ 10 / cognitive ≤ 15 (13.1–13.2); ≤ 500 LOC per file (13.3)
  - _Requirements: 1.4, 3.8, 4.3, 6.3, 13.3_
- [ ] 10.4 Wire the daemon entrypoint end to end
  - Config → storage init → services → tool registration → server run; starts with only token + allowed roots set, no worker needed; wires the StatsCollector OTel sink to the real Telemetry when the feature is enabled; graceful SIGTERM shutdown
  - Observable: `cargo run` boots, `/healthz` returns 200, authenticated MCP works end to end, audit lines are written, clean shutdown on SIGTERM
  - _Depends: 10.1, 10.2, 10.3_
  - Complexity caps (Req 13): ≤ 400 non-test LOC (13.4); CC ≤ 10 / cognitive ≤ 15 (13.1–13.2); ≤ 500 LOC per file (13.3)
  - _Requirements: 8.1, 9.3, 9.4, 13.3_

- [ ] 11. Validation: deterministic integration and security tests (worker stubbed, no model access)
- [ ] 11.1 Shared ephemeral-port test harness + session handoff E2E tests
  - Common harness (ephemeral port, temp data dir, test token, stub worker); E2E: resume → work → checkpoint → resume in a second harness; automatic breadcrumb append; 6,000-char truncation; zero-LLM handoff
  - Observable: the full handoff flow is green over real HTTP
  - _Depends: 10.4_
  - Test file ≤ 800 LOC (13.3)
  - _Requirements: 2.1, 2.2, 2.3, 2.4, 2.5, 2.6_
- [ ] 11.2 smart_read four-stage, cache, and invalidation tests
  - Targeted slice, below-threshold direct, security bypass with audited reason, cache hit (identical summary, zero worker calls, < 50 ms), content-change invalidation, missing-file error, threshold env default 150
  - Observable: all stages green against the stub worker; the cache-hit test asserts worker call count 0 and latency < 50 ms
  - _Depends: 10.4_
  - Test file ≤ 800 LOC (13.3)
  - _Requirements: 3.1, 3.2, 3.3, 3.4, 3.5, 3.6, 3.7, 3.8, 3.9, 9.2_
- [ ] 11.3 boilerplate_write round-trip tests
  - Worker stub returns fenced code; assert fence stripping, file on disk with matching SHA-256, one-line ack, audit + breadcrumb; missing reference → error with no write
  - Observable: round-trip green; the missing-reference test writes nothing
  - _Depends: 10.4_
  - Test file ≤ 800 LOC (13.3)
  - _Requirements: 4.1, 4.2, 4.3, 4.4_
- [ ] 11.4 memory search/get/record tests
  - Record → immediately searchable; compact index rows only; type filter; no-match message; batch get by validated IDs; FTS quote-injection attempt safe; index search < 15 ms
  - Observable: the mem cycle is green; the search-latency assert passes
  - _Depends: 10.4_
  - Test file ≤ 800 LOC (13.3)
  - _Requirements: 5.1, 5.2, 5.3, 5.4, 5.5, 5.6_
- [ ] 11.5 audit redaction and record-integrity tests
  - Every redaction pattern class never appears in the raw log bytes; `<private>` spans scrubbed; every audit line parses as JSON and its `record_sha256` verifies
  - Observable: the audit suite is green; a planted secret is provably absent from the log file bytes
  - _Depends: 10.4_
  - Test file ≤ 800 LOC (13.3)
  - _Requirements: 7.1, 7.2, 7.3_
- [ ] 11.6 security, concurrency, and performance tests
  - Path traversal rejected + audited; boilerplate reference outside allowed roots rejected with zero bytes to the worker; wrong bearer token rejected; concurrent harness sessions with no state corruption; cached read/resume < 50 ms; worker endpoint down → explicit error + WORKER_ERROR audit
  - Observable: all green; the concurrency test runs N parallel sessions and asserts final state consistency
  - _Depends: 10.4_
  - Test file ≤ 800 LOC (13.3)
  - _Requirements: 1.2, 6.3, 9.1, 9.2, 13.5_

- [ ] 12. Validation: deployment configuration artifacts
- [ ] 12.1 Ship the systemd unit and logrotate configuration
  - Unit with env-file reference + restart policy; logrotate daily, 90-day retention, compress, dateext, 0640, optional postrotate batch ship to a self-hosted OpenObserve or S3-compatible store
  - Observable: `systemd-analyze verify` passes on the unit; `logrotate --debug` parses the config without error
  - _Depends: 10.4_
  - _Requirements: 8.1, 8.2, 7.4, 8.5_
- [ ] 12.2 Ship the SKILL.md playbook and example harness connection configs
  - Playbook under 200 tokens: resume first; prefer `smart_read` over raw reads of large files (offset/limit for edits); search the memory index before fetching details; checkpoint before switching harnesses; example configs for Claude Code CLI, Antigravity, OpenCode, Claude Desktop; a snippet documenting agent-side OTLP export to OpenObserve configured in the harness itself; verify the design's data-lifecycle table still matches implemented behavior
  - Observable: playbook ≤ 200 tokens by the documented estimate; each example config is syntactically valid
  - _Depends: 10.4_
  - _Requirements: 8.3, 8.4, 1.3, 11.4, 11.5_
- [ ] 12.3 Ship the Docker image and docker-compose setup
  - Multi-stage Dockerfile (builder compiles with `--features otel`; runtime stage = minimal non-root image containing only the `mmhmem` binary); compose with env_file, MCP port, volumes for the data dir, the audit log dir (host-side logrotate rotates the bind mount, no container restart), and allowed roots read-only; healthcheck via `/healthz`; restart policy; `.env.example` documenting every variable
  - Observable: `docker compose config` validates; the built image's payload is only the binary; the container becomes healthy and audit lines land on the host-mounted log volume
  - _Depends: 10.4, 9.1_
  - _Requirements: 8.6, 8.7, 11.2, 13.3_

- [ ] 13. Validation: end-to-end measurement suite with a real harness
- [ ] 13.1 Build the E2E measurement suite (gated by `E2E_LLM=1`)
  - Fixture repo + fixed scripted OpenCode task set (read-heavy, boilerplate-heavy); compose stack up; baseline mode (mmhmem disconnected) vs mmhmem mode (MCP + SKILL.md connected); collect harness-reported per-session prompt/completion tokens; compute per-task and aggregate deltas; assert the configured input-token reduction threshold on read-heavy tasks; persist the measured numbers as `client_reported` into `token_stats` and the audit log; emit a human-readable report
  - Observable: with `E2E_LLM` unset, `cargo test` skips the suite; with `E2E_LLM=1` and the stack up, the suite prints measured baseline-vs-mmhmem deltas and passes the threshold assertion
  - _Depends: 10.4, 12.3_
  - Test file ≤ 800 LOC (13.3); non-test diff ≤ 400 lines (13.4)
  - _Requirements: 12.1, 12.2, 12.3, 10.4, 13.4_

- [ ] 14. Validation: CI pipeline with blocking quality gates
- [ ] 14.1 Add CI: rustfmt check, blocking `cargo clippy --all-targets -- -D warnings` (Req 13 caps), deterministic `cargo test`, informational `tokei` LOC report, release build, Docker build with the `otel` feature, and an optional `E2E_LLM=1` job via compose when model credentials are available
  - Observable: CI config present and all locally runnable steps pass; the clippy gate demonstrably blocks a CC > 10 function
  - _Depends: 11.6, 12.3, 13.1_
  - _Requirements: 13.1, 13.2, 13.5, 13.6, 12.3, 11.2_

## Implementation Notes

- Host toolchain is `x86_64-pc-windows-msvc` (user confirmed MSVC preferred, 2026-11-01); `cc`-based C builds (rusqlite `bundled`) verified working. Do not switch to gnu/mingw.
- `.gitignore` must stay UTF-8 — a UTF-16LE copy written via PowerShell once broke git's parsing of it (fixed 2026-11-01); keep `/target`, `/.env`, `*.env`.
- rmcp 3.x's real cargo feature name is `transport-streamable-http-server` (design text abbreviates it as "streamable-http-server").
- `Cargo.lock` is a generated lockfile and is excluded from the Req 13.4 non-test LOC cap; hand-written diffs are what count.
- clippy 0.1.98 / rustc 1.98.1: the `cyclomatic_complexity` lint is a deprecated alias of `cognitive_complexity` (two-key clippy.toml aborts with "duplicate field"). Enforce the McCabe-style cap via `cognitive-complexity-threshold = 15` in clippy.toml plus `[lints.clippy] cognitive-complexity = "warn"` in Cargo.toml so `-D warnings` blocks. Req 13.1 amended accordingly (2026-11-01).
- `tokei` is not installed on this host; scripts/lint.ps1 prints a skip note and CI (task 14.1) should install it for the informational LOC report. Never a runtime dependency (Req 13.5).
