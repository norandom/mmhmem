# Implementation Plan

All tasks are required. Each executable task is sized for approximately 1–3 hours.
File ownership follows the approved design's File Structure Plan. Parallel markers
apply only to the core tasks after foundation completion; shared fixtures are frozen
before those tasks start. Documentation updates accompany their implementation or
verification task rather than forming standalone work.

- [x] 1. Establish shared verification foundations
- [x] 1.1 Define executable owned-instruction fixtures and test drivers
  - Cover UTF-8, BOM, LF/CRLF, no final newline, user text, valid ownership, malformed markers, and dedicated-file conflicts.
  - Define byte-exact installation, update, removal, and no-op outputs, including cross-OS exchange cases.
  - Done when both helper test drivers can consume the same golden corpus and distinguish expected changes from preservation failures.
  - _Boundary: Owned Markdown Update Contract test infrastructure_
  - _Requirements: 3.1, 3.2, 3.3, 3.4, 3.5_

- [x] 1.2 Establish isolated adoption-test infrastructure and prerequisite reporting
  - Add the opt-in test entrypoint, synthetic eligible repository, test-owned store, counting worker, process timeouts, and cleanup.
  - Check harness availability, CLI capabilities, model settings, and existing authentication without copying credentials or private conversations.
  - Define allowlisted report fields, isolated-CI versus authenticated-local modes, and explicit PASS, FAIL, and NOT_VERIFIED outcomes.
  - Done when synthetic events and missing-prerequisite cases produce sanitized reports and default test runs make no paid model requests.
  - _Boundary: Adoption Evidence Runner_
  - _Requirements: 4.1, 4.2, 4.3, 4.4, 4.5_

- [x] 2. Implement discovery, guidance, and safe owned-section operations
- [x] 2.1 (P) Describe tool selection and serialize truthful MCP annotations
  - Update all nine descriptions with purpose, selection conditions, limitations, and the approved conservative annotation matrix.
  - Preserve tool names and argument schemas; align shared initialization guidance with exact reads, authorized fallback, and checkpoint rules.
  - Keep protocol handlers thin and within source-size limits; this task owns protocol metadata and its tests, not portable skill text.
  - Done when stdio and HTTP discovery tests confirm unchanged contracts and explicit wire annotations, including OTel-compatible external-interaction hints.
  - _Boundary: MCP Discovery Metadata_
  - _Depends: 1.1, 1.2_
  - _Requirements: 1.1, 1.2, 1.3, 1.4, 1.5, 2.2, 2.3, 2.4, 2.5_

- [x] 2.2 (P) Package concise reading guidance and improve skill discoverability
  - Produce canonical ordinary-inspection guidance with targeted exact reads, explicit user preferences, native workflow preservation, and bounded failure fallback.
  - Improve the portable skill's reading/token-optimization discovery while retaining checkpoint behavior and its estimated 200-token budget.
  - Regenerate marketplace artifacts and connect routing checks alongside existing checkpoint/distribution checks.
  - Done when canonical/package checks pass and guidance neither treats summaries as exact evidence nor promises savings from connectivity alone.
  - _Boundary: Routing Guidance_
  - _Depends: 1.1, 1.2_
  - _Requirements: 2.2, 2.3, 2.4, 2.5_

- [x] 2.3 (P) Implement native Windows owned-section planning and safe commits
  - Implement identical marker parsing, install/update/remove plans, and unchanged-content handling against the frozen shared fixtures.
  - Preserve unrelated bytes and reject ambiguous ownership, undecodable input, reparse destinations, and unsupported dedicated-rule frontmatter.
  - Retain one backup, recheck original content, stage beside the target, atomically replace, and prevent newly appeared targets from being overwritten.
  - Done when native fixture and injected backup/write/replace/concurrent-change tests preserve originals on failure and dry-run writes nothing.
  - _Boundary: Owned Markdown Update Contract — PowerShell helper_
  - _Depends: 1.1, 1.2_
  - _Requirements: 3.1, 3.2, 3.3, 3.4, 3.5_

- [x] 2.4 (P) Implement equivalent owned-section operations for the shell installer
  - Implement the approved parser and commit contract with the shell installer's existing Python runtime.
  - Match frozen shared byte outputs, reject unsafe ownership/symlink/frontmatter cases, and preserve Unicode/newline/BOM content.
  - Cover one-backup retention, no-op behavior, no-clobber creation, atomic replacement, concurrent changes, removal, and no-write dry-run.
  - Done when helper fixture and failure-injection tests pass without modifying the shared fixture corpus owned by foundation work.
  - _Boundary: Owned Markdown Update Contract — Python helper_
  - _Depends: 1.1, 1.2_
  - _Requirements: 3.1, 3.2, 3.3, 3.4, 3.5_

- [ ] 3. Integrate safe routing onboarding with both installers
- [ ] 3.1 Integrate Windows selection, preflight, bootstrap, and removal
  - Resolve selected global/project destinations and plan all instruction changes before selected harness writes.
  - Refuse inconsistent configuration overrides and legacy/unknown-version ownership conflicts; report disabled or unverified loading without changing user controls.
  - Integrate guidance/helpers with registration, removal, and dry-run; fetch required removal helper payloads through the root bootstrap.
  - Report changed paths, partial completion/errors, and fresh-session requirements; update Windows usage instructions with the tested behavior.
  - Done when selected/unselected and scope tests show one owned section, preserved user content, no affected-harness writes after failed preflight, and working bootstrap removal.
  - _Boundary: Installer Integration — Windows_
  - _Depends: 2.2, 2.3_
  - _Requirements: 2.1, 3.1, 3.2, 3.3, 3.4, 3.5_

