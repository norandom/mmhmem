# Optional shared server

Deploy a shared server for cross-machine memory, central audit or multiple
users. Local clients on Linux, Windows and macOS connect through the same
Streamable HTTP MCP interface with bearer authentication.

The daemon serves plain HTTP. An administrator owns any reverse proxy/load
balancer and TLS termination; their configuration is outside this guide.

## Docker Compose

```sh
git clone https://github.com/norandom/mmhmem.git
cd mmhmem
cp .env.example .env
# Edit .env: set the token, allowed roots and optional worker settings.
docker compose up -d --wait
curl http://localhost:8443/healthz
```

The compose stack mounts data, audit logs and workspace roots. Workspace
mounts default to read-only; use `:rw` when `boilerplate_write` should write.
Keep mount paths consistent with `AGENT_MEM_ALLOWED_ROOTS`.

For systemd and logrotate on a Linux host, follow the
[deployment guide](../deploy/README.md#1-systemd--logrotate-linux-host).
The hardened unit needs a corresponding `ReadWritePaths=` entry for each
allowed workspace root that should be writable.

## Configuration

Server configuration is environment-only; `.mmhmem.json` is not read.
`AGENT_MEM_TOKEN` and `AGENT_MEM_ALLOWED_ROOTS` are mandatory. Startup fails
if either is absent or invalid.

| Variable | Default | Purpose |
|---|---|---|
| `AGENT_MEM_PORT`, `AGENT_MEM_MCP_PATH` | `8443`, `/mcp` | Listener port and MCP path |
| `AGENT_MEM_DATA_DIR` | `/var/lib/agent-memory` | Durable state |
| `AGENT_MEM_LOG_FILE` | `/var/log/agent-memory/audit.jsonl` | Audit output |
| `WORKER_API_URL`, `WORKER_MODEL`, `WORKER_API_KEY` | Unset | Optional worker endpoint |
| `SHUNT_MIN_LINES` | `150` | Summarization threshold |
| `OTEL_EXPORTER_OTLP_ENDPOINT` | Unset | Optional Docker-image OTel export |

See [`.env.example`](../.env.example) for the full annotated configuration.
The GNU/OTel Docker image is separate from the default static musl Linux
release; exports remain inactive until a collector is configured.

## Connect agents

For Claude Code:

```sh
claude mcp add --transport http --scope user mmhmem \
  http://mmhmem.internal:8443/mcp \
  --header "Authorization: Bearer ${MMHMEM_TOKEN}"
```

Other examples: [OpenCode](../deploy/examples/opencode.md),
[Antigravity](../deploy/examples/antigravity-mcp_config.json),
[Claude Desktop](../deploy/examples/claude-desktop.md), and
[agent OTel export](../deploy/examples/agent-otel.md). Install the
[workflow playbook](../deploy/skills/unified-memory/README.md) once per client machine.

Every MCP request requires the bearer token; `/healthz` is open for health
checks. Paths are confined to allowed roots before reading or sending data to
the worker. Audit logs use host-side logrotate and reopen on each append, so
rotation needs no restart. `mmhmem healthcheck` probes the local health endpoint.
