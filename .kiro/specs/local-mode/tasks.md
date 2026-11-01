# Implementation Plan

> **House caps (Requirements 13.1–13.5) apply to every implementation task in this plan:**
> cognitive complexity ≤ 15 per function (blocking clippy gate), ≤ 500 LOC per source file,
> ≤ 80 LOC per function, ≤ 400 added non-test lines per task diff, test files ≤ 800 LOC.
> Work that would exceed a cap is split, never silenced. Conventional commit per task, push after each.

## Task Format Template

Use whichever pattern fits the work breakdown:

### Major task only
- [ ] {{NUMBER}}.{{PARALLEL_MARK}} {{TASK_DESCRIPTION}}
  - {{DETAIL_ITEM_1}} (Include details only when needed. If the task stands alone, omit bullet items.)
  - _Requirements: {{REQUIREMENT_IDS}}_

### Major + Sub-task structure
- [ ] {{MAJOR_NUMBER}}. {{MAJOR_TASK_SUMMARY}}
- [ ] {{MAJOR_NUMBER}}.{{SUB_NUMBER}}{{SUB_PARALLEL_MARK}} {{SUB_TASK_DESCRIPTION}}
  - {{DETAIL_ITEM_1}}
  - {{DETAIL_ITEM_2}}
  - {{OBSERVABLE_COMPLETION_ITEM}} (At least one detail item should state the observable completion condition for this task.)
  - _Boundary: {{COMPONENT_NAMES}}_ (Only for (P) tasks. Omit when scope is obvious.)
  - _Depends: {{TASK_IDS}}_ (Only for non-obvious cross-boundary dependencies. Most tasks omit this.)
  - _Requirements: {{REQUIREMENT_IDS}}_ (IDs only; do not add descriptions or parentheses.)

> **Parallel marker**: Insert ` (P)` immediately after the task number for tasks that can be executed in parallel. Omit the marker when running in `--sequential` mode.
>
> **Optional test coverage**: When a sub-task is deferrable test work tied to acceptance criteria, mark the checkbox as `- [ ]*` and explain the referenced requirements in the detail bullets.

## Tasks

- [ ] 1. Foundation: local-mode configuration layer
- [x] 1.1 Per-OS user directory resolution
  - Resolve the per-user application-data location for each supported operating system from environment variables only (XDG data home with home-directory fallback on Linux, the standard Windows application-data variable, the macOS convention), joining the fixed `mmhmem` folder, with an injected-lookup design so tests never touch the real environment.
  - Fail fast with a clear configuration error naming the missing variable when the user-level location cannot be determined — never guess or fall back to a system path.
  - Provide the default local audit-file location derived from the data directory.
  - Observable: unit tests pass for each operating system branch covering present, absent, and blank variables, and the resolver output feeds the local profile defaults.
  - _Requirements: 3.1, 7.1_
- [x] 1.2 Local configuration profile
  - Add the profile dimension to the runtime configuration: a serve profile with exactly today's validation (token and roots required, identical error variants and defaults) and a local profile that starts with zero mandatory environment variables.
  - In the local profile: token optional and unused, confinement roots optional (empty means unconfined), data directory and audit file defaulting to the per-user locations from 1.1, and the existing override variables honored unchanged.
  - Add the new tuning variables (store lock-wait timeout, audit rotation size threshold, audit rotation retention count) parsed with documented defaults in both profiles, rejecting invalid values with clear errors.
  - Leave the serve-profile loading path byte-identical in behavior so the existing configuration tests pass unmodified.
  - Observable: a spawned local-mode process needs no environment to start, and the existing serve validation tests still pass without modification.
  - _Requirements: 2.5, 3.2, 3.3, 3.4, 6.2, 7.3, 9.2_
- [x] 1.3 (P) Optional path confinement
  - Give the confinement checker an explicit unconfined state: an empty configured root set allows all paths in local mode, while any non-empty set keeps today's deny-by-default enforcement including audited denials.
  - Prove serve mode cannot reach the unconfined state (serve validation requires at least one root), so the new branch is unreachable there.
  - Observable: unit tests show unconfined behavior with no roots, unchanged denial behavior with roots set, and the denial reason still reaches the audit trail.
  - _Boundary: Path confinement service_
  - _Requirements: 3.3, 12.5_

