# Implementation Plan: Static Linux Release

Requirements, design and tasks are approved. Task approval was granted by the user on 2026-11-02.
Each executable task targets a 1–3 hour work unit; compilation and hosted CI
waiting time may extend elapsed time. All tests below are required.

Execution follows the canonical CI entrypoint for Linux work and native
PowerShell for Windows work. No Git Bash or ad hoc WSL execution is allowed.
File ownership follows the approved design's File Structure Plan.
Production Rust, dependency versions, Dockerfile, Windows/macOS assets and
marketplace packages remain outside the change boundary.

## Tasks

- [x] 1. Establish build and probe prerequisites
- [x] 1.1 Establish the isolated musl toolchain
  - Select the exact approved compiler, install the musl Rust target and native C compiler, and isolate target flags and build caches from GNU/OTel builds.
  - Retain bounded build jobs and disabled incremental compilation; record compiler, linker and image identity.
  - Make the setup exercisable through the canonical CI entrypoint as a status-only check, without exporting an unverified release binary.
  - Done when the CI setup check compiles and inspects a minimal static target fixture and reports the selected toolchain without invoking Git Bash or running application binaries directly in WSL.
  - _Boundary: Static builder_
  - _Requirements: 1.1, 3.1, 3.2, 3.6, 3.7_

- [x] 1.2 Establish runtime and evidence test infrastructure
  - Prepare the approved Debian and Alpine images, pristine startup stages, a scratch startup stage, and instrumented probe stages.
  - Supply readelf, Python probe tooling, temporary certificate generation and CA-store setup inside CI; separate these from application runtime prerequisites.
  - Establish isolated writable stores, bounded child-process execution, captured diagnostics, report destinations and fresh validation IDs.
  - Define test fixtures matching the approved bundle schema, including source identity, exact artifact checksum and required runtime-case names.
  - Done when both runtime environments and disposable hostname/certificate fixtures can be created through CI, and schema fixtures are available to downstream component tests.
  - _Boundary: Artifact verifier test infrastructure_
  - _Requirements: 1.3, 2.1, 2.6, 2.7, 3.5, 3.7_

- [x] 2. Implement candidate production and independent verification components
- [x] 2.1 Produce the static application candidate and target regression results
  - Build the existing locked dependency graph using the musl C compiler and target-specific Rust linker; preserve the existing crypto provider.
  - Run musl-target default-feature lint and the deterministic regression suite, then produce the release candidate outside the build cache.
  - Record active target dependencies and confirm optional telemetry is not enabled.
  - Fail explicitly on native compilation or regression failures; never substitute a GNU artifact.
  - Done when the canonical CI build check produces a candidate internally and reports passing static-target tests with its version and checksum.
  - _Boundary: Static builder_
  - _Depends: 1.1_
  - _Requirements: 1.1, 1.3, 1.4, 3.1, 3.2, 3.3, 3.4_

- [x] 2.2 (P) Enforce static ELF acceptance
  - Validate architecture, ELF class, interpreter declarations, required shared libraries and GLIBC version requirements for an explicit file.
  - Accept legitimate static PIE metadata without mistaking every dynamic metadata section for dynamic linkage.
  - Exercise positive static and negative dynamic/incorrect-architecture fixtures; include malformed input and checksum mismatch.
  - Done when the inspector emits structured evidence for valid static inputs and fails every incompatible fixture with an actionable diagnostic.
  - _Boundary: Artifact verifier ELF inspection_
  - _Depends: 1.1, 1.2_
  - _Requirements: 1.1, 1.2, 1.3, 3.4, 3.5_

- [x] 2.3 (P) Provide the deterministic HTTPS worker fixture
  - Serve the existing worker response format through a hostname covered by a disposable certificate SAN.
  - Expose request receipts and unique response content for successful worker requests; retain TLS diagnostics for rejected connections.
  - Generate fresh trusted and untrusted CA fixtures per validation ID without embedding private keys in exported evidence.
  - Done when an independent test client accepts the trusted fixture, rejects the untrusted fixture and can correlate an accepted request with its receipt.
  - _Boundary: Artifact verifier worker fixture_
  - _Depends: 1.2_
  - _Requirements: 2.6, 2.7, 3.4_

