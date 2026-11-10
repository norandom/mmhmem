# Development Environment (Windows host + Podman containers)

How to build, test, and validate mmhmem from this Windows workstation. The
daemon itself targets Linux; the dev loop runs natively, Linux-side checks run
in containers through Podman (the `docker` command) and Dagger is the CI
engine everywhere. Since 2026-11-10 the Debian WSL build VM is retired (see
"Containers" below).

## Dev shell: PowerShell 5.1 (native)

- Use `;` as a command separator — never `&&` (PowerShell 5.1 alias failure).
- Never `2>/dev/null` (maps to `D:\dev\null`); redirect to `$null` instead.
- Env vars via `$env:NAME = "value"`.
- Double-quoted strings expand `$var` — when passing `$?`/`$PATH` through to
  WSL `sh`, single-quote the whole command or the wrong shell expands it.

## Rust toolchain (host)

- `x86_64-pc-windows-msvc` (user-confirmed; do not switch to gnu/mingw).
  rusqlite `bundled` compiles via `cc`; long first build is normal.
- rustc/cargo 1.98.1 pinned in `rust-toolchain.toml` (rust-analyzer component
  pinned too — make the editor use the rustup proxy if it complains).
- **Never `cargo clean`** — compile times are long; build once, then iterate.
- Kill stale daemons before building (`Get-Process mmhmem | Stop-Process
  -Force`); `scripts/lint.ps1` does this automatically. A leaked daemon holds
  `target\debug\mmhmem.exe` and blocks all builds. **Side effect:** this also
  kills the opencode-connected mmhmem MCP child — opencode then shows the
  server as "error" and does not auto-respawn it; run
  `opencode service restart` after the gate to reconnect (expect
  `tools=9` in the log).
- Test runner: **cargo-nextest** (`cargo nextest run`, config in
  `.config/nextest.toml`); much faster than `cargo test` for this I/O-heavy
  suite. `scripts/lint.ps1` falls back to `cargo test` if absent.
- Wrap long runs: `powershell -NoProfile -File scripts\with-timeout.ps1
  -Seconds 900 -Command cargo -ArgString "nextest run"`.
- **Feature-enabled builds (`--features otel`) need `CARGO_INCREMENTAL=0`**
  on this host: with incremental on, rustc 1.98.1 crashes with
  STATUS_STACK_BUFFER_OVERRUN at test-binary codegen. Also cap jobs
  (`-j 2`) for the no-run build. `cargo check --features otel --all-targets`
  is unaffected and stays the fast local smoke for gated code.

## Quality gates (blocking, same locally and in CI)

`scripts/lint.ps1` runs: `cargo fmt --check` → `cargo clippy --all-targets --
-D warnings` → nextest. `clippy.toml` caps cognitive complexity at 15
(clippy ≥ 1.98: `cyclomatic_complexity` is a deprecated alias of
`cognitive_complexity` — a two-key clippy.toml aborts; the cognitive cap plus
`[lints.clippy] cognitive-complexity = "warn"` is the enforced guard). Req 13
caps live in every task's acceptance criteria: ≤ 500 LOC per `src/` file,
≤ 80 LOC per function, ≤ 400 non-test lines per task diff, tests ≤ 800 LOC.

## Git on Windows

