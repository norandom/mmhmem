# Research: ci-build-time

Measured 2026-11-03 from GitHub Actions logs (`gh run view --log`). No
estimates; every duration below comes from a recorded run.

## Job wall-clock, last six CI runs on `main`

| Run | Commit | Dagger gate | Windows | macOS | Note |
|---|---|---|---|---|---|
| 41626826480 | 6dd7e86 (v0.3.3) | 896 s | 642 s | 361 s | version bump, native caches missed |
| 41624244750 | a199a4f (v0.3.2) | 468 s (failed) | 420 s | 544 s | version bump |
| 41622944969 | 7e79fdf | 731 s | 198 s | 174 s | warm native caches |
| 41620475289 | — | 739 s | 234 s | 112 s | warm |
| 41620015753 | — | 920 s | 189 s | 154 s | warm |
| 41619456036 | — | 908 s | 233 s | 171 s | warm |

The Dagger gate is the critical path on every green run.

## Stage timeline, run 41626826480 (Dagger job, 896 s)

| Stage | Start (UTC) | Duration |
|---|---|---|
| Engine pull/start + module load (type definitions 36 s) | 11:21:18 | ~50 s |
| musl candidate: clippy + tests + release, `CARGO_BUILD_JOBS=2` | 11:22:21 | 365 s |
| Static gates, installer candidate, distribution (Debian + Alpine) | 11:28:26 | ~48 s |
| `cargo fmt --check` | 11:29:14 | 2 s |
| glibc clippy (default features) | 11:29:16 | 60 s |
| glibc clippy (`--features otel`) | 11:30:17 | 19 s |
| glibc `cargo test` | 11:30:35 | 92 s |
| Docker image build (otel release compile 155 s) | 11:32:07 | 162 s |
| compose config + deploy artifacts | 11:34:49 | ~10 s |
| tokei `cargo install` from source | 11:34:59 | 64 s |
| Second workflow step: static-release export | 11:36:09 | ~5 s (same engine, cached) |

## Findings

1. **Strictly sequential.** `MmhmemCi.all` in `ci/src/mmhmem_ci/main.py`
   awaits each stage in turn. The musl build, glibc clippy/test, Docker
   build and distribution checks have no data dependency on each other,
   except `deploy_artifacts`, which consumes the verified musl binary.
2. **No cross-run cache on hosted runners.** Each job starts a fresh engine
   container, so `dag.cache_volume(...)` volumes start empty. The tokei
   rebuild on every run confirms it.
3. **Parallelism capped for the dev host.** `CARGO_BUILD_JOBS=2` in both
   `rust_container` and `musl_container` is documented as an OOM guard for
   the ~6 GB WSL engine VM. `ubuntu-latest` for public repositories has
   4 vCPU and 16 GB RAM, so CI compiles at half the available width.
4. **Duplicate compilation.** Linux compiles the crate graph four times:
   musl debug (clippy + tests), musl release, glibc debug (clippy + tests,
   default and otel), glibc otel release (Docker). `src/` has no
   `target_env` cfg, so musl and glibc default-feature clippy/tests cover
   the same source. The otel feature does gate code
   (`src/telemetry.rs` has `cfg(not(feature = "otel"))` branches), so the
   otel clippy pass is not redundant.
5. **Native cache key churn.** Windows/macOS use `actions/cache` keyed only
   on `hashFiles('Cargo.lock')` with no `restore-keys`. Release commits bump
   the crate version, which rewrites `Cargo.lock` and forces a cold build.
6. **Test runner.** Steering names cargo-nextest as the faster local runner
   for this I/O-heavy suite; CI uses plain `cargo test` everywhere.
7. **Release publisher.** `release.yml` runs after CI via `workflow_run`
   and takes ~1.5 min. Its tag-push path ran 16 min on v0.3.3 while CI was
   in flight. It is adjacent, not owned by this spec.

## Lower bound with overlap only

If independent stages overlapped and nothing else changed, the critical
path would be engine start (~50 s) + musl candidate (365 s) + its dependent
gates (~60 s) ≈ 8 min, with glibc and Docker work (~5.5 min combined)
running alongside. Higher cargo parallelism on the runner lowers it further.
Memory headroom for concurrent cargo builds on 16 GB is a design question
that must be measured, not assumed.

## Design discovery (light, extension of the existing pipeline)

Consulted: `kiro-spec-design` skill with its light discovery, synthesis and
review-gate rules; steering `dev-environment.md`.

### Integration points found

- `MmhmemCi.all` (`ci/src/mmhmem_ci/main.py`) is the only CI entrypoint
  that sequences stages. Every other function is independently callable and
  stays unchanged in behaviour.
