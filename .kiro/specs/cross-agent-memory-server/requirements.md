# Requirements Document

## Introduction
mmhmem is a self-hosted, cross-agent memory and I/O optimization server for AI coding harnesses. It is deployed as a single daemon on a Linux host and consumed by heterogeneous agents (Claude Code, Google Antigravity, OpenCode, Claude Desktop) through a standard remote MCP endpoint plus a portable skill playbook — replacing client-side lifecycle hooks and MITM API proxies. It delivers lossless mid-task session handoff between harnesses, deterministic shunting of expensive I/O to a cheap worker model, progressive-disclosure long-term project memory, and a tamper-evident local audit trail. The implementation language is compiled (Rust recommended, Go as fallback); all spec artifacts are in English.

## Boundary Context (Optional)
- **In scope**: the server daemon and its remote MCP tool surface; Bearer-token authentication; the live cross-agent session bus (working state + automatic breadcrumbs); the deterministic read/write shunt engine with content-addressed caching and security-critical bypass; progressive-disclosure long-term memory (index search + batch detail fetch + recording observations); the worker-model client; the redacted, hash-verified JSONL audit trail; environment-based configuration; deployment artifacts (systemd unit, logrotate config, skill playbook, example harness connection configurations); automated tests.
- **Out of scope**: the agent harness clients themselves; hosting/operating the worker LLM; TLS certificate issuance; any web viewer UI; forking or patching existing memory plugins; real-time SIEM streaming (batch shipping of rotated logs is allowed); multi-host clustering or replication.
- **Adjacent expectations**: harnesses are expected to support remote Streamable HTTP MCP and Agent Skills (SKILL.md); the worker model endpoint is expected to be OpenAI-compatible chat completions; the Linux host provides systemd and logrotate. mmhmem does not own any of these.

## Requirements

### Requirement 1: Remote MCP protocol surface and authentication
**Objective:** As a harness operator, I want every supported agent harness to connect to one central memory endpoint over a standard protocol, so that no per-harness client hooks are needed.

#### Acceptance Criteria
1. The mmhmem server shall expose its tools via a remote MCP endpoint using the Streamable HTTP transport at a configurable path (default `/mcp`) and port (default 8443).
2. When a client connects without a valid Bearer token, the mmhmem server shall reject the request with an authentication error.
3. When a client presents a valid Bearer token, the mmhmem server shall accept MCP connections from Claude Code CLI, Google Antigravity (IDE and CLI), OpenCode, and Claude Desktop.
4. The mmhmem server shall expose the tools `session_resume_or_handoff`, `smart_read`, `boilerplate_write`, `mem_search`, `mem_get_observations`, and `mem_record_observation`.
5. The mmhmem server shall never require client-side shell lifecycle hooks or an LLM API reverse proxy to function.

### Requirement 2: Cross-agent live session bus
**Objective:** As an engineer switching mid-task between harnesses, I want the new agent to inherit the full in-flight working state of the previous agent, so that I never re-explain active context.

#### Acceptance Criteria
1. When an agent calls `session_resume_or_handoff` for a project without supplying updated working state, the mmhmem server shall return the stored active working state, the last active harness, the active objective, and the 15 most recent tool breadcrumbs for that project.
2. When an agent calls `session_resume_or_handoff` with updated working state, the mmhmem server shall store the checkpoint (working state, objective, calling harness, timestamp) keyed by project and task ID, overwriting any prior checkpoint for that project.
3. When no active session exists for a project, the mmhmem server shall initialize a new task ID and report that no prior session was found.
4. When any connected agent invokes any mmhmem tool, the mmhmem server shall automatically append a timestamped breadcrumb recording the calling harness, tool name, target resource, and action summary — without requiring the agent to call a checkpoint command.
5. The mmhmem server shall cap the structured working state at 1,500 tokens worth of content — measured as ~4 characters per token, i.e. a 6,000-character budget, truncated at the last complete line before the limit — to keep handoff payloads small.
6. When a handoff occurs, the mmhmem server shall preserve 100% of in-flight state without invoking any LLM summarization call.

### Requirement 3: Deterministic read shunting (smart_read)
**Objective:** As a cost-conscious team, I want large file reads to be answered by a cheap worker model instead of the frontier model, so that bulk I/O tokens never enter the expensive context window.

