# Proactive session checkpoints

## Problem and evidence

An OpenCode user works normally and exits with Ctrl+C or clears context.
The installed guidance currently asks for a checkpoint before switching
harnesses, but neither action guarantees an opportunity to run that step.

The reported `spec-kit-ears-tdd` case had an existing OpenCode conversation,
two mmhmem resume breadcrumbs, and no mmhmem active-session checkpoint.
Inspection of the earlier conversation found a resume call without a
checkpoint payload. Its chat history survived; it was never converted into
mmhmem working state. This was not a local `.opencode` storage split.

## Intended change

Make checkpointing a routine part of agent work: save the initial objective,
save meaningful milestones, and save changed state before yielding a final
response. Restore the latest confirmed checkpoint after interruption or
context clearing. The user should not need to remember a separate save step.

## Boundary candidates

- Portable agent guidance, installed recap commands, their packaged copies,
  and regression evidence for checkpoint/resume workflows.
- OpenCode is the motivating case; equivalent guidance applies to every
  harness that already receives the unified-memory playbook.
- Ctrl+C can interrupt immediately. Recovery covers the last acknowledged
  checkpoint, not arbitrary unsaved work after it.
- A native `/clear` may be consumed entirely by the harness. Pre-clear saving
  is required only when the agent receives advance notice and can still act.
- No client hooks, signal interception, background transcript ingestion,
  harness-service restart requirement, or worker-model dependency.
- Agent compliance remains a limitation of this protocol-native approach.
  Guaranteed saving on every keypress would require a separate integration
  decision and is not silently promised by this follow-up.

## Existing specification touchpoints

- `corpus-warmup-prune`, Requirement 9: recap commands and clear-context wording.
- `local-mode`, Requirement 5: shared per-user state and cross-harness handoff.
- `cross-agent-memory-server`: existing session checkpoint/resume contract.

## Workflow

Requirements are drafted for human review. Design, tasks, and implementation
remain unapproved. This follow-up does not reopen completed runtime features.
