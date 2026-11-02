---
description: Pull the last mmhmem session recap after a context clear
---

Call the `mmhmem` MCP tool `session_resume_or_handoff` WITHOUT `updated_working_state_md` — project: the current project name; agent_harness: "claude-code". Then present the returned working state, objective, and breadcrumbs as a concise recap of the last session. If the response says no active session was found, say so in one line and stop.
