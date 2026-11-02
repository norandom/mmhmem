# Static Linux release: final implementation validation

Validated on 2026-11-02 against source revision
`a3fcaee1ceb98c7f9b4e9e6bd8097c779428d766`.

## Validation Report

- DECISION: MANUAL_VERIFY_REQUIRED
- IMPLEMENTATION_QUALITY: PASS
- MECHANICAL_RESULTS:
  - Tests: PASS — the fresh canonical `all` pipeline and follow-up export exited 0; fresh native PowerShell 5.1 installer, wrapper and publisher regressions also exited 0.
  - TBD/TODO grep: CLEAN — the only placeholder-shaped match was the existing `mktemp` filename template `XXXXXX`, not deferred implementation.
  - Secrets grep: CLEAN — matches were test-generated bearer tokens and a deliberately invalid fixture token; no production credentials were introduced.
  - Smoke boot: PASS — the same candidate started in scratch and pristine Debian/Alpine images, served local MCP and authenticated HTTP MCP, and shut down cleanly.
- INTEGRATION:
  - Cross-task contracts: PASS — builder, inspector, runtime reports, assembler, schema checker, native wrapper and draft publisher agree on artifact identity and evidence format.
  - Shared state consistency: PASS — public-MCP observation/checkpoint recovery survived process restart and installer update.
  - Boundary audit: PASS — only the separately approved upstream repairs described below extend beyond static-release infrastructure.
- COVERAGE:
  - Requirements mapped: 29/29 original acceptance criteria are accounted for; 27 remain in scope.
  - Verified implementation criteria: 25.
  - Publication-dependent criteria: 4.1 and 5.1 remain pending operator action.
  - Excluded criteria: 2.6 and 2.7, by the user's explicit TLS scope amendment.
- DESIGN:
  - Architecture drift: None outside the recorded scope amendments and approved upstream repairs.
  - Dependency direction: CI orchestration invokes repository-owned black-box checkers; probes use CLI/MCP/HTTP and do not import application internals or write the database directly. The publisher consumes verified hosted artifacts and cannot rebuild the executable.
  - File Structure Plan vs actual: Core planned files are present. Focused assembly, installer, native schema-validation and positional-transport helpers keep those responsibilities in their owning infrastructure boundaries.
- OWNERSHIP: LOCAL — the remaining action is the release transition, not an unresolved code defect.
- UPSTREAM_SPEC: N/A — the approved `local-mode` and `corpus-warmup-prune` prerequisite defects are repaired and revalidated.
- BLOCKED_TASKS: No unresolved implementation blocker. Task 3.3 is withdrawn. Independent task-7 readiness review approved; all in-scope implementation tasks are complete.
- REMAINING_MANUAL_ACTION: Use a successful trusted CI run on the chosen tag's exact commit, assemble the complete existing draft, invoke guarded delivery, publish through the operator release process, confirm the latest Linux endpoint's checksum, and only then apply the prepared static portability wording. No real release upload or publication was invoked during this implementation run.

The decision is deliberately not `GO` for full feature acceptance: publication
and latest-endpoint confirmation have not occurred. The implementation and
reviewable bundle passed their automated quality gates.

## Fresh commands and evidence

| Command | Result | Evidence |
|---|---|---|
| `powershell -NoProfile -File scripts/dagger-ci.ps1 -Call all -ExportPath tmp/static-release-final` | Exit 0 | `tmp/static-final-all.log` |
| `powershell -NoProfile -File scripts/test-install.ps1` | Exit 0 | `tmp/final-native-installer.log` |
| `powershell -NoProfile -File scripts/test-dagger-ci.ps1` | Exit 0 | `tmp/final-native-wrapper.log` |
| `powershell -NoProfile -File scripts/test-publish-linux-release.ps1` | Exit 0 | `tmp/final-native-publisher.log` |
| Independent native reconciliation of executable/SHA256SUMS/lockfile/runtime identities and report inventory | Pass | `tmp/static-final-evidence.json` |
| `git diff --check` | Exit 0 | Final working-tree check |

The canonical pipeline passed every summary gate: static release verification,
verified-candidate installation/update, Debian/Alpine distribution checks,
rustfmt, default and OTel clippy, the GNU deterministic suite, the unchanged
OTel Docker payload, both compose configurations, and systemd/logrotate
deployment validation. The static gate also ran locked musl lint, its full
deterministic suite and optimized release build. The optional credentialed
model-measurement suite is not part of this deterministic release gate.

