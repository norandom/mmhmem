# Requirements Document

## Project Description (Input)

The blocking Dagger gate rebuilds every dependency from source on each hosted
run, about five times per run, including the C builds of the TLS stack
(aws-lc) and bundled SQLite, and it also compiles tokei for an informational
report. Add a compile cache that persists across hosted runs (sccache with
the GitHub Actions cache backend inside the Dagger build containers), move
the tokei LOC report to its own non-blocking cached job, keep the existing
TLS stack and make its C build cacheable, and document what the TLS stack is
for. Findings are in `research.md`; the user's decisions are in `brief.md`.
The user requested this work through the Kiro specification workflow, with
its requirements, design and tasks approval gates.

## Introduction

This specification governs how compiled dependency outputs are reused across
CI runs, how that cache is secured, where the LOC report runs, and the stated
role of the TLS stack. It completes the cross-run caching that ci-build-time
deferred in its Decision 5.

## Boundary Context

- **In scope:** compile caching for every Rust build in the blocking gate
  (musl clippy, tests and release, glibc otel clippy, the otel Docker image),
  the cache credential hand-off into build containers, cache fallback and
  reporting, the tokei LOC job, and documentation of the TLS stack.
- **Out of scope:** changing the TLS crypto provider or any product code,
  release publication logic, the native Windows and macOS cache steps,
  larger or self-hosted runners, and caching outside GitHub Actions.
- **Adjacent expectations:** ci-build-time's stage runner, memory guard and
  timing summary keep working; static-linux-release's same-artifact
  verification and bundle manifest stay unchanged; the Dockerfile stays
  buildable on its own; cross-agent-memory-server Requirement 13.5 (an
  informational tokei report in CI) stays satisfied.
- **Baseline:** cold hosted gate 666 s (run 41643349698); local warm gate
  252 s.

## Requirements

### Requirement 1: Compiled dependencies are reused across hosted runs

**Objective:** As a maintainer, I want the gate to reuse compiled dependencies
from earlier runs, so that a push compiles only what changed.

#### Acceptance Criteria

1. When a hosted gate run starts and an earlier run on the same branch has stored cache entries for the same toolchain and target, the CI Pipeline shall reuse cached compilation outputs for unchanged crates in every Rust build of the gate, including the otel Docker image build.
2. The CI Pipeline shall include in the cache the C code that dependency build scripts compile, including the TLS stack and bundled SQLite, wherever those build scripts accept a compiler wrapper; any dependency whose C build cannot be cached shall be named in the validation report.
3. If the cache backend is unreachable, rejects the credential, or is over its storage limit, the CI Pipeline shall complete the gate without the cache and report the same verdict it would report with the cache.
4. Where the gate runs locally through the wrapper, the CI Pipeline shall run without any GitHub credential and without changing the local default behaviour.
5. When five consecutive green `main` runs follow a run that populated the cache, the CI Pipeline shall complete the blocking gate job in at most 360 seconds, measured as their median.
6. When the cache holds no usable entries, the CI Pipeline shall complete the blocking gate job in at most 110 percent of the 666-second cold baseline.

### Requirement 2: Cache integrity and credential safety

**Objective:** As a maintainer, I want the cache to speed up builds without
weakening release integrity or leaking credentials.

#### Acceptance Criteria

1. The CI Pipeline shall keep producing the Linux release bundle through the existing same-artifact verification, with an unchanged manifest schema and artifact identity.
2. The CI Pipeline shall use cache entries only from runs whose cache scope GitHub grants to the current run, so that pull-request runs cannot write entries that `main` or tag runs restore.
3. The CI Pipeline shall pass the cache credential into build containers only as a secret, and shall never write it to logs, cache entries, image layers, job summaries or uploaded artifacts.
4. The Dockerfile shall remain buildable with a plain `docker build` that has no cache credential and no cache service.
5. When the Rust toolchain version or the build target changes, the CI Pipeline shall not reuse cache entries produced by a different toolchain or target.

### Requirement 3: TLS stack kept and explained

**Objective:** As a maintainer, I want to know why a TLS library is compiled
at all and to keep the shipped crypto unchanged, so that build speed does not
come from a security-relevant swap.

#### Acceptance Criteria

1. The shipped binaries shall keep the current TLS stack: rustls with the aws-lc-rs crypto provider.
2. The development guide shall state that the TLS stack serves only outbound HTTPS, namely worker-model API calls and, in the otel Docker image, the OTLP exporter, and that the mmhmem server terminates no TLS itself.
3. The development guide shall state that the TLS stack's C build is cached rather than replaced, and how to confirm its cache hits.

### Requirement 4: LOC report in its own non-blocking job

**Objective:** As a maintainer, I want the informational LOC report kept but
out of the gate's way, so that it no longer costs gate time.

#### Acceptance Criteria

1. The CI Pipeline shall produce the per-file tokei LOC report in a job separate from the blocking gate that runs concurrently with it.
2. The CI Pipeline shall keep the workflow verdict and release publication independent of the LOC job's outcome.
3. When a cached tokei binary of the pinned version exists, the LOC job shall not compile tokei.
4. When the LOC job finishes, the CI Pipeline shall write the report to that job's summary.
5. The blocking gate shall no longer run the LOC stage, and the local wrapper shall still offer the report on demand.

### Requirement 5: Cache visibility

**Objective:** As a maintainer, I want to see whether the cache worked on
each run, so that a silent cache failure does not go unnoticed.

#### Acceptance Criteria

1. When the blocking gate finishes, the CI Pipeline shall write the compile-cache hit and miss counts to the job summary.
2. If the cache was disabled or unavailable for a run, the CI Pipeline shall state that in the job summary.
3. When acceptance is measured, the CI Pipeline's timing baseline shall be updated to the measured warm-cache stage durations.
