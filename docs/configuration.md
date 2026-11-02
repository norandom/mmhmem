# Configuration

Local mode needs no mandatory environment variables. Every agent uses the
same per-user defaults; [installation](installation.md) lists store locations.

## Worker settings

A worker model is needed for large-file summaries and boilerplate generation.
It can be any OpenAI-compatible chat-completions endpoint. Session memory,
project memory and targeted reads work without one.

Create `~/.mmhmem.json` for all projects, or `./.mmhmem.json` for one project:

```json
{
  "worker_model": "your-model",
  "worker_api_url": "http://localhost:11434/v1/chat/completions",
  "worker_api_key": ""
}
```

Use the model name and URL supplied by your provider; hosted providers may
require an API key. Per-setting precedence is **environment > project file >
global file > default**. Only `worker_model`, `worker_api_url` and
`worker_api_key` are read. Unknown keys are ignored and preserved by installer
updates. Invalid JSON fails startup and names the file.

Installer options `--worker-model`, `--worker-url` and `--worker-api-key`
(PowerShell: `-WorkerModel`, `-WorkerUrl`, `-WorkerApiKey`) write this file
instead of storing worker secrets in agent registrations. Keep keys out of
version control; project `.mmhmem.json` is gitignored.

## Local environment reference

| Variable | Default | Purpose |
|---|---|---|
| `AGENT_MEM_DATA_DIR` | Per-user directory | Relocate durable state |
| `AGENT_MEM_LOG_FILE` | `<data dir>/audit.jsonl` | Relocate the audit file |
| `AGENT_MEM_ALLOWED_ROOTS` | Unset | Optional comma-separated workspace confinement roots |
| `AGENT_MEM_BUSY_TIMEOUT_MS` | `5000` | Shared-store lock wait in milliseconds |
| `AGENT_MEM_AUDIT_MAX_BYTES` | `10485760` | Local audit rotation threshold |
| `AGENT_MEM_AUDIT_ROTATE_KEEP` | `5` | Rotated audit files to retain |
| `SHUNT_MIN_LINES` | `150` | Minimum file length for worker summarization |
| `WORKER_API_URL`, `WORKER_MODEL`, `WORKER_API_KEY` | Unset | Override worker-file settings |

With roots unset, local tools can access paths available to your user. Set
roots to enforce confinement; denied accesses are audited. The local process
opens no listener and needs no bearer token. Its only outbound connection is
the worker endpoint you configure.

HTTPS workers retain the client's existing distribution trust-store behavior.
Trust-store administration is outside mmhmem's release setup. Remote
deployment uses separate [environment-only configuration](server.md#configuration).
