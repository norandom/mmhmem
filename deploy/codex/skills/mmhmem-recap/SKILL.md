---
name: mmhmem-recap
description: Recover the latest mmhmem checkpoint, worker status, and 3-day token savings
---

Call `mmhmem` MCP `session_resume_or_handoff` WITHOUT `updated_working_state_md`, using the same exact project identity and agent_harness: "codex". No worker model or service restart is needed for recovery.

Present a concise, user-focused recap:
1. **Worker & Token Savings**: Report the worker model self-test status and tokens saved over the last 3 days. If the worker model is missing or not answering, warn clearly that **token savings are not active** and why.
2. **Where We Left Off**: Present saved objective, working state, and checkpoint save time separately from timestamped breadcrumbs. If no checkpoint exists, show available breadcrumbs as recorded activity without a saved working-state summary; if neither exists, explicitly report no saved state or recorded activity. An initialized task ID is not a saved checkpoint. Harness chat history is separate: the original conversation may create a checkpoint; do not claim history is missing or automatically imported.

During subsequent work, checkpoint the accepted objective and next step before execution when no checkpoint exists. Save changed state after each task unit, validation batch, plan-changing decision, or blocker; before a final response and an agent-controlled clear/handoff. Skip unchanged saves. Use `updated_working_state_md` and `active_objective` with the same project identity: concise objective, verified versus pending work, decisions, blockers, next steps; no credentials. Await acknowledgement; warn if the latest changes were not saved on failure/unavailability and continue without invented memory.

Ctrl+C or native clear may offer no save opportunity. The last acknowledged checkpoint is the recovery boundary; do not claim interrupted work was saved. Instruction-driven saves depend on agent compliance.
