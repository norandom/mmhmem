# Validation: ci-dependency-cache

## Task 4.1: local regression gates (2026-11-03)

Revision c467778, local WSL Dagger engine through `scripts/dagger-ci.ps1`
with default options (no cache options, no credential). Logs under
`tmp/ci-cache-4.1-all.log` (development artifact, not committed).

| Check | Result | Requirement |
|---|---|---|
| Pre-commit gate on every task commit | PASS | — |
| CI unit tests: stages, cargo_jobs, all_pipeline, sccache, sccache_stats, dockerfile_cache, timing_summary | PASS on Python 3.12 and 3.11 | 1.2, 1.3, 2.3, 2.4, 5.1, 5.2 |
| Workflow contract test, Windows PowerShell 5.1 and PowerShell 7 | PASS | 2.2, 2.3, 4.1–4.5 |
| Full `all`, defaults | PASS, exit 0, 221 s; all nine stages passed | 1.4, 2.1 |
| Cache line with no options | every stage `{"enabled": false, "reason": "cache off"}` | 1.4, 5.2 |
| Cache-service or sccache traffic in the local log | none | 1.4 |
| Bundle manifest | produced and checked by the unchanged same-artifact path | 2.1 |
| Plain Docker build without build args or secret (`-Call docker-build`) | PASS, payload `mmhmem 0.4.1` | 2.4 |
| sccache wrapper against the real v0.18.0 binary (Windows host) | records a Rust cache hit on the second build | 1.2, 5.1 |

Pending for task 4.2 (hosted): the `env://` secret syntax on Dagger 0.21.11,
the cached Dockerfile branch, real hit counts including C objects, and the
warm and cold timing targets.

## Task 4.2: hosted acceptance

### Run 1, cache-populating (run 41661893251, revision 8b80e14, 2026-11-03)

All jobs green: gate 686 s, Windows 271 s, macOS 191 s, LOC job 59 s
(parallel to the gate).

| Stage | Cache | Hits | Misses | Rust misses | C misses | Duration |
|---|---|---|---|---|---|---|
| static_release | on | 0 | 1,294 | 500 | 520 | 592 s |
| docker_build | on | 0 | 587 | 189 | 261 | 453 s |
| clippy_otel | on | 0 | 590 | 192 | 261 | 224 s |

- `env://` secret arguments are accepted by Dagger 0.21.11, and the cached
  Dockerfile branch (download, checksum, sourced secret) ran (1.1).
- C/C++ compiles of build scripts, including aws-lc-sys and the bundled
  SQLite, go through sccache: 520 musl and 261 glibc C compiles were cache
  misses, so they are stored and cacheable (1.2).
- Cold gate 686 s is within 110 percent of the 666 s baseline (733 s): 1.6
  PASS.
- The LOC report ran in its own job and wrote its summary (4.1, 4.4).