- [ ] 2. Core storage: multi-process access and local audit
- [x] 2.1 Multi-process-safe store access
  - Make every store connection wait for a busy store up to the configurable timeout (default 5 seconds) instead of failing immediately, so multiple local processes on one machine serialize through the store engine without corruption, lost updates, or partial actions.
  - Apply the timeout at open, keep it configurable through the profile variable, and restate the storage concurrency invariant: per-process connection, lock never held across an await within a process, store-level serialization across processes, no lock files or coordination services.
  - Keep every existing public storage interface unchanged so the existing suite runs unmodified.
  - Observable: a unit test shows the configured wait is in effect on the connection, and a lock that clears within the timeout no longer produces an error.
  - _Requirements: 6.1, 6.2, 6.3, 6.4, 9.4_
- [x] 2.2 Read-only store opening for reporting
  - Add a sibling open mode that never creates files or applies the schema, opens with read-only flags, and clearly reports a missing store file to the caller instead of inventing an empty one.
  - Observable: unit tests show no file is created when absent, writes are refused, and reads work against a pre-populated store.
  - _Requirements: 8.5, 8.6_
- [ ] 2.3 (P) Size-bounded local audit rotation
  - Add a rotation step to the audit write path: when the configured size threshold is exceeded before an append, run the rename cascade (active file becomes the newest rotated file, older files shift, the oldest is dropped) keeping at most the configured number of rotated files.
  - Gate the step on the configuration so serve mode (rotation disabled) keeps today's external logrotate contract exactly.
  - Preserve record integrity: records are never truncated or rewritten, and every rotated file still contains only complete, digest-verifiable records.
  - Observable: unit tests with a tiny threshold show rotation occurs at the boundary, retention is respected, and all records remain verifiable across the file set.
  - _Boundary: Audit writer_
  - _Requirements: 7.1, 7.2, 7.3, 7.4_

- [ ] 3. Core: local stdio serving
- [ ] 3.1 Stdio serving entry
  - Enable the stdio transport in the pinned protocol library (one added cargo feature, no version change) and add the local-mode runner: initialize diagnostics strictly to standard error, load the local profile, build the existing application state (which creates the per-user store on first run), and serve the existing MCP tool handler over the stdio transport until input closes.
  - On input end, flush telemetry and exit cleanly with success; on initialization failure, print a clear error to standard error and exit non-zero without partially initializing.
  - Open no network listener anywhere in the local path; the bearer-auth concern stays serve-only by construction.
  - Keep standard output reserved exclusively for protocol frames under every code path including errors.
  - Observable: a spawned process performs the MCP handshake over standard input/output with zero environment variables set and exits 0 after its input is closed.
  - _Requirements: 2.1, 2.2, 2.3, 2.5, 2.6, 3.2, 4.1, 4.2, 12.1, 12.4_
