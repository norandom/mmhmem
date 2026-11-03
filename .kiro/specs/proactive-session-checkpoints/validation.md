# Integration Validation

Date: 2026-11-03. Runtime baseline: `b67d69c`; parent also removed
nonsemantic trailing EOF blank lines in the guidance verifier and task list.
All six tasks passed independent review and are marked complete.

## Validation Report

- DECISION: GO for the implemented feature contracts, subject to parent review
  and final task-state update. No release publication or installed-user update
  is claimed.
- MECHANICAL_RESULTS:
  - Native formatting: `cargo fmt --check`, exit 0.
  - Native lint: `cargo clippy --all-targets -- -D warnings`, exit 0.
  - Full deterministic suite: `cargo nextest run`, exit 0; 244 passed,
    0 skipped. This includes LOC caps, audit/redaction, isolation, size limits,
    HTTP, stdio, multiprocess, and worker-independent recovery.
  - Distribution: `powershell -NoProfile -File scripts/dagger-ci.ps1 -Call distribution`,
    exit 0. Debian bookworm and Alpine 3.22 pass installer checks and marketplace
    reproducibility, including checkpoint guidance scenarios, token budget,
    mutation fixtures, and canonical/package parity.
  - Feature-enabled lint: `powershell -NoProfile -File scripts/dagger-ci.ps1 -Call clippy`,
    exit 0; strict all-target default and `otel` checks both pass in the
    canonical container environment.
  - Native guidance recheck after whitespace cleanup:
    `py -3.12 -B scripts/check-checkpoint-guidance.py --self-test`, exit 0.
  - Smoke boot: `target/debug/mmhmem.exe --help`, exit 0. Full-suite
    stdio tests also spawn the built artifact, initialize MCP, checkpoint,
    terminate/reconnect, and resume persisted state.
  - Placeholder/secret review: no new unfinished production paths or real
    credentials; test secrets are synthetic redaction fixtures.
- INTEGRATION:
  - Cross-task contracts: PASS. The service provides persisted save timestamps;
    the pure formatter consumes them; the existing handler delegates while
    retaining worker notices. Recap commands request the existing tool and
    faithfully describe every recovery case.
  - Shared state consistency: PASS. No schema, project-key, or tool-argument
    changes. Checkpoint and breadcrumb commit together; audit failure rolls back
    both database writes; acknowledgements follow commit.
  - Boundary audit: PASS. MCP depends on services, services on storage;
    formatter has no storage/worker imports. No hooks, transcript import,
    signal handling, new dependencies, or background scheduler were introduced.
- COVERAGE:
  - Requirements mapped: 19/19 acceptance criteria; matrix below.
  - Coverage gaps: none for the specified instruction and persistence contracts.
- DESIGN:
  - Architecture drift: none. Service tests were extracted into an adjacent
    test module to preserve the existing source LOC cap. Generated plugin
    manifests reflect canonical metadata, through the existing builder.
  - Dependency direction: no violations.
  - File Structure Plan versus actual: matches, with the service test-module
    extraction and generated metadata noted above.
  - Revalidation triggers: timestamp, response/task-ID parsing, audit ordering,
    token budget and package parity all rechecked by relevant tests/gates.
- OWNERSHIP: LOCAL.
- UPSTREAM_SPEC: N/A.
- BLOCKED_TASKS: none.
- REMEDIATION: none.

## Requirements Evidence

| Criteria | Implementation and fresh evidence |
|---|---|
| 1.1 | Canonical/server/recap initial-objective instructions; guidance scenario and mutation checks |
| 1.2 | Four milestone triggers in each entry point; independent per-entry scenario checks |
| 1.3 | Changed-state final-response instructions; guidance checks |
| 1.4 | Unchanged-state save skip; guidance checks |
| 1.5 | Exact identity and concise verified/pending state instructions; HTTP isolation/redaction/cap assertions |
| 2.1 | Advance-notice clear/handoff instructions in all entry points; scenario checks |
| 2.2 | Ctrl+C/native-clear limitation; abrupt-process test recovers checkpoint A rather than unsaved B |
| 2.3 | Fresh HTTP-session and stdio-process recovery, no worker or service restart required |
| 2.4 | Service timestamp fidelity and formatter UTC/epoch tests; transport save-time assertions |
| 3.1 | History-only formatter and HTTP tests expose breadcrumbs without invented state |
| 3.2 | Empty recovery formatter and fresh-store tests explicitly report no state/activity |
| 3.3 | Recap instructions and response explain separate harness history and original-conversation recovery |
| 3.4 | Task-ID initialization creates no checkpoint; formatter/service tests distinguish it from saving |
| 4.1 | Failure warnings and acknowledgement guidance; stdio error test has no success receipt |
| 4.2 | Service breadcrumb/audit rollback tests preserve prior state; abrupt-exit and unread-reply tests preserve committed state |
| 4.3 | Canonical/package bytes, three recap templates, metadata builders and two-userland distribution gate |
| 4.4 | Full suite retains redaction, exact project isolation, size caps and worker-independent operation |
| 4.5 | Full suite covers checkpoint/disconnect/resume, later unsaved state, both missing-state cases and save failures |
| 4.6 | Per-entry workflow checks and mutation fixtures cover milestones, final-response differences and interruption limits |

## Verification Result

- STATUS: VERIFIED.
- CLAIM_TYPE: FEATURE_GO.
- CLAIM: Implemented proactive-checkpoint instructions, truthful recovery, and
  persistence failure contracts pass the required integration gates.
- EVIDENCE: Full native suite, real-artifact smoke/transport recovery, canonical
  two-userland distribution and default/otel lint, plus requirements and
  architecture assessments above.
- GAPS: Live model compliance is not measured by static instruction checks.
  The specification explicitly makes instruction-driven save frequency best
  effort. Work after the last successful checkpoint can still be lost.
- NOTES: New recovery metadata requires the new binary; the save cadence
  requires updated installed guidance. Existing running agents acquire those
  artifacts through their normal restart/reconnect lifecycle. No store migration
  or service configuration change is required.

## Evidence Logs

All logs are local ignored artifacts under `tmp/`:
`checkpoint-task6-fmt.log`, `checkpoint-task6-clippy.log`,
`checkpoint-task6-tests.log`, `checkpoint-task6-distribution.log`,
`checkpoint-task6-dagger-clippy.log`, `checkpoint-task6-smoke.log`,
and `checkpoint-task6-guidance.log`.
