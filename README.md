# mmhmem

<p align="center">
  <img src="assets/mmhmem.jpg" alt="mmhmem motto" width="560">
</p>

A self-hosted memory and I/O-optimization layer for AI coding agents. One
binary gives Claude Code, Google Antigravity, OpenCode, and Claude Desktop a
shared place to keep live session state, so you can switch tools mid-task
without losing context, and so bulk file reads stop costing frontier-model
tokens.

mmhmem runs in two modes from the same binary:

- **Local mode (the default install, no server).** Each harness spawns
  `mmhmem stdio` as a child process. Zero configuration: no token, no
  daemon, no network. One per-user store holds sessions, the shunt cache,
  project memory, and token stats — every harness on the machine shares
  it, so checkpointing in your terminal agent and resuming in your IDE
  works with nothing installed in between. `mmhmem stats` shows your
  savings any time.
- **Serve mode (the optional upgrade).** One Linux daemon serves all the
  same nine tools over Streamable HTTP with bearer auth for cross-machine
  access, central audit/compliance, central ops (logrotate, OTel), and
  multi-user deployments.

Agents connect through standard protocol-native interfaces; a portable
SKILL.md playbook teaches them the workflow; every action lands in a
redacted, hash-verified audit log. No SaaS, no telemetry, no proxy between
your agents and their model providers.

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
| mmhmem binary | yes | download the prebuilt asset from the latest release (Linux x86_64, macOS Apple Silicon and Intel, Windows x64) or build with Rust (pinned in `rust-toolchain.toml`, `cargo build --release`); SQLite (WAL + FTS5) is compiled in, there are no runtime dependencies |
| Worker model endpoint | optional | any OpenAI-compatible chat-completions endpoint (Ollama, vLLM, or an API key). Without it, shunting is off; session memory, project memory, and targeted reads all still work |
| Linux host or Docker | only for serve mode | the local layer needs nothing but the binary; the server adds systemd on a Linux host, or any Docker host for the compose route |
| Reverse proxy | optional, recommended for serve-mode TLS | the daemon serves plain HTTP; terminate TLS in front of it |
| OpenObserve or any OTLP endpoint | optional | for OTel traces/metrics/logs and the logrotate batch ship of audit archives |

## Quickstart: local mode (no server)

**Install from the latest release** — no Rust toolchain, no `git clone`.
Releases are cut irregularly; the latest one is always the tested variant to
install. Every platform ships as a release asset with a stable name, so the
`releases/latest/download/…` URLs below always fetch the newest tested build:

| Platform | Asset |
|---|---|
| Linux (x86_64) | `mmhmem-linux-x86_64` |
| macOS (Apple Silicon) | `mmhmem-macos-aarch64` |
| macOS (Intel) | `mmhmem-macos-x86_64` |
| Windows (x64) | `mmhmem.exe` |

The installer needs only itself and four companion files (the playbook and the
recap command templates) — fetched raw, still no checkout:

**Linux / macOS**

```sh
# 1) binary for your platform
curl -fsSL -o mmhmem \
  https://github.com/norandom/mmhmem/releases/latest/download/mmhmem-linux-x86_64
# macOS Apple Silicon: .../mmhmem-macos-aarch64, Intel: .../mmhmem-macos-x86_64
chmod +x mmhmem

# 2) installer + companion files (raw fetch, no checkout)
d=$(mktemp -d)
b=https://raw.githubusercontent.com/norandom/mmhmem/HEAD/deploy
mkdir -p "$d/deploy/skills/unified-memory" "$d/deploy/commands/claude" \
         "$d/deploy/commands/opencode" "$d/deploy/commands/codex"
curl -fsSL -o "$d/deploy/install_local.sh"                  "$b/install_local.sh"
curl -fsSL -o "$d/deploy/skills/unified-memory/SKILL.md"    "$b/skills/unified-memory/SKILL.md"
curl -fsSL -o "$d/deploy/commands/claude/mmhmem-recap.md"   "$b/commands/claude/mmhmem-recap.md"
curl -fsSL -o "$d/deploy/commands/opencode/mmhmem-recap.md" "$b/commands/opencode/mmhmem-recap.md"
curl -fsSL -o "$d/deploy/commands/codex/mmhmem-recap.md"     "$b/commands/codex/mmhmem-recap.md"

# 3) install for the harnesses you use (here: Claude Code, user-wide)
sh "$d/deploy/install_local.sh" --binary "$PWD/mmhmem" --global --agent claude
```

**Windows**