- [x] 2.4 (P) Validate release-bundle consistency
  - Implement the approved schema and checksum checks against explicit bundle inputs.
  - Require the correct target, revision, required runtime records, linkage/regression status and artifact identity.
  - Reject unknown schema versions, missing or duplicate records, malformed hashes and contradictory passed states.
  - Done when valid synthetic bundles pass and tampered bytes, revisions, targets and evidence fail deterministic checker tests.
  - _Boundary: Bundle exporter schema checker_
  - _Depends: 1.2_
  - _Requirements: 3.4, 3.5, 4.1_

- [ ] 3. Exercise the release file through public application interfaces
- [x] 3.1 Verify local MCP behavior and persistent state
  - Upstream local-mode task 2.1 repair approved, implemented and independently reviewed; fresh downstream local-probe verification is required before completion.
  - Accept an explicit candidate path and expected checksum; do not select Cargo's implicit test executable.
  - Check version and MCP initialization/tool discovery, write an observation and checkpoint through public tools, then restart against the same isolated store.
  - Assert memory search/detail retrieval and both existing statistics output formats after the recorded session.
  - Bound every child operation and capture stderr on failure.
  - Done when the probe report proves the named candidate recovered state across process restart and returned the expected CLI/MCP results.
  - _Boundary: Artifact verifier application probe_
  - _Depends: 1.2, 2.1, 2.2_
  - _Requirements: 2.1, 2.2, 2.3, 2.4, 3.4_

- [x] 3.2 Verify server startup and authentication
  - Extend the same explicit-artifact probe with server startup, health success, unauthenticated MCP rejection and authenticated MCP initialization.
  - Confirm bounded shutdown and useful failure diagnostics without changing production authentication behavior.
  - Done when the probe report includes all server cases for the same candidate checksum and leaves no child process running.
  - _Boundary: Artifact verifier application probe_
  - _Requirements: 2.1, 2.5, 3.4_

- [ ] 3.3 Verify hostname resolution and TLS trust through the candidate
  - Connect the worker fixture by its Dagger service hostname and install only the intended test CA through normal runtime trust-store mechanisms.
  - Trigger actual worker-backed reads using fresh stores and non-security-sensitive files exceeding the shunt threshold.
  - Require the unique delegated response and matching receipt for the trusted case; prevent cache hits or raw-read fallback from satisfying success.
  - Repeat with a fresh process and untrusted CA, asserting rejection under the existing tool error/fallback contract and no accepted worker request.
  - Done when both trust outcomes are reported for the candidate without TLS bypasses, public network dependencies or model credentials.
  - _Boundary: Artifact verifier application probe_
  - _Depends: 2.3, 3.1, 3.2_
  - _Requirements: 2.1, 2.6, 2.7, 3.4_

- [ ] 4. Integrate verification with the canonical release pipeline
- [ ] 4.1 Gate bundle creation on same-artifact verification
  - Integrate the builder, ELF inspector, pristine/scratch startup checks and every application probe into the canonical Dagger gate.
  - Run both runtime images against the same candidate bytes and verify checksums before and after their probes.
  - Assemble the bundle only after target regression, linkage and all runtime cases pass; run the schema checker on the final result.
  - Preserve failed diagnostics without creating a success or publishable bundle.
  - Done when the gate returns a complete verified bundle on success and fails closed after an injected verification error.
  - _Boundary: Integration — static builder, artifact verifier, bundle exporter_
  - _Depends: 2.1, 2.2, 2.4, 3.1, 3.2, 3.3_
  - _Requirements: 1.1, 1.2, 1.3, 1.4, 2.1, 2.2, 2.3, 2.4, 2.5, 2.6, 2.7, 3.3, 3.4, 3.5_

- [ ] 4.2 Export verified Linux artifacts locally and from hosted CI
  - Route existing release consumption and the full gate through the verified candidate while preserving the File-returning compatibility contract.
  - Capture native-host source revision, cleanliness and validation identity before source sync; mark dirty local results non-publishable.
  - Pass verified checkout/run identity from hosted CI, export and upload the uniquely named Linux bundle without rebuilding or modifying its executable.
  - Restrict local exports to repository scratch; invalidate verification-result caching across runs while retaining compilation caches.
  - Done when local and hosted entrypoints use the same gate and the exported bundle records matching source, target, runtime and artifact identities.
  - _Boundary: Integration — bundle exporter, native wrapper, hosted CI_
  - _Depends: 4.1_
  - _Requirements: 3.1, 3.2, 3.4, 3.5, 3.6, 3.7, 4.1, 4.6_

