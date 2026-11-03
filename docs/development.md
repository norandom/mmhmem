# Development and releases

## Pre-commit checks

Enable the versioned hook in each clone:

```sh
git config --local core.hooksPath .githooks
```

The hook requires Python 3 with PyYAML, Git, and the pinned Rust toolchain
(including rustfmt and Clippy). Windows uses Python 3.12 through `py` and
PowerShell; Linux/macOS use `python3`, Bash, and jq for marketplace generation.
It checks the staged snapshot in repository `tmp/`, preserving unstaged edits
and the index. Missing tools and failed checks block the commit.

Checks cover marketplace generation and skill discovery, checkpoint/routing
guidance, native installer regressions, Rust formatting, and the existing
`loc_caps` rules: at most 500 lines per source file, 80 lines per function,
and 800 lines per Rust test file. Clippy runs with warnings denied for both
default and OTel features, enforcing cognitive complexity at 15. These are
the same Rust limits used by CI, not a separate set of thresholds.

Builds use `target/pre-commit/`, two jobs, and disabled incremental compilation;
the first run builds a separate cache. The hook never stops running agents.
Run `python3 scripts/pre-commit.py` (Windows: `py -3.12 scripts/pre-commit.py`)
to check the index manually. Full cross-platform CI remains required for releases.

## Native development builds

With a checkout and the repository's Rust toolchain:

```sh
cargo build --release --locked
cargo test
```

This is a native development build, not the portable Linux distribution gate.
To register it locally, use `bash deploy/install_local.sh` on Linux/macOS or
`powershell -File deploy\install_local.ps1` on Windows. The installer can find
`target/release/mmhmem` or accept an explicit binary path.

## Default Linux release build

The canonical Dagger pipeline defaults to a static
`x86_64-unknown-linux-musl` release. It builds the locked dependency graph,
checks ELF linkage, exercises the same executable on Debian and Alpine,
and exports a checksum-bound verification bundle.

On the Windows development host:

```powershell
powershell -NoProfile -File scripts\dagger-ci.ps1 -Call all -ExportPath tmp/verified-linux
```

Choose a fresh output directory. The wrapper captures revision and cleanliness
before CI sync; dirty local bundles are non-publishable. WSL is CI transport
only, never a user deployment target. See the [release procedure](../deploy/README.md#4-verified-linux-release-transition)
for hosted artifacts, guarded draft delivery, publication checks and rollback.

The same pipeline runs in [GitHub Actions](../.github/workflows/ci.yml):
formatting, default/OTel clippy, deterministic GNU and musl tests, runtime and
installer checks, the unchanged OTel Docker image, compose validation and
systemd/logrotate checks. Native Windows/macOS jobs build their own artifacts.
The tokei report is informational. A separate optional credentialed
model-measurement job compares token use with a real OpenCode harness.

### Gate concurrency and cargo jobs

`all` takes two tuning options. `concurrency` is how many independent stages
run at once; `cargo_jobs` bounds rustc parallelism in every Rust compile.
Local defaults are `1` and `2` (sequential, memory-safe on the ~6 GB WSL
engine). CI passes `--concurrency=3 --cargo-jobs=4` to the gate. The bundle
export step must pass the same `--cargo-jobs=4` so the engine reuses the
in-session musl build; change both workflow steps together. Locally:

```powershell
powershell -NoProfile -File scripts\dagger-ci.ps1 -Call all -ExtraArgs @('--concurrency=2','--cargo-jobs=4')
```

Before any build, `all` checks the engine's MemTotal against
`min(concurrency, heavy stages) x 3.0 GiB + 2.0 GiB` and refuses, for example:

```text
--concurrency=2 needs 8.0 GiB of engine memory (2 heavy stages x 3.0 GiB + 2.0 GiB reserve), but the engine has 5.8 GiB; lower --concurrency
```

After the gate, an always-run step (`ci/scripts/timing_summary.py`) writes a
per-stage table of status, start, duration and baseline to the job summary.
A stage over 1.5 x its baseline in `ci/timing-baseline.json` is marked `slow`;
the table never changes the job verdict. Update the baseline when stage
costs change on purpose.
