# Research & Design Decisions

## Summary
- **Feature**: `local-mode`
- **Discovery Scope**: Extension (light, integration-focused) of the validated `cross-agent-memory-server` feature; all discovery run in main context (user mandate: foreground only, no subagents).
- **Key Findings**:
  - rmcp 3.8.0 ships the stdio server transport behind the `transport-io` feature (`transport-io = ["transport-async-rw", "tokio/io-std", "tokio/net"]`); `stdio_pipes()` is the recommended entry (non-blocking Linux pipes, blocking fallback on Windows/macOS, usable unconditionally).
  - The existing `Db` (`Arc<Mutex<Connection>>` + `with_conn` via `spawn_blocking`) can become multi-process-safe without changing its public API: add SQLite `busy_timeout` at open; WAL is already on. This keeps the 97-test suite unmodified.
  - `Config` field types and `from_env` semantics are pinned by the inline config tests (part of the 97): the profile split must be additive (new `from_env_local` + `Profile` enum), and `token` stays `String` (empty in local mode).
  - Help-text/CLI tests assert with `contains`, so adding `serve`/`stdio`/`stats` subcommands is purely additive.
  - MSVC binaries cannot be cross-compiled from the Linux CI containers; GitHub Actions native runners are the standard route for Windows/macOS release builds.

## Research Log