```powershell
# 1) binary
irm https://github.com/norandom/mmhmem/releases/latest/download/mmhmem.exe -OutFile mmhmem.exe

# 2) installer + companion files (raw fetch, no checkout)
$d = "$env:TEMP\mmhmem-install"
$b = 'https://raw.githubusercontent.com/norandom/mmhmem/HEAD/deploy'
New-Item -ItemType Directory -Force -Path "$d\deploy\skills\unified-memory",
    "$d\deploy\commands\claude", "$d\deploy\commands\opencode", "$d\deploy\commands\codex" | Out-Null
irm "$b/install_local.ps1" -OutFile "$d\deploy\install_local.ps1"
irm "$b/skills/unified-memory/SKILL.md" -OutFile "$d\deploy\skills\unified-memory\SKILL.md"
irm "$b/commands/claude/mmhmem-recap.md" -OutFile "$d\deploy\commands\claude\mmhmem-recap.md"
irm "$b/commands/opencode/mmhmem-recap.md" -OutFile "$d\deploy\commands\opencode\mmhmem-recap.md"
irm "$b/commands/codex/mmhmem-recap.md" -OutFile "$d\deploy\commands\codex\mmhmem-recap.md"

# 3) install for the harnesses you use (here: Claude Code, user-wide)
powershell -File "$d\deploy\install_local.ps1" -Binary "$PWD\mmhmem.exe" -Global -Agent claude
```

`HEAD` in the raw URLs is the repo's default branch under whatever name, so
the fetches track it automatically.

Or build it yourself (`cargo build --release`) and let the installer find the
binary in `target/release`. The installer registers every harness it finds
(Claude Code, OpenCode, Claude Desktop, Antigravity, Codex), asks whether to install
globally or per project and which harnesses, ships the `/mmhmem-recap` command
for Claude Code and OpenCode, writes worker settings into `.mmhmem.json`
(`-WorkerModel` / `--worker-model` and friends), and undoes itself with
`--remove` / `-Remove`:

```sh
sh deploy/install_local.sh                   # macOS / Linux
powershell -File deploy\install_local.ps1    # Windows
```

That is the whole install for Claude Code; OpenCode, Antigravity, and
Claude Desktop have equivalent one-liners in `deploy/examples/` (the
`*-local` files). The per-user store is created on first use —
`~/.local/share/mmhmem` on Linux, `%APPDATA%\mmhmem` on Windows,
`~/Library/Application Support/mmhmem` on macOS. No environment variables
are required; optional ones (worker model, data-dir override, audit
rotation) are listed under Configure below.

Check what the layer saved you:

```sh
mmhmem stats            # per day, project, and harness
mmhmem stats --json     # machine-readable
```

Install the playbook once per machine (see Connect your agents), and
you have the full efficiency layer without a server anywhere.

## Configure

**Local mode is env-optional**: nothing is required. The per-user defaults
cover a fresh machine; every variable is still honored when set.

| Variable | Default (local) | Meaning |
|---|---|---|
| `AGENT_MEM_DATA_DIR` / `AGENT_MEM_LOG_FILE` | per-OS user dir / `<data dir>/audit.jsonl` | relocate the store or the audit file |
| `AGENT_MEM_ALLOWED_ROOTS` | unset = unconfined | comma-separated workspace roots; when set, reads/writes are confined and denials are audited |
| `AGENT_MEM_BUSY_TIMEOUT_MS` | `5000` | how long concurrent local processes wait for the shared store before erroring |
| `AGENT_MEM_AUDIT_MAX_BYTES` / `AGENT_MEM_AUDIT_ROTATE_KEEP` | `10485760` / `5` | local audit rotation threshold and retention (no logrotate needed on user machines) |
| `SHUNT_MIN_LINES` / `WORKER_API_URL` / `WORKER_MODEL` / `WORKER_API_KEY` | `150` / unset | the shunt threshold and worker model |

### Worker settings file: `.mmhmem.json` (local mode)

Local mode also reads a small JSON config for the **worker settings only**:

- `~/.mmhmem.json` — your global worker config (one place for all projects)
- `./.mmhmem.json` — the project's own worker config (gitignored — it can hold your API key)

```json
{ "worker_model": "mistral-small-2603", "worker_api_url": "https://api.mistral.ai/v1/chat/completions", "worker_api_key": "…" }
```

- Every setting is optional; per setting the precedence is **environment
  variable > project file > global file > built-in default** — so a project
  can pin a model while the key stays in the global file.
- Only `worker_model`, `worker_api_url`, `worker_api_key` are read; unknown
  keys are ignored, other keys are preserved when the installer updates it.
- Serve mode is untouched (env-only). A file that exists but is not valid
  JSON fails fast at startup, naming the file.
- The local installers write this file (`-WorkerModel` / `-WorkerUrl` /
  `-WorkerApiKey`, or `--worker-*` in `install_local.sh`) instead of embedding
  worker env into harness registrations; the key is never echoed.
- The file is in `.gitignore`; keep the API key out of version control.

**Serve mode is env-only and fails fast at startup.** Two variables are
required:

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

## Install the server (optional upgrade)

Reach for serve mode when you need cross-machine access, central audit and
compliance, central ops, or multiple users behind one bearer token. Both
paths below start the same daemon (`mmhmem serve`, also the no-argument
default).

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

