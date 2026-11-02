# Implementation Plan

> **House caps (Requirements 6.1–6.5) apply to every implementation task in this plan:**
> cognitive complexity ≤ 15 per function (blocking clippy gate), ≤ 500 LOC per source file,
> ≤ 80 LOC per function, ≤ 400 added non-test lines per task diff, test files ≤ 800 LOC.
> Work that would exceed a cap is split, never silenced. Conventional commit per task, push after each.
> The pre-existing deterministic suite passes unmodified after every task (Req 5.1).

## Tasks

- [x] 1. Foundation: warmup orchestration over the shunt path
- [x] 1.1 Warmup service with batch outcomes
  - Implement the warmup orchestration over the existing per-file shunt stages: for each path in the batch, record one outcome — delegated (worker called, cache and statistics rows written by the reused engine path), cached (content and fingerprint hit, no worker call), skipped with reason (below threshold, security-critical), missing (file not found or unreadable — counted, never fatal), denied (path outside the allowed roots — counted, never fatal), or failed (worker call failed for this file — counted, never fatal, so one flaky call cannot waste the batch's spent tokens).
  - Enforce the 25-path batch cap with a clear pre-flight error, and propagate the existing worker-unconfigured error when any file needs delegation and no worker is set.
  - Record one audit event summarizing the batch counts (the delegated path keeps its own per-file events).
  - Accept the warmup question as an explicit input (the tool and the playbook share the canonical summarization question so warmup rows match later reads).
  - Observable: unit tests pass for the cap error, a seeded cache row counting as cached with zero worker calls, both skip reasons, a missing file not failing the batch, a denied path not failing the batch, a per-file worker failure not failing the batch, and the no-worker error.
  - _Requirements: 1.2, 1.3, 1.4, 1.6, 1.7, 1.8_
- [x] 1.2 Warmup report rendering
  - Render the batch summary (delegated, cached, skipped by reason, missing, denied, failed, worker tokens) as the tool response text, with a hint listing per-file skip/failure reasons capped at a sane length.
  - Observable: unit tests show the report includes every counter and the per-file reason list.
  - _Requirements: 1.5_

- [x] 2. Foundation: stale-cache pruning service
- [x] 2.1 Staleness scan and deletion
  - Implement the staleness predicate over the shunt-cache rows: a row is stale iff its file no longer exists or the file's current content hash no longer matches the stored hash; unreadable files count as missing.
  - Implement the preview (counts only), the deletion of exactly the stale rows inside the existing short-transaction discipline, and one audit summary event with the counts by reason.
  - Prove nothing else is ever selected or deleted: statistics, observations, sessions, and breadcrumbs are untouched.
  - Observable: unit tests against a seeded store show correct grouped counts for fresh, changed, and missing-file rows, deletion removing exactly the stale set, the audit event recorded, and fresh rows surviving.
  - _Requirements: 3.1, 3.3, 3.5, 3.6, 4.3_
- [x] 2.2 Delegation-ready report shape
  - Expose the grouped counts (removed-missing, removed-changed, kept-fresh) as one report type reused by the CLI and the tool, with a preview marker.
  - Observable: unit tests show the report shape and preview marker.
  - _Requirements: 3.2, 4.2_

- [x] 3. Tool surface: `shunt_warmup` and `shunt_prune` in both modes
- [x] 3.1 Tool handlers and registration
  - Add thin handlers in the shared tool layer: `shunt_warmup` (paths list, optional question) calling the warmup service, and `shunt_prune` (optional dry-run flag) calling the prune service; register both in the shared router so serve and local modes behave identically; add descriptions in the existing style.
  - Observable: the tools appear in `tools/list` in both modes with correct argument schemas; a handler error surfaces as the house's actionable JSON-RPC error.
  - _Requirements: 1.1, 1.5, 4.1, 4.2, 5.2_
- [x] 3.2 Tool-count assertions updated
  - Update this repository's own local-mode tests that pinned the six-tool list to expect nine (eight from this spec's warmup/prune/notice work plus `mem_search_breadcrumbs`), keeping the pre-existing suite untouched.
  - Observable: the updated suite passes; `git diff` shows no modification to any test file that predates the local-mode feature.
  - _Requirements: 5.1, 5.4_
- [x] 3.3 Worker-unconfigured session-start notice
  - In the shared `session_resume_or_handoff` handler: when no worker model is configured, append a one-line notice to the resume response naming the missing setting and where to configure it (`.mmhmem.json` `worker_model`); when a worker is configured, the notice is absent. No other response content changes.
  - Observable: tests show the notice present without a worker and absent with one, with the rest of the response unchanged.
  - _Requirements: 8.1, 8.2_

- [x] 4. CLI surface: `mmhmem prune`
- [x] 4.1 Prune subcommand
  - Add the `prune` subcommand with a `--dry-run` flag against the local profile store: missing store → clear message and exit 0; present store → preview or delete with the grouped report printed; wire the help text line.
  - Observable: CLI integration tests against a temp store cover the report, real deletion, dry-run no-op, and the missing-store message with exit 0.
  - _Requirements: 3.1, 3.2, 3.4_

