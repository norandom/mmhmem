# Implementation Plan

Existing Rust, SQLite, Python, and Dagger infrastructure is reused; no new runtime,
schema, or installer setup is required. Each task is a 1–3 hour work unit.
Tasks are sequential because response integration and server guidance share the
MCP router, and verification consumes the finalized runtime and guidance contracts.
All tasks retain the existing LOC, redaction, isolation, and size-cap gates.

- [x] 1. Make checkpoint replacement atomic and expose persisted save time
  - Save working state and its checkpoint breadcrumb in one transaction; write the audit event before commit and acknowledge only after commit.
  - Return the existing stored timestamp on recovery and the same captured timestamp on acknowledgement, without a schema or MCP argument change.
  - Preserve history-before-resume-event ordering, best-effort usage recording, and one-way database-to-audit lock ordering.
  - Add deterministic failure fixtures and service tests showing breadcrumb/audit failures leave the previous checkpoint content and timestamp intact, with no success receipt.
  - Verify timestamp presence/absence and document the audit-append/commit gap and lost-reply-after-commit behavior in the service contract.
  - _Boundary: Checkpoint service_
  - _Requirements: 2.4, 3.4, 4.1, 4.2, 4.4, 4.5_

- [x] 2. Integrate truthful recovery presentation into MCP responses
  - Introduce the private pure formatter and wire it into the existing tool handler while retaining worker notices for every recovery case.
  - Render saved checkpoint, history without checkpoint, and neither state nor history as distinct cases; expose recorded activity without synthesizing working state.
  - Show UTC save time and epoch milliseconds separately from timestamped breadcrumbs; handle timestamp ties, checkpoint-event labels, and formatting failure honestly.
  - Keep recognizable response phrases and a predictable task-ID line; update existing test extraction in this integration task so the changed text does not break unrelated assertions.
  - Formatter and focused HTTP tests demonstrate all three cases, correct acknowledgements, and that generating a task ID does not save a checkpoint.
  - _Boundary: Recovery presentation, Runtime verification (explicit integration)_
  - _Depends: 1_
  - _Requirements: 2.3, 2.4, 3.1, 3.2, 3.3, 3.4, 4.1, 4.5_

- [x] 3. Integrate proactive checkpoint instructions across agent entry points
  - Update canonical and server instructions to save the initial objective, changed milestones, and changed final responses using the same project identity and concise resumable state.
  - Distinguish pending from verified work and changed from acknowledged state; avoid duplicate unchanged saves and warn on failures without claiming success.
  - Update all three recap commands to expose history-only recovery, explain separate harness history, and save before agent-controlled handoff/clear when possible.
  - Regenerate packaged guidance through existing builders and align usage guidance with Ctrl+C/native-clear limits, worker-free recovery, and normal restart/reconnect only for loading updated artifacts.
  - The installed-source and packaged instructions contain equivalent triggers and limits, preserve the playbook token budget, and explain that guidance compliance is best effort.
  - _Boundary: Canonical agent guidance, Recap guidance, Generated guidance, Recovery presentation, User guidance (explicit integration)_
  - _Depends: 2_
  - _Requirements: 1.1, 1.2, 1.3, 1.4, 1.5, 2.1, 2.2, 2.3, 3.1, 3.2, 3.3, 3.4, 4.1, 4.3, 4.6_

- [x] 4. Add workflow and distribution regression checks
  - Add the guidance check and invoke it from the existing marketplace gate, preserving the existing distribution pipeline.
  - Cover initial objective, all four milestone triggers, changed/unchanged final response, advance-notice clear/handoff, direct Ctrl+C, failed save, and history-only recovery across shipped instructions.
  - Check token budget, canonical/package parity, and recap consistency; review semantic scenarios alongside mechanical assertions.
  - Demonstrate that a missing required trigger or divergent package fails the check using isolated fixtures, and that finalized guidance passes.
  - Record static-check and review evidence separately from live agent-compliance evidence; do not claim deterministic agent behavior.
  - _Boundary: Guidance verification_
  - _Depends: 3_
  - _Requirements: 1.1, 1.2, 1.3, 1.4, 1.5, 2.1, 2.2, 3.1, 3.3, 4.1, 4.3, 4.6_

- [x] 5. Verify interruption recovery across transports and processes
  - Extend HTTP, stdio, and multiprocess coverage using an isolated store and no worker configuration.
  - Save checkpoint A, record later activity without saving state B, disconnect/terminate the client, and resume A through a fresh connection/process with its timestamp and separate activity.
  - Exercise missing-checkpoint cases with and without breadcrumbs, exact project isolation, redaction, state-size limits, and preserved worker notices.
  - Use deterministic service failure tests from task 1 for pre-commit failure instead of racing Ctrl+C against a commit; assert persisted newer state remains recoverable when a reply is lost after commit where the fixture permits.
  - Tests demonstrate recovery requires neither importing harness chat nor restarting a service, and report no false success on failed saves.
  - _Boundary: Runtime verification (explicit transport integration)_
  - _Depends: 1, 2, 4_
  - _Requirements: 1.5, 2.2, 2.3, 2.4, 3.1, 3.2, 3.4, 4.1, 4.2, 4.4, 4.5_

- [ ] 6. Validate the integrated feature against repository gates
  - Run formatting, strict clippy, and the full native deterministic suite using the established nextest/cargo fallback; include mandatory LOC, audit, isolation, and size-cap checks.
  - Run the canonical Dagger distribution gate and feature-enabled checks through the sanctioned CI wrapper; resolve regressions within the owning task boundaries.
  - Smoke the newly built local stdio artifact through checkpoint/resume, then verify requirement coverage, cross-task integration, and design alignment.
  - Record fresh commands, exit codes, review results, and any limitations; completion requires all mandatory gates passing and no unresolved blocking findings.
  - Verify compatibility notes accurately require a new binary for new response metadata and updated guidance for save cadence, without claiming a release was published.
  - _Boundary: Runtime verification, Guidance verification (explicit final integration)_
  - _Depends: 1, 2, 3, 4, 5_
  - _Requirements: 2.3, 4.3, 4.4, 4.5, 4.6_






