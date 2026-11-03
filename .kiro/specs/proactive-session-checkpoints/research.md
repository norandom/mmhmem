# Research and Design Decisions

## Summary

- Feature: proactive-session-checkpoints.
- Discovery scope: existing-system extension, light integration discovery.
- Skills consulted: kiro-spec-design, design principles, light discovery,
  synthesis and design review gate. Core steering reused from current context.
- No external APIs or dependencies are introduced; repository contracts are
  the primary evidence. No web research is needed for this extension.

## Research Log

### Save and recovery contract

Sources: `src/services/session_bus.rs`, `src/storage/schema.rs`,
`src/mcp/tools.rs`, `tests/handoff.rs`, `tests/local_stdio.rs`,
`tests/multiprocess.rs`.

Resume-only calls create a task ID and breadcrumb, not an active-session
checkpoint. The stored row already includes `updated_at_epoch`, but it is
not loaded into ResumeView. Breadcrumbs are loaded even without a checkpoint,
then hidden by an early MCP return. Current test helpers parse the whole
response tail as a task ID and must be updated when response sections expand.

### Failed acknowledgement versus persisted state

Checkpoint currently upserts the row, then inserts a breadcrumb, then writes
audit. A failure after the upsert can replace the previous checkpoint while
returning an error. Approved requirement 4.2 therefore needs a bounded
checkpoint-transaction change, not just new wording. The synchronous
AuditWriter can be called from the existing database connection closure.
An audit append and SQLite commit cannot be made jointly atomic without a
larger redesign; database recovery is authoritative after a lost reply.

### Guidance and packaging

Sources: `deploy/skills/unified-memory/SKILL.md`, the three
`deploy/commands/*/mmhmem-recap.md` templates, `src/mcp/tools.rs`, and
`scripts/check-marketplace.py`.

Guidance currently saves only before switching harnesses. Recap templates
stop on missing checkpoints. Marketplace CI enforces an estimated 200-token
playbook cap and byte parity with the generated package. Existing installers
already copy all affected artifacts, so no new installer behavior is needed.

## Architecture Pattern Evaluation

| Option | Decision | Reason |
|---|---|---|
| Strengthen existing guidance and recovery contract | Selected | Small protocol-native extension; no new runtime prerequisite |
| Exit hooks or native clear interception | Rejected | Conflicts with steering and cannot cover arbitrary abrupt termination |
| Import OpenCode transcripts automatically | Deferred | Separate privacy, ownership and harness-version integration scope |
| Background checkpoint scheduler | Rejected | Server cannot summarize client work that has not been reported |

## Design Decisions

- Generalize Ctrl+C, clear, and harness switching as recovery from the latest
  persisted checkpoint; do not implement three lifecycle adapters.
- Reuse the existing checkpoint tool, database fields, audit writer, `time`
  dependency and package builders rather than adding libraries or a schema.
- Keep rendering in one private formatter module to respect tool-router LOC
  limits; keep persistence in SessionBus rather than adding a repository layer.
- Use one checkpoint transaction for state and breadcrumb, with audit before
  commit. Do not treat an audit line alone as a successful save receipt.
- Present timestamped activity separately from saved state. Equal timestamps
  and the checkpoint's own breadcrumb cannot prove later unsaved work.

## Risks and Mitigations

- Models can ignore guidance: disclose this limit; distinguish instruction
  checks from live behavior and avoid an automatic-save guarantee.
- Audit/commit or commit/reply gaps: deterministic rollback tests, explicit
  acknowledgement semantics, and authoritative recovery from committed state.
- Text consumers can parse task IDs too broadly: update known extraction tests.
- Mixed installed versions: release binary and refresh guidance together;
  retain old-binary compatibility but disclose missing newer recovery metadata.

## Review Gate

Requirements coverage, concrete file boundaries, allowed dependencies,
revalidation triggers, persistence failure paths and rollout were reviewed.
The persistence-ordering issue was resolved within approved requirement 4.2.
All 19 acceptance criteria have explicit design/test mappings. No new scope
approval is assumed; design approval and task generation remain pending.

## Task-plan review (2026-11-03)

- Design approved by the user before task generation.
- Applied kiro-spec-tasks generation and parallel-analysis rules.
- Six executable major tasks, no nested subtasks or parallel markers. Shared
  response/guidance ownership and downstream validation justify sequential order.
- Task-plan gate: PASS; all 19 acceptance criteria mapped, explicit integration
  boundaries and observable deliverables present, existing infrastructure reused.
- Independent task-graph sanity review: PASS. Reviewer read requirements,
  design, generation rules, and the draft directly. No blocking findings.
  The review's reported criterion count was corrected to 19 by mechanical coverage.
- Tasks generated but not approved; implementation remains pending task approval.