- `core.autocrlf=true`: working tree may be CRLF, repo is LF. Linux-consumed
  artifacts (deploy/**, Dockerfile, compose, .env.example, *.sh, *.service,
  .github/**) are pinned to LF via `.gitattributes` — keep new Linux-side
  files under those patterns.
- `.gitignore` must stay UTF-8 — a UTF-16LE rewrite once broke git's parsing.
- One conventional commit per task, push to `origin/main` after each.

## Containers: Podman (as `docker`), compose, native Dagger

Owner decision 2026-11-10 (verified on the host the same day):

- `docker` is Podman 5.8.3 via Podman Desktop; `docker compose` is v5.7.0.
  The machine `podman-machine-default` is rootful and runs on a WSL2 kernel
  that Podman manages itself (`podman machine list` shows VM type `wsl`).
  The default `docker context` is `podman-machine-default-root`.
- **The Debian WSL distro is retired for this project.** Do not run
  `wsl -d Debian ...`; do not rsync sources into WSL; do not validate
  anything inside a WSL shell. Linux-side work runs in containers started
  from the native Windows shell.
- **Prefer compose over ad hoc `docker run`.** The repo already has
  `docker-compose.yml` (production stack, `docker compose config` is a CI
  check) and `tests/e2e/compose.e2e.yml` (override used by the gated
  `E2E_LLM=1` suite). New local container runs should be a compose file or
  an override on these, started with `docker compose -f ... up --wait`.
- Dagger v0.21.11 is a native Windows CLI (`dagger version` reports
  `image+podman://registry.dagger.io/engine:v0.21.11`); its engine runs as
  the Podman container `dagger-engine-v0.21.11`. `dagger core version`
  connects in about 10 s.
- `scripts/dagger-ci.ps1` still rsyncs the repo into the retired Debian
  distro (`wsl -d Debian -e wslpath ...`) and must be reworked before use:
  the native path is `dagger call` from the repo root against the `ci/`
  module with the source excluded by `.daggerignore` (it already lists
  `target`-adjacent noise; confirm `target/` and `tmp/` are excluded before
  the first native run, since crawling `target/` once killed the engine).
  Until that rework lands, treat the container CI as GitHub-Actions-only
  and say so instead of improvising a WSL path.
- Still true: no Git Bash for shipping scripts; use native PowerShell for
  Windows work. Shipping shell scripts target ordinary Linux/macOS and must
  not depend on WSL, Podman, Git Bash, or workstation paths.
- Linux deploy artifact checks (`systemd-analyze verify`, `logrotate
  --debug`) run inside a container, not on the host; the Dagger pipeline
  does them as root with the real `agent-memory` user.

## Temp files & scratch: project dir only

- All operations — including temporary and scratch files (scripts, repros,
  JSON payloads, logs) — stay inside the project directory. Use `tmp/` at
  the repo root (git-ignored). Never stage anything in AppData or system
  temp folders (`%LOCALAPPDATA%\Temp`, `%APPDATA%`, ...) or other user
  folders; if project files are found there, clean them up.

## CI: Dagger (native Podman engine locally + GitHub Actions, one pipeline)

- CI is the Dagger module `ci/` (Python SDK). Local entrypoint was
  `powershell -NoProfile -File scripts\dagger-ci.ps1` (defaults to `-Call all`;
  pass `-Call fmt|clippy|test|release|docker-build|...` for one gate); since
  2026-11-10 that wrapper is stale because it rsyncs into the retired
  Debian WSL distro (the DrvFS 9p filesync problems it worked around no
  longer apply to the native CLI). Rework it to call the native `dagger`
  against the Podman engine before relying on it. Actions runs the same module
  via dagger-for-github (`.github/workflows/ci.yml`, workdir ci,
  `call all --src ..` — a fresh checkout has no target/). Never maintain two
  scripts. The pipeline owns Linux-container work: fmt, blocking clippy
  (default AND `--features otel` — plain clippy never compiles gated code),
  the deterministic test suite (incl. the machine-enforced Req 13.3 LOC-cap
  test), informational tokei report, release build, Docker build with the
  `otel` feature, compose-config validation, and systemd/logrotate artifact
  verification as root with the real `agent-memory` user. The optional
  E2E_LLM=1 job runs on the runner (native compose + model secrets).
  The `windows` and `macos` jobs in ci.yml are the one documented
  exception to "one pipeline": local-mode release builds + clippy +
  tests on native GH runners, because the canonical MSVC and Apple
  toolchains cannot run in the Linux containers and `cfg(windows)` /
  `cfg(macos)` code must not escape the blocking lint gate (see
  `.kiro/specs/local-mode/design.md`). Runner minutes matter on private
  repos — cargo caching keeps repeat runs fast.
- The Podman machine is configured with 2 GiB RAM and 12 CPUs (`podman
  machine list`, 2026-11-10): cargo builds in the pipeline stay bounded
  (`CARGO_BUILD_JOBS=2`, `CARGO_INCREMENTAL=0`) because unbounded rustc
  OOM-killed the dagger engine mid-run on the previous 6 GB VM; raise the
  machine's memory (`podman machine set --memory`) before expecting the
  full pipeline to run locally.
- First `dagger call` after engine bootstrap is slow (image pulls, uv setup,
  apt installs); cache volumes make later calls fast.
- Module internals live in `ci/src/mmhmem_ci/main.py`; `ci/sdk/` is
  generated (gitignored) — regenerate with `dagger develop` inside `ci/`.

## Environment-specific test notes

- Tests use ephemeral ports and raw tokio TCP stubs — no extra dev-deps,
  no Docker needed for the deterministic suite.
- Windows paths in hand-built JSON break (`\` escapes) — build test args with
  `serde_json::json!`.
- OTel is behind the `otel` cargo feature; the plain build must contain zero
  opentelemetry crates (`cargo tree` guard).

---
_Document environment facts and failure modes, not task progress._
