# Research: ci-dependency-cache

Collected 2026-11-03 at commit 6388a03 (v0.4.1).

## Where the gate's time goes

| Measurement | Value | Source |
|---|---|---|
| Hosted gate, concurrency 3, cold | 666 s | run 41643349698 |
| Docker otel release compile | 7 min 17 s (196 crates) | same run |
| Static release chain (musl clippy, tests, release, runtime gates) | 572 s | same run |
| tokei `cargo install` | 2 min 36 s (124 crates) | same run |
| otel clippy | 2 min 36 s | same run |
| Local gate, warm cache volumes | 252 s; static release 118 s, Docker 34 s | `.kiro/specs/ci-build-time/validation.md` |
| Native Windows / macOS with cache fallback | 264 s / 210 s | run 41648521001 (v0.4.0 bump) |

- Each hosted job starts a new Dagger engine, so `dag.cache_volume(...)` and
  BuildKit cache mounts are empty on every run.
- `Cargo.lock` has 271 packages. `cargo tree -e normal` lists 212 for default
  features and 237 with `otel`.
- C builds in the graph: `aws-lc-sys` 0.47.0 (through `aws-lc-rs` 1.19.0 and
  `rustls` 0.23.46, the default provider of `reqwest` 0.13.5 with `rustls`),
  `libsqlite3-sys` 0.39.1 (rusqlite `bundled`), plus `cc`-based crates.
  `cmake` 0.1.58 is in the lock file.

## What the TLS stack is for

- `src/services/worker.rs`: the worker-model HTTP client (`reqwest`), which
  reaches remote API providers over HTTPS.
- otel Docker image only: the OTLP exporter (`opentelemetry-otlp` with
  `reqwest-rustls`).
- Not used for: the mmhmem server, which terminates no TLS (an external
  reverse proxy does, per the static-linux-release scope amendment); the
  local health probe in `src/main.rs`, which is plain HTTP; local workers
  such as Ollama over `http://`.

## sccache facts (to confirm in design)

- sccache wraps rustc through `RUSTC_WRAPPER=sccache` and supports a GitHub
  Actions cache backend enabled with `SCCACHE_GHA_ENABLED`
  ([sccache GHA docs](https://docs.rs/crate/sccache/latest/source/docs/GHA.md),
  [mozilla-actions/sccache-action](https://github.com/mozilla-actions/sccache-action)).
  The action README says sccache older than 0.10.0 probably will not work.
- Older docs require `ACTIONS_CACHE_URL` and `ACTIONS_RUNTIME_TOKEN` in the
  environment and export them with `actions/github-script`
  ([Android mirror of GHA.md](https://android.googlesource.com/toolchain/sccache/+/HEAD/docs/GHA.md)).
  The variables needed by the current cache service were not confirmed by
  search. Design must check the current sccache release notes.
- Changing `SCCACHE_GHA_VERSION` invalidates the whole cache. The backend has
  a 10 GB repository limit and checks the backend per rustc invocation
  ([Depot write-up](https://depot.dev/blog/sccache-in-github-actions)).
- Not cacheable: crates that invoke the linker (bin, dylib, cdylib,
  proc-macro) and incrementally compiled crates
  ([sccache Rust.md](https://github.com/mozilla/sccache/blob/master/docs/Rust.md)).
  The gate already sets `CARGO_INCREMENTAL=0`. The mmhmem bin crate and
  proc-macro crates (serde_derive, tokio-macros, schemars_derive and others)
  will always compile.
- C code from build scripts: `RUSTC_WRAPPER` alone covers rustc. cc-rs
  documentation suggests it prefixes C compiles with the wrapper, and a
  `CC`/`CMAKE_C_COMPILER_LAUNCHER` wrapper is the documented route for
  CMake. Whether `aws-lc-sys` 0.47 uses cc or CMake by default, and whether
  its C objects become cache hits, is unverified and must be measured.

## GitHub cache scoping

- Workflows on the default branch restore only caches created on the default
  branch; pull-request runs can read the base branch's caches but write into
  their own ref scope. This is GitHub's documented cache isolation model and
  is what keeps PR runs from poisoning caches used by `main` and tag runs.

## Constraints from existing specs

- Same-artifact Linux verification (static-linux-release) and the bundle
  manifest must not change.
- The Dockerfile is a deploy artifact and must stay buildable with plain
  `docker build`.
- cross-agent-memory-server Requirement 13.5 requires an informational
  tokei LOC report in CI.
- ci-build-time Requirement 4.3 (no secrets in persisted caches) and the
  local WSL memory guard stay in force.

## Design discovery (2026-11-03, primary sources)

- **GHA credentials, current docs:** sccache `docs/GHA.md` on `main` says the
  backend needs `ACTIONS_RESULTS_URL` and `ACTIONS_RUNTIME_TOKEN`, exported
  with an `actions/github-script` step, and `SCCACHE_GHA_ENABLED=on`.
  `SCCACHE_GHA_RW_MODE=READ_ONLY` makes a run read-only. On a service rate
  limit the build continues and storage may be skipped
  ([GHA.md](https://raw.githubusercontent.com/mozilla/sccache/main/docs/GHA.md)).
- **aws-lc-sys 0.47.0 builder:** `builder/main.rs` `get_builder` picks the cc
  builder unless FIPS, an explicit CMake request, no-asm or a sanitizer is
  set, or bindgen is required. Bindgen is required only without pregenerated
  bindings; `src/` ships `x86_64_unknown_linux_musl_crypto.rs` and
  `x86_64_unknown_linux_gnu_crypto.rs`. So both gate targets use the cc
  builder.
- **cc 1.10.0:** its docs (`src/lib.rs`) say that when `RUSTC_WRAPPER`'s file
  name is `sccache`, cc prefixes it to the C compiler command unless `CC`
  already names a wrapper. So `RUSTC_WRAPPER=sccache` alone caches the C
  objects of aws-lc-sys and the bundled SQLite.
- **Dagger Dockerfile builds:** `Directory.docker_build` accepts
  `build_args`, `target` and `secrets` (mounted at `/run/secrets/<name>`).
  A builder-stage build and the full image build with identical inputs share
  the builder stage within one engine session.
- **sccache server lifetime:** each Dagger `with_exec` is a separate process
  tree, so the sccache server and its statistics live only for that exec.
  Statistics must be read in the same exec as the build.