- [ ] 3.2 CLI subcommands serve and stdio
  - Add the `serve` subcommand (identical to today's no-argument daemon) and the `stdio` subcommand (the local-mode runner from 3.1) to the entrypoint dispatch, keeping the no-argument path, flags, and the healthcheck subcommand exactly as they are.
  - Update the help text to document both subcommands additively, without altering existing lines' meaning.
  - Observable: `mmhmem --help` lists serve, stdio, and healthcheck; `mmhmem serve` starts the daemon; `mmhmem stdio` starts local mode; the existing CLI tests pass unmodified.
  - _Requirements: 1.1, 1.2, 1.4, 9.3_
- [ ] 3.3 Stdio integration tests
  - Spawn the real binary as a local-mode child process with piped standard input/output and drive the MCP handshake with hand-rolled JSON-RPC lines (no new dev-dependencies).
  - Verify the full local contract: the initialize handshake answers with the serve-mode identity and tool surface; all six tools are listed; a direct read works with no worker configured; a read that requires shunting without a worker returns the clear worker-unconfigured error; a malformed input line is logged to standard error and the process keeps serving subsequent valid requests; closing standard input ends the process with exit code 0; standard output carries only protocol frames with all diagnostics on standard error.
  - Observable: the stdio integration suite passes against the real binary, proving resilience to malformed input and clean EOF shutdown.
  - _Requirements: 2.1, 2.2, 2.3, 2.4, 2.6, 4.1, 4.2, 4.3, 4.4_

- [ ] 4. Core: token-savings statistics command
- [ ] 4.1 Read-only stats aggregation and rendering
  - Implement the report behind the future `stats` subcommand: open the local store read-only (using 2.2), and when the store is missing print a clear no-statistics message and exit successfully.
  - Aggregate the token-stats records by day, project, and harness, summing separately the estimated avoided input tokens (all non-client-reported kinds) and the client-reported values, plus the worker prompt and completion tokens spent.
  - Render a fixed-width human-readable table with an estimate label and a totals row, and emit the same aggregates as machine-readable JSON when the JSON flag is passed.
  - Preserve the labeling discipline: estimates stay marked as estimates, client-reported values stay labeled by source, and nothing computed is ever presented as measured.
  - Observable: unit tests against a seeded temporary store show correct per-day aggregates, table and JSON output agree, and the store is untouched afterwards.
  - _Depends: 2.2_
  - _Requirements: 8.1, 8.2, 8.3, 8.4, 8.5, 8.6_
- [ ] 4.2 Stats subcommand wiring
  - Wire the `stats` subcommand (with its JSON flag) into the dispatch and document it in the help text additively.
  - Observable: `mmhmem stats` prints the local report; `mmhmem stats --json` prints machine-readable output; `--help` documents it.
  - _Requirements: 1.3, 1.4_
- [ ] 4.3 Stats CLI integration tests
  - Seed a temporary store through the library API with varied kinds, days, projects, and harnesses; run the real binary as a child process and verify the table rows, totals, the JSON flag's payload, and the labeling.
  - Verify the empty-store path end to end: a fresh user directory yields the friendly message and exit code 0.
  - Verify read-only behavior: the store file's bytes are identical before and after running the command.
  - Observable: the integration test suite covers table, JSON, empty, and read-only paths against the real binary.
  - _Requirements: 8.1, 8.2, 8.3, 8.4, 8.5, 8.6_

- [ ] 5. Integration: shared per-user store validation
- [ ] 5.1 Same-machine multi-process sharing tests
  - Spawn two local-mode child processes against one temporary user directory and prove the full sharing contract: one harness's checkpoint is fully visible to the other's resume (state, last harness, objective, breadcrumbs); a shunt summary seeded by one process is served to the other as a cache hit with zero worker calls; an observation recorded by one is immediately searchable by the other; token-stats rows from both accumulate in one shared table.
  - Include a concurrent first-open scenario (both processes started against a not-yet-existing store) and a concurrent write scenario proving no corruption, no lost updates, and no partial actions.
  - Prove no coordination beyond the processes themselves is needed: no lock files, no daemons, no setup steps.
  - Observable: the multiprocess integration tests pass deterministically on the development host.
  - _Requirements: 2.6, 5.1, 5.2, 5.3, 5.4, 5.5, 6.1, 6.3, 6.4_
- [ ] 5.2 Local configuration behavior integration tests
  - Exercise the local profile end to end against the real binary: zero-environment spawn succeeds and the store is created on first use; explicit data-directory and audit-file overrides relocate everything; configured confinement roots are enforced with denials audited; unset roots leave reads unconfined.
  - Cover the fail-fast path: an unusable user directory produces a clear error and non-zero exit with no partial initialization.
  - Observable: the local-configuration integration suite covers defaults, overrides, confinement on/off, and startup failure against the real binary.
  - _Requirements: 2.5, 2.6, 3.2, 3.3, 3.4, 4.4_
- [ ] 5.3 Audit rotation integration tests
  - Drive real audit traffic through a local-mode process with a small rotation threshold and verify: rotation happens within the threshold boundary, the retention count is respected, and every record across the active and rotated files verifies against its digest with identical record shape and redaction as serve mode.
  - Observable: the rotation suite passes with records intact and the audit contract unchanged.
  - _Requirements: 7.1, 7.2, 7.3, 7.4_

- [ ] 6. (P) Cross-platform release pipeline jobs
  - Add native-runner jobs for Windows and macOS to the repository's existing CI workflow: each job runs the same blocking lint gate (deny-warnings clippy across all targets), the deterministic test suite, and a release build, then uploads the platform artifact with a documented name.
  - Keep every container-based gate and the Linux release path untouched in the existing pipeline module; coordinate with any in-flight edits to that module from parallel sessions before touching it, and make no behavioral change to its existing calls.
  - Note the deliberate scope: the container pipeline remains the single source for container work; the native jobs exist only because the canonical Windows and macOS toolchains cannot run in Linux containers, and OS-specific code must not escape the lint gate.
  - Observable: the workflow runs the Windows and macOS jobs green (lint, tests, release artifact uploaded) alongside the unchanged Linux pipeline.
  - _Boundary: CI workflow configuration_
  - _Requirements: 10.1, 10.2, 10.3, 10.4, 10.5_

- [ ] 7. Client-first documentation
- [ ] 7.1 (P) README and install restructure
  - Restructure the README and installation docs to lead with the local story: download or build the binary, connect each harness to the spawned local mode, optionally configure a worker model for shunting, and present the server as the optional upgrade for cross-machine access, central audit and compliance, central operations, and multi-user bearer auth.
  - Document the local trust boundary and egress contract explicitly: no token, per-user store, the worker endpoint as the only outbound connection.
  - Validate the quickstart by following it as written against a fresh directory: every command runs and the efficiency layer works without the server.
  - Observable: the quickstart commands are executable exactly as documented and the docs present local-first with the server as optional.
  - _Boundary: README and install documentation_
  - _Requirements: 11.1, 11.4, 11.5_
- [ ] 7.2 (P) Per-harness stdio examples and skill update
  - Add stdio connection examples for each supported harness (Claude Code CLI, Google Antigravity, OpenCode, Claude Desktop) beside the existing server examples, using the documented spawn-based stdio form.
  - Update the skill playbook so its tool guidance is connection-neutral (applies identically to local and serve mode), and verify it remains under its 200-token cap.
  - Re-check every example command against the implemented CLI surface once tasks 3.2 and 4.2 have landed.
  - Observable: each harness has a copy-pasteable local-mode example, and the skill playbook stays under the token cap with connection-neutral wording.
  - _Boundary: Deploy examples, skill playbook_
  - _Depends: 3.2, 4.2_
  - _Requirements: 11.2, 11.3_

- [ ] 8. Final validation: serve-mode non-regression and house gates
- [ ] 8.1 Full regression and unmodified-suite proof
  - Run the complete local gate chain (format check, deny-warnings clippy, full test suite) and prove the serve contract held: the pre-existing test files are byte-identical (no modifications to any test that existed before this feature), the suite passes, and the complexity and size cap test passes on all new files.
  - Confirm the serve surface end to end: the daemon starts from the no-argument path exactly as before, healthcheck still answers, and the blocking gates run at unchanged strictness in the container pipeline.
  - Observable: a clean run of the full gate chain passes with zero modifications to pre-existing tests, plus a recorded diff proof.
  - _Requirements: 9.1, 9.3, 9.4, 13.1, 13.2, 13.3, 13.4, 13.5_
- [ ] 8.2 Plain-binary guards and deployment-artifact checks
  - Verify the plain binary guarantees survive the feature: the dependency-tree check still shows zero observability crates in the plain build, no telemetry or SaaS calls exist anywhere in the local path, and standard-error diagnostics never leak the token or secrets.
  - Confirm every serve deployment artifact (container image definition, compose file, service unit, logrotate configuration) is unchanged.
  - Observable: the guard checks pass and the deployment artifacts show zero diff.
  - _Requirements: 9.5, 12.2, 12.3, 12.4_