- [x] 5. Documentation and playbook
- [x] 5.1 (P) Playbook and README updates
  - Add one playbook line telling agents to offer warming the corpus with `shunt_warmup` on a new project's key files, and the canonical question alignment; verify the playbook stays under 200 tokens.
  - Update the README: nine-tool list, warmup guidance in the quickstart area, and `mmhmem prune` in day-to-day operation with the dry-run example.
  - Observable: the playbook line is present under the token cap, and the README lists all new surfaces.
  - _Boundary: Docs and playbook_
  - _Requirements: 2.1, 5.3_

- [x] 6. Integration: end-to-end warmup and prune through a stdio child
- [x] 6.1 Warmup integration with a real worker delegation
  - Spawn the real binary in local mode with the worker endpoint pointed at an in-test stub: call `shunt_warmup` with one above-threshold file and assert delegated=1 with worker tokens counted, a cached second call counts as a hit with no further worker traffic, the `shunt_delegated` statistics row exists, and the batch is audited.
  - Cover the mixed batch end to end: one cached, one below-threshold, one security-critical, one missing file, one path outside the allowed roots (denied) → all counted correctly in one response.
  - Observable: the integration suite passes against the real binary.
  - _Requirements: 1.2, 1.3, 1.4, 1.5, 1.8_
- [x] 6.2 Prune tool integration
  - Through a stdio child: seed fresh, changed, and missing-file cache rows; call `shunt_prune` with dry-run (preview counts, rows intact) then without (stale rows gone, fresh row survives, observations and statistics untouched).
  - Observable: the tool integration test passes with before/after row counts.
  - _Requirements: 4.1, 4.2, 4.3_

- [x] 7. Final validation: parity and non-regression (executed after Majors 8-10)
- [x] 7.1 Full regression and unmodified-suite proof
  - Run the complete gate chain (format, deny-warnings clippy, full test suite) and prove the protected pre-existing suite is byte-identical except the declared spec-driven updates (tests/audit_redaction.rs: event-type count 15→16 for the WARMUP event, Req 1.8; tests/shunt.rs: Shunted destructure gains the usage field, Req 1.5; tests/local_stdio.rs: tool count, Req 5.4).
  - Confirm the tool count and behavior parity hold in both modes and the size-cap test passes on all new files.
  - Observable: a clean full gate run with the recorded diff proof.
  - _Requirements: 5.1, 5.2, 5.4, 6.1, 6.2, 6.3, 6.4, 6.5_

- [x] 8. Worker configuration file: `.mmhmem.json` loader and installer write
- [x] 8.1 Loader with per-setting precedence (TDD)
  - Implement `config/mmhmem_file.rs`: `MmhmemFile { worker_model, worker_api_url, worker_api_key }` (serde, every field optional) loaded from the global `~/.mmhmem.json` and the per-project `./.mmhmem.json`; merge per setting in `Config::load_local` with the precedence environment > project file > global file > built-in defaults (Option-based, so one setting can come from each level); a file that exists but is not valid JSON fails fast at startup naming the offending file (`ConfigError::InvalidMmhmemFile`); the serve profile's environment-only path stays untouched.
  - Prove the worker API key never appears in logs or diagnostics: verify the Config redaction path actually redacts `worker_api_key`, not just the bearer token.
  - Observable: unit tests cover the per-setting precedence matrix, absent-everywhere defaults, invalid-JSON fail-fast naming the file, redaction of the key, and unchanged serve behavior.
  - _Requirements: 7.1, 7.2, 7.3, 7.4, 7.5_
- [x] 8.2 Installers write `.mmhmem.json` (replaces env embedding)
  - Replace the interim worker env injection in both installers with writing the configured worker settings (model, API URL, API key) into the global or project `.mmhmem.json` per the scope selection (JSON-merged; the key is never echoed); point the unset notices at the file; keep `--dry-run` honest about the write; `--remove` uninstalls registrations, skills, and command templates but never deletes the store or `.mmhmem.json`.
  - Observable: dry-run on host PowerShell and WSL bash shows the config write; a real run produces a valid `.mmhmem.json` the loader accepts; the key never appears in installer output.
  - _Requirements: 7.6_
- [x] 8.3 Docs and ignore entry
  - README section: `.mmhmem.json` location (global + project), precedence (environment > project > global > defaults), purpose (worker settings only), and the keep-keys-out-of-version-control recommendation; add `.mmhmem.json` to `.gitignore`.
  - Observable: README documents all of it; `.gitignore` contains the entry.
  - _Requirements: 7.7_

