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
Users install releases with the one-liners in the [installation guide](installation.md).

## Installer layout

- `src/setup/` is the installer: `mmhmem setup` and `mmhmem update`.
- `deploy/bootstrap/install.{sh,ps1}.tmpl` are rendered per release by
  `scripts/render-bootstraps.ps1` (version and archive SHA-256 filled in) and
  published as the release assets `install.sh` / `install.ps1`.
  `scripts/test-bootstraps.ps1` checks them offline.
- The root `install.ps1` / `install.sh` are stubs for the old
  `raw.githubusercontent.com/.../HEAD/install.*` one-liners. They print the
  new command, download and change nothing, and exit with code 2 (piped into
  `iex`, `install.ps1` sets `$LASTEXITCODE` to 2 instead of exiting).
  `scripts/test-install-stub.ps1` and `scripts/test-install-stub.sh` check
  them, through `scripts/test-install.ps1` / `scripts/test-install.sh`.
- The old installer engine (`lib.*`, `deploy/install_local.*`,
  `deploy/manage_*`, `deploy/*_onboarding.*`, `deploy/check_worker.*`,
  `deploy/install_binary.ps1`) and its tests stay until the release after
  the stubs ships (reliable-installers task 9.3). With a checkout you can
  still run `bash deploy/install_local.sh` or
  `powershell -File deploy\install_local.ps1` against `target/release/mmhmem`
  or an explicit binary path; it is not a supported user install.
- `tests/install_docs.rs` checks that README.md, the user docs, the release
  notes template (`.github/release-notes.md`) and the stubs use the release
  one-liners only.
- The installer end-to-end gate is `.github/workflows/installer-e2e.yml`; it
  runs in CI only.

### Published-release install

`.github/workflows/published-install.yml` runs the README one-liners
exactly as written, against the published `releases/latest` assets on
github.com, on the same five legs as the installer e2e (Linux, macOS arm64,
macOS x86_64, Windows PowerShell 5.1, PowerShell 7). It starts after every
successful "Publish release" run that published the latest release, weekly,
and by hand (`workflow_dispatch`).

Each leg seeds the same throwaway home, downloads the latest tag's
`SHA256SUMS`, checks `SHA256SUMS.sig` with the key in
`src/setup/release_keys.rs` and the platform archive against both, runs the
one-liner without a terminal, and asserts with the installer e2e's own
checks (`scripts/e2e-run.ps1 -Phase published-release`, `published-install`,
`assert-published`; the phases are in `scripts/e2e-published.ps1`): exit code
0, the install record names that tag, the installed program is the one in
the verified archive, Claude Code, Codex and OpenCode are recorded with the
default features (memory and the read-routing hook) and pass their own
checks, the seeded files are unchanged, and nothing is left running.

It is evidence, not a gate: no branch protection rule and no release step
waits for it, and it needs no secret. A red run means the published release
or the one-liners need a look; it never blocks a merge or a release.

## Releases

The version bump commit (`chore(release): version X.Y.Z`) changes
`Cargo.toml` and `Cargo.lock` and also regenerates the plugin manifests, so
the `unified-memory` plugin carries the same version as the program and
agents notice a new plugin with every release (the catalogs track `main`).
Set `"version"` in `.marketplace/skills.json` to the new version, then run
`.\build-marketplace.ps1 -ManifestsOnly` (Windows) or
`bash build-marketplace.sh --manifests-only`, and commit the four
`plugins/unified-memory/**/plugin.json` files with the bump.
`scripts/check-marketplace.py` enforces it: it fails when
`.marketplace/skills.json` or any plugin manifest names a version other than
the `[package] version` in `Cargo.toml`, and says how to regenerate. It runs
in the pre-commit hook and in CI (the Dagger `distribution` function).

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

### Compile cache (sccache)

Every hosted gate run starts a fresh Dagger engine, so Dagger cache volumes
are empty each time. The gate therefore runs every Rust build (musl clippy,
tests and release, glibc otel clippy, the otel Docker builder stage) under
sccache v0.18.0 with the GitHub Actions cache. Unchanged crates come back
from the cache instead of being recompiled.

- **Modes:** `--cache-mode=read-write` on pushes to `main`,
  `--cache-mode=read-only` on pull requests, so pull requests reuse `main`'s
  cache without writing to it. `off`, or no cache options, builds exactly as
  before; local runs through `scripts/dagger-ci.ps1` pass none and need no
  credential.
- **Credentials:** the workflow exports `ACTIONS_RESULTS_URL` and
  `ACTIONS_RUNTIME_TOKEN` and passes them as Dagger secrets
  (`--cache-url`, `--cache-token`). They reach containers only as secret
  variables or the Dockerfile's `sccache_env` BuildKit secret, never as
  build arguments, logs or image layers.
- **Fallback:** if sccache cannot start or the cache service fails, the
  build runs uncached with the same verdict, and the summary says why.
- **Results:** the job summary's "Compile cache (sccache)" table lists hits
  and misses per stage, with C/C++ hits separately.
- **Purge:** bump `GHA_CACHE_VERSION` in `ci/src/mmhmem_ci/sccache.py`.
  Toolchain and target changes produce new cache keys on their own.
- **Dockerfile:** the builder stage uses sccache only when CI passes the
  `SCCACHE_URL`/`SCCACHE_SHA256` build arguments and the secret; a plain
  `docker build` is unchanged.

### TLS stack

mmhmem uses rustls with the aws-lc-rs crypto provider, the default of
`reqwest`'s `rustls` feature. It serves only outbound HTTPS:

- worker-model API calls from the summary worker client, and
- in the otel Docker image, the OTLP exporter.

The mmhmem server terminates no TLS itself; a reverse proxy does. The local
health probe and local workers such as Ollama over `http://` do not use it.

`aws-lc-sys` compiles C code in its build script. That build is cached, not
replaced: on both CI targets it builds through the `cc` crate, which wraps
the C compiler with sccache whenever `RUSTC_WRAPPER` names it. A warm run
shows non-zero "C hits" for the musl and Docker stages in the compile-cache
table; aws-lc-sys and the bundled SQLite are the main C sources.

### LOC report

The informational per-file LOC report (`tokei --files`, Requirement 13.5 of
the cross-agent-memory-server spec) runs in its own `loc-report` workflow
job, alongside the gate. It never affects the verdict or a release, and it
reuses a cached tokei binary of the pinned version. Locally:

```powershell
powershell -NoProfile -File scripts\dagger-ci.ps1 -Call loc-report
```