- [ ] 5. Implement guarded draft-release delivery
- [ ] 5.1 Verify release-run provenance before upload
  - Resolve the requested tag and trusted successful CI run, requiring the exact matching commit and all required job results.
  - Download only the named bundle from that run and check schema, publishable state and all artifact hashes.
  - Require an existing draft release; reject conflicting Linux assets and arbitrary local binary inputs.
  - Done when offline fake-gh cases reject wrong repository/workflow, failed jobs, wrong commit, missing artifact, dirty source and tampered evidence before any upload.
  - _Boundary: Draft publisher_
  - _Depends: 2.4, 4.2_
  - _Requirements: 3.4, 3.5, 4.1, 4.6_

- [ ] 5.2 Deliver exact verified bytes idempotently
  - Upload the validated binary, checksum and verification files without rebuild, force overwrite or changes to other platform assets.
  - Re-download the draft asset and compare its hash before reporting success.
  - Treat an identical existing asset as success and a conflicting asset as failure; handle interrupted uploads through remote-state checks.
  - Done when offline publisher tests prove upload/re-download equality, retry behavior and absence of release-creation/publication actions.
  - _Boundary: Draft publisher_
  - _Requirements: 4.1, 4.6_

- [ ] 6. Validate installation compatibility and release guidance
- [ ] 6.1 Preserve installer behavior with the static candidate
  - Exercise isolated first install and update with existing memory and worker-setting fixtures, using the candidate supplied by the verified bundle.
  - Check stable latest-release URL/asset selection, dry-run no-op behavior, failed-download preservation and unsupported-platform refusal.
  - Preserve existing native Windows tests and verify Windows/macOS names and marketplace paths are unaffected.
  - Done when installer regression output proves the candidate installs without a checkout/compiler and preserves the required existing state.
  - _Boundary: Installer and guidance — installer regression_
  - _Depends: 4.2_
  - _Requirements: 4.2, 4.3, 4.4, 4.5, 4.6_

- [ ] 6.2 Prepare accurate support and transition guidance
  - Document the approved support floor, baseline CPU, tested userlands, actual tested kernel distinction, download/shell prerequisites and CA trust behavior.
  - Provide the verified-bundle export, draft upload, final checksum confirmation and rollback procedure.
  - Keep the current glibc caveat while the latest release is dynamic; prepare the static claim as a clearly conditional post-publication update.
  - Keep WSL/Git Bash out of user installation targets and distinguish the unchanged GNU/OTel Docker image.
  - Done when guidance is checked against the current downloadable asset and the pending transition text matches the approved support contract.
  - _Boundary: Installer and guidance — support documentation_
  - _Depends: 4.2, 5.2, 6.1_
  - _Requirements: 5.1, 5.2, 5.3, 5.4, 5.5_

- [ ] 7. Perform final integrated verification and readiness review
  - Run the complete canonical quality gates, static-target suite, both runtime matrices, bundle validation, installer regressions and native publisher tests on the final candidate revision.
  - Reconcile checksum and provenance across the exported bundle and all reports; verify no out-of-boundary application, Docker or marketplace changes entered the patch.
  - Record supported/tested environment limits and map fresh evidence to every acceptance criterion.
  - Done when the implementation has fresh passing evidence and a reviewable verified bundle; explicitly retain pending publication-dependent acceptance items rather than claiming full feature completion early.
  - _Boundary: Integration — complete static-release path_
  - _Depends: 4.2, 5.2, 6.1, 6.2_
  - _Requirements: 1.1, 1.2, 1.3, 1.4, 2.1, 2.2, 2.3, 2.4, 2.5, 2.6, 2.7, 3.1, 3.2, 3.3, 3.4, 3.5, 3.6, 3.7, 4.1, 4.2, 4.3, 4.4, 4.5, 4.6, 5.1, 5.2, 5.3, 5.4, 5.5_

## Execution and Review Notes

