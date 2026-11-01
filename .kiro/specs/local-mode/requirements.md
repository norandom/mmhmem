# Requirements Document

## Introduction
mmhmem is a self-hosted, cross-agent memory and I/O optimization layer for AI coding harnesses (Claude Code, Google Antigravity, OpenCode, Claude Desktop). Today the entire token-efficiency layer — the `smart_read`/`boilerplate_write` shunt with its content-hash cache, the cross-agent session bus, long-term project memory, token statistics, and the audit trail — is reachable only through the Linux server daemon over Streamable HTTP with bearer auth. This spec extends the validated `cross-agent-memory-server` feature so that the same single binary also runs locally: each harness spawns `mmhmem stdio` as a stdio MCP server backed by a per-user shared store, all six tools work with full parity, multiple harnesses on the same machine share cache, sessions, memory, and token statistics (same-machine handoff with no server), and a `mmhmem stats` command prints human-readable token-savings from the local store. The deployment model becomes client-first with the server optional: the Linux container server remains the upgrade path for cross-machine access, central audit/compliance, central ops, and multi-user bearer auth. The existing serve mode must not regress (its 97/97 deterministic suite stays green), and the storage layer generalizes from single-process to multi-process-safe access, superseding the validated spec's single-connection invariant. All artifacts are in English; the implementation extends the existing Rust codebase.

