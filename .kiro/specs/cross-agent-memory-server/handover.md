# Session Handover — cross-agent-memory-server (mmhmem)

Written 2026-11-01 when the session was handed to a fresh context. Everything is
committed and pushed; the working tree is clean.

## Where things are

- Repo: `D:\Source\mmhmem` — git branch `main`, remote `origin` =
  `https://github.com/norandom/mmhmem.git` (all work pushed).
- Spec: `.kiro/specs/cross-agent-memory-server/` — `spec.json`
  (phase `implementation`; requirements/design/tasks all approved),
  `requirements.md` (Req 1–13), `design.md`, `tasks.md` (checkboxes are the
  source of truth for progress; `## Implementation Notes` at the bottom has
  hard-won gotchas), `breath.toml` (scope guard), `research.md`, `handover.md`.
- Steering: `.kiro/steering/{product,tech,structure}.md`.

## Status

Tasks **1.1–1.3, 2.1–2.2, 3.1, 4.1, 5.1–5.2, 6.1, 7.1, 8.1, 9.1, 10.1–10.4, 11.1–11.6**
are done, committed, pushed, and marked `[x]` in `tasks.md`.
**Remaining: 12.1–12.3 (deploy artifacts: systemd+logrotate, SKILL.md+examples,
Dockerfile+compose+.env.example), 13.1 (E2E measurement suite, E2E_LLM=1),
14.1 (CI).** After those: `/kiro-validate-impl cross-agent-memory-server`
then `kiro-verify-completion` for the FEATURE_GO claim.

Validation is green right now: **93/93 tests pass** via nextest in ~5 s;
`cargo fmt --check` clean; `cargo clippy --all-targets -- -D warnings` clean
(cognitive-complexity ≤ 15 gate active via `clippy.toml` + `[lints.clippy]`).

## Environment (Windows, mind the quirks)

- Host: `x86_64-pc-windows-msvc` (MSVC confirmed by user), rustc/cargo 1.98.1,
  `cc` works (rusqlite `bundled` compiles). rust-analyzer 1.98.1 component is
  pinned in `rust-toolchain.toml` (`components = [...]`) — if the editor says it
  is missing, make the extension use the rustup proxy.
- PowerShell 5.1: use `;` as separator (never `&&`), never `2>/dev/null`
  (maps to `D:\dev\null`), env vars via `$env:NAME=...`, `Get-Process`/`Stop-Process`.
- **Compile times are long — never `cargo clean`.** Build once, then iterate
  with nextest. Wrap long runs in the timeout wrapper:
  `powershell -NoProfile -File scripts\with-timeout.ps1 -Seconds 900 -Command cargo -ArgString "nextest run"`
- **Kill stale `mmhmem` processes before building** (`Get-Process mmhmem |
  Stop-Process -Force`) — a leaked daemon from an interrupted run holds
  `target\debug\mmhmem.exe` and blocks all builds. `scripts/lint.ps1` does this
  automatically now; `tests/daemon_smoke.rs` has a KillOnDrop guard.
- Test runner: **cargo-nextest 0.9.150** in `~/.cargo/bin` (config
  `.config/nextest.toml`). Canonical commands: `cargo nextest run`,
  `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`,
  gate = `powershell -NoProfile -File scripts\lint.ps1`.
- Tests use ephemeral ports and raw tokio TCP stubs — no extra dev-deps.

## How work is done here (user-mandated)

- **Foreground only, no subagents.** I implement directly, apply kiro-review
  inline, verify with fresh command output, then **one conventional commit per
  task** (`feat(cross-agent-memory-server): ...`) + `git push -q origin main`.
- Req 13 caps are enforced by the gate: cognitive ≤ 15/fn (clippy, blocking),
  ≤ 500 LOC per `src/` file (split by responsibility when hit — that is why
  shunt is `shunt.rs` + `shunt_cache.rs` + `shunt_types.rs`), ≤ 400 non-test LOC
  per task, test files ≤ 800 LOC. `cargo fmt` runs after every write — **the
  edit tool's oldString must match post-fmt text; re-read the file after fmt
  before editing.**
- rmcp 3.8 gotchas (all hit and solved): `#[tool_router]` + `#[tool_handler]`
  must live in the SAME module; `Parameters` is at
  `rmcp::handler::server::wrapper::Parameters`; results use `ContentBlock::text`;
  `get_info` returns `InitializeResult` (set `server_info.name/version`);
  `LocalSessionManager::default()`; server = axum 0.8.9 `nest_service` of
  `StreamableHttpService::new(factory, session_manager, config)`; bearer auth is
  an axum middleware with `subtle` ct_eq, `/healthz` exempt; tool errors surface
  as JSON-RPC `-32603` with the Display message; responses are SSE (`data:` lines).
- opentelemetry 0.33: needs `experimental_async_runtime` + `rt-tokio` on the SDK;
  use `with_simple_exporter` for spans (the batch processor panics under
  rt-tokio); feature-gated behind `otel` — default `cargo tree` has zero
  opentelemetry crates (Req 11.2 verified).
- Windows paths in hand-built JSON break (`\` escapes) — build test args with
  `serde_json::json!`.

## Architecture recap (what exists)

`src/`: `config.rs` (env-only, fail-fast), `storage/{schema,db,audit}.rs`
(WAL SQLite via spawn_blocking-only mutex, redact+hash O_APPEND JSONL),
`services/{worker,session_bus,shunt,shunt_cache,shunt_types,boilerplate,memory,stats,paths}.rs`,
`telemetry.rs` (otel feature), `mcp/{server,params,tools}.rs`, `main.rs`.
Six MCP tools over Streamable HTTP + bearer auth; audit + breadcrumbs + token_stats;
worker client (OpenAI-compatible, temp 0); shunt with SHA-256 cache + security
bypass + allowed-roots confinement (both boilerplate paths confined BEFORE any
worker call); OTel optional to self-hosted OpenObserve.

## Next steps for the new session

1. `git pull` (should be up to date), then read `tasks.md` checkboxes +
   `## Implementation Notes`.
2. Sanity: `powershell scripts\with-timeout.ps1 -Seconds 900 -Command cargo -ArgString "nextest run"` → expect 93/93.
3. Implement 12.1 (deploy/systemd unit + logrotate.d config), 12.2
   (deploy/skills SKILL.md <200 tokens + deploy/examples harness configs +
   agent-side OTLP doc), 12.3 (Dockerfile multi-stage dedicated binary +
   docker-compose.yml + .env.example; host-side logrotate on the bind-mounted
   log volume; healthcheck via /healthz), 13.1 (tests/e2e gated by E2E_LLM=1:
   compose stack + scripted OpenCode sessions on a fixture repo, baseline vs
   mmhmem, assert measured input-token reduction threshold, persist measured
   numbers as client_reported), 14.1 (CI: fmt, clippy gate, nextest, tokei
   informational, release build, docker build --features otel, optional E2E job).
4. Commit+push per task; then `/kiro-validate-impl cross-agent-memory-server`
   and `kiro-verify-completion` (FEATURE_GO needs full suite + smoke + coverage).