- [ ] 3.2 Integrate Unix onboarding with the same ownership and compatibility rules
  - Apply the same selected destinations, all-plan preflight, override/version/loading checks, and honest completion behavior.
  - Wire helper and canonical payloads through install, update, remove, and dry-run, including root bootstrap removal downloads.
  - Connect installer test entrypoints and update Unix usage instructions; coordinate shared documentation after Windows integration.
  - Done when shell scope/selection, malformed-content, no-write dry-run, and bootstrap removal tests match the Windows ownership contract.
  - _Boundary: Installer Integration — Unix_
  - _Depends: 2.2, 2.4, 3.1_
  - _Requirements: 2.1, 3.1, 3.2, 3.3, 3.4, 3.5_

- [ ] 4. Integrate behavioral acceptance and honest accounting evidence
- [ ] 4.1 Verify cache accounting and non-saving read paths
  - Exercise a cold summary and same-identity repeat with unchanged content and a counting worker.
  - Confirm the repeat hits cache, adds no worker request, and updates existing stats with expected attribution.
  - Cover direct, targeted, security-bypass, session-only, and failed-worker cases without changing service policy, schemas, or savings formulas.
  - Done when deterministic assertions establish positive cache behavior and no false summary/cache savings on negative paths.
  - _Boundary: MCP and Accounting Integration Tests_
  - _Depends: 2.1_
  - _Requirements: 1.4, 2.3, 2.4, 4.2, 4.3_

- [ ] 4.2 Integrate real Claude instruction loading and tool-selection evidence
  - Install into the synthetic fixture using the actual installer and launch fresh bounded sessions with normal tools and permissions.
  - Parse CLI events for ordinary inspection, matching-identity repeat, targeted evidence, and unavailable-worker fallback scenarios.
  - Record effective instruction sources and ambient-instruction limitations; do not inject skill text or force the desired tool.
  - Done when parser/negative cases distinguish natural adoption, full-native-read-first failure, identity mismatch, unavailable prerequisites, and unsupported events in sanitized reports.
  - _Boundary: Adoption Integration — Claude_
  - _Depends: 1.2, 2.1, 3.1, 3.2_
  - _Requirements: 2.1, 2.2, 2.3, 2.4, 4.1, 4.2, 4.3, 4.4, 4.5_

- [ ] 4.3 Integrate real OpenCode instruction loading and tool-selection evidence
  - Add fresh standalone JSON runs against actual installer-produced instructions and the shared synthetic environment.
  - Verify normal tool availability and effective global/project instruction sources without an injected-instructions shortcut.
  - Extend sanitized event extraction for ordinary reads, repeat identity, exact evidence, and bounded fallback.
  - Done when parser tests and missing-auth/unsupported-event cases produce accurate outcomes using the same evidence contract as Claude.
  - _Boundary: Adoption Integration — OpenCode_
  - _Depends: 4.2_
  - _Requirements: 2.1, 2.2, 2.3, 2.4, 4.1, 4.2, 4.3, 4.4, 4.5_

- [ ] 5. Validate integrated installation and real adoption
- [ ] 5.1 Verify cross-installer safety and distribution behavior
  - Exchange exact generated outputs between installers for update/removal, including Unicode, newline variants, and ownership failures.
  - Exercise selected/unselected harnesses, both scopes, overrides, disabled loading, backups, concurrency, partial failure, dry-run, and bootstrap payloads.
  - Run native suites in PowerShell 5 and 7 and shell/distribution suites through the canonical Dagger entrypoint, using disposable configurations.
  - Done when all applicable safety/distribution gates pass and no test modifies real user harness settings.
  - _Boundary: Installer Integration Verification_
  - _Depends: 3.1, 3.2_
  - _Requirements: 3.1, 3.2, 3.3, 3.4, 3.5_

- [ ] 5.2 Run regression gates and publish sanitized live acceptance evidence
  - Run formatting, Rust tests, strict default/OTel clippy, and marketplace/checkpoint/routing checks on the integrated change.
  - Execute both real harnesses with explicit models; capture versions, effective instructions, synthetic prompt, tool sequence, actual cache identity, worker counts, and stats deltas.
  - Record per-criterion evidence and tested acceptance commands alongside the implementation validation report and usage instructions.
  - Done only when required deterministic gates and both natural-adoption runs pass; missing credentials or inconclusive live cache evidence stays NOT_VERIFIED and leaves completion pending.
  - Never substitute a forced MCP invocation for live adoption, and do not claim client-reported savings from estimates.
  - _Boundary: Final Integration Verification_
  - _Depends: 4.1, 4.2, 4.3, 5.1_
  - _Requirements: 1.1, 1.2, 1.3, 1.4, 1.5, 2.1, 2.2, 2.3, 2.4, 2.5, 4.1, 4.2, 4.3, 4.4, 4.5_

## Task Plan Review

- All 20 requirement IDs mapped; design components, prerequisites, ownership, and dependencies checked.
- 5 major groups, 13 executable subtasks; 4 core tasks can run in parallel after foundations pass.
- Task-plan review gate: PASS.
- Task-graph sanity review: PASS (independent); no blocking findings.
- Tasks remain unapproved until human review under the project phase gate.


## Implementation Notes
- Adoption subprocesses must drain stdout/stderr concurrently while enforcing timeout and kill/reap; flood-output and delayed-write regressions cover both.
- Shared server instructions now live in src/mcp/instructions.rs; checkpoint checker follows that constant and retains scenario mutation tests.
- Preserve LF when rewriting canonical SKILL.md; shell builder rejects CRLF frontmatter. Native routing/checkpoint and Dagger Debian/Alpine distribution parity verified.
- Owned instruction commits recheck source bytes before replacement; this detects changes but does not provide cross-process locking.
