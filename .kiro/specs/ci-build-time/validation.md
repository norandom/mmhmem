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
