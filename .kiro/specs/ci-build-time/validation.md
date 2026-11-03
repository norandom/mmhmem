# Validation: ci-build-time

## Task 4.1: local regression gates and memory guard (2026-11-03)

Revision fe3313b, run through `scripts/dagger-ci.ps1` on the local WSL Dagger
engine. The working tree had unrelated untracked files, so the wrapper
reported `source_clean=False`; no tracked file differed from the commit.
Logs: `tmp/ci-build-time-4.1/` (development artifacts, not committed).

| Check | Result | Requirement |
|---|---|---|
| Pre-commit gate on every task commit (marketplace, installer tests, fmt, size caps, clippy default and otel) | PASS on all commits | 2.1 |
| CI unit tests: stage runner, cargo jobs, pipeline plan, timing summary (61 tests) | PASS on Python 3.12 and 3.11 | 2.3, 2.4, 3.2, 3.4, 5.1, 5.3 |
| Workflow contract test, Windows PowerShell 5.1 and PowerShell 7 | PASS | 2.5, 3.1, 3.3, 4.1 |
| Full `all` with defaults (concurrency 1, cargo jobs 2) | PASS, 252 s with warm local caches; all ten stages passed in plan order | 2.1, 3.1, 3.2 |
| Engine MemTotal reported to the guard | 5.8 GiB; the guard needs 5.0 GiB at concurrency 1 | 3.4 |
| Peak WSL VM working set during the default run (Windows `vmmemWSL`, sampled every 5 s) | 4,789 MiB | 3.4 |
| `all --concurrency=2` | Refused after 2.9 s, before any build: "--concurrency=2 needs 8.0 GiB of engine memory (2 heavy stages x 3.0 GiB + 2.0 GiB reserve), but the engine has 5.8 GiB; lower --concurrency" | 3.4 |

### Local stage timings (warm caches, sequential; not a hosted baseline)

| Stage | Duration |
|---|---|
| static_release | 122.3 s |
| docker_build | 38.4 s |
| clippy_otel | 6.2 s |
| loc_report | 3.7 s |
| stage_runner_tests | 0.6 s |
| distribution | 21.2 s |
| fmt | 0.4 s |
| compose_config | 1.3 s |
| static_installer | 24.0 s |
| deploy_artifacts | 1.1 s |

### Notes

- The VM working set covers the whole WSL VM, not only the engine, so it is
  an upper bound for engine use at concurrency 1. It stays below the 5.8 GiB
  MemTotal, and the guard's 5.0 GiB requirement at concurrency 1 holds.
- Hosted acceptance (tasks 4.2 and 4.3) is pending and needs pushes to `main`.

## Task 4.2: first hosted run (2026-11-03, accepted by the user as is)

Run 41643349698 on b08668c, `ubuntu-latest` 4 vCPU / 16 GB, `--concurrency=3 --cargo-jobs=4`. All jobs green.

| Job | Before (run 41626826480) | After |
|---|---|---|
| Blocking Dagger gate | 896 s | 666 s |
| Windows | 642 s (cache miss on version bump) | 635 s (expected miss: new key prefix) |
| macOS | 361 s | 538 s (expected miss: new key prefix) |

| Stage | Start | Duration |
|---|---|---|
| static_release | 0 s | 572 s |
| docker_build | 0 s | 453 s |
| clippy_otel | 0 s | 169 s |
| loc_report | 169 s | 157 s |
| distribution | 326 s | 44 s |
| static_installer | 572 s | 20 s |
| deploy_artifacts | 572 s | 8 s |
| others | | under 6 s each |

Findings:

- The gate is 230 s faster than the last baseline but above the 540 s target (Requirement 1.1 NOT_VERIFIED).
- Overlap made the heavy stages slower individually: static_release took 572 s against 413 s sequentially, docker_build 453 s against 162 s. Three cargo stages at 4 jobs each oversubscribe 4 vCPU, and the static chain is the critical path.
- The peak runner memory line was written to the job summary, which the API does not expose; it was not read back. The memory budget was not changed.
- Candidate follow-ups: run the static chain alone at full width and the other heavy stages at lower width or after it, or try concurrency 2; cross-run caching per Decision 5.
- Task 4.3 (five-run medians) was not run; the user accepted the result at this point.