- [x] 9. On-demand session recap: `/mmhmem-recap` command templates
- [x] 9.1 Templates and installer wiring
  - Add `deploy/commands/claude/mmhmem-recap.md` and `deploy/commands/opencode/mmhmem-recap.md`: static markdown, description-only frontmatter, body instructing the agent to call `session_resume_or_handoff` without `updated_working_state_md` (with the template's own harness string and the current project) and to present the returned working state, objective, and breadcrumbs as a concise recap of the last session.
  - Wire both installers to copy the matching template into the harness's command directory when that harness is selected (Claude Code → `.claude/commands/`, OpenCode → `.opencode/command/`, user-global or project scope mirroring the existing skill-copy seam); `--remove` removes exactly the copied template; `--dry-run` lists the action; desktop and antigravity get no command changes.
  - Observable: dry-run lists the command-file actions in both installers; a real run installs the template and `--remove` removes exactly it (verified on host PowerShell and WSL bash); the store and `.mmhmem.json` are untouched by removal.
  - _Requirements: 9.1, 9.2, 9.3, 9.4, 9.5_
- [x] 9.2 Playbook and README recap lines
  - One playbook line: after a context clear, the recap command (or a direct `session_resume_or_handoff` call) restores the last session's working state; verify the playbook stays under 200 tokens. README mentions `/mmhmem-recap` in day-to-day operation.
  - Observable: playbook line present under the token cap; README mentions the recap command.
  - _Requirements: 9.6_

- [x] 10. Breadcrumb history search: `mem_search_breadcrumbs`
- [x] 10.1 Breadcrumb search service
  - Implement `services/breadcrumbs.rs`: project-scoped literal-substring search over `session_breadcrumbs` (tool name, target resource, action summary) with LIKE wildcards escaped, blank query = all crumbs newest first, optional harness filter, limit clamped to default 20 / max 100, ordered newest first; read-only over stored rows; one audit event per call plus its own breadcrumb (mem_search parity — constructed with `Some(bus)` like MemoryService, wired in `AppState::build`); no schema change, no FTS.
  - Observable: unit tests against a seeded store cover substring matches across all three fields, wildcard escaping, the harness filter, the limit clamp, blank-query listing, no-match, project isolation, and a proof that no existing row changes.
  - _Requirements: 10.2, 10.3, 10.4, 10.5_
- [x] 10.2 Tool handler and registration
  - Add `BreadcrumbSearchParams` (project, agent_harness, query, optional harness filter, limit) and a thin shared handler rendering matches newest first with timestamp, harness, tool, resource, and summary, or a clear no-match message; register in the shared router with a description in the existing style; serve and local behave identically.
  - Observable: `tools/list` shows the tool in both modes with the correct argument schema; handler tests cover the render and the no-match message.
  - _Requirements: 10.1, 10.2, 10.4_
- [x] 10.3 Integration through a stdio child
  - End to end through the real binary in local mode: generate real breadcrumbs (a checkpoint plus a smart_read call), then `mem_search_breadcrumbs` finds them by keyword and by harness filter, newest first, with no stored row modified.
  - Observable: the integration suite passes against the real binary.
  - _Requirements: 10.1, 10.2, 10.3_

- [x] 11. Amendment (2026-11-02): isolate worker-unconfigured stdio tests from the machine's global config
- [x] 11.1 Blank-set the worker vars in every test spawn helper
  - Same leak as `tests/local_stdio.rs` (local-mode task 9.2): env-REMOVEd WORKER_* fell back to the armed global `~/.mmhmem.json` on dev hosts, so `warmup_worker_unconfigured_error_matches_smart_read` saw a configured worker — one smart_read call even reached the real Mistral worker (746 ms). Fixed in `tests/warmup_prune.rs` (the failing case) and swept uniformly through `tests/stats_cli.rs`, `tests/audit_rotation.rs`, `tests/prune_cli.rs`, and `tests/multiprocess.rs` (same pattern, no failing assertion today). Blank wins over the file lookup and is treated as missing.
  - Observable: the full deterministic suite passes on a host with an armed global worker config, and no test reaches the network
  - _Requirements: 3.7, 8.1_
  - Verified: full gate green 219/219 (nextest 610140ac, 2026-11-02)

- [x] 12. Amendment (2026-11-02): shunt_warmup stats rows commit before the response returns
- [x] 12.1 Inherit the inline stats commit from the shunt engine (cross-agent-memory-server task 16.1)
  - warmup rides the same `record_stat` path; the macOS-deterministic failure `warmup_delegates_for_real_then_serves_cached` (stats row invisible from a second connection) is fixed by the awaited INSERT
  - Observable: the test passes deterministically; no change needed in the test itself
  - _Requirements: 1.2_
  - Verified: full gate green 219/219 (nextest aef79cb8, 2026-11-02)

## Implementation Notes

- 2026-11-02, task 6.1 fixture repair: static-release musl validation exposed `/tmp/.tmpAMZ0rq/big.rs` matching the existing case-insensitive `pam` security bypass. Non-sensitive warmup fixture roots now use unique numeric directory names, with a regression against the actual predicate. Full canonical GNU tests and independent formatting/review passed; dependent static-release validation must rerun. Production security behavior is unchanged.