- Parallel work is limited to 2.2, 2.3 and 2.4 after group 1. They own
  separate scripts and must not modify orchestration or shared probe code.
  All shared orchestration/probe changes remain sequential.
- Requirement 5 explicitly includes documentation, so task 6.2 is retained
  despite the generic task-generation preference for code-only work.
- Release publication is an operator checkpoint, not a coding task. The
  implementation does not automatically create or publish a release. Once a
  complete release is published, verify the latest endpoint checksum and apply
  the prepared portability wording before closing Requirements 4.1 and 5.1 as accepted.
- Task-plan review: passed (29 acceptance criteria mapped; dependencies and boundaries checked).
- Independent task-graph sanity review: PASS; no blocking task-local fixes.

## Implementation Notes

- Task 3.2: Independent `server-probe` passed health/authentication/Streamable HTTP initialization and graceful SIGTERM shutdown on both userlands (`tmp/server-probe-review-deadline.log`). SSE includes an empty data/id/retry preamble; skip empty events. Socket inactivity timeouts are insufficient: the probe uses a Linux main-thread absolute HTTP deadline, verified with slow headers/body and cleanup regressions.
- Task 3.1 resumed after approved upstream repair `8db5f54`: fresh independent `local-probe` passed component tests, musl build/ELF checks and explicit-candidate local cases on Debian/Alpine (`tmp/local-probe-review.log`). Current candidate SHA-256 is `fa3bffb60baf890e224a71371e969b513e742dca647d56d84b217e09867c96fc`. Statistics assert populated recorded-session rows and existing typed zero-valued aggregates, not invented nonzero savings. Final bundle inspection must use the release basename.
- Task 3.1: Earlier explicit-candidate local probes passed Debian and Alpine; final verification exposed a genuine upstream concurrent-first-open storage failure. Independent debug reproduced child exit 1 (`database is locked: Error code 5`) within 0.5 seconds with a 5000 ms timeout. Debug returned `STOP_FOR_HUMAN`; no production workaround, test filtering or acceptance retry was applied. Partial probe changes remain uncommitted pending upstream repair and fresh review. Evidence: `tmp/local-probe-final.log`, `tmp/diagnose-fresh-open.log`; scope report: `implementation-blocker.md`.
- Task 2.4: `check_bundle.py --bundle DIR --expected-revision SHA [--require-publishable]` validates artifact and referenced report consistency. Runtime reports require passed status, validation ID and matching name/image/kernel/architecture/cpu/artifact_sha256/cases. Independent `bundle-check` passed ten checker and five support tests (`tmp/bundle-check-review.log`).
- Task 2.3: Reusable `worker_service()` uses Dagger `as_service(args=...)`, explicitly starts/stops across probes, and exposes public roots plus HTTPS `/receipts`. Independent `worker-fixture` passed hostname/trust/rejection/receipt checks (`tmp/worker-fixture-review.log`). Health checks can produce initial TLS EOF diagnostics; compare later diagnostics against their baseline.
- Task 2.2: `inspect_static.py` accepts explicit binary/checksum/report inputs and validates static PIE without rejecting all dynamic metadata. Six positive/negative component tests passed independent canonical `static-inspection` (`tmp/static-inspection-review.log`). Final candidate inspection remains an integration gate.
- Task 2.1: Locked musl lint, full deterministic tests and release build passed independent review (`tmp/static-build-review.log`), producing version 0.2.2, SHA-256 `41ebf9c37f19a7385d039a5d87030205c1e5aee04efe1afb65ba3e2481511556`. The first suite exposed an upstream random-path fixture bug, repaired separately under corpus-warmup-prune in `c9b11a2` before revalidation. Candidate remains internal pending linkage/runtime gates.
- Task 1.2: Dagger caches whole function results; validation IDs must be explicit host-generated inputs, propagated through every startup/probe/certificate stage. Independent fresh `runtime-setup` passed after resolving this review finding. Shared synthetic bundle/case fixtures live in `ci/scripts/release_fixtures.py` and `release_support.py`.
- Task 1.1: Debian musl-gcc injects a dynamic interpreter for static PIE; retain the musl-only `-Wl,--no-dynamic-linker` flag. Canonical `static-setup` and existing deterministic `test` passed independent review (logs under `tmp/static-setup-review*.log`).
