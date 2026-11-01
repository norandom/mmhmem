# mmhmem

A self-hosted memory and I/O-optimization server for AI coding agents. One
daemon gives Claude Code, Google Antigravity, OpenCode, and Claude Desktop a
shared place to keep live session state, so you can switch tools mid-task
without losing context, and so bulk file reads stop costing frontier-model
tokens.

mmhmem replaces per-tool memory plugins and their fragile client-side hooks
with a single Linux service. Agents connect to one MCP endpoint (Streamable
HTTP with a bearer token), a portable SKILL.md playbook teaches them the
workflow, and every action the server takes lands in a redacted,
hash-verified audit log that plain logrotate can rotate. No SaaS, no
telemetry, no proxy between your agents and their model providers.

## What you get

- Live cross-agent handoff: checkpoint in one harness, resume in another.
  The server keeps the working state plus a breadcrumb trail of the last 15
  tool actions per project, with no LLM call in the handoff path.
- Deterministic token shunting: files at or above 150 lines (configurable)
  are summarized by a cheap worker model and cached by content hash, so the
  second agent reading the same file pays nothing. Security-critical paths
  (auth, crypto, concurrency) always bypass the worker. Targeted reads with
  offset and limit always go direct.
- Long-term project memory with progressive disclosure: a compact full-text
  index first, full details only for the observation IDs an agent picks.
  Token-savings statistics are recorded per project, harness, and task.
- A tamper-evident audit trail: one JSON line per action, secrets scrubbed
  before write, SHA-256 digest per record.

## What you need to install

| Component | Required | Notes |
|---|---|---|
| Linux host or Docker | one of the two | systemd on a Linux host, or any Docker host for the compose route |
| mmhmem binary | yes | build with Rust (pinned in `rust-toolchain.toml`, `cargo build --release`) or let the multi-stage Dockerfile build it; the runtime image contains the binary and nothing else |
| SQLite, runtimes, services | no | SQLite (WAL + FTS5) is compiled into the binary; there are no runtime dependencies |
| Worker model endpoint | optional | any OpenAI-compatible chat-completions endpoint (Ollama, vLLM, or an API key). Without it, shunting is off; session memory, project memory, and targeted reads all still work |
| Reverse proxy | optional, recommended for TLS | the daemon serves plain HTTP; terminate TLS in front of it |
| OpenObserve or any OTLP endpoint | optional | for OTel traces/metrics/logs and the logrotate batch ship of audit archives |

## Configure

The daemon is env-only and fails fast at startup. Two variables are required:

| Variable | Meaning |
|---|---|
| `AGENT_MEM_TOKEN` | bearer token every harness must present |
| `AGENT_MEM_ALLOWED_ROOTS` | comma-separated workspace roots; every `smart_read` and `boilerplate_write` path is canonicalized and confined to these |

The ones most people touch next:

| Variable | Default | Meaning |
|---|---|---|
| `AGENT_MEM_PORT` / `AGENT_MEM_MCP_PATH` | `8443` / `/mcp` | where the MCP endpoint listens |
| `SHUNT_MIN_LINES` | `150` | files at or above this line count go to the worker model |
| `WORKER_API_URL` / `WORKER_MODEL` / `WORKER_API_KEY` | unset | the shunt worker; leave all three unset to run without one |
| `AGENT_MEM_DATA_DIR` / `AGENT_MEM_LOG_FILE` | `/var/lib/agent-memory` / `/var/log/agent-memory/audit.jsonl` | durable state and the audit log |
| `OTEL_EXPORTER_OTLP_ENDPOINT` (+ `OTEL_EXPORTER_OTLP_HEADERS`, `OTEL_SERVICE_NAME`) | unset | optional OTel export, compiled only into the Docker image; inert when unset |

The full annotated list is `.env.example` at the repo root. It doubles as
the template for both install paths.

## Install

### Path A: Docker Compose (fastest)

```sh
git clone https://github.com/norandom/mmhmem.git
cd mmhmem
cp .env.example .env        # edit: token, allowed roots, worker (optional)
docker compose up -d --wait
curl http://localhost:8443/healthz   # expect 200
```

