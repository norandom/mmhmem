# Development Environment (Windows host + Debian WSL build VM)

How to build, test, and validate mmhmem from this Windows workstation. The
daemon itself targets Linux; the dev loop runs natively, Linux-side checks run
in WSL containers, and Dagger is the CI engine everywhere.

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
  `target\debug\mmhmem.exe` and blocks all builds.
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

## WSL: Debian build VM only

- Distro `Debian` (default), user `mc`. Other distros exist (`Debian-MW`,
  `NixOS`, `NixOS-AI`) — do not use them for this project.
- **Never run the mmhmem service directly inside WSL.** WSL is a build/CI VM:
  containers, Dagger, and Linux-side validation only.
- **WSL is a build env for CI — never for execution, storing data, or use as
  a scratchpad.** Do not stage, store, or hand-sync any files into WSL
  yourself (no `/tmp` scratch trees, no git-archive staging). The ONLY
  sanctioned source staging is the `~/mmhmem-ci` rsync performed by
  `scripts\dagger-ci.ps1`, which is the ONE local dagger entrypoint — do not
  invent `--src` recipes against DrvFS mounts or WSL paths (filesync off
  DrvFS hits 9p metadata conflicts; crawling `target/` once killed the
  engine). All build outputs export back to the Windows repo (D:). Never
  execute mmhmem binaries inside WSL; Linux-side artifact validation runs
  inside Dagger containers only.
- Non-login shells lack `~/.local/bin`: call tools by full path,
  `wsl -d Debian -e bash -c '~/.local/bin/dagger version'`.
- `sudo` needs a password — no passwordless root; write checks as unprivileged
  or inside containers.
- `wsl.exe` stdout can arrive UTF-16LE-garbled; if output looks mangled,
  capture to a file inside WSL and `cat` it.
- Tools available in Debian WSL: docker 29.9 client (`docker compose config`
  works), dagger v0.21.8 (`~/.local/bin/dagger`), python3.12 + uv/uvx,
  systemd-analyze 257, logrotate 3.22. **No Go** — Dagger modules use the
  Python SDK.
- Validate Linux deploy artifacts here: `systemd-analyze verify
  deploy/systemd/mmhmem.service` (install the binary first or expect the
  missing-binary error; DrvFS mounts also show 0777 permission-bit noise —
  both are environmental, not unit defects) and `logrotate --debug` (the
  `su agent-memory` directive needs the install-time user; substitute `root`
  for a pure syntax check).

## Temp files & scratch: project dir only

- All operations — including temporary and scratch files (scripts, repros,
  JSON payloads, logs) — stay inside the project directory. Use `tmp/` at
  the repo root (git-ignored). Never stage anything in AppData or system
  temp folders (`%LOCALAPPDATA%\Temp`, `%APPDATA%`, ...) or other user
  folders; if project files are found there, clean them up.

## CI: Dagger (local WSL + GitHub Actions, one pipeline)

- CI is the Dagger module `ci/` (Python SDK). Local entrypoint:
  `powershell -NoProfile -File scripts\dagger-ci.ps1` (defaults to `-Call all`;
  pass `-Call fmt|clippy|test|release|docker-build|...` for one gate). The
  wrapper rsyncs the repo (minus target/, .git, secrets) into the WSL home
  (~/mmhmem-ci) and runs dagger there: dagger's source filesync OFF the
  DrvFS mount hits 9p metadata-cache conflicts ("size changed during sync")
  and crawling target/ once killed the engine. Actions runs the same module
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
- The dev-host WSL VM has ~6 GB RAM: cargo builds in the pipeline are
  bounded (`CARGO_BUILD_JOBS=2`, `CARGO_INCREMENTAL=0`) because unbounded
  rustc OOM-killed the dagger engine mid-run (observed twice). The engine
  container is per-session (stops between calls) — a call right after
  another's cleanup may hit "engine is shutting down"; just retry.
- First `dagger call` after engine bootstrap is slow (image pulls, uv setup,
  apt installs); cache volumes make later calls fast. `wsl.exe` mangles
  nested quotes — keep the bash -c payload quote-free or write the script
  to a file.
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
