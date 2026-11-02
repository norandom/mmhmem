# Daily use

The installed playbook guides agents through this workflow:

1. Resume the project session before starting work.
2. Prefer `smart_read` for large files; use offset and limit for exact edits.
3. Search the memory index, then fetch details for relevant observation IDs.
4. Checkpoint working state before switching agents.

The latest 15 tool actions per project form a breadcrumb trail. Handoffs do
not call a language model. Large-file summaries are cached by content hash;
another agent reading unchanged content can reuse the summary without a
worker call. Authentication, cryptography and concurrency paths bypass the
worker, as do targeted reads.

## Statistics and cache maintenance

```sh
mmhmem stats
mmhmem stats --json
mmhmem prune --dry-run
mmhmem prune
```

Statistics are grouped by day, project and agent. Estimated savings and
client-reported values remain separately labelled. Pruning removes stale
cache rows for missing or changed files; preview with `--dry-run` first.
Ask your agent to call `shunt_warmup` for key project files, up to 25 per batch,
to prepare summaries before the first read.

After a context clear, `/mmhmem-recap` retrieves working state, objective and
breadcrumbs. The installers supply it for Claude Code and OpenCode without
client-side hooks.

## MCP tools

| Tool | Purpose |
|---|---|
| `session_resume_or_handoff` | Resume or checkpoint working state |
| `smart_read` | Read exact lines or summarize/cache large files |
| `boilerplate_write` | Generate files through the worker |
| `mem_search` | Search the compact memory index |
| `mem_get_observations` | Fetch selected observation details |
| `mem_record_observation` | Store a project observation |
| `shunt_warmup` | Prepare summaries in a batch |
| `shunt_prune` | Remove stale cache entries |
| `mem_search_breadcrumbs` | Search project action history |

## State and audit

The data directory holds SQLite state for sessions, breadcrumbs, cache,
observations and statistics. Stop active mmhmem processes before copying
the directory as a filesystem backup.

Local audit logs use built-in rotation. Each JSON line has a SHA-256 digest;
secrets are redacted before writing, including bearer headers, key-assigned
secrets, Slack tokens and `<private>` spans. See [configuration](configuration.md)
for locations and rotation settings. Central deployment operations are in
the [server guide](server.md).