The compose file bind-mounts three host directories: the data dir, the
audit log dir (host-side logrotate owns rotation; the daemon re-creates the
file per write, so rotation never needs a restart), and the workspace roots
(read-only by default; switch `:ro` to `:rw` if `boilerplate_write` should
write inside the container). Keep the mount path in sync with
`AGENT_MEM_ALLOWED_ROOTS`.

### Path B: systemd on a Linux host

Build the release binary, then follow `deploy/README.md`. In short: create
the `agent-memory` system user, copy `deploy/examples/mmhmem.env` to
`/etc/mmhmem/mmhmem.env`, install the binary, the unit
(`deploy/systemd/mmhmem.service`), and the logrotate config
(`deploy/logrotate.d/agent-memory`), then
`systemctl enable --now mmhmem`.

One systemd detail that bites people: the unit hardens the daemon with
`ProtectSystem=strict`, so every entry in `AGENT_MEM_ALLOWED_ROOTS` needs a
matching uncommented `ReadWritePaths=` line in the unit. The daemon both
reads and writes inside those roots.

## Connect your agents

Each harness needs the endpoint URL and the token. Ready-made snippets are
in `deploy/examples/`:

| Harness | Example |
|---|---|
| Claude Code CLI | `deploy/examples/claude-code.md` |
| Google Antigravity | `deploy/examples/antigravity-mcp_config.json` |
| OpenCode | `deploy/examples/opencode.md` |
| Claude Desktop | `deploy/examples/claude-desktop.md` |
| Agent-side OTLP export to OpenObserve | `deploy/examples/agent-otel.md` |

For Claude Code it is one command:

```sh
claude mcp add --transport http --scope user mmhmem \
  http://mmhmem.internal:8443/mcp \
  --header "Authorization: Bearer ${MMHMEM_TOKEN}"
```

Then install the playbook once per machine so every session follows the
same protocol (resume first, prefer `smart_read` for large files, search the
index before fetching details, checkpoint before switching tools):

```sh
mkdir -p ~/.claude/skills/unified-memory
cp deploy/skills/unified-memory/SKILL.md ~/.claude/skills/unified-memory/
```

The playbook is under 200 tokens and works in every supported harness; see
`deploy/skills/unified-memory/README.md` for the per-harness install paths.

## Day-to-day operation

- Liveness: `GET /healthz` (no auth) or `mmhmem healthcheck` from the host.
- Audit: `/var/log/agent-memory/audit.jsonl` (or your `AGENT_MEM_LOG_FILE`).
  One JSON line per action, every line self-verifying via its SHA-256
  digest. Logrotate keeps 90 compressed days at 0640; a commented
  `postrotate` block ships archives to OpenObserve if you want that.
- State: one SQLite file under the data dir (sessions, breadcrumbs, shunt
  cache, observations, token stats). Back it up with any file-level tool.
- Six MCP tools do the work: `session_resume_or_handoff`, `smart_read`,
  `boilerplate_write`, `mem_search`, `mem_get_observations`,
  `mem_record_observation`.

## CI

The pipeline is a Dagger module (`ci/`) with the same entrypoints locally
and in GitHub Actions (`.github/workflows/ci.yml`): fmt, clippy with the
complexity caps on both the default and the `otel` feature build, the full
deterministic test suite, a release build, the Docker image with `--features
otel`, compose-config validation, and pristine systemd/logrotate checks; a
tokei LOC report is informational. An optional model-gated job
(`E2E_LLM_ENABLED=true` plus worker and provider secrets) measures real
token savings with an OpenCode harness, baseline versus mmhmem.

From the dev host, `powershell -NoProfile -File scripts\dagger-ci.ps1` runs
the same pipeline on the WSL build VM.

## Security model

One bearer token, compared in constant time, required on every MCP request;
`/healthz` is the only open route. Every agent-supplied path, including the
`boilerplate_write` reference, is confined to `AGENT_MEM_ALLOWED_ROOTS`
before anything is read or sent anywhere, and a reference outside the roots
is rejected before a single byte reaches the worker model. The audit
redaction filter scrubs key-assigned secrets, bearer headers, Slack tokens,
and `<private>` spans before anything is written. The plain release binary
makes no network calls except to the worker endpoint you configure; the
optional OTel path is compiled only into the Docker image and stays inert
until you point it at your own collector.
