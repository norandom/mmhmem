# Daily use

The installed playbook guides agents through this workflow:

1. Resume the project session before starting work.
2. Prefer `smart_read` for large files; use offset and limit for exact edits.
3. Search the memory index, then fetch details for relevant observation IDs.
4. Save the accepted objective and next step before execution if no checkpoint exists.
5. Checkpoint changed state after each task unit, validation batch, plan-changing
   decision, or blocker, and before returning a final response or initiating a
   clear/handoff. Unchanged state needs no duplicate save.

Use the same exact project identity for resume and checkpoint. A concise
checkpoint contains the objective, verified and pending work, decisions,
blockers, and next steps. Keep credentials out. Only a successful acknowledgement
confirms saving; on failure the agent should warn that the latest changes were
not saved and continue without inventing memory. These instruction-driven
saves depend on agent compliance.

Ctrl+C and native context clearing may offer no opportunity to save. Recovery
uses the last acknowledged checkpoint; activity after it may not be captured
as working state. Neither exiting nor clearing automatically saves a chat.

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

After a context clear, `/mmhmem-recap` retrieves the latest checkpoint without
a service restart or worker model. The installers supply recap guidance for
Claude Code, OpenCode, and Codex without client-side hooks. Checkpoint save time
and timestamped breadcrumbs are shown separately: breadcrumbs record activity
and do not reconstruct an unsaved working-state summary.

If no checkpoint exists, recap shows available activity or explicitly reports
no saved state or recorded activity. A new task ID does not mean state was saved.
Harness chat history is separate from mmhmem: reopen the original conversation
to create a checkpoint from it. Recap does not import that history or establish
whether it exists.

The new recovery timestamps and complete history-only responses require an
updated binary; proactive save cadence requires updated skills/commands.
Refresh them through the installer, then use the harness's normal restart or
reconnect lifecycle to load updates. Recovery itself requires no restart and
there is no store migration or new service configuration.

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
| `status` | Report version and store health (used by the installer's check) |

## State and audit

The data directory holds SQLite state for sessions, breadcrumbs, cache,
observations and statistics. Stop active mmhmem processes before copying
the directory as a filesystem backup.

Local audit logs use built-in rotation. Each JSON line has a SHA-256 digest;
secrets are redacted before writing, including bearer headers, key-assigned
secrets, Slack tokens and `<private>` spans. See [configuration](configuration.md)
for locations and rotation settings. Central deployment operations are in
the [server guide](server.md).
