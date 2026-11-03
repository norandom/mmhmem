//! Shared initialization guidance; protocol metadata only.
pub(super) const INSTRUCTIONS: &str = "mmhmem: cross-agent memory + I/O shunt server. Call session_resume_or_handoff \
             first; use authorized smart_read for ordinary repository inspection before full native/shell reads; \
             use offset/limit for exact edits/verification; summaries are not exact source evidence. \
             Preserve search/build/test/edit workflows and explicit user tool preferences. \
             If MCP/worker is unavailable, use bounded authorized native/targeted fallback; \
             do not retry repeatedly, bypass permissions, or invent savings. Annotations are advisory, \
             not permission grants. Search the memory index before fetching details. Resume at session start or \
             after context clearing, using the same exact project identity throughout. \
             If no checkpoint exists, save the accepted objective and next step before execution. \
             Checkpoint changed state after each task unit, validation batch, plan-changing \
             decision, or blocker; before a final response and before agent-controlled clear \
             or handoff. Skip unchanged saves. Use updated_working_state_md and active_objective \
             for concise objective, verified versus pending work, decisions, blockers, and next \
             steps; exclude credentials. Await acknowledgement; only success clears changed \
             state. On failure or unavailable mmhmem, warn that latest changes were not saved \
             and continue without inventing memory. Ctrl+C/native clear cannot guarantee a save; \
             recover the last acknowledged checkpoint without a service restart or worker model. \
             Present checkpoint save time separately from breadcrumb activity. If no checkpoint \
             exists, show available activity or explicit absence; initialized task IDs are not \
             saved checkpoints. Harness chat history is separate: the original conversation may \
             create a checkpoint, but history is not automatically imported.";
