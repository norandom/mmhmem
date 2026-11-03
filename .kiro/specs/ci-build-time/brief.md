# Brief: ci-build-time

## Problem

Every push to `main` waits about 15 minutes for the blocking CI gate before
the release publisher can run. The time is spent almost entirely on cold Rust
compilation inside one Dagger session whose stages execute strictly one after
another. Native Windows/macOS jobs lose their cargo cache on every version
bump, so release commits are the slowest commits.

## Evidence (see research.md)

- Blocking gate (Dagger) on the last five green pushes: 731 s to 920 s.
- The Dagger `all` function awaits each stage sequentially; independent
  stages never overlap.
- Dagger cache volumes are empty on every GitHub-hosted run. `tokei` is
  rebuilt from source each run (64 s), proving the volumes do not persist.
- `CARGO_BUILD_JOBS=2` exists to protect the ~6 GB WSL engine VM from OOM.
  It also applies on the 4-vCPU/16 GB hosted runner.
- The musl candidate build already runs clippy, tests and a release build.
  The glibc clippy (default) and glibc tests then compile the same code again.
- The Docker stage compiles a third, otel-enabled release build.
- Windows/macOS cache keys hash `Cargo.lock`. Each version bump changes it,
  so Windows went from ~200 s warm to 642 s on the v0.3.3 commit.

## Approach direction (for design, not binding)

1. Overlap independent Dagger stages in CI, with a bounded concurrency
   setting that keeps the local WSL run sequential and memory-safe.
2. Size cargo parallelism to the executing host instead of a fixed 2.
3. Remove redundant compilation only where the target/feature coverage
   matrix stays intact, or where the user approves a documented reduction.
4. Keep informational work (tokei) off the critical path or prebuilt.
5. Make native-runner caches survive version-only `Cargo.lock` changes.
6. Evaluate cross-run persistence for Dagger caches on hosted runners.

## Boundary Candidates

- Owned: `.github/workflows/ci.yml`, the Dagger module under `ci/`,
  `scripts/dagger-ci.*` entrypoints, CI timing reporting.
- Constraints inherited: one Dagger pipeline locally and in CI
  (dev-environment steering), same-artifact release verification
  (static-linux-release), blocking gates and size caps
  (cross-agent-memory-server Req 13), native Windows/macOS gates
  (local-mode Req 10).
- Not owned: release publication logic, E2E model-gated job, product code.
