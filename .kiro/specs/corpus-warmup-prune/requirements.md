# Requirements Document

## Introduction
mmhmem's token-efficiency corpus accrues only from live agent activity: a fresh install or a new project starts with an empty shunt cache and empty token statistics, so no savings register until the big files are organically read — and after a large refactor, the content-addressed shunt cache keeps stale rows (file deleted or content changed) that can never be served again. This feature adds two agent/operator surfaces: an on-demand corpus warmup (an explicit file list is pushed through the real shunt path, populating the cache and the statistics with real, honest events) and staleness-based cache pruning on both the CLI and a tool. Both surfaces exist identically in serve and local modes. The pre-existing deterministic suite passes unmodified; the statistics honesty discipline is absolute (no synthetic rows; computed estimates stay labeled as estimates).

## Boundary Context (Optional)
- **In scope**: the `shunt_warmup` MCP tool (explicit paths list, batch cap, per-file skip/miss handling, honest stats, audited); the `prune` CLI and the `shunt_prune` MCP tool (stale-row removal only: missing file or changed content; dry-run/preview; grouped count report; audited); the worker configuration file `.mmhmem.json` for the local profile (global `~/.mmhmem.json` plus per-project `./.mmhmem.json`, JSON format, worker settings only, precedence environment > project > global > defaults, installer writes it, serve stays environment-only); the worker-unconfigured session notice; the installer-shipped on-demand `/mmhmem-recap` session-recap command templates for Claude Code and OpenCode (playbook wording for harnesses without command support); the `mem_search_breadcrumbs` tool (literal-substring search over the session breadcrumb history, no schema change); playbook and documentation updates; tests including updates to this repo's own recent tests that pinned the previous tool count.
- **Out of scope**: TTL- or size-based cache eviction; synthetic or seeded statistics rows; schema changes; automatic prune triggers; eviction of token stats, observations, sessions, or breadcrumbs (prune touches only shunt-cache rows); cross-machine operation; config-file support for serve mode; non-worker settings in `.mmhmem.json` (future extension).
- **Adjacent expectations**: this feature EXTENDS the validated `cross-agent-memory-server` spec (tool surface grows from six to nine, both transports, shared handlers) and the implemented `local-mode` feature (whose own new tests pinned the six-tool list and are updated here — the protected pre-existing suite does not pin the tool count, verified). The warmup tool honors the same path confinement and security-critical bypass rules as the existing read shunt. The `.mmhmem.json` loader amends the local profile's environment-only configuration model (owner decision 2026-11-01, recorded in research.md); the serve profile's environment-only contract is untouched.

## Requirements

### Requirement 1: Corpus warmup tool (`shunt_warmup`)
**Objective:** As an agent starting work on a project, I want to push a curated list of key files through the shunt path in one call, so that the cache and the savings statistics are real from the first session instead of building up over days.

#### Acceptance Criteria
1. The mmhmem binary shall expose a `shunt_warmup` tool that accepts an explicit list of file paths together with the project and harness context, in both serve and local modes.
2. When the tool runs, each listed file shall go through the same shunt evaluation as a targeted large read: eligible files (at or above the threshold, not security-critical, not already cached) are delegated to the configured worker model, their summaries cached by content, and a real `shunt_delegated` statistics row recorded and committed before the tool's response returns (amendment 2026-11-02: fire-and-forget stats writes raced cross-connection readers and could lose rows on quick shutdown).
3. When a listed file is already cached for the same content and query fingerprint, the tool shall count it as a cache hit and make no worker call.
4. When a listed file is below the shunt threshold or security-critical, the tool shall skip it and state the reason in the response; when a listed file is missing or unreadable, the tool shall report it without failing the rest of the batch.
5. The tool's response shall summarize the batch: delegated count, cache-hit count, skipped counts by reason, failed count, and the worker tokens spent.
6. If the list exceeds 25 paths, the tool shall return a clear error asking the agent to split the batch.
7. If no worker model is configured and any listed file would require delegation, the tool shall return the same clear worker-unconfigured error as a shunted read.
8. Every warmup action shall be audited with the same record discipline as the underlying shunt actions.

### Requirement 2: Playbook offers the warmup
**Objective:** As a harness operator, I want the shipped playbook to tell agents the warmup exists, so that new projects get a warm corpus without me teaching every agent manually.

#### Acceptance Criteria
1. The SKILL.md playbook shall instruct agents to offer warming the corpus with `shunt_warmup` on a new project's key files, in one line, remaining under its 200-token cap.

### Requirement 3: Prune CLI (`mmhmem prune`)
**Objective:** As an operator after a big refactor, I want stale shunt-cache rows gone from the local store, so the cache stops carrying dead weight and reports stay meaningful.

#### Acceptance Criteria
1. The mmhmem binary shall provide a `prune` subcommand that removes stale shunt-cache rows: rows whose file no longer exists, and rows whose file's current content hash no longer matches the stored hash.
2. The prune command shall support a dry-run mode that reports what would be removed without deleting anything.
3. The prune command shall report the outcome grouped by staleness reason and shall leave token statistics, observations, sessions, and breadcrumbs untouched.
4. When the store does not exist yet, the prune command shall print a clear message and exit successfully.
5. The prune command shall record an audit event summarizing what it removed (counts by reason).
6. The prune command shall operate safely while other processes use the same store (bounded waiting under the existing busy-timeout contract, no partial deletions).

### Requirement 4: Prune tool (`shunt_prune`)
**Objective:** As an agent that just executed a huge refactor, I want to prune the stale cache rows from the session itself, so the next reads do not sit behind dead weight someone else would have to clean.

