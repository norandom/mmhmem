# Implementation Plan

> **House caps (Requirements 6.1–6.5) apply to every implementation task in this plan:**
> cognitive complexity ≤ 15 per function (blocking clippy gate), ≤ 500 LOC per source file,
> ≤ 80 LOC per function, ≤ 400 added non-test lines per task diff, test files ≤ 800 LOC.
> Work that would exceed a cap is split, never silenced. Conventional commit per task, push after each.
> The pre-existing deterministic suite passes unmodified after every task (Req 5.1).

## Tasks

- [ ] 1. Foundation: warmup orchestration over the shunt path
- [ ] 1.1 Warmup service with batch outcomes
  - Implement the warmup orchestration over the existing per-file shunt stages: for each path in the batch, record one outcome — delegated (worker called, cache and statistics rows written by the reused engine path), cached (content and fingerprint hit, no worker call), skipped with reason (below threshold, security-critical), or missing (unreadable file counted, never fatal).
  - Enforce the 25-path batch cap with a clear pre-flight error, and propagate the existing worker-unconfigured error when any file needs delegation and no worker is set.
  - Record one audit event summarizing the batch counts (the delegated path keeps its own per-file events).
  - Accept the warmup question as an explicit input (the tool and the playbook share the canonical summarization question so warmup rows match later reads).
  - Observable: unit tests pass for the cap error, a seeded cache row counting as cached with zero worker calls, both skip reasons, a missing file not failing the batch, and the no-worker error.
  - _Requirements: 1.2, 1.3, 1.4, 1.6, 1.7, 1.8_
- [ ] 1.2 Warmup report rendering
  - Render the batch summary (delegated, cached, skipped by reason, missing, worker tokens) as the tool response text, with a hint listing per-file skip reasons capped at a sane length.
  - Observable: unit tests show the report includes every counter and the per-file reason list.
  - _Requirements: 1.5_

- [ ] 2. Foundation: stale-cache pruning service
- [ ] 2.1 Staleness scan and deletion
  - Implement the staleness predicate over the shunt-cache rows: a row is stale iff its file no longer exists or the file's current content hash no longer matches the stored hash; unreadable files count as missing.
  - Implement the preview (counts only), the deletion of exactly the stale rows inside the existing short-transaction discipline, and one audit summary event with the counts by reason.
  - Prove nothing else is ever selected or deleted: statistics, observations, sessions, and breadcrumbs are untouched.
  - Observable: unit tests against a seeded store show correct grouped counts for fresh, changed, and missing-file rows, deletion removing exactly the stale set, the audit event recorded, and fresh rows surviving.
  - _Requirements: 3.1, 3.3, 3.5, 3.6, 4.3_
- [ ] 2.2 Delegation-ready report shape
  - Expose the grouped counts (removed-missing, removed-changed, kept-fresh) as one report type reused by the CLI and the tool, with a preview marker.
  - Observable: unit tests show the report shape and preview marker.
  - _Requirements: 3.2, 4.2_

- [ ] 3. Tool surface: `shunt_warmup` and `shunt_prune` in both modes
- [ ] 3.1 Tool handlers and registration
  - Add thin handlers in the shared tool layer: `shunt_warmup` (paths list, optional question) calling the warmup service, and `shunt_prune` (optional dry-run flag) calling the prune service; register both in the shared router so serve and local modes behave identically; add descriptions in the existing style.
  - Observable: the tools appear in `tools/list` in both modes with correct argument schemas; a handler error surfaces as the house's actionable JSON-RPC error.
  - _Requirements: 1.1, 1.5, 4.1, 4.2, 5.2_
- [ ] 3.2 Tool-count assertions updated
  - Update this repository's own local-mode tests that pinned the six-tool list to expect eight, keeping the pre-existing suite untouched.
  - Observable: the updated suite passes; `git diff` shows no modification to any test file that predates the local-mode feature.
  - _Requirements: 5.1, 5.4_

- [ ] 4. CLI surface: `mmhmem prune`
- [ ] 4.1 Prune subcommand
  - Add the `prune` subcommand with a `--dry-run` flag against the local profile store: missing store → clear message and exit 0; present store → preview or delete with the grouped report printed; wire the help text line.
  - Observable: CLI integration tests against a temp store cover the report, real deletion, dry-run no-op, and the missing-store message with exit 0.
  - _Requirements: 3.1, 3.2, 3.4_

- [ ] 5. Documentation and playbook
- [ ] 5.1 (P) Playbook and README updates
  - Add one playbook line telling agents to offer warming the corpus with `shunt_warmup` on a new project's key files, and the canonical question alignment; verify the playbook stays under 200 tokens.
  - Update the README: eight-tool list, warmup guidance in the quickstart area, and `mmhmem prune` in day-to-day operation with the dry-run example.
  - Observable: the playbook line is present under the token cap, and the README lists all new surfaces.
  - _Boundary: Docs and playbook_
  - _Requirements: 2.1, 5.3_

- [ ] 6. Integration: end-to-end warmup and prune through a stdio child
- [ ] 6.1 Warmup integration with a real worker delegation
  - Spawn the real binary in local mode with the worker endpoint pointed at an in-test stub: call `shunt_warmup` with one above-threshold file and assert delegated=1 with worker tokens counted, a cached second call counts as a hit with no further worker traffic, the `shunt_delegated` statistics row exists, and the batch is audited.
  - Cover the mixed batch end to end: one cached, one below-threshold, one security-critical, one missing file → all counted correctly in one response.
  - Observable: the integration suite passes against the real binary.
  - _Requirements: 1.2, 1.3, 1.4, 1.5, 1.8_
- [ ] 6.2 Prune tool integration
  - Through a stdio child: seed fresh, changed, and missing-file cache rows; call `shunt_prune` with dry-run (preview counts, rows intact) then without (stale rows gone, fresh row survives, observations and statistics untouched).
  - Observable: the tool integration test passes with before/after row counts.
  - _Requirements: 4.1, 4.2, 4.3_

- [ ] 7. Final validation: parity and non-regression
- [ ] 7.1 Full regression and unmodified-suite proof
  - Run the complete gate chain (format, deny-warnings clippy, full test suite) and prove the protected pre-existing suite is byte-identical (zero modifications; only this feature's new or updated files appear in the diff).
  - Confirm the tool count and behavior parity hold in both modes and the size-cap test passes on all new files.
  - Observable: a clean full gate run with the recorded diff proof.
  - _Requirements: 5.1, 5.2, 5.4, 6.1, 6.2, 6.3, 6.4, 6.5_
