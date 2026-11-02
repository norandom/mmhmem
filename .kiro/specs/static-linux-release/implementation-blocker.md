# Static release implementation blocker

## Status

The user approved the separate upstream storage repair and subsequent
resumption of static-release implementation on 2026-11-02. The upstream repair
has passed independent review, GNU tests, default/OTel clippy, formatting and
musl tests/release. Fresh downstream local-probe validation and review remain
completed successfully for task 3.1. The original blocker is resolved.

This is a historical blocker report. All in-scope implementation tasks have
now passed their reviews and final automated verification. The upstream
repair and task 3.1 are complete. Full release-transition acceptance still
requires the operator publication/latest-checksum checkpoint; see
`validation.md` for final status and evidence.

## Reproduced upstream defect

The final canonical local-probe run failed the existing musl-target
`concurrent_fresh_open_is_safe` test in `tests/multiprocess.rs` before artifact
probes could execute. A separate bounded diagnostic through the canonical
Dagger entrypoint reproduced a real process failure on its first pair:

```text
mmhmem stdio: initializing local-mode state (store, services, audit):
database is locked: Error code 5
```

One process initialized successfully with `busy_timeout_ms: 5000`; the other
exited with status 1 within 0.5 seconds. The precise failing initialization
statement still needs instrumentation. The production failure itself is
directly established, rather than inferred from the original test's EOF.

Ownership is upstream `local-mode` task 2.1, Requirements 6.1–6.4, principally
`src/storage/db.rs` initialization. The static-release design explicitly
excludes production Rust changes and requires scope review for them.

## Evidence and completed work

- `tmp/local-probe-final.log`: final canonical failure; probe component tests
  passed before the existing musl regression failed.
- `tmp/diagnose-fresh-open.log`: independent diagnostic and captured startup
  error. Temporary diagnostic code was removed afterward.
- `tmp/local-probe-green-second.log`: earlier explicit candidate passed local
  CLI/MCP, restart persistence and populated statistics on Debian and Alpine.
  This earlier result does not establish current task completion.
- `tmp/static-build-review.log`: independent passing musl lint, deterministic
  suite and release build for task 2.1.
- `tmp/static-inspection-review.log`, `tmp/worker-fixture-review.log` and
  `tmp/bundle-check-review.log`: passing independent component checks.

No release bundle has been exported or published. No production Rust,
dependency, Docker image or other-platform release changes were made.

## Approved repair plan (completed)

The user authorized this separate upstream `local-mode` storage repair,
which was implemented and independently reviewed before resumption:

1. Identify the failing initialization statement and handle contention within
   the configured timeout, preserving public APIs and storage semantics.
2. Add concurrent-first-open and timeout regressions, and retain child stderr
   in multiprocess startup-failure diagnostics.
3. Independently review and verify the upstream correction, then rerun the
   canonical musl suite and `local-probe` before accepting task 3.1.

The debug protocol returned `NEXT_ACTION: STOP_FOR_HUMAN`. No test filtering,
automatic retry-until-green loop or downstream production workaround was used.