The fresh native publisher suite passed two read-only valid cases, 43
provenance/bundle rejection cases with zero writes, and exact delivery,
idempotent/partial retry, lost-acknowledgement, mutation, state-race and
download-equality scenarios against fake `gh` responses. Those are offline
delivery tests, not actual release uploads.

Earlier negative component evidence remains applicable to unchanged code:
`tmp/static-inspection-review.log` proves static-PIE acceptance and incompatible
ELF rejection; `tmp/static-gate-green.log` proves actual-candidate failed-runtime
injection removes verification/checksum success files and retains diagnostics.
The assembler, its tests and the schema checker have no diff between their
accepted task-4.1 revision `687a96b` and this final source revision. They were
not needlessly rerun as an extra broad suite. The fresh pipeline separately
revalidated the final assembled bundle before export.

No TLS-specific test entrypoint, certificate fixture test or outbound-worker
TLS acceptance scenario was invoked by the final gate. The previously built
standalone HTTPS fixture remains an optional CI utility, not a release-gate
dependency. Production TLS behavior is unchanged; reverse proxy/load-balancer
TLS is administrator-owned and outside this implementation.

## Reviewable final bundle

- Directory: `tmp/static-release-final/`
- Manifest: `tmp/static-release-final/verification.json`
- Source revision: `a3fcaee1ceb98c7f9b4e9e6bd8097c779428d766`
- Validation ID: `all-37e379aed3c5473f952e9152c0d23e62`
- Package version: `0.2.2`
- Target: `x86_64-unknown-linux-musl`
- Executable: `mmhmem-linux-x86_64`
- Size: 20,188,912 bytes
- SHA-256: `fa3bffb60baf890e224a71371e969b513e742dca647d56d84b217e09867c96fc`
- Cargo.lock SHA-256: `b214bcac0aac73d89182922193440c48de6cd045281c06806e35ddad12dee8e0`
- Compiler: `rustc 1.98.1 (48a229cea 2026-09-01)`
- Cargo: `cargo 1.98.1 (797e8a9bc 2026-08-05)`
- Build image: `docker.io/library/rust:1.98-slim-bookworm@sha256:ff521445a372125ed4f76e1453a1f8098f2d05332d1601d30db1c1f62757e730`
- Source cleanliness: `false`; publishable: `false`.

The local result is intentionally non-publishable because the pre-existing
untracked `.openidea/` directory and root `mmhmem-linux-x86_64` file remain in
the worktree. They were preserved. The native wrapper captured cleanliness
before source sync. Tracked implementation sources were committed at the
revision above before final validation began.

Independent reconciliation matched the executable hash, exact SHA256SUMS
content, lockfile hash, source/validation identifiers and both runtime case
sets. `tmp/static-final-evidence.json` inventories SHA-256 hashes for all nine
retained reports, without modifying the verified bundle.

The ELF report identifies a legitimate ELF64 x86-64 static PIE (`DYN`) with no
PT_INTERP, DT_NEEDED or GLIBC version requirements. The scratch and pristine
version starts passed. Functional probes run in instrumented images containing
test-tool dependencies; they do not claim those images lack OpenSSL tools.

| Runtime | Resolved image | Actual kernel | Result |
|---|---|---|---|
| Debian 12/bookworm | `docker.io/library/debian:bookworm-slim@sha256:99c0b25c46d0214d5a2091092e7908907dcaecd6a962cde0ac700acee222b6f3` | `6.18.54.1-microsoft-standard-WSL2` | All nine cases passed |
| Alpine 3.22 | `docker.io/library/alpine:3.22@sha256:2e0b81d60597f449baa56dc25d355367869d2dca4cd57e866f1e2b52af3eb55f` | `6.18.54.1-microsoft-standard-WSL2` | All nine cases passed |

Both reports identify `x86_64` on an AMD Ryzen AI 9 HX 370 host. Required cases
are `pristine_startup`, `version`, `mcp_initialize`, `tools_list`, `persistence`,
`stats_text`, `stats_json`, `server_health` and `server_auth`. The same checksum
is recorded in each report. Containers share the host kernel: this establishes
userland portability, not tests on separate distribution kernels. Linux 5.10
and baseline x86-64/SSE2 are the declared support policy; neither the minimum
kernel nor SSE2-only hardware was physically tested by this run. WSL is CI
infrastructure only and is not a documented user installation target.