#### Acceptance Criteria
1. When `smart_read` is called with explicit offset and limit parameters, the mmhmem server shall return exactly the requested line range, prefixed with exact line numbers (`L<n>:`), and shall not delegate to the worker model.
2. When the target file is smaller than the configured line threshold (default 150 lines), the mmhmem server shall return the full line-numbered content directly to the calling agent.
3. When the target file path matches the security-critical pattern (authentication, cryptography, secrets, tokens, RBAC, JWT, TLS, certificates, mutexes/locks, concurrency, permissions, sudo, PAM — case-insensitive), the mmhmem server shall bypass the worker model and return the full line-numbered content directly, recording the bypass reason in the audit trail.
4. When the target file meets the shunt threshold and is not security-critical, the mmhmem server shall compute a SHA-256 content hash of the file and a fingerprint of the normalized question, and if a cached summary exists for that (path, hash, fingerprint) tuple, shall return the cached summary immediately without any worker-model call.
5. When no cached summary exists, the mmhmem server shall send the line-numbered file content and the agent's question to the configured worker model, requesting concise bullet points where every bullet begins with an exact symbol name and line range.
6. When the worker model returns a summary, the mmhmem server shall store it in the content-addressed cache keyed by (file path, content hash, query fingerprint) so that any later read of the unchanged file by any harness returns the cached summary in under 50 ms with zero worker-model invocations.
7. When the file content changes (different SHA-256), the mmhmem server shall not serve the stale cached summary.
8. If the target file does not exist, the mmhmem server shall return a clear error message to the calling agent.
9. The shunt threshold shall be configurable via environment variable, defaulting to 150 lines.

### Requirement 4: Boilerplate write shunting (boilerplate_write)
**Objective:** As a developer generating repetitive tests or stubs, I want generation to happen outside the frontier model's token budget, so that boilerplate costs neither input nor output frontier tokens.

#### Acceptance Criteria
1. When `boilerplate_write` is called with a target path, a reference file path, and a specification prompt, the mmhmem server shall send the reference file content and the specification to the configured worker model with instructions to follow the reference file's conventions and output only raw source code.
2. When the worker model responds, the mmhmem server shall strip any markdown code fences, write the generated code directly to the target path (creating parent directories as needed), and return only a one-line confirmation with the output SHA-256 checksum, line count, worker model, and latency.
3. If the reference file does not exist, the mmhmem server shall return a clear error message and shall not write anything to the target path.
4. When `boilerplate_write` completes, the mmhmem server shall record the write in the audit trail and as a session breadcrumb.

### Requirement 5: Progressive-disclosure long-term memory
**Objective:** As an agent returning to a project after weeks, I want to search a compact index first and pull full details only for relevant entries, so that historical context costs a fraction of loading raw history.

#### Acceptance Criteria
1. When `mem_search` is called with a query, the mmhmem server shall return a compact results table (ID, type, calling harness, title, files touched) of matching observations for the project, with per-result cost of roughly 50–100 tokens, and shall not return full narratives.
2. When `mem_search` is called with a type filter, the mmhmem server shall restrict results to observations of that type (decision, bugfix, feature, refactor, discovery, change).
3. When `mem_search` finds no matches, the mmhmem server shall return a clear "no observations matched" message.
4. When `mem_get_observations` is called with a list of IDs, the mmhmem server shall return the full narrative, facts, and files touched only for those validated IDs.
5. When `mem_record_observation` is called, the mmhmem server shall durably store the observation (project, task, harness, type, title, narrative, facts, files touched, timestamp) and make it searchable through the full-text index immediately.
6. The mmhmem server shall return search index results in under 15 ms for typical projects.

### Requirement 6: Worker model integration
**Objective:** As an operator, I want the shunt worker to be any local or API model I choose, so that I am not locked into a vendor.

#### Acceptance Criteria
1. The mmhmem server shall call the worker model through a configurable OpenAI-compatible chat completions endpoint (URL, model name, and API key configurable via environment).
2. The mmhmem server shall send worker requests with temperature 0 for deterministic output.
3. If the worker endpoint is unreachable or returns an error, the mmhmem server shall return a clear error to the calling agent and record the failure in the audit trail; it shall never silently return stale or empty content as if it were a fresh summary.
4. When a worker call completes, the mmhmem server shall record the worker model name and round-trip latency in the audit trail.

### Requirement 7: Audit trail and secret redaction
**Objective:** As a security professional, I want every server action recorded in a tamper-evident, secret-free, Unix-native log, so that I can forensically reconstruct any session with standard tools.