Local mode (spawned stdio child, no token) and serve mode (Streamable
HTTP with bearer auth) both connect with ready-made snippets in
`deploy/examples/`:

| Harness | Local mode (stdio) | Serve mode (HTTP) |
|---|---|---|
| Claude Code CLI | `claude mcp add --transport stdio --scope user mmhmem -- mmhmem stdio` | `deploy/examples/claude-code.md` |
| OpenCode | `deploy/examples/opencode-local.json` | `deploy/examples/opencode.md` |
| Google Antigravity | `deploy/examples/antigravity-mcp_config-local.json` | `deploy/examples/antigravity-mcp_config.json` |
| Claude Desktop | `deploy/examples/claude-desktop.md` (local section) | `deploy/examples/claude-desktop.md` |
| Agent-side OTLP export to OpenObserve | — | `deploy/examples/agent-otel.md` |

For Claude Code against a server it is one command:

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
curl -fsSL -o ~/.claude/skills/unified-memory/SKILL.md \
  https://raw.githubusercontent.com/norandom/mmhmem/HEAD/deploy/skills/unified-memory/SKILL.md
# (or, from a checkout: cp deploy/skills/unified-memory/SKILL.md ~/.claude/skills/unified-memory/)
```

The playbook is under 200 tokens and works in every supported harness; see
`deploy/skills/unified-memory/README.md` for the per-harness install paths.

## Day-to-day operation

- Savings: `mmhmem stats` (human table) or `mmhmem stats --json`, from the
  per-user store — no server needed.
- Warmup: on a new project, have the agent call `shunt_warmup` on its key
  files (max 25 per batch) — real worker summaries cached up front, so the
  first reads of those files are already cheap.
- Prune: `mmhmem prune --dry-run` previews stale cache rows (file gone or
  content changed), `mmhmem prune` removes exactly those — the same logic
  the `shunt_prune` tool offers agents.
- After a context clear: `/mmhmem-recap` (shipped by the installers for
  Claude Code and OpenCode) pulls the last session recap — working state,
  objective, and breadcrumbs — on demand, no hooks.
- Liveness (serve mode): `GET /healthz` (no auth) or `mmhmem healthcheck`
  from the host.
- Audit: serve mode writes `/var/log/agent-memory/audit.jsonl` (or your
  `AGENT_MEM_LOG_FILE`) rotated by logrotate; local mode writes
  `<data dir>/audit.jsonl` with built-in size rotation. One JSON line per
  action, every line self-verifying via its SHA-256 digest.
- State: one SQLite file under the data dir (sessions, breadcrumbs, shunt
  cache, observations, token stats). Back it up with any file-level tool.
- Nine MCP tools do the work: `session_resume_or_handoff`, `smart_read`,
  `boilerplate_write`, `mem_search`, `mem_get_observations`,
  `mem_record_observation`, `shunt_warmup`, `shunt_prune`,
  `mem_search_breadcrumbs` (keyword search over the project's breadcrumb
  history — what earlier sessions did beyond the last 15 crumbs).

## CI

The pipeline is a Dagger module (`ci/`) with the same entrypoints locally
and in GitHub Actions (`.github/workflows/ci.yml`): fmt, clippy with the
complexity caps on both the default and the `otel` feature build, the full
deterministic test suite, a release build, the Docker image with `--features
otel`, compose-config validation, and pristine systemd/logrotate checks; a
tokei LOC report is informational. Windows and macOS jobs run the same
blocking clippy and test suite plus release builds on native runners and
upload the per-OS binaries as artifacts. An optional model-gated job
(`E2E_LLM_ENABLED=true` plus worker and provider secrets) measures real
token savings with an OpenCode harness, baseline versus mmhmem.

From the dev host, `powershell -NoProfile -File scripts\dagger-ci.ps1` runs
the same pipeline on the WSL build VM.

## Security model

**Local mode** is a spawned child inside your user session: it opens no
network socket, requires no token (the harness that spawned it is the trust
boundary), and stores everything under your user account. The only
outbound connection is the optional worker model endpoint. The audit
trail and redaction are identical to serve mode; `AGENT_MEM_ALLOWED_ROOTS`
is optional but enforced when set, and every denial is audited.

**Serve mode**: one bearer token, compared in constant time, required on
every MCP request; `/healthz` is the only open route. Every agent-supplied
path, including the `boilerplate_write` reference, is confined to
`AGENT_MEM_ALLOWED_ROOTS` before anything is read or sent anywhere, and a
reference outside the roots is rejected before a single byte reaches the
worker model. The audit redaction filter scrubs key-assigned secrets,
bearer headers, Slack tokens, and `<private>` spans before anything is
written. The plain release binary makes no network calls except to the
worker endpoint you configure; the optional OTel path is compiled only
into the Docker image and stays inert until you point it at your own
collector.