## Boundary Context (Optional)
- **In scope**: the `mmhmem stdio` entry and the stdio MCP transport; a local config profile (per-OS user-level path defaults, no bearer token, zero mandatory environment); multi-process-safe access to the per-user store; operation of all six existing tools in local mode, including same-machine multi-harness sharing (session handoff, shunt cache, memory, token stats); the local audit trail (record parity plus bounded rotation); the `mmhmem stats` CLI (human-readable table and optional `--json`); cross-platform release builds (Windows, Linux, macOS) in the CI pipeline; the client-first README/install restructure, per-harness stdio connection examples, and SKILL.md guidance update; automated tests.
- **Out of scope**: cross-machine operation in local mode (the server's job); new MCP tools; the harness clients themselves; remote stats aggregation; any serve-mode behavior change; multi-user authentication in local mode (single user-account trust boundary); cross-machine sync, clustering, or replication; OTel in the plain local binary (stays Docker-image-only); a local-to-server data migration story (possible future spec).
- **Adjacent expectations**: harnesses are expected to support spawn-based stdio MCP (e.g. `claude mcp add --transport stdio mmhmem -- mmhmem stdio`); the worker model endpoint remains an OpenAI-compatible chat-completions endpoint (optional in local mode). Upstream, this spec EXTENDS the validated `cross-agent-memory-server` spec: its services, storage schema, and tool handlers are the base, and its storage concurrency invariant (a single connection behind an async mutex; the lock never held across an await) is explicitly superseded here by a multi-process-safe design (WAL already on; busy timeout plus retry; per-process connections). The validated server suite revalidates through that storage rework — it must pass unmodified. mmhmem does not own the harnesses or the worker endpoint.

## Requirements

### Requirement 1: One binary, two operating modes
**Objective:** As an adopter, I want the same single binary to run as today's daemon or as a local stdio process, so that local mode is not a second product to install and maintain.

#### Acceptance Criteria
1. The mmhmem binary shall provide a serve mode that starts today's daemon, reachable via a `serve` subcommand.
2. The mmhmem binary shall provide a local mode, started via a `stdio` subcommand, that serves MCP over standard input and standard output.
3. The mmhmem binary shall provide a `stats` subcommand that prints token-savings statistics from the per-user store (Requirement 8).
4. The mmhmem binary shall document the serve, stdio, and stats entries in its help output.

### Requirement 2: Local mode over stdio (spawned by the harness)
**Objective:** As a harness operator, I want each harness to spawn mmhmem as a local stdio MCP server with zero configuration, so that local mode works out of the box on any machine.

#### Acceptance Criteria
1. When a harness starts `mmhmem stdio`, the mmhmem binary shall serve the six MCP tools over the stdio transport (requests on stdin, responses on stdout) without opening any network listener.
2. While running in stdio mode, the mmhmem binary shall write all diagnostics to standard error only, keeping standard output exclusively for MCP messages.
3. When the parent process closes the child's standard input or exits, the mmhmem binary shall flush pending state and shut down cleanly, exiting with success.
4. If malformed input arrives on standard input, the mmhmem binary shall not terminate; subsequent valid MCP requests shall still be answered.
5. The mmhmem binary shall start in local mode with no mandatory environment variables and no configuration file, and shall become ready to serve without network access and without waiting on any external service.
6. When the per-user store does not yet exist on first start, the mmhmem binary shall create it automatically with the same schema as serve mode, so the first tool call succeeds without a setup step.

### Requirement 3: Local config profile — per-OS user paths, no token
**Objective:** As a local user, I want local mode to store everything under my user account with sensible per-OS defaults and no credentials, so that a spawned child inside my trust boundary needs no operator.

#### Acceptance Criteria
1. The local profile shall default the per-user store to the conventional per-user application-data location of the operating system: `~/.local/share/mmhmem` on Linux, `%APPDATA%\mmhmem` on Windows, `~/Library/Application Support/mmhmem` on macOS.
2. The local profile shall start without `AGENT_MEM_TOKEN`; no bearer authentication is required for a spawned child inside the user's trust boundary.
3. Where `AGENT_MEM_ALLOWED_ROOTS` is set in local mode, the mmhmem binary shall enforce path confinement with the same semantics as serve mode; where unset, local mode shall start and operate without confinement.
4. The local profile shall honor the existing environment overrides (data directory, audit file, shunt threshold, worker endpoint), so users can relocate or tune the local store without code changes.

### Requirement 4: Full tool parity in local mode
**Objective:** As an agent working in any harness, I want all six tools to behave identically whether mmhmem runs as the server or locally, so that skills and habits transfer one-to-one.

#### Acceptance Criteria
1. The mmhmem binary shall expose the tools `session_resume_or_handoff`, `smart_read`, `boilerplate_write`, `mem_search`, `mem_get_observations`, and `mem_record_observation` in local mode.
2. The local mode shall implement each tool with the semantics of the validated cross-agent-memory-server requirements — including the 150-line shunt threshold (configurable), the security-critical bypass, content-hash caching, the 1,500-token working-state cap, automatic breadcrumbs, and identical error messages.
3. When a tool call in local mode receives the same input as in serve mode, the mmhmem binary shall produce an equivalent observable response (same format, same fields, same error behavior).
4. Where a worker model endpoint is configured in local mode, shunting shall use it as in serve mode; where it is not configured, the tools shall operate fully wherever the worker is not needed (direct reads, cache hits, session bus, memory) and shall return a clear error where shunting is required.

### Requirement 5: Shared per-user store — same-machine multi-harness
**Objective:** As an engineer running several harnesses on one machine, I want them to share one per-user store, so that handoff, cache, memory, and stats work locally exactly as they do through the server.

#### Acceptance Criteria
1. While two or more harnesses each run their own `mmhmem stdio` process under the same user account, all processes shall operate on the same per-user store.
2. When one harness checkpoints working state and another harness calls `session_resume_or_handoff` for the same project, the second harness shall receive the stored working state, last active harness, objective, and breadcrumbs — same-machine handoff with no server involved.
3. When a harness reads via `smart_read` a file already summarized by another harness (unchanged content, same query fingerprint), it shall receive the cached summary with zero worker-model calls.
4. When one harness records an observation, another harness's `mem_search` shall find it immediately.
5. Token-savings events from all local processes shall accumulate in one shared token-stats table in the per-user store, so `mmhmem stats` reflects every harness.

### Requirement 6: Multi-process-safe storage
**Objective:** As a user running several local processes (and possibly a same-machine serve instance), I want concurrent store access to be safe, so that parallel harnesses never corrupt or lose data.

#### Acceptance Criteria
1. While multiple mmhmem processes access the per-user store concurrently, the mmhmem binary shall keep the store consistent: no corruption, no lost updates, no partially applied tool actions.
2. When a writer finds the store busy, the mmhmem binary shall wait up to a configurable timeout (default 5 seconds) and retry before failing.
3. If the store remains busy beyond the timeout, the mmhmem binary shall return a clear error to the calling agent and shall leave no partial state.
4. The mmhmem binary shall support the same-machine concurrent scenarios of Requirement 5 without any additional setup, service, or coordination step beyond spawning the mmhmem processes.

### Requirement 7: Local audit trail
**Objective:** As a security-conscious user, I want the same tamper-evident, secret-free audit records locally under my user directory, so that I keep forensic reconstruction without a server.

#### Acceptance Criteria
1. The local profile shall record the same redacted, hash-chained, single-line JSON audit records as serve mode — same event types and fields (timestamp, harness, project, structured details, self-verifying digest) — to a per-user audit file defaulting to `<data dir>/audit.jsonl`.
2. The local audit writer shall apply the same secret-redaction classes as serve mode before any write (bearer tokens, API keys, passwords, private tags).
3. To keep the audit trail bounded on machines without log-rotation tooling, the local profile shall rotate the audit file when it reaches a configurable size threshold (default 10 MiB), retaining a configurable number of rotated files (default 5).
4. When an audit write fails (for example, disk full), the mmhmem binary shall fail the mutating tool action with a clear error rather than commit unaudited state.

### Requirement 8: Token-savings statistics CLI (`mmhmem stats`)
**Objective:** As a cost-conscious adopter without a server, I want my token savings measured on my own machine over time, so that the efficiency layer proves itself with numbers rather than claims.

#### Acceptance Criteria
1. When run as `mmhmem stats`, the mmhmem binary shall print a human-readable table of token-savings statistics from the local profile's per-user store, aggregated by day with per-project and per-harness breakdown.
2. Each aggregate row shall show the frontier input tokens avoided (separately summing computed estimates and client-reported measurements) and the worker-model tokens spent (prompt and completion).
3. The stats output shall preserve the labeling discipline of the token-stats records: estimates marked as estimates, client-reported values labeled by source; computed values shall never be presented as measured.
4. Where the `--json` flag is passed, the mmhmem binary shall emit the same aggregates as machine-readable JSON.
5. When the local store contains no statistics, `mmhmem stats` shall print a clear no-statistics message and exit successfully.
6. The `mmhmem stats` command shall be read-only: it shall not mutate the store, the audit log, or session state.

### Requirement 9: No serve-mode regression
**Objective:** As an operator of the validated server, I want local mode to change nothing about serve mode, so that existing deployments and the validated guarantees stay intact.

#### Acceptance Criteria
1. The existing deterministic server suite (97/97 tests) shall pass unmodified after every change of this spec.
2. The serve profile's configuration contract shall remain unchanged: `AGENT_MEM_TOKEN` and `AGENT_MEM_ALLOWED_ROOTS` required, same defaults (paths, port, MCP path, shunt threshold), same fail-fast behavior on missing configuration.
3. The no-argument invocation shall keep starting the daemon exactly as before, and `--version`, `--help`, and the `healthcheck` subcommand shall keep their current behaviors.
4. The storage generalization shall preserve serve-mode storage semantics under single-process access: no new errors, lock waits, or other observable behavior through the serve surface.
5. The serve-mode deployment artifacts (Docker image, compose file, systemd unit, logrotate configuration) shall remain valid and unchanged.

### Requirement 10: Cross-platform delivery (Windows, Linux, macOS)
**Objective:** As an adopter on any desktop OS, I want a ready-to-run binary for my machine, so that installing mmhmem locally is a download, not a build.

#### Acceptance Criteria
1. The CI pipeline shall produce release builds of the single mmhmem binary for Linux, Windows, and macOS from the same codebase, as part of the standard pipeline.
2. The release build for each supported OS shall be self-contained: the embedded store compiled in, no separate database service, no external runtime dependencies beyond the OS itself.
3. The local-mode feature set (stdio entry, six tools, stats command, per-OS user paths) shall run with the same behavior on all three supported operating systems.
4. The blocking CI gates (formatting, lint with the complexity caps of Requirement 13, deterministic test suite) shall apply to the release builds without reduced strictness.
5. The existing Linux release and container builds shall continue to be produced unchanged.

### Requirement 11: Client-first documentation and harness onboarding
**Objective:** As a new adopter, I want documentation that starts with the local install and presents the server as optional, so that I can benefit in minutes without any infrastructure.

#### Acceptance Criteria
1. The README and installation documentation shall present the client-first model: local mode is the required core (each harness spawns `mmhmem stdio`), the server is the optional upgrade (cross-machine access, central audit/compliance, central ops, multi-user bearer auth).
2. The mmhmem distribution shall ship stdio connection examples for Claude Code CLI (`claude mcp add --transport stdio mmhmem -- mmhmem stdio`), Google Antigravity, OpenCode, and Claude Desktop.
3. The SKILL.md playbook shall be updated to guide agents to the same tool habits in local and serve mode, remaining under 200 tokens.
4. The documentation shall state the local-mode trust boundary and egress contract: no bearer token; per-user store; the worker endpoint as the only outbound connection.
5. When followed on a fresh machine, the documented local install alone shall deliver the full efficiency layer without the server.

### Requirement 12: Local-mode security posture
**Objective:** As a security-conscious user, I want local mode to open no new attack surface and leak nothing, so that a spawned helper stays inside my trust boundary.

#### Acceptance Criteria
1. While running in local mode, the mmhmem binary shall open no listening network socket; all MCP communication shall occur over stdio.
2. The local mode shall perform no telemetry, analytics, or SaaS calls; the only outbound connection shall be the configured worker-model endpoint (when set).
3. The plain release binary used for local mode shall contain no observability-exporting code paths; the OTel capability stays compile-gated and Docker-image-only as in the validated spec.
4. Where the local profile logs configuration at startup, it shall log a redacted form, never bearer tokens or secrets.
5. When local mode denies a file access due to configured path confinement (Requirement 3), the mmhmem binary shall record the denial reason in the audit trail.

### Requirement 13: Code complexity and size caps (anti-explosion, house rule)
**Objective:** As a maintainer, I want the hard, machine-enforced caps to apply to this feature too, so the codebase cannot explode while local mode is implemented.

#### Acceptance Criteria
1. Every function in `src/` shall keep cognitive complexity at or under 15, machine-enforced as a blocking CI gate via the existing clippy configuration; silencing the lint requires written justification in the change.
2. No source file in `src/` shall exceed 500 lines and no function shall exceed 80 lines; when a module reaches a cap it shall be split by responsibility, not silenced.
3. Test files are exempt from the per-function caps but capped at 800 lines per file.
4. Every implementation task of this spec shall keep its non-test diff at or under 400 added lines; larger work shall be split into multiple tasks at task-generation time.
5. Task generation shall copy these caps into every implementation task's acceptance criteria, so each task is individually verifiable against them.