#### Acceptance Criteria
1. Before any tool execution result or state mutation is persisted, the mmhmem server shall pass all recorded inputs and outputs through a redaction filter.
2. The redaction filter shall scrub bare secrets assigned to keys such as `api_key`, `secret`, `token`, `password`, `MYSQL_PWD`, `dbPassword`, `authorization`; `Authorization: Bearer` headers; Slack `xapp-` and `xoxe-` tokens; and content inside user-supplied `<private>...</private>` tags.
3. When any mmhmem tool executes or mutates state, the mmhmem server shall atomically append a single-line JSON record containing ISO-8601 UTC timestamp, millisecond epoch, event type, agent harness, project, structured details, and a self-verifying SHA-256 record digest to the audit log file (default `/var/log/agent-memory/audit.jsonl`).
4. The mmhmem server shall open the audit file in append mode per write so that standard logrotate rotation (daily, compressed, dated) works without restarting the daemon.
5. The audit log path and data directory shall be configurable via environment variables (defaults `/var/log/agent-memory/audit.jsonl` and `/var/lib/agent-memory`).

### Requirement 8: Deployment, operations, and harness playbook
**Objective:** As an infrastructure operator, I want to deploy and operate the server with standard Linux tooling and bind all harnesses with a portable playbook, so that adoption requires no custom client code.

#### Acceptance Criteria
1. The mmhmem server shall run as a single self-hosted process manageable by systemd, with all runtime configuration supplied via environment variables.
2. The mmhmem server shall ship a logrotate configuration that rotates the audit log daily, retains 90 days compressed with 0640 permissions, and optionally batch-ships each rotated file to a self-hosted OpenObserve endpoint.
3. The mmhmem server shall ship a portable SKILL.md playbook instructing agents to (a) resume the session first, (b) prefer `smart_read` over raw reads of large files and use offset/limit for edits, (c) search the memory index before fetching details, and (d) checkpoint before switching harnesses — kept under 200 tokens.
4. The mmhmem server shall ship example connection configurations for Claude Code CLI (`claude mcp add --transport http ... --header "Authorization: Bearer ..."`), Google Antigravity (`mcp_config.json` with `serverUrl` and headers), OpenCode (global config remote HTTP MCP), and Claude Desktop (Custom Connectors).
5. The mmhmem server shall contain no telemetry, analytics, or cloud-sync calls to any external SaaS.
6. The mmhmem server shall ship a multi-stage Dockerfile whose runtime image contains only the dedicated `mmhmem` binary (non-root user, minimal base image), plus a `docker-compose.yml` that runs it with env-file configuration, publishes the MCP port (default 8443), and bind-mounts the data directory, the audit log directory, and the allowed workspace roots (read-only) as volumes.
7. When deployed via compose, the audit log shall remain rotatable without restarting the container: by default via host-side logrotate against the bind-mounted log volume; optionally via a logrotate sidecar container.

### Requirement 9: Non-functional: concurrency, performance, self-containment
**Objective:** As an operator running several agents in parallel, I want the server to handle concurrent harnesses safely and cheaply, so that memory access never becomes the bottleneck.

#### Acceptance Criteria
1. While multiple harnesses are connected simultaneously, the mmhmem server shall handle their requests concurrently without cross-session state corruption.
2. The mmhmem server shall serve cached shunt summaries and session resumes in under 50 ms.
3. The mmhmem server shall persist all durable state locally on the host (no network dependency for its own state) and shall start without any external service except the optionally used worker model endpoint.
4. When the worker model is not configured or not needed (cache hits, direct reads, session bus), the mmhmem server shall operate fully without it.

### Requirement 10: Token-savings statistics (per harness and per task, over time)
**Objective:** As an operator, I want measured per-agent and per-session token-savings statistics on the server over time, so the I/O optimization is demonstrated with numbers rather than claims.