### rmcp 3.8.0 stdio transport API
- **Context**: Req 2.1 — serve MCP over stdio with the same tool handlers.
- **Sources Consulted**: vendored crate source `rmcp-3.8.0/src/transport/io.rs`, `Cargo.toml` feature map; repo `Cargo.toml` (rmcp `=3.8.0`, only `transport-streamable-http-server` enabled today).
- **Findings**:
  - `rmcp::transport::stdio()` returns `(tokio::io::Stdin, tokio::io::Stdout)`; `stdio_pipes()` returns a platform-optimized pair — on Linux it reopens the pipe FDs non-blocking via `/proc/self/fd` (keeps the process's own stdio blocking), on Windows/macOS it falls back to the blocking handles; "can be used unconditionally".
  - `stdio_pipes` panics on Linux when called outside a Tokio runtime with IO enabled — the stdio entry must construct the transport inside `block_on`.
  - Serving completes when stdin reaches EOF — the shutdown contract of Req 2.3 falls out of the transport, not custom signal handling.
  - rmcp's per-line JSON-RPC decode behavior on malformed input is not pinned by the docs read; treated as a validation hook (test Req 2.4 with garbage-then-valid input) with a fallback plan (a filtering reader wrapper around stdin) if rmcp terminates the stream.
- **Implications**: one additive cargo feature (`transport-io`), no new crate; the same `McpServer` (ServerHandler in `mcp/tools.rs`) serves both transports; diagnostics MUST be routed to stderr because `tracing_subscriber::fmt()` defaults to stdout and would corrupt the MCP stream.

### rusqlite / SQLite multi-process safety
- **Context**: Req 6.1–6.3, Req 5 — concurrent processes on one per-user store.
- **Sources Consulted**: `src/storage/db.rs`, `src/storage/schema.rs` (WAL + `synchronous=NORMAL` + idempotent DDL + `user_version` guard); rusqlite `Connection::busy_timeout` / `PRAGMA busy_timeout`.
- **Findings**:
  - WAL permits N readers + 1 writer across processes; a second writer receives `SQLITE_BUSY`, which `busy_timeout` converts into a bounded wait. Short locked sections (the existing `with_conn` pattern) keep contention tiny.
  - `Db::open_at` currently sets `journal_mode=WAL`, `synchronous=NORMAL`, applies schema. Adding `busy_timeout` there (default 5,000 ms, config override) changes no signature; `Db::open`/`open_at`/`open_for_config`/`with_conn` keep their exact shapes.
  - Concurrent first-open of a fresh store by two processes is safe: DDL is `IF NOT EXISTS` and the `user_version` guard makes re-application a no-op; the DDL batch serializes on the database lock like any writer.
  - `synchronous=NORMAL` under WAL is safe for multi-process access (durability contract already documented in the upstream spec for single-process; checkpointing stays automatic).
- **Implications**: the superseded invariant ("one connection behind an async mutex; lock never held across an await") becomes: per-process single connection behind the same mutex (unchanged within a process), cross-process safety from WAL + busy_timeout; no pool crate, no lock files, no coordination service.

### Config test-compatibility constraints (Req 9.1: suite passes unmodified)
- **Context**: the profile split must not break the 97.
- **Sources Consulted**: `src/config.rs` inline tests (`defaults_applied_when_only_required_vars_set` asserts `cfg.token: String`, field defaults), `tests/cli.rs`, `src/lib.rs` tests.
- **Findings**:
  - `Config.token` must remain `String` (local mode uses `""`); `allowed_roots: Vec<PathBuf>` remains (local: empty = unconfined); new fields (`busy_timeout_ms`, audit rotation knobs, `profile`) are additive with defaults, so `from_env` results change only by the new fields.
  - Help and CLI tests assert with `contains` — new subcommands and help lines are additive.
  - `tests/cli.rs` spawns the real binary (`CARGO_BIN_EXE_mmhmem`) — the same pattern extends to stdio/stats child-process tests.
- **Implications**: `Config::from_env` keeps serve semantics; a new `from_env_local` shares the parsing helpers with local defaults/validation. The main.rs dispatch gains `serve` (equivalent to no-arg), `stdio`, `stats`.

### Confinement semantics with empty roots
- **Context**: Req 3.3 — `AGENT_MEM_ALLOWED_ROOTS` optional in local mode, enforced when set.
- **Sources Consulted**: `src/services/paths.rs` (`AllowedRoots::from_config` canonicalizes roots; non-canonicalizable roots are dropped, deny-by-default; an empty root set would deny every path).
- **Findings**: `AllowedRoots` needs an explicit "unconfined" state: an empty configured root set must mean allow-all (local default), while serve mode never sees an empty set (config fail-fast guarantees ≥1 root, so the allow-all branch is unreachable there).
- **Implications**: one small seam in `AllowedRoots` (empty → unconfined); serve behavior provably unchanged (unreachable branch); unit tests cover both modes.

### Per-OS user directories without a new crate
- **Context**: Req 3.1 — `~/.local/share/mmhmem`, `%APPDATA%\mmhmem`, `~/Library/Application Support/mmhmem`.
- **Sources Consulted**: XDG Base Directory spec ( `$XDG_DATA_HOME`, fallback `$HOME/.local/share`), Windows known env (`APPDATA`), macOS convention (`~/Library/Application Support`); steering tech.md "dependency set stays minimal".
- **Findings**: the three resolutions need only `std::env::var` lookups (~30 lines) and are unit-testable with an injected lookup closure (the config module's established pattern). The `dirs` crate solves the same problem but adds a dependency for no functional gain here.
- **Implications**: build a small `local_dirs` resolver (build-vs-adopt: adopt `dirs` REJECTED — minimal dependency set, injectable pure functions suffice; documented per design-synthesis build-vs-adopt).

### Cross-platform release builds (Windows, Linux, macOS)
- **Context**: Req 10.1–10.3.
- **Sources Consulted**: steering dev-environment.md (Dagger is the CI engine; Linux containers; Windows dev host uses `x86_64-pc-windows-msvc`); GitHub Actions runner docs (windows-latest, macos-latest).
- **Findings**:
  - `x86_64-pc-windows-msvc` cannot be cross-compiled from a Linux container (needs MSVC + Windows SDK); the `windows-gnu`/mingw target would cross-compile but produces a different, non-canonical Windows toolchain artifact and was rejected for release consistency with the supported dev host.
  - macOS cross-compilation (osxcross) requires the macOS SDK with licensing friction; native runners are the standard.
  - Linux `clippy` never compiles `cfg(windows)` code — the native Windows job must run clippy + the deterministic suite for OS-gated code to stay under the same blocking standards (Req 10.4).
- **Implications**: the Dagger module keeps ALL container work (fmt, clippy×2, tests, Linux release, Docker); the GH workflow gains native-runner jobs that run clippy + deterministic tests + release builds for Windows and macOS. This is a deliberate, documented exception to "one pipeline, never maintain two scripts": the jobs run on machines the container pipeline cannot provide; the gate definitions (commands, thresholds) stay identical.

### Local audit rotation
- **Context**: Req 7.3 — bounded audit growth on machines without logrotate.
- **Sources Consulted**: `src/storage/audit.rs` (append-per-write via `OpenOptions::append(true).create(true)`, no persistent handle — the exact property that makes host logrotate safe).
- **Findings**: because every write opens the file fresh, renaming the file between writes is trivially safe; a size check + rename cascade (`audit.jsonl` → `.1` → … → `.N`) slots into the existing write path. Cross-process rotation races are benign: a racing writer may append one extra line into a just-renamed file; records are per-line self-verifying (SHA-256 per record), so integrity is per-record and never chain-ordered across files.
- **Implications**: build a small `audit_rotate` module (build-vs-adopt: logrotate everywhere REJECTED — not available on Windows/macOS user machines; a log shipping daemon REJECTED — out of boundary).

### Stats CLI read-only access
- **Context**: Req 8.5, 8.6 — empty store message; read-only guarantee.
- **Sources Consulted**: `src/storage/db.rs` (`open_at` applies schema — a write), rusqlite open flags (`SQLITE_OPEN_READ_ONLY`); `token_stats` DDL in schema.rs (columns `kind`, `ts_epoch_ms`, sums needed).
- **Findings**: `mmhmem stats` must NOT use `Db::open` (it would create the store and stamp schema). A read-only open (`Connection::open_with_flags(...READ_ONLY)`) + tolerant queries (missing tables → empty aggregates) deliver Req 8.5/8.6; date bucketing uses `date(ts_epoch_ms/1000, 'unixepoch')`.
- **Implications**: a `Db::open_read_only` sibling + the stats aggregation live in the CLI stats module; no schema changes anywhere in this spec.

## Architecture Pattern Evaluation

| Option | Description | Strengths | Risks / Limitations | Notes |
|--------|-------------|-----------|---------------------|-------|
| Reuse the service graph for both transports | One `AppState::build` + `McpServer`; `stdio` is a new serving entry | Full tool parity by construction (Req 4); no code fork; serve code untouched | Slightly larger `AppState` surface exposed to stdio path | SELECTED — parity is a hard requirement |
| Separate local-mode server component | A dedicated local server wrapping services | Isolation | Duplicates handler wiring; parity drifts; doubles maintenance | REJECTED |
| Per-process connections + busy_timeout | Keep `Arc<Mutex<Connection>>` per process; WAL + busy_timeout across processes | No new deps; public API unchanged; suite stays green | Bounded waits under heavy write contention (mitigated by short locked sections + 5 s default) | SELECTED |
| Connection pool (r2d2 or hand-rolled) | Multiple connections per process | Higher intra-process read parallelism | New dependency; more contention against other processes; unneeded at harness scale | REJECTED |
| Built-in size-based audit rotation | Rename cascade in the audit write path | Works on every OS; no tooling assumptions | Benign cross-process rename races (documented) | SELECTED |
| GH Actions native runners for win/mac artifacts | Runner-per-OS release builds + clippy + tests | Canonical MSVC/darwin toolchains; OS-gated code linted | Second CI surface (deliberate, documented exception) | SELECTED |

## Design Decisions

### Decision: one ServerHandler, two transports
- **Context**: Req 2.1, 4.1–4.3 — identical tool semantics in both modes.
- **Alternatives Considered**: (1) new local server component; (2) reuse `McpServer` + add `stdio` serving entry.
- **Selected Approach**: `src/mcp/stdio.rs` builds `AppState` from the local profile and serves `McpServer` over `rmcp::transport::stdio_pipes()`; `mcp/server.rs` (HTTP) is not modified.
- **Rationale**: parity is structural — both transports dispatch into the same handlers and services.
- **Trade-offs**: the stdio entry must own tracing-to-stderr and EOF shutdown itself.
- **Follow-up**: pin rmcp's malformed-input behavior with a test (Req 2.4).

### Decision: profile as a config dimension, not a config fork
- **Context**: Req 3.1–3.4, 9.2 — different defaults/validation per mode; zero breakage.
- **Alternatives Considered**: (1) second `LocalConfig` type; (2) one `Config` + `Profile` enum + `from_env_local`.
- **Selected Approach**: additive `Profile` on `Config`; `Config::load()`/`from_env` keep serve semantics byte-for-byte; `Config::load_local()` resolves per-OS user paths, treats token as optional-empty and roots as optional-empty; new env vars (`AGENT_MEM_BUSY_TIMEOUT_MS`, `AGENT_MEM_AUDIT_MAX_BYTES`, `AGENT_MEM_AUDIT_ROTATE_KEEP`) parse with defaults in both profiles.
- **Rationale**: the 97-test suite (incl. inline config tests asserting field types) passes unmodified; one struct keeps `AppState::build` shared.
- **Trade-offs**: `token: String` is technically `""` in local mode — documented as unused (no HTTP listener is constructed).
- **Follow-up**: unit tests for per-OS defaults with injected env; assert serve `from_env` rejects missing token exactly as before.

### Decision: supersede the storage invariant in-place
- **Context**: Req 6.1–6.4, 9.1/9.4 — multi-process safety; serve suite revalidates.
- **Alternatives Considered**: (1) adopt a pool/lock-file layer; (2) add `busy_timeout` + keep per-process single connection.
- **Selected Approach**: `Db::open_at` gains `busy_timeout` (default 5 s, `AGENT_MEM_BUSY_TIMEOUT_MS`); the doc invariant is rewritten to the multi-process form (per-process connection; WAL + busy_timeout across processes; lock still never held across an await within a process).
- **Rationale**: SQLite's native mechanism; no public API change; existing `with_conn` callers and tests untouched.
- **Trade-offs**: pathological >5 s write contention surfaces as a clear error (Req 6.3) rather than indefinite blocking — acceptable per requirements.
- **Follow-up**: multi-process integration tests (two stdio children on one store; concurrent fresh-open).

### Decision: native-runner CI jobs for Windows/macOS
- **Context**: Req 10.1–10.5.
- **Alternatives Considered**: (1) cross-compile everything inside Dagger (mingw/osxcross); (2) native GH Actions runner jobs; (3) skip non-Linux artifacts.
- **Selected Approach**: Linux stays fully in the Dagger module (all gates + Linux release + Docker); the workflow adds `windows-latest` and `macos-latest` jobs running clippy (`-D warnings`), the deterministic suite, and release builds, uploading artifacts.
- **Rationale**: MSVC/darwin toolchains cannot run in the Linux containers; OS-gated code otherwise escapes the lint/test gates.
- **Trade-offs**: a second CI surface — mitigated by keeping the gate commands identical and container work single-sourced in the Dagger module.
- **Follow-up**: runner minutes/cache usage (cargo cache via the workflow's built-in cache); matrix pinned to the same toolchain (`rust-toolchain.toml` applies).

## Risks & Mitigations
- rmcp stdio malformed-line behavior unpinned — test garbage-then-valid (Req 2.4) in the integration suite; fallback: a filtering stdin wrapper in `mcp/stdio.rs`.
- `stdio_pipes` panics off-runtime on Linux — construct transport inside `block_on` (design states the order; test spawns real child processes).
- Windows/macOS runner cost/availability for private repos — artifacts are per-workflow-run uploads; release publishing stays manual/tag-triggered; document in CI notes.
- Concurrent first-open of a fresh store (two harnesses race on first run) — covered by `IF NOT EXISTS` DDL + busy-wait; dedicated multiprocess test.
- Cross-process audit rotation races — accepted as benign (per-record digests; worst case one extra line in a rotated file); documented in the audit component.
- Long first build on runners (rusqlite bundled) — cargo caching in the workflow; not a correctness risk.
- Another session is concurrently modifying `ci/src/mmhmem_ci/main.py` and `src/lib.rs` (uncommitted at design time) — implementation tasks touching those files must rebase on the then-current state, not on this design's line references.

## References
- rmcp 3.8.0 source (`src/transport/io.rs`, `Cargo.toml` feature `transport-io`) — vendored crate, verified in the local cargo registry.
- SQLite WAL and busy_timeout documentation (`sqlite.org/pragma.html#pragma_busy_timeout`, WAL mode) — mechanism basis for Req 6.
- XDG Base Directory Specification — Linux user data dir resolution.
- Upstream spec: `.kiro/specs/cross-agent-memory-server/{requirements,design}.md` — superseded storage invariant (design.md line 109) and the 97-test revalidation contract.
- MCP stdio transport convention (client spawns server; JSON-RPC over stdin/stdout; EOF ends the session).
