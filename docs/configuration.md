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
global file > default**. Only `worker_model`, `worker_api_url`,
`worker_api_key`, `update_check` and `shunt_content_cache_max_rows` are
read. Unknown keys are ignored and
preserved by updates. Invalid JSON fails startup and names the file.

`mmhmem setup` never stores worker secrets in agent registrations; keep them
in this file. Tick **Worker model** in setup to be shown how to connect one.
Keep keys out of version control; project `.mmhmem.json` is gitignored.

## Update notice

When a newer stable mmhmem release exists, the session-start answer for an
agent ends with one line such as `mmhmem v0.5.2 is available (installed:
v0.5.1). To update, run in a terminal: mmhmem update`. It is only a note:
nothing is downloaded or installed, and `mmhmem update` still checks the
signature and checksum. The release lookup runs in the background at most
once a day when `mmhmem stdio` starts and is kept in `update-check.json` in
the data directory; a tool call never waits for it, and without network
nothing changes. Server mode (`mmhmem serve`) never shows the note. To turn
it off, set `MMHMEM_NO_UPDATE_CHECK=1` or add `"update_check": false` to
`.mmhmem.json`.

## Local environment reference

| Variable | Default | Purpose |
|---|---|---|
| `AGENT_MEM_DATA_DIR` | Per-user directory | Relocate durable state |
| `AGENT_MEM_LOG_FILE` | `<data dir>/audit.jsonl` | Relocate the audit file |
| `AGENT_MEM_ALLOWED_ROOTS` | Unset | Optional comma-separated workspace confinement roots |
| `AGENT_MEM_BUSY_TIMEOUT_MS` | `5000` | Shared-store lock wait in milliseconds |
| `AGENT_MEM_AUDIT_MAX_BYTES` | `10485760` | Local audit rotation threshold |
| `AGENT_MEM_AUDIT_ROTATE_KEEP` | `5` | Rotated audit files to retain |
| `SHUNT_MIN_LINES` | `150` | Minimum file length for worker summarization and for the read hook's redirect (whole files and line ranges) |
| `SHUNT_CONTENT_CACHE_MAX_ROWS` | `5000` | Row cap for the `shunt_content` cache (`.mmhmem.json` key `shunt_content_cache_max_rows`); oldest-hit rows are evicted beyond it |
| `WORKER_API_URL`, `WORKER_MODEL`, `WORKER_API_KEY` | Unset | Override worker-file settings |
| `MMHMEM_NO_UPDATE_CHECK` | Unset | `1` turns off the update notice and its release lookup |

With roots unset, local tools can access paths available to your user. Set
roots to enforce confinement; denied accesses are audited. The local process
opens no listener and needs no bearer token. Its outbound connections are
the worker endpoint you configure and, unless turned off, the once-a-day
release lookup on GitHub for the [update notice](#update-notice).

HTTPS workers retain the client's existing distribution trust-store behavior.
Trust-store administration is outside mmhmem's release setup. Remote
deployment uses separate [environment-only configuration](server.md#configuration).
