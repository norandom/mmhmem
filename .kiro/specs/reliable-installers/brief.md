# Brief: reliable-installers

## Problem

No install of mmhmem has been verified end to end. The installer stack is
about 4,300 lines in three languages: `install.ps1` / `install.sh`
bootstraps, `deploy/install_local.ps1` (699 lines) and
`deploy/install_local.sh` (949 lines), PowerShell helpers
(`manage_hooks.ps1`, `manage_routing.ps1`, `hooks_onboarding.ps1`,
`routing_onboarding.ps1`, `check_worker.ps1`, `install_binary.ps1`) and
Python helpers (`manage_hooks.py`, `manage_routing.py`,
`routing_onboarding.py`, `check_worker.py`). Each harness integration exists
twice, once per shell, and the parts are tested in isolation, never as one
installed result.

Observed failure on 2026-11-04: running `install.ps1` from the checkout,
menu option 1, failed with `404: Not Found` at `install.ps1:53`.

- **Root cause:** the bootstrap takes its file list from the copy that
  runs, downloads the files from GitHub `HEAD`
  (`raw.githubusercontent.com/norandom/mmhmem/HEAD/deploy/...`) and
  downloads the binary from `releases/latest` (v0.4.1). These are three
  different versions. The local `main` is 16 commits ahead of `origin/main`,
  so `hooks_onboarding.ps1`, `manage_hooks.ps1` and
  `opencode/mmhmem-read-routing.js` do not exist at remote `HEAD`.
- **Latent break after a push:** once these commits are pushed, `HEAD`
  scripts will install hooks that call `mmhmem hook`, while `releases/latest`
  still serves a binary without that command until a new release is cut.
- **No checksum check:** the release publishes `SHA256SUMS`, but neither
  bootstrap verifies the binary against it.
- **Menu gaps:** the bootstrap menu offers only install or remove. Hooks are
  offered only inside `install_local.ps1` interactive mode, and
  `install.ps1 -Hooks` fails without a value.

## User direction (2026-11-04)

- Success means the agents work with mmhmem, checked by the installer, not
  that the installer's commands ran.
- Earlier runs broke config files; that must never happen again.
- No project-specific flags. A user who knows nothing about the project
  must find one clear installer per platform and complete it.
- Never install or update from a repository checkout; users never do this.

## Approach direction (for design, not binding)

1. **One implementation in the binary.** A `mmhmem setup` subcommand owns
   detection, MCP registration, playbooks, skills and commands, routing
   instructions, hooks, worker config and removal for every harness on
   every OS. Assets are embedded at build time (`include_str!`), so the
   content always matches the binary. Use established crates rather than
   hand-written editing: `toml_edit` (Codex `config.toml`, preserves
   formatting), `serde_json` / a JSONC-preserving editor (Claude and
   OpenCode settings), `dialoguer` or `inquire` (menu), `etcetera` or
   `directories` (per-OS paths).
2. **Thin, version-pinned bootstraps.** `install.sh` and `install.ps1`
   resolve a single release tag, download the platform binary for that tag,
   verify it against that tag's `SHA256SUMS`, and run `mmhmem setup`. They
   never fetch scripts from `HEAD`. Evaluate `dist` (cargo-dist), whose
   generated shell and PowerShell installers already do this download step,
   against the existing verified publish pipeline (`publish-ci-release.ps1`,
   `SHA256SUMS-native`, `verification.json`).
3. **Retire the duplicated script and Python helpers** once `setup` covers
   their behaviour, keeping their test cases as Rust tests.
4. **End-to-end verification in CI** on Linux, macOS and Windows: install
   from locally served release artifacts into an isolated home, re-run to
   prove idempotence, remove, and check that user files are restored.

## Relation to other specs

- Supersedes the installer-script parts of `install-one-liner` and the
  installer half of `read-routing-hooks` tasks.
- `worker-config-permissions` (requirements not yet approved) becomes a
  requirement of `mmhmem setup` instead of two script changes.
