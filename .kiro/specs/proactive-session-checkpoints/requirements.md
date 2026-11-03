# Requirements Document

## Introduction

Engineers using OpenCode and other mmhmem-connected agents interrupt work
with Ctrl+C or clear context without issuing a separate save instruction.
The current playbook only asks agents to checkpoint before switching
harnesses. Consequently, a substantial chat can exist without any resumable
mmhmem checkpoint. This follow-up makes checkpointing part of normal agent
work and makes recovery accurately describe what was saved.

## Boundary Context

- **In scope:** proactive agent guidance, checkpoint acknowledgement and
  recovery presentation, recap commands, matching installed/package guidance,
  and focused regression coverage. OpenCode is the primary acceptance case.
- **Out of scope:** intercepting Ctrl+C or native `/clear`, client hooks,
  background transcript ingestion, importing historical chats, continuous
  autosave, and redesigning storage or project identity.
- **Adjacent expectations:** use existing project-scoped checkpoint/resume
  operations, shared per-user storage, redaction, and checkpoint size limits.
  Preserve the playbook token budget and packaged-skill parity checks.
- **Guarantee:** persisted checkpoints remain recoverable; instruction-driven
  checkpoint frequency depends on agent compliance. Work after the last
  acknowledged checkpoint may be lost when interrupted. No exit handler or
  service restart is required or assumed.

## Requirements

### Requirement 1: Checkpoint during ordinary work
**Objective:** As an agent user, I want progress saved during work so that I do not have to remember a separate save command.

#### Acceptance Criteria
1. When an agent begins a task and no prior checkpoint exists, the agent guidance shall instruct it to save the accepted objective and next step before beginning task execution.
2. When an agent completes a task unit, finishes a validation batch, makes a decision that changes the plan, or encounters a blocker that changes the next step, the agent guidance shall instruct it to checkpoint the changed working state before proceeding to the next unit of work.
3. When an agent is about to return a final response and resumable state has changed since its last acknowledged checkpoint, the agent guidance shall instruct it to save that state before yielding to the user.
4. If resumable state has not changed since the last acknowledged checkpoint, the agent guidance shall not require a duplicate checkpoint solely because a response is ending.
5. When saving a checkpoint, the agent guidance shall require the same project identity used to resume and a concise record of the objective, completed work, significant decisions, blockers, and next steps, with pending work distinguished from verified results.

### Requirement 2: Honest interruption and clear-context behavior
**Objective:** As a user who exits with Ctrl+C or clears context, I want recovery expectations to match what was actually persisted.

#### Acceptance Criteria
1. When an agent receives advance notice of a handoff or context clear and can still act, the agent guidance shall instruct it to checkpoint changed state before initiating that transition.
2. When the user interrupts execution or clears context without an agent-controlled save opportunity, the documented workflow shall identify the last acknowledged checkpoint as the recovery boundary and shall not claim that the interrupted work was saved.
3. When the agent starts with fresh context or the user invokes recap, the recovery workflow shall retrieve the latest checkpoint for the same project without requiring a service restart or a worker model.
4. When recovery returns a checkpoint, the recovery presentation shall identify its save time and distinguish its saved working state from any later recorded breadcrumbs.

### Requirement 3: Useful recovery without a checkpoint
**Objective:** As a returning user, I want an accurate explanation when chat history exists but no mmhmem checkpoint was saved.

#### Acceptance Criteria
1. If no checkpoint exists but project breadcrumbs exist, the recovery response and recap guidance shall expose those breadcrumbs and label them as recorded activity without a saved working-state summary.
2. If neither a checkpoint nor breadcrumbs exist for the requested project, the recovery workflow shall explicitly report that mmhmem has no saved state or recorded activity for that project.
3. If no checkpoint exists, the recap guidance shall explain that harness chat history is separate and may be used in the original conversation to create a checkpoint, without claiming that the history is missing or has been automatically imported.
4. When recovery initializes a task identifier without saving working state, the recovery presentation shall not describe that identifier as a successfully saved checkpoint.

### Requirement 4: Failures, distribution, and verification
**Objective:** As a user, I want consistent behavior across installed agents without false save claims or new dependencies.

#### Acceptance Criteria
1. If a checkpoint operation fails, the agent guidance shall require a concise warning that the latest changes were not saved and shall not treat the attempt as an acknowledged checkpoint.
2. When a checkpoint attempt fails or execution stops before acknowledgement, mmhmem shall retain the last successfully persisted checkpoint for subsequent recovery.
3. When updated guidance is installed or packaged, every supported harness receiving checkpoint/recap guidance shall receive equivalent save triggers and interruption limits without requiring client hooks.
4. The checkpoint and recovery workflow shall preserve existing secret redaction, project isolation, checkpoint size limits, and worker-independent operation.
5. The verification suite shall cover checkpoint-then-disconnect-and-resume, interruption before a newer checkpoint, absent-checkpoint recovery with and without breadcrumbs, and checkpoint failure without a false success claim.
6. The workflow verification shall cover changed versus unchanged final responses, milestone triggers, Ctrl+C-independent recovery, and consistency between canonical guidance, packaged skills, and recap templates.