#### Acceptance Criteria
1. For every shunt event the mmhmem server shall compute an estimate of frontier input tokens avoided — (raw file chars − returned summary chars) ÷ 4 for delegated reads, raw file chars ÷ 4 for cache hits, and reference-file chars ÷ 4 plus generated chars ÷ 4 for boilerplate writes — and record it with the audit event and in a durable `token_stats` table.
2. The mmhmem server shall record worker-model token usage (prompt/completion tokens from the worker API's `usage` response field) for every delegated call.
3. All statistics shall be stored keyed by (project, workspace_task_id, agent_harness, timestamp, kind) and emitted as OTel metrics when OTel is enabled (Req 11).
4. `session_resume_or_handoff` shall accept an optional client-reported usage payload (harness-reported frontier prompt/completion tokens, stored with `kind=client_reported`); the server shall never require it and shall treat it as the only source of true frontier token counts.
5. Server-computed values shall be labelled as estimates (÷4 chars/token heuristic); client-reported values shall be labelled with their source. The server shall never invent frontier token counts.

### Requirement 11: Optional OTel observability and data-lifecycle documentation
**Objective:** As an operator with a self-hosted OpenObserve system, I want optional OTLP traces, metrics, and logs from the server, and a documented map of which data passes through the server and which logs and traces remain on disk.

#### Acceptance Criteria
1. When `OTEL_EXPORTER_OTLP_ENDPOINT` (and optional `OTEL_EXPORTER_OTLP_HEADERS`) are set, the mmhmem server shall export OTLP spans around each MCP tool call (tool, harness, project, outcome, latency), metrics (tool latency histogram, shunt cache hit rate, tokens-avoided counters, worker token counters), and redacted audit log records; when unset, the server shall make no OTLP calls (off by default).
2. OTel support shall be compiled behind a cargo feature (`otel`) that is enabled only in the Docker image; the plain release binary shall not include it.
3. Every OTel payload shall pass the same redaction filter as the audit writer; the only permitted egress remains the configured worker endpoint, the configured OTLP endpoint, and logrotate's batch ship. The JSONL audit remains the system of record; OTel is optional observability on top.
4. The design document shall contain a data-lifecycle table: for each data class (tool arguments, file contents, worker payloads, shunt summaries, frontier prompts and native harness I/O, audit records, token stats, OTel telemetry) — what passes through the server, where it persists, and its retention.
5. Agent-side tracing (harness spans and traces to OpenObserve) shall be configured in the harness itself (e.g. OpenCode's own OTLP export) and documented in `deploy/examples/`; mmhmem ships no client code for it.

### Requirement 12: End-to-end measurement with a real harness
**Objective:** As a skeptical operator, I want an e2e suite that runs a real agent harness (OpenCode with its configured model, e.g. Mistral) against the server in docker compose on a fixture repository — with and without mmhmem — and asserts measured token savings from harness-reported usage.

#### Acceptance Criteria
1. The e2e suite shall bring up the compose stack, run scripted OpenCode sessions on fixed fixture tasks (read-heavy and boilerplate-heavy) in baseline mode (mmhmem not connected) and mmhmem mode (MCP endpoint + SKILL.md connected), and collect per-session prompt/completion token usage as reported by the harness.
2. The suite shall compute per-task and aggregate input/output token deltas, assert that mmhmem mode reduces frontier input tokens by at least a configured threshold on read-heavy tasks, and persist the measured numbers (not estimates) via `client_reported` entries in `token_stats` and the audit log.
3. The suite shall be gated behind an environment flag (e.g. `E2E_LLM=1`) so plain `cargo test` without model access runs only the deterministic integration tests; CI may run the full suite.

### Requirement 13: Code complexity and size caps (anti-explosion)
**Objective:** As a maintainer, I want hard, machine-enforced caps on code complexity and size, so the codebase cannot explode while the feature is implemented.

#### Acceptance Criteria
1. Every function in `src/` shall have a hard McCabe-style complexity cap, machine-enforced and blocking in CI: cognitive complexity ≤ 15 via clippy's `cognitive_complexity` lint with `cognitive-complexity-threshold = 15` in `clippy.toml` and `-D warnings` (on clippy ≥ 1.98 the former `cyclomatic_complexity` lint is only a deprecated alias, so the cognitive cap at 15 is the enforced guard, complemented by the ≤ 80-LOC-per-function cap in 13.3). `#[allow]` on this lint requires a written justification in the change.
2. Every function in `src/` shall have cognitive complexity ≤ 15 (clippy `cognitive_complexity`, threshold in `clippy.toml`, same enforcement).
3. No source file in `src/` shall exceed 500 lines and no function shall exceed 80 lines; when a module reaches the cap it shall be split by responsibility, not silenced with `#[allow]`. Test files are exempt from the per-function caps but capped at 800 lines per file.
4. Every implementation task shall keep its non-test diff at or under 400 added lines; work that would exceed this shall be split into multiple tasks at task-generation time.
5. CI shall run `cargo clippy --all-targets -- -D warnings` (which includes the Req 13.1/13.2 caps) as a blocking gate, plus an informational per-file LOC report via `tokei` (dev tool only, never a runtime dependency).
6. Task generation shall copy these caps into every implementation task's acceptance criteria, so each task is individually verifiable against them.