Hosted integration has independent evidence: successful
[CI run 41495286781](https://github.com/norandom/mmhmem/actions/runs/41495286781)
for revision `2339ae1b7f868b33595586824cb7325324755864` exported the uniquely
named clean, publishable bundle. Its executable has the same checksum, and
native validation passed read-only (`tmp/hosted-bundle-validation.log`). That
earlier run is evidence of hosted routing, not proof of final-HEAD CI success.
The same validated runtime source subsequently passed
[CI run 41502943129](https://github.com/norandom/mmhmem/actions/runs/41502943129)
at 2026-11-02 19:58 CET, including Windows, macOS and the Dagger quality job.
Its unexpired artifact is
`linux-static-a3fcaee1ceb98c7f9b4e9e6bd8097c779428d766-41502943129-1`.
The downloaded bundle at `tmp/static-release-hosted-final/` passed independent
native PowerShell 5.1 schema/evidence validation with `source_clean: true`,
`publishable: true`, validation ID `ci-41502943129-1`, and the same executable
checksum as the local bundle. Evidence: `tmp/final-hosted-bundle-validation.log`.
This is a verified hosted artifact, not a published release.

## Acceptance-criterion traceability

| Criterion | Status | Concrete evidence |
|---|---|---|
| 1.1 | VERIFIED | Final ELF report, scratch/pristine startup, same-file public-interface runtime probes. |
| 1.2 | VERIFIED | Strict inspector rejects PT_INTERP/DT_NEEDED; incompatible/static-PIE fixture evidence and fresh candidate inspection. |
| 1.3 | VERIFIED | Scratch and untouched Debian/Alpine startup; linkage report proves no loader/shared-library requirements. Instrumented functional tests are described separately. |
| 1.4 | VERIFIED | Target-suite active dependency graph and `optional_telemetry: false`; existing rustls/AWS-LC provider retained. |
| 2.1 | VERIFIED | Candidate checksum/version agree across manifest, linkage, scratch and both runtime reports, with pre/post probe hashing. |
| 2.2 | VERIFIED | `mcp_initialize` and `tools_list` pass on both userlands; existing nine-tool surface present. |
| 2.3 | VERIFIED | Public observation/checkpoint writes, clean EOF, second process, search/detail/checkpoint recovery. |
| 2.4 | VERIFIED | `stats_text` and `stats_json` contain the recorded session row and existing typed aggregates; zero-valued public client-usage aggregates are not mislabeled as savings. |
| 2.5 | VERIFIED | Health 200, missing/wrong bearer 401, authenticated initialize 200, correct identity/tools and clean SIGTERM exit. |
| 2.6 | EXCLUDED | User explicitly omitted all TLS-specific tests, including outbound worker compatibility tests. |
| 2.7 | EXCLUDED | Same explicit TLS scope amendment; no TLS policy was changed. |
| 3.1 | VERIFIED | Native wrapper and hosted workflow call the same Dagger module/gate; actual local and final-source hosted bundles validated. |
| 3.2 | VERIFIED | Explicit full source revision, unchanged committed lockfile, locked target commands and lockfile checksum; dirty local output is non-publishable. |
| 3.3 | VERIFIED | Fresh musl default-feature lint, deterministic suite, release build and scoped runtime gate passed. |
| 3.4 | VERIFIED | Awaited fail-closed assembly/checker, retained failure diagnostics, unchanged actual-candidate failure-injection evidence, native publisher rejection cases. |
| 3.5 | VERIFIED | Exported source, target, checksum, validation, image, kernel, CPU and named runtime-case records reconcile. |
| 3.6 | VERIFIED | Native PowerShell entrypoint, positional checked-in transport, native wrapper regressions; no Git Bash or ad hoc WSL validation. |
| 3.7 | VERIFIED | Linux builds, executable starts and runtime probes occur only inside Dagger containers; WSL transports CI sources/results. |
| 4.1 | MANUAL_PENDING | Offline exact-byte draft delivery passes; actual guarded delivery/publication and latest-endpoint checksum confirmation have not been invoked. |
| 4.2 | VERIFIED | Both candidate installer scenarios use the stable latest-release Linux URL and stub compiler/checkout tools to prove independence. |
| 4.3 | VERIFIED | Both userlands recover public-MCP memory after update and preserve worker-setting bytes and harness configuration. |
| 4.4 | VERIFIED | Dry-run and failed-download installer cases preserve binary/configuration; dry-run makes no binary download under the existing companion-download contract. |
| 4.5 | VERIFIED | Unsupported-platform test refuses selection and preserves state. |
| 4.6 | VERIFIED | Existing Windows/macOS names/jobs and marketplace paths preserved; native Windows installer passes; publisher writes only three Linux delivery files in offline tests. |
| 5.1 | MANUAL_PENDING | Prepared static wording stays conditional until actual publication and latest checksum confirmation. |
| 5.2 | VERIFIED | Current `v0.2.2` Linux SHA-256 remains `309a88e752cac0adf0e2c4d17c0bb5991dc091b67e10011342918c7af902ab7d`; matching artifact has glibc loader, so its caveat is retained. |
| 5.3 | VERIFIED | README/deploy guidance states architecture, Debian/Alpine baselines, 5.10/SSE2 policy, actual tested-kernel distinction and installer prerequisites. |
| 5.4 | VERIFIED | Guidance retains the unchanged distribution-trust-store requirement and rejection behavior, with administration outside scope and no TLS setup/testing claims. |
| 5.5 | VERIFIED | Normal Linux installation is distinguished from Windows-native use and CI-only WSL transport; no Git Bash installation path. |

## Boundary and readiness assessment

Against pre-implementation revision `032c28a`, Cargo manifests/lockfile,
Dockerfile, production installers, persistent schema, worker code and
marketplace/package directories have no changes. Windows/macOS workflow jobs
retain their build/asset mechanics; changes to the workflow are confined to
Linux provenance/export/upload configuration.

The production Rust exception is explicit and separately owned: user-approved
`local-mode` repair `8db5f54` adds bounded SQLite initialization contention
handling and atomic rollback without changing public APIs or schema DDL/version.
Its GNU/default+OTel/musl regressions passed, and this final pipeline revalidated
dependent runtime/installer behavior. Test-only `corpus-warmup-prune` repair
`c9b11a2` removes accidental security-pattern matches from random fixture paths.
No downstream workaround, skipped failing application test, crypto-provider
substitution or GNU fallback was introduced.

The publisher requires an existing draft and a successful trusted workflow run
on the tag's exact commit. It checks clean/publishable bundle identity, refuses
conflicting assets, rechecks draft/local state before writes, and verifies
downloaded bytes. The operator still needs to create/assemble/publish the
complete release through the existing process. Publication is not an automatic
implementation action. The latest release remains unchanged.

## Verification Result

- STATUS: VERIFIED
- CLAIM_TYPE: TEST_OR_BUILD
- CLAIM: Final-source automated build, runtime, installer and offline publisher gates passed and produced a reviewable verified local bundle.
- EVIDENCE: Fresh exit-0 commands above, final manifest/report reconciliation and boundary checks.
- GAPS: Local output is non-publishable; the trusted final-source hosted bundle is clean and publishable. No real release upload or publication has been performed.
- NOTES: Independent task-7 review approved after checking fresh logs, source identity, exact checksum file, report hashes and both runtime case sets. The controller has marked task 7 complete.

## Feature Acceptance Result

- STATUS: MANUAL_VERIFY_REQUIRED
- CLAIM_TYPE: FEATURE_GO
- CLAIM: All original in-scope release-transition acceptance criteria are satisfied.
- EVIDENCE: Automated implementation evidence covers 25 criteria; two publication-dependent criteria remain explicitly pending, and two TLS criteria are excluded.
- GAPS: Actual guarded draft delivery/publication, latest-endpoint checksum confirmation, and subsequent application of the prepared static portability wording.
- NOTES: Do not declare full feature acceptance or publication until those operator checkpoints are verified.

## User-requested documentation follow-up

After runtime validation, the README was shortened to per-platform Linux,
Windows and macOS installation, essential use, and one shared optional-server
section. Detailed installation, configuration, usage, server and development
guides now live in `docs/`. Linux release builds are explicitly static musl by
default; the current legacy v0.2.2 download caveat remains accurate.

`Self-Hosted Cross-Agent Memory Solutions.md` moved to `docs/concept.md`
without content changes. Its SHA-256 remains
`9799d594f7b38281bb0851a2453e453d643a69ab4e8d097f2e41e846185ab2da`.
Independent PowerShell 5.1 documentation review passed 31 local links/anchors,
three command examples, code fences, README image and concept hash checks
(`tmp/streamlined-docs-review.log`). Deployment cross-references and the
post-publication wording instructions were updated to match the new structure.

These changes and controller-owned task/status records are documentation
only. Runtime source, dependencies, CI scripts, workflow and Docker inputs
are unchanged from the validated `a3fcaee` revision. The passing runtime
checks were not needlessly repeated for prose and file moves.
