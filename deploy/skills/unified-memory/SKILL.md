---
name: unified-memory
description: Playbook for the mmhmem cross-agent memory server — resume, smart reads, memory index, checkpoint.
---

1. **Resume first.** At session start call `session_resume_or_handoff` with your project name and harness to load working state and breadcrumbs.
2. **Smart reads.** Never raw-read large files. Use `smart_read`; pass `offset` and `limit` to get exact numbered lines when editing.
3. **Search before details.** Run `mem_search` first; fetch only the IDs you need with `mem_get_observations`; record durable findings via `mem_record_observation`.
4. **Checkpoint before switching harnesses.** Call `session_resume_or_handoff` with `updated_working_state_md` and `active_objective`; pass `client_usage` when the harness reports tokens.
