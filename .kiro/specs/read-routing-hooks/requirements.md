# Requirements Document

## Project Description (Input)

Agents rarely route file reads through `smart_read`, so mmhmem records almost
no savings: on 2026-11-03 Claude Code made no summary reads at all while
reading dozens of large files natively, and the user sees the same on the
server. Advisory routing rules are ignored under load. Make adoption visible
in `mmhmem stats`, then offer opt-in, per-harness hooks that redirect only
full reads of large, non-security-critical files to `smart_read`, pass
everything else through, fail open, and earn the trust a hook needs: explicit
consent, transparent and minimal behaviour, respect for harness approval and
organisation policy, and clean removal. Findings are in `brief.md` and
`research.md`. The user requested this work through the Kiro specification
workflow, with its requirements, design and tasks approval gates.

## Introduction

This specification governs how read activity is reported per harness, and
the optional hooks that steer native full-file reads toward `smart_read` in
harnesses that support pre-tool hooks.

## Delivery Order

The user set the priority on 2026-11-03: correct functionality first, then
periphery. Design and tasks follow this order:

1. **Core routing:** the hook decision (Requirement 2), fail-open behaviour
   (Requirement 3) and security properties (Requirement 5) for Claude Code
   (6.1). This is what produces savings.
2. **Acceptance:** consent, trust and policy handling in the installers
   (Requirement 4), without which agents and users will not run the hook.
3. **Coverage:** Codex and OpenCode (6.2 to 6.4).
4. **Periphery:** adoption statistics (Requirement 1) and the measured-effect
   report (Requirement 7), which reads the hook's audit events (5.4).

## Boundary Context

- **In scope:** adoption statistics in `mmhmem stats`; a hook decision
  command shipped in the mmhmem binary; the Claude Code hook; an OpenCode
  plugin if its interface supports the same contract; installer options to
  add, show and remove hooks; hook audit events.
- **Out of scope:** changing the shunt engine's routing or summaries;
  blocking edits, writes or targeted reads; hooks for harnesses without a
  pre-tool interception point; installing hooks at project scope by default;
  writing any harness approval or trust record on the user's behalf.
- **Adjacent expectations:** cross-agent-memory-server Requirement 1.5 stays
  true: mmhmem works fully without any hook, and hooks are never required.
  The hook mirrors the shunt engine's rules (targeted reads and
  security-critical or short files are never redirected). agent-tool-routing
  guidance stays installed.

## Requirements

### Requirement 1: Adoption visibility

**Objective:** As a user, I want `mmhmem stats` to show how each harness reads
files, so that I can tell an unused tool from a tool that saves nothing.

#### Acceptance Criteria

1. When `mmhmem stats` runs, the mmhmem stats command shall show, per day, project and harness, the number of summary reads, cache hits, exact-slice reads, direct reads of short or security-critical files, and hook redirects.
2. The mmhmem stats command shall show those counts even for a day, project and harness without any savings row.
3. When `--json` is requested, the mmhmem stats command shall include the same counts under stable field names.
4. The mmhmem stats command shall keep the existing savings fields and labels unchanged.
5. The mmhmem stats command shall keep opening the store read-only.

### Requirement 2: Redirect only reads a summary helps

**Objective:** As a user who installed the hook, I want only the reads that a
summary can replace to be redirected, so that the agent's normal work is
never obstructed.

#### Acceptance Criteria

1. When an agent requests a full native read of a file at or above the configured line threshold whose path is not security-critical, the read-routing hook shall decline the read and tell the agent to call `smart_read` for a summary, or to use an exact line range when it needs exact text.
2. When an agent requests a read with an explicit line range, the read-routing hook shall let it proceed.
3. When the file is below the line threshold, is security-critical by the shunt engine's rules, is not a regular text file, or does not exist, the read-routing hook shall let the read proceed.
4. Where no worker model is configured for mmhmem, the read-routing hook shall let every read proceed, because no summary can be produced.
5. When an agent repeats the same full read of the same file shortly after a redirect, the read-routing hook shall let it proceed, so an agent that needs the whole file is never deadlocked.
6. Where a harness lets the hook intercept shell commands, the read-routing hook shall apply criteria 2.1 to 2.5 to whole-file shell reads (such as `cat`, `type` or `Get-Content` on a single file) and shall let partial readers (`head`, `tail`), filtered, piped or multi-file commands proceed. (Corrected 2026-11-03 during task 1.3: `head` and `tail` print only part of a file, as the approved design already states.)
7. The read-routing hook shall never decline edit, write or search tools.
8. When an agent edits a file it was redirected away from, the edit shall succeed after an exact-range read of the lines it changes; if a harness requires a full read before editing, the read-routing hook shall let that read proceed.

