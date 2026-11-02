---
name: unified-memory
description: Playbook for mmhmem cross-agent memory (local or server) — resume, smart reads, memory index, checkpoint.
---

1. **Resume first.** At session start — and again after any context clear (or via `/mmhmem-recap`) — call `session_resume_or_handoff` to restore the last working state and breadcrumbs.
2. **Smart reads.** Never raw-read large files; use `smart_read` (offset+limit for edits). Offer `shunt_warmup` on new projects' key files to prefill the cache.
3. **Search before details.** `mem_search` first; fetch IDs via `mem_get_observations`; record findings via `mem_record_observation`.
4. **Checkpoint before switching harnesses.** Re-call with `updated_working_state_md` and `active_objective`; include `client_usage` when tokens are reported.
