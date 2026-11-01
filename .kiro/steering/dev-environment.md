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

## CI: Dagger (local WSL + GitHub Actions, one pipeline)

- CI is a Dagger module (Python SDK) run locally via the WSL dagger CLI and
  later from GitHub Actions with the same entrypoints — never maintain two
  scripts. PowerShell drives the dev loop; the Dagger pipeline owns
  Linux-container work (build, clippy gate, tests, Docker image with the
  `otel` feature, deploy-artifact validation, optional E2E_LLM job).

## Environment-specific test notes

- Tests use ephemeral ports and raw tokio TCP stubs — no extra dev-deps,
  no Docker needed for the deterministic suite.
- Windows paths in hand-built JSON break (`\` escapes) — build test args with
  `serde_json::json!`.
- OTel is behind the `otel` cargo feature; the plain build must contain zero
  opentelemetry crates (`cargo tree` guard).

---
_Document environment facts and failure modes, not task progress._