- Only `static_installer` and `deploy_artifacts` depend on the verified
  bundle. `deploy_artifacts` reaches it through `self.release`, so it inherits
  any new parameter of `static_release`.
- The workflow's second step calls `static-release` again and relies on the
  engine's in-session cache. Any new build parameter must be passed with the
  same value to both steps, otherwise step two recompiles the musl build.
- `scripts/dagger-ci.ps1` accepts `--name=value` function options through
  `-ExtraArgs`, so new options need no wrapper change.
- `ci/scripts/check_static_setup.py` asserts `CARGO_BUILD_JOBS == "2"`. It
  runs only through `static_setup`, which keeps the default.
- `scripts/test-dagger-ci.ps1` already asserts workflow text, so it is the
  place to pin the new workflow arguments.
- `publish-ci-release.ps1` consumes artifacts named `linux-static-*`,
  `mmhmem-x86_64-pc-windows-msvc` and `mmhmem-macos`. These names are a
  contract and must not change.
- `Cargo.toml` has no custom release profile (no LTO), so a release compile
  has ordinary rustc memory needs.

### External facts

- `ubuntu-latest` for public repositories is a 4-vCPU, 16 GB runner
  ([GitHub docs](https://docs.github.com/en/enterprise-cloud@latest/actions/how-tos/using-github-hosted-runners/using-github-hosted-runners/about-github-hosted-runners),
  [GitHub blog](https://github.blog/news-insights/product-news/github-hosted-runners-double-the-power-for-open-source/)).
- `dagger/dagger-for-github` exposes the call's standard output as the step
  output `stdout` in current versions (`output` in v6 and earlier)
  ([action repository](https://github.com/dagger/dagger-for-github)).
  The exact name for v8.4.1 must be confirmed against its `action.yml`
  during implementation.
- Debian's `tokei` package version in bookworm could not be confirmed from
  search results ([tracker](https://tracker.debian.org/pkg/rust-tokei)).
  Implementation must check `apt-cache policy tokei` in the gate image.

#### Implementation finding (task 2.3)

- Checked 2026-11-03 against the Debian sources archive API
  ([sources.debian.org/api/src/rust-tokei/](https://sources.debian.org/api/src/rust-tokei/)):
  `rust-tokei` (binary package `tokei`) exists only as 12.1.2-6 in trixie
  and 15.0.0-1 in forky/sid. Bookworm has no `tokei` package, so
  `apt-get install tokei` is not possible in `rust:1.98-slim-bookworm`.
- A trixie-based image would add a new container image, which the design's
  Allowed Dependencies forbid. The Decision 4 fallback was taken:
  `loc_report` keeps `tokei --version >/dev/null 2>&1 || cargo install tokei --locked`
  (about 64 s on a cold engine) and moves to position 4 of `ALL_STAGE_PLAN`,
  right after the three heavy stages. At concurrency 3 it takes the first
  slot a heavy stage frees (after `clippy_otel`) and finishes well inside
  `static_release`, so it stays off the critical path. At concurrency 1 the
  total time is unchanged; only the order differs.
- `Swatinem/rust-cache` falls back to caches from earlier lockfiles, but its
  `cache-hit` output reports exact matches only, so it cannot report a
  fallback restore (Requirement 5.2).

### Synthesis

- **Generalization:** overlap, failure aggregation and timing are one
  concern: a small stage runner that takes named stages with dependencies.
  It serves Requirements 2.3, 2.4, 3.2, 3.3 and 5.1 through one interface.
- **Build vs adopt:** the stage runner is built, about 80 lines of asyncio.
  Dagger has no stage scheduler of its own, and the Python SDK already runs
  on asyncio. For native caches, `actions/cache/restore` and
  `actions/cache/save` are adopted over `Swatinem/rust-cache`, because the
  restore action reports the matched key and the project already uses
  `actions/cache`.
- **Simplification:** cross-run Dagger cache persistence (Dagger Cloud,
  sccache with the GitHub cache backend, exported target directories) is
  deferred. The measured lower bound without it is close to the target.
  It becomes a follow-up only if validation misses Requirement 1.1.
  cargo-nextest is rejected for CI: test execution is about 5 s of the
  92 s test stage, and the rest is compilation.

### Risks

- Concurrent cargo stages on 4 vCPU compete for CPU. The real gain must be
  measured rather than assumed. A dedicated task records it.
- Peak memory for three concurrent cargo stages is estimated, not measured.
  The memory guard budget is a named constant that implementation measures.
