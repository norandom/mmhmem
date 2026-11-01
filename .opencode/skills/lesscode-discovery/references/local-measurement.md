# Local empirical workflow for mmhmem

Lesscode checkout: D:/Source/lesscode. Specimen: D:/Source/mmhmem.
Read D:/Source/lesscode/docs/experiments/local-adoption/README.md and its linked
source manifest, baseline and toolchain records before repeating measurements.
The Rust analyzer was installed through Cargo, version 0.0.25 on Rust 1.98.1:
D:/Source/lesscode/.tools/rust-code-analysis-cargo-0.0.25/bin/rust-code-analysis-cli.exe.

From the Lesscode checkout, choose a fresh external output:

```powershell
uv.exe run --no-project --python 3.12.10 --with-requirements docs/experiments/local-adoption/requirements-analysis.lock python docs/experiments/local-adoption/measure.py docs/experiments/local-adoption/mmhmem.json --out .runs/local-adoption/mmhmem/NEW/baseline.json --production-copy .runs/local-adoption/mmhmem/NEW/production
```

That command records tracked source identity and category line counts. It withholds Rust
NLOC/function/CCN values after a demonstrated parser limitation; use
Mozilla rust-code-analysis for Rust structure and logical lines, following the
reproduction section in the report. Lizard's NLOC and RCA's PLOC are different
metrics. The reviewed inline-test line ranges are specific to the recorded
snapshot: re-review them when source changes. Never silently reuse stale splits.

Keep measurement output outside mmhmem, exclude .env, credentials, target,
vendored packages and agent tooling, and distinguish production from tests,
fixtures and research support. Keep public contracts, security boundaries,
audit/redaction, failure ordering and side effects intact. Tests remain evidence.

First nominate a complete responsibility and name the concrete existing API or
simpler implementation that could replace it. Exact clones and per-function
complexity only locate candidates. Include callers, residual code, adapters,
configuration and dependencies in the replacement count. Retain a no-change
option and disproving cases. No deletion quota or compulsory pattern.

This installation authorizes read-only measurement and discovery. Do not rewrite
application code, weaken tests, change CI or commit upstream as part of a scan.
If a subsequent rewrite is requested, use isolated copies, unchanged tests and
source-bound evidence before adoption. Do not start platform validation as a
substitute for measuring and simplifying real code.