#### Acceptance Criteria
1. The mmhmem binary shall expose a `shunt_prune` tool with the same staleness semantics as the prune command, callable in both serve and local modes.
2. The tool shall support a preview mode that reports the counts without deleting anything.
3. The tool shall never remove anything except provably stale cache rows, so an agent can call it without risk to memory, sessions, statistics, or the audit trail.

### Requirement 5: Parity and non-regression
**Objective:** As the maintainer of the validated server and the local-mode feature, I want both new surfaces to arrive with full parity and zero regression, so the existing guarantees hold unchanged.

#### Acceptance Criteria
1. The pre-existing deterministic suite (the suite that existed before the local-mode feature) shall pass unmodified after every change of this spec.
2. Both new tools shall behave identically in serve and local modes through the shared tool handlers.
3. The help text, README tool list, and playbook shall list the new surfaces (nine MCP tools plus the `prune` command).
4. This repository's own local-mode tests that pinned the previous six-tool list shall be updated to include the new tools, and every updated or new test shall state which requirement of this spec it verifies.

### Requirement 6: Code complexity and size caps (house rule)
**Objective:** As a maintainer, I want the hard, machine-enforced caps to apply to this feature too, so the codebase cannot grow unbounded while the surface expands.

#### Acceptance Criteria
1. Every function in `src/` shall keep cognitive complexity at or under 15, machine-enforced as a blocking CI gate via the existing clippy configuration.
2. No source file in `src/` shall exceed 500 lines and no function shall exceed 80 lines.
3. Test files are exempt from the per-function caps but capped at 800 lines per file.
4. Every implementation task of this spec shall keep its non-test diff at or under 400 added lines.
5. Task generation shall copy these caps into every implementation task's acceptance criteria.

### Requirement 7: Worker configuration file (`.mmhmem.json`, local profile)

#### Acceptance Criteria
1. The local profile shall read the worker settings (model, API URL, API key) from a per-project `./.mmhmem.json` and a global `~/.mmhmem.json`, in JSON format, applying each setting with the precedence: process environment, then project file, then global file, then built-in defaults.
2. When no config file exists or a setting is absent everywhere, the local profile shall behave exactly as before (defaults; the worker stays off when no model is configured).
3. If a config file exists but is not valid JSON, the local profile shall fail fast at startup with a clear error naming the offending file.
4. The serve profile shall remain environment-only; the configuration file shall not change any serve behavior.
5. The worker API key from any source shall never appear in logs or diagnostics (redaction parity with the bearer token).
6. The installers shall write the configured worker settings into the global or project `.mmhmem.json` instead of embedding worker variables into harness registrations, and their notices shall point to the file.
7. The documentation shall state the file's location, precedence, and purpose, and recommend keeping API keys out of version control; the repository's `.gitignore` shall ignore `.mmhmem.json`.

### Requirement 8: Worker-unconfigured notice
**Objective:** As an agent working in a project with no worker model configured, I want the server to tell me at session start, so I know shunting and warmup are unavailable instead of discovering it on the first big read.

#### Acceptance Criteria
1. When no worker model is configured, the session-start tool's response shall include a one-line notice naming the missing setting and where to configure it, without changing any other response content.
2. Where a worker model is configured, the notice shall not appear.

### Requirement 9: On-demand session recap command (`/mmhmem-recap`)
**Objective:** As an agent user starting a fresh context (after a client context clear or restart), I want an on-demand, installer-shipped command that pulls the last session recap, so that working context is one keystroke away without any hook configuration.

#### Acceptance Criteria
1. The repository shall ship recap command templates under `deploy/commands/` whose content instructs the agent to call `session_resume_or_handoff` without `updated_working_state_md` and to present the returned working state, objective, and breadcrumbs as a recap of the last session.
2. The installers shall copy the Claude Code template into the `.claude/commands/` directory (user-global or project scope, matching the installer's scope selection) when Claude Code is among the selected harnesses.
3. The installers shall copy the OpenCode template into the `.opencode/command/` directory (user-global or project scope, matching the installer's scope selection) when OpenCode is among the selected harnesses.
4. For harnesses without user-defined command support (Claude Desktop, Antigravity), the installers shall make no command changes, and the playbook shall carry the recap instruction instead.
5. The installers' remove mode shall remove the installed command templates without ever deleting the store or `.mmhmem.json`, and dry-run mode shall list every command-file action before performing it.
6. The playbook shall tell agents, in one line within its token cap, that after a context clear the recap command (or a direct `session_resume_or_handoff` call) restores the last session's working state.

### Requirement 10: Breadcrumb history search (`mem_search_breadcrumbs`)
**Objective:** As an agent resuming or auditing past work, I want to search the project's breadcrumb history by keyword, so that I can find what earlier sessions did beyond the fifteen most recent crumbs the resume view returns.

#### Acceptance Criteria
1. The mmhmem binary shall expose a `mem_search_breadcrumbs` tool that searches the calling project's breadcrumb rows for a literal substring across tool name, target resource, and action summary, in both serve and local modes.
2. The tool shall return matches newest first, each showing harness, tool name, target resource, action summary, and timestamp, capped by a limit argument (default 20, maximum 100); a blank query shall list the project's breadcrumbs newest first under the same cap.
3. The tool shall accept an optional harness filter that restricts matches to one calling harness.
4. When no breadcrumb matches, the tool shall return a clear no-match message.
5. The tool shall be read-only over stored history: it shall not modify or delete any existing row in any table (its own audit event and breadcrumb follow the existing per-call discipline).
