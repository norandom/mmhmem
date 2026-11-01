# Brief: local-mode

## Problem
Today the token-efficiency layer (smart_read/boilerplate_write shunt, content-hash cache, token stats) only exists inside the mmhmem server. An adopter without the server gets nothing, and token-savings insight is only visible through the server's store. Deployment reality: one Linux container server, but clients (Claude Code, Antigravity, OpenCode, Claude Desktop) run locally on Linux, Windows, and macOS. The install model is client-server with the server optional: clients alone must benefit from the efficiency layer, including local stats insight.

## Current State
The `cross-agent-memory-server` spec is validated and complete (25/25 tasks, 97/97 tests): a single Rust binary serving the six MCP tools over Streamable HTTP with bearer auth, system-level paths, a single-process storage design (one connection behind a tokio mutex), Docker/systemd deployment, Dagger CI (linux containers). No stdio transport, no user-level profile, no stats CLI, no win/mac distribution.

## Desired Outcome
The same binary runs in two modes: `mmhmem serve` (today's daemon) and `mmhmem stdio` (local mode). Local mode: each harness spawns mmhmem as a stdio MCP server (rmcp `transport-io`); a per-user shared store (~/.local/share/mmhmem or %APPDATA%\mmhmem) backs ALL six tools, so two harnesses on the same machine share the shunt cache, sessions (same-machine handoff), memory, and token stats; a `mmhmem stats` CLI prints human-readable token-savings (per project/harness/day, worker tokens spent) from the local store. The server becomes an optional upgrade for cross-machine access, central audit/compliance, central ops (logrotate, OTel aggregation), and multi-user bearer auth.

## Approach
One binary, two transports, two config profiles (user-confirmed 2026-11-01):
- Transport layer: add rmcp stdio (`transport-io`) alongside the existing Streamable HTTP transport; same tool handlers, same services.
- Profiles: `serve` profile (system paths, AGENT_MEM_TOKEN required, full audit) vs `local` profile (user-level paths per OS, no token — a spawned child process is inside the user's trust boundary; audit behavior for local mode to be decided in requirements).
- Storage: generalize from single-connection-mutex to multi-process-safe access (WAL already on; add busy_timeout + retry; per-process connections). This supersedes the existing spec's "one connection behind an async mutex, lock never held across await" invariant.
- Distribution: cross-platform release builds (linux, windows, macOS) in the Dagger CI; install story becomes client-first (local layer required, server optional).
- Viability verified: rmcp stdio server transport exists (`rmcp::transport::stdio()`), rusqlite WAL multi-process is standard SQLite; repo already builds/tests on the Windows dev host.

## Scope
- **In**: stdio transport + `mmhmem stdio` entry; local config profile with per-OS user paths; multi-process-safe storage layer; all six tools in local mode; `mmhmem stats` CLI (human table; optional --json); cross-platform CI release builds; local-mode connection examples per harness (`claude mcp add --transport stdio mmhmem -- mmhmem stdio` style) + SKILL.md guidance update; README/install restructure (client-first, server optional)
- **Out**: cross-machine operation in local mode (server's job); new MCP tools; harness clients themselves; remote stats aggregation; changes to serve-mode behavior (the 97/97 suite must stay green)

## Boundary Candidates
- Transport/profile layer (new `mcp` wiring + `config` profile concept) vs storage concurrency model (shared with the existing spec) vs stats CLI (new surface over existing token_stats schema) vs distribution/CI (extends the Dagger module)

## Out of Boundary
- Any cross-machine sync, clustering, or replication in local mode; multi-user auth in local mode (single user account trust boundary); OTel in the plain local binary (stays Docker-image-only)

## Upstream / Downstream
- **Upstream**: the validated `cross-agent-memory-server` spec — its services, storage schema, and tool handlers are the base this spec extends; its `storage/db.rs` concurrency invariant is explicitly superseded here
- **Downstream**: README/installation restructure; per-harness local-mode examples; possible future "switch local↔server" migration story

## Existing Spec Touchpoints
- **Extends**: `cross-agent-memory-server` (storage concurrency invariant supersession; config gains a profile dimension; new entrypoints alongside `serve`)
- **Adjacent**: none

## Constraints
- No regression to serve mode: the existing 97/97 suite must pass unchanged (multi-process storage rework must preserve its semantics under single-process access)
- Req 13 caps apply (cognitive ≤ 15/fn, ≤ 500 LOC/src file, ≤ 80 LOC/fn, ≤ 400 non-test LOC/task)
- rmcp 3.x pinned (transport-io feature verified); rusqlite bundled; dependency set stays minimal
- Windows dev host + Debian WSL build VM; foreground-only workflow (user mandate)
- Spec artifacts in English