### Requirement 3: Fail open and stay fast

**Objective:** As a user, I want a broken or slow hook to never stop my agent.

#### Acceptance Criteria

1. If the hook cannot read its input, its configuration, the target file or the mmhmem settings, then the read-routing hook shall let the tool call proceed.
2. If the hook encounters any internal error, then the read-routing hook shall let the tool call proceed and record the error without the tool input's content.
3. The read-routing hook shall decide within 200 milliseconds for files up to 10 MB on the user's machine, without any network call.
4. The read-routing hook shall not depend on the mmhmem MCP server being reachable.

### Requirement 4: Consent, trust and policy

**Objective:** As a user or organisation, I want to decide knowingly whether a
hook runs on my agents' tool calls, so that the hook is accepted rather than
smuggled in.

#### Acceptance Criteria

1. The installers shall never add a hook unless the user explicitly opts in for that harness, interactively or with a dedicated flag; a default install shall add none.
2. Before adding a hook, the installers shall show the exact configuration entry, the file it goes into, what the hook can see and decide, and how to remove it.
3. The installers shall add hooks only at user scope, never into a project's shared settings.
4. Where a harness or organisation policy disables or restricts user hooks, the installers shall not add the hook, and shall say which policy prevented it.
5. Where a harness requires the user to review or approve new hooks, the installers shall leave that step to the harness, never write or alter the harness's approval or trust records, and tell the user to complete it.
6. When the user removes mmhmem or only the hook, the installers shall remove the hook entry and leave every other hook and setting byte-identical.
7. The installers shall manage the hook entry idempotently: a repeated install keeps one entry.
8. When mmhmem is updated, the installers shall keep the hook entry byte-identical where possible, so a harness that approves hooks by content (such as Codex's trusted content hash) does not ask for approval again; when the entry must change, the installers shall say that re-approval is needed.
9. The read-routing hook shall coexist with other hooks on the same event, such as other memory plugins, and shall not depend on running before or after them.

### Requirement 5: Hook security properties

**Objective:** As a security-conscious user, I want the hook to be minimal and
inspectable, so that running it on every tool call is a small, known risk.

#### Acceptance Criteria

1. The hook entry shall invoke the installed mmhmem binary by absolute path with fixed arguments, and shall never pass tool input through a shell.
2. The read-routing hook shall only read the hook input, the target file's metadata and line count, and the mmhmem settings; it shall write nothing except its audit event and the repeat-read state file in the mmhmem data directory, which holds only hashed keys and times. (Clarified 2026-11-03 during task 2.2 review: the approved design's repeat window needs this state.)
3. The read-routing hook shall make no network connections and shall never read or emit secrets, including the worker API key.
4. The read-routing hook shall record each redirect and pass-through decision in the audit trail with the harness, the decision, the reason and the file path, but never the file content.
5. The read-routing hook shall treat its input as untrusted, and shall resolve the target path the same way the harness would before applying the rules.

### Requirement 6: Harness coverage

**Objective:** As a user of several harnesses, I want the same behaviour
wherever the harness allows it.

#### Acceptance Criteria

1. The read-routing hook shall support Claude Code's pre-tool hook for its native read tool and its shell tools.
2. The read-routing hook shall support Codex's pre-tool hook for its shell tool, recognising whole-file reads in the shell command, and the installers shall respect Codex's hooks feature flag, its managed policy and its per-hook trust and enablement state.
3. The installers shall offer an OpenCode integration for OpenCode's current plugin interface (v2), with the same decision logic, which catches all of its own errors so it fails open, because an error thrown by an OpenCode plugin blocks the tool call.
4. Where a harness has no approval step of its own, such as OpenCode plugins, the installers' explicit opt-in (4.1, 4.2) shall be the only way the integration is enabled.
5. Where a harness offers no pre-tool interception point, or its hooks are disabled by the user or by policy, the installers shall say that only advisory guidance applies to it.

### Requirement 7: Measured effect

**Objective:** As the product owner, I want evidence that hooks increase real
savings without hurting agents, so that the hook stays only if it helps.

#### Acceptance Criteria

1. The validation report shall compare adoption statistics from at least one working session per enabled harness before and after enabling the hook.
2. The validation report shall record redirects, the share of redirects followed by a `smart_read` call, repeat-read pass-throughs, and the estimated tokens avoided.
3. If redirects are mostly followed by repeat full reads instead of summaries, the validation report shall mark the hook's effect NOT_VERIFIED.
