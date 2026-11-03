# Requirements Document

## Project Description (Input)

The blocking CI gate on `main` takes 12 to 15 minutes per push, and release
commits are the slowest because native-runner caches miss on every version
bump. Reduce CI wall-clock time without weakening any blocking gate, without
splitting the single Dagger pipeline into separate local and CI scripts, and
without breaking the memory-constrained local WSL run. Findings and measured
baselines are in `research.md`. The user requested this work through the Kiro
specification workflow, with its requirements, design and tasks approval gates.

## Introduction

This specification governs how long the CI workflow takes to report a
verdict on a push, and how that time is measured. It does not change what
the product does or what the release artifacts contain.

## Boundary Context

- **In scope:** wall-clock time of the CI workflow jobs on push and pull
  request, stage scheduling and cache behaviour of the Dagger module, cargo
  parallelism settings, native Windows/macOS job caching, and CI timing
  reports.
- **Out of scope:** product source changes, release asset contents, the
  release publication workflow, the model-gated E2E measurement job, and
  changes to toolchain versions.
- **Adjacent expectations:** the same-artifact Linux release verification
  (static-linux-release), the blocking quality gates and size caps
  (cross-agent-memory-server Requirement 13), and the native Windows/macOS
  gates (local-mode Requirement 10) keep their current pass/fail semantics.
  The single Dagger entrypoint stays the only local CI path
  (dev-environment steering).
- **Baseline:** blocking Dagger gate 731 s to 920 s on warm-native green
  runs; Windows 642 s and macOS 361 s after a version bump (research.md).

## Requirements

### Requirement 1: Blocking gate wall-clock target

**Objective:** As a maintainer, I want the blocking CI verdict sooner, so that
pushes and releases do not wait 15 minutes.

#### Acceptance Criteria

1. When a push to `main` triggers CI on a GitHub-hosted runner, the CI Pipeline shall complete the blocking Dagger gate job in at most 540 seconds, measured as the median of five consecutive green runs.
2. When a push to `main` triggers CI, the CI Pipeline shall report the whole-workflow verdict (Dagger, Windows and macOS jobs) in at most 600 seconds, measured as the median of five consecutive green runs.
3. If a change raises the median blocking gate time above the target, the CI Pipeline shall still report the gate verdict unchanged and the timing report shall show the regression.

### Requirement 2: Gate coverage preserved

**Objective:** As a maintainer, I want faster CI without weaker CI, so that a
green run still means what it means today.

#### Acceptance Criteria

1. The CI Pipeline shall keep every currently blocking check blocking: formatting, clippy with warnings denied on default and otel features, the deterministic test suite, the size caps, the verified static Linux candidate, installer and distribution checks, the Docker image payload check, compose validation and deploy artifact validation.
2. The CI Pipeline shall keep a passing check for each target and feature combination checked today, unless the design documents an equivalence argument for dropping that combination and the user approves it.
3. If any stage fails, the CI Pipeline shall fail the job and identify the failing stage by name, as it does when stages run sequentially.
4. When stages run concurrently, the CI Pipeline shall not let one stage's failure mask or cancel the reporting of another stage's failure output.
5. The CI Pipeline shall continue to publish the same verified Linux bundle artifact, with an unchanged identity and manifest format, for the release publisher to consume.

### Requirement 3: One pipeline, local memory safety

**Objective:** As a developer on the Windows host, I want the local Dagger run
to keep working on the ~6 GB WSL engine, so that faster CI does not break
the local gate.

#### Acceptance Criteria

1. The Dagger Module shall expose the same entrypoints to the local `scripts/dagger-ci` wrapper and to GitHub Actions.
2. Where the run is local through the wrapper, the Dagger Module shall default to the current memory-safe behaviour: sequential stages and bounded cargo jobs.
3. Where the run is on a CI runner, the Dagger Module shall use stage concurrency and cargo parallelism chosen from an explicit, recorded setting rather than an implicit host probe.
4. If the configured concurrency would exceed the engine's available memory, the Dagger Module shall fail with a message naming the setting, rather than letting the engine be OOM-killed silently.

### Requirement 4: Cache effectiveness

**Objective:** As a maintainer, I want caches to survive routine commits, so
that release commits are not the slowest commits.

#### Acceptance Criteria

1. When only the crate version in `Cargo.toml` and `Cargo.lock` changes, the native Windows and macOS jobs shall restore a prior dependency cache and complete within 120% of their warm-cache median.
2. When a dependency is added, removed or updated, the native jobs shall not reuse build outputs that belong to the old dependency set for the final result.
3. Where cross-run persistence for Dagger caches is enabled, the CI Pipeline shall keep secrets and credentials out of any persisted cache.
4. The CI Pipeline shall not rebuild informational tooling from source on the critical path of every run.

### Requirement 5: Timing visibility

**Objective:** As a maintainer, I want per-stage durations in each run, so
that I can see where time goes without parsing raw logs.

#### Acceptance Criteria

1. When the blocking Dagger gate finishes, the CI Pipeline shall write per-stage start time and duration to the job summary.
2. The CI Pipeline shall record whether each native job restored its cache, in the job summary.
3. If a stage exceeds its recorded baseline duration by more than 50%, the CI Pipeline shall mark that stage in the summary without failing the job.
