# Research: reliable-installers

Findings gathered on 2026-11-04 from the repository at `main` (`34e66f9`,
16 commits ahead of `origin/main`) and the v0.4.1 release.

## 1. Why the latest install failed

`install.ps1` (menu option 1) failed with `404: Not Found` at line 53.

- The file list is hard-coded in the running copy of `install.ps1`.
- Each file is fetched from `raw.githubusercontent.com/norandom/mmhmem/HEAD`.
- The binary is fetched from `releases/latest/download/mmhmem.exe` (v0.4.1).
- `deploy/hooks_onboarding.ps1`, `deploy/manage_hooks.ps1` and
  `deploy/opencode/mmhmem-read-routing.js` exist locally but not at remote
  `HEAD`; verified with `git cat-file -e origin/main:deploy/<file>`.

`install.sh` uses the same `HEAD` + `releases/latest` pattern.

## 2. Defects in the current integration code that can break config files

| Location | Defect | Effect |
|---|---|---|
| `install_local.ps1` `Merge-Json` / `Remove-JsonKey` | Round-trips the whole file through `ConvertFrom-Json` / `ConvertTo-Json -Depth 10` | Comments are lost (OpenCode allows JSONC); key order and formatting are rewritten; values nested deeper than 10 levels are silently turned into strings; Windows PowerShell 5.1 can collapse one-element arrays |
| `install_local.ps1` OpenCode section | Writes `mcp.servers.mmhmem` | **Refuted in task 1.1: not a defect.** OpenCode 2.2.5's own `opencode mcp add` writes `mcp.servers.<name>`, and its schema marks the direct `mcp.<name>` form deprecated. See §8 |
| `install_local.ps1` Codex section | Hand-maintained `# BEGIN mmhmem` marker block, refuses multi-line strings | Not a TOML parser; any user edit inside or near the block, or a pre-existing `[mcp_servers.mmhmem]` table, produces a duplicate-table error in Codex |
| Claude Desktop detection | Stops the whole install if more than one MSIX profile exists | Install of every other agent aborts |
| All writes | Direct `WriteAllText`, no temp file, no backup, no validation | An interrupted or wrong write leaves a broken file and nothing to restore |
| Both shells | Same logic written twice (699 + 949 lines) plus 1,379 lines of Python helpers | Behaviour drifts between platforms; Python is a hidden runtime requirement |

## 3. Verification gap

Unit tests exist for each script (`scripts/test-*.ps1`, `scripts/test_*.py`),
but nothing runs the user's command and then asks an agent whether mmhmem
works. "Installed" has meant "the script exited 0".

## 4. Release assets today (v0.4.1)

`mmhmem-linux-x86_64`, `mmhmem-macos-aarch64`, `mmhmem-macos-x86_64`,
`mmhmem.exe`, `SHA256SUMS`, `SHA256SUMS-native`, `verification.json`.
No archives, no signature, no installer scripts. Neither bootstrap reads
`SHA256SUMS`. The user reported a `.tgz` being downloaded on Windows; no
`.tgz`, `.tar.gz` or `.zip` handling exists in any `*.ps1`, `*.sh` or
workflow file, so the source of that download is still unidentified.

## 5. Options considered for the download step

| Option | Assessment |
|---|---|
| `dist` (cargo-dist) generated installers | Proven shell/PowerShell download, unpack and PATH logic. But it wants to own the release workflow (conflicts with the trusted `publish-ci-release.ps1` flow and `verification.json`), its installers end after placing the binary (no supported hand-off into an interactive setup in the same terminal), and its default Unix archive is `.tar.xz`. |
| Keep the current scripts and patch them | Leaves the duplicated logic and the missing parsers in place; does not meet Requirements 3, 5 and 8. |
| **Thin per-release bootstraps + `mmhmem setup` in Rust** (chosen) | Bootstraps are generated per release with the release's hashes built in, so they cannot mix versions. Everything else uses maintained Rust crates and one test suite. |

## 6. Libraries to adopt

Versions are confirmed against crates.io and pinned exactly when each crate
is added, as `tech.md` requires. Each new dependency is checked with
`cargo tree -d` for duplicate major versions (notably `reqwest`, `rustls`,
`windows-sys`).

| Task | Library | Why / notes |
|---|---|---|
| 2.1 Atomic replace, backups | `tempfile` (`NamedTempFile::persist`), `fs-err` | Same-folder temp file + rename (`MoveFileEx` on Windows); `fs-err` puts the path into every IO error, which feeds the plain error messages |
| 2.1 Owner-only key file | `std::os::unix::fs::OpenOptionsExt::mode`; `windows` / `windows-sys` | No maintained high-level Windows ACL crate (`windows-acl` is unmaintained), so a small wrapper around `CreateFileW` with a security descriptor |
| 2.2 JSON/JSONC that keeps formatting | `jsonc-parser` (`cst` feature, from dprint) | One of the few Rust libraries that edits JSON while keeping comments and whitespace; `serde_json` with `preserve_order` keeps order only |
| 2.3 TOML | `toml_edit` | Used by `cargo add`; edits one table and keeps comments and formatting |
| 2.5 Review-screen diff | `similar` | Line diffs; also used by `insta` |
| 3.x Detecting agents, paths | `which`, `etcetera` | `etcetera` gives the per-OS state and config folders |
| 3.x Running agent CLIs | `std::process` or `duct` | `duct` handles pipes and timeouts more cleanly |
| 4.1 Functional check | `rmcp` (`client`, `transport-child-process`) | Same MCP SDK as the server; starts the command exactly as configured |
| 5.2 Signing in CI | `zipsign` CLI | Embeds an ed25519 signature into each `.zip` / `.tar.gz`; the same key signs `SHA256SUMS` as a detached signature |
| 5.2 Attestations | `actions/attest-build-provenance` | Independent check via `gh attestation verify` |
| 5.4, 7.1 Download, verify, unpack, replace | **`self_update`** (adopted) | GitHub releases backend, picks the asset by target triple, unpacks `.zip` and `.tar.gz`, verifies `zipsign` signatures (`signatures` feature), replaces the running binary through `self-replace`. Configure `default-features = false` with rustls, `archive-zip`, `archive-tar`, `compression-zip-deflate`, `compression-flate2`, `signatures`. It is blocking, so it runs in `spawn_blocking` |
| 5.4 `SHA256SUMS` signature | `zipsign-api` / `ed25519-dalek` | Verifies the detached signature with the same embedded public keys |
| 6.2 Menu | `ratatui` + `crossterm` | Full-screen, works in the Windows console; `inquire` / `cliclack` were rejected because they only give sequential prompts, not a stateful menu |
| 6.2 Menu tests | `ratatui::backend::TestBackend` + `insta` | Snapshot every screen |
| 7.1 `PATH` on Windows | `winreg` + `WM_SETTINGCHANGE` broadcast | As `rustup` does |
| Progress | `indicatif` | Downloads and per-agent progress |
| 8.x End-to-end | `assert_cmd`, `assert_fs`, `expectrl`, `httpmock` (or `tiny_http`) | `expectrl` drives the menu in a pseudo-terminal on Unix and ConPTY on Windows; `httpmock` serves release assets locally |

## 7. Installers to borrow structure from

- `rustup-init.sh` and rustup's Windows `PATH` code: platform detection,
  `curl | sh` with `/dev/tty`, `PATH` handling. Model for task 5.3.
- The `uv` / `ruff` installers generated by `cargo-dist`: PowerShell 5.1
  compatibility and `Expand-Archive` use at large scale.
- Cross-client MCP installers exist, mostly on npm. They need Node, so
  they're not adopted; use them only as a cross-check for config paths,
  with the agents' official docs taking precedence.

## 8. Confirmed agent facts (task 1.1, 2026-11-04)

Sources are the official docs (URLs below) and the agents' own CLIs on Windows
11: Claude Code 2.1.302, codex-cli 0.166.0, OpenCode 2.2.5. Fixtures are in
`tests/fixtures/setup/agents/`.

| Agent | Registration (confirmed) | Non-interactive add / remove | Check command and output |
|---|---|---|---|
| Claude Code | User scope is stored in `~/.claude.json` (docs: code.claude.com/docs/en/mcp, scope table). A stdio entry has `command`, `args` and `env`, and `type` is optional | `claude mcp add --scope user --transport stdio mmhmem -- <bin> stdio`; `claude mcp remove mmhmem -s user` (the remove hint is printed by `get`) | `claude mcp get mmhmem` prints `Scope`, `Status: ✔ Connected` (U+2714), `Type`, `Command`, `Args`. `claude mcp list` health-checks every server, including remote ones, and is slow. **Use `get`** |
| Codex | `[mcp_servers.mmhmem]` with `command`, `args`, optional `env`, `cwd`, `startup_timeout_sec`, `enabled` in `~/.codex/config.toml` (learn.chatgpt.com/docs/extend/mcp, redirected from developers.openai.com/codex/mcp) | `codex mcp add mmhmem -- <bin> stdio`; `codex mcp remove mmhmem` | `codex mcp get mmhmem` (text) and `codex mcp list --json` (array of `{name, enabled, transport{type, command, args, env, env_vars, cwd}, …}`). **`--json` prints env values unmasked**, so it is parsed in memory and never logged. It doesn't report connection status: the check proves connectivity with its own MCP handshake |
| Codex hooks | `~/.codex/hooks.json` `{hooks:{<Event>:[{matcher, hooks:[{type:"command", command, timeout}]}]}}`. Non-managed hooks must be reviewed and trusted by exact hash in `/hooks` (learn.chatgpt.com/codex/hooks) | — | Approval stays a pending user step |
| OpenCode | `mcp.servers.<name> = {type:"local", command:[bin, "stdio"], enabled?, environment?, cwd?, timeout?}`. Global `~/.config/opencode/opencode.json` **or** `opencode.jsonc`; both exist on this machine and both are loaded (`opencode debug config` lists them as separate sources). Subfolders are plural: `plugins/`, `commands/`, `skills/` (singular folders still accepted) | `opencode mcp add [--global] mmhmem -- <bin> stdio` writes the entry itself. No remove subcommand in 2.2.5 | `opencode mcp list` prints `✓ <name>  connected`. See the race below |
| Claude Desktop | `mcpServers.<name> = {command, args, env?}` in `claude_desktop_config.json`: `%APPDATA%\Claude\` (Windows), `~/Library/Application Support/Claude/` (macOS) (modelcontextprotocol.io/quickstart/user). On this machine only the MSIX profile `…\Packages\Claude_pzs8sxrjxfjjc\LocalCache\Roaming\Claude` exists, and it **has no config file**; the MSIX path is not in the official docs | none | none; functional check only |

### OpenCode `mcp list` answers before servers start

OpenCode 2.x CLI commands talk to a background service (`opencode service`).
Each working folder gets its own instance in that service. On the first
request for a folder, and on every `--standalone` run, `mcp list` prints
`No MCP servers configured` **before** plugins load and MCP servers start; the
debug log shows the line between "location services booted" and "loading
plugin". Later requests to the warm instance print `✓ mmhmem connected`.
Reproduced several times: a folder's first query reported none, and the
following three reported connected for both the global entry and a
project entry.

Consequences:
- A single `opencode mcp list` is not evidence either way. The OpenCode
  check polls `opencode mcp list` (not `--standalone`) until mmhmem appears
  as connected or 30 s pass. A missing entry only counts as failure after
  the timeout, and the MCP handshake (design §6 step 2) stays the primary
  proof.
- This race likely explains earlier "OpenCode doesn't see mmhmem" reports
  after installs whose config was in fact correct.
- Side effect: each probed folder stays open in the service until it
  restarts. Two scratch folders from this investigation
  (`%TEMP%\oc-probe`, `%TEMP%\oc-proj-19e1779a`) are locked by it for now.
  The installer runs its OpenCode check from the user's home folder and
  never creates project folders.

## 9. Windows `.tgz` report (task 1.3, 2026-11-04)

- No mmhmem release has published a `.tgz`, `.tar.gz` or `.zip`. Assets per
  release (`gh release view`): v0.2.0 and v0.2.1 have `mmhmem-linux-x86_64`
  and `mmhmem.exe`; v0.2.2 adds the macOS binaries; v0.3.1, v0.3.3 and v0.4.1
  add `SHA256SUMS`, `SHA256SUMS-native` and `verification.json`. Windows has
  always been a bare `mmhmem.exe`.
- Repository history (`git log --all -S tgz` / `-S tar.gz`): the only use is
  the sccache download in the Linux `Dockerfile` builder stage, plus spec text.
  No `*.ps1`, `*.sh`, workflow or `plugins/` file downloads an archive.
- Likely origin outside this repository: OpenCode installs its npm plugins
  (`@tarquinen/opencode-dcp@latest` in the global config) from npm, and npm
  packages are `.tgz` tarballs. This is unconfirmed, since the exact output
  the user saw is not available.
- Design consequence: unchanged. Windows gets a `.zip` unpacked with
  `Expand-Archive`, and the CI gate asserts that the Windows path never calls
  `tar`.

## 10. Check call (task 1.2, 2026-11-04)

Every existing tool appends breadcrumb and audit records, according to its
description and the code: `mem_search_breadcrumbs` calls
`BreadcrumbService::crumb` after its audit event. The user chose a new
read-only `status` tool that writes one `STATUS_CHECK` audit record (the
steering says every action is audited) and no breadcrumbs, sessions or
observations.

- `src/services/status.rs`: reads table count and `PRAGMA journal_mode`;
  answers `mmhmem <version>: store ok (<n> tables, journal wal)`.
- MCP annotations: read-only, idempotent, not destructive, not open-world.
- Proof: `tests/local_stdio.rs::status_tool_changes_no_store_rows_and_audits_once`
  seeds a session, then shows that the row counts of `active_sessions`,
  `session_breadcrumbs`, `observations`, `shunt_file_cache` and `token_stats`
  are unchanged after the call, and that exactly one new audit line,
  `STATUS_CHECK`, was written.
- The tool list grows from 9 to 10; `docs/usage.md` is updated.

## 11. Pinned library set (task 1.4, 2026-11-04)

Pinned in `Cargo.toml` (exact versions), resolved with `cargo update -w`:

| Crate | Version | Licence | Use |
|---|---|---|---|
| self_update | 1.3.3 | MIT | GitHub release lookup, download, unpack, replace. Features: `reqwest`, `rustls`, `github`, `archive-tar`, `compression-tar-gz`, `archive-zip`, `compression-zip-deflate`, `signatures`, `checksums`; no defaults, no progress bar |
| zipsign-api | 0.2.2 | MIT OR Apache-2.0 | `verify-tar`, `verify-zip` (production); `sign-zip`, `sign-tar` (dev only) |
| ratatui / crossterm | 0.30.2 / 0.29.0 | MIT / MIT | menu |
| toml_edit | 0.25.18 | MIT OR Apache-2.0 | Codex config |
| jsonc-parser | 0.34.0 (`cst`) | MIT | JSON/JSONC |
| tempfile / fs-err | 3.27.0 / 3.3.2 | MIT OR Apache-2.0 | atomic writes (tempfile moved from dev to normal deps) |
| similar | 3.2.1 | Apache-2.0 | diffs |
| which / etcetera | 8.0.6 / 0.11.0 | MIT / MIT OR Apache-2.0 | detection, folders |
| indicatif | 0.18.6 | MIT | progress |
| winreg (Windows only) | 0.56.0 | MIT | PATH |
| insta / assert_fs / httpmock / zip (dev) | 1.49.0 / 1.1.4 / 0.9.0 / 9.3.0 | Apache-2.0 / MIT OR Apache-2.0 / MIT / MIT | tests |

Not added yet, because nothing uses them before their tasks: `duct` (3.x,
optional), `assert_cmd` and `expectrl` (8.1).

Findings:
- **One HTTP/TLS stack.** `self_update` reuses the existing `reqwest`
  0.13.5 / rustls; `cargo tree -d -e normal` shows no second `reqwest`,
  `rustls`, `hyper` or `tokio`.
- **Accepted duplicates:**
  - `sha2`/`digest` 0.10 (from `ed25519-dalek` 2.2 inside `zipsign-api`)
    next to our 0.11;
  - `base64` 0.22/0.23 (0.23 already came from `hyper-util`);
  - `syn` 2/3 (build time only).
- **`self_update` checksums:** its `checksums` feature verifies the archive
  against our `SHA256SUMS` asset (`checksum_from_asset`), so §2's checksum
  check needs no own code.
- **Empty key set disables signature checks.** The `self_update` docs say
  an empty `verifying_keys` set silently **disables** signature checking.
  `release.rs` (task 5.4) must refuse to build an updater without at least
  one embedded key, and test that.
- **The signature is bound to the archive's file name** (zipsign context).
  Renaming an asset after signing breaks verification, so the pipeline signs
  the archive under its final published name.
- **Lockfile grows from 264 to 431 crates** (167 added, many dev-only
  through `httpmock`). Binary size impact is measured in task 6.2 once the
  menu code links `ratatui`.
- **WTFPL `terminfo`** appears in `Cargo.lock` only through `ratatui`'s
  optional `termwiz` backend and is not compiled
  (`cargo tree -i terminfo --target all -e all` prints nothing). Task 6.2
  sets `ratatui` to `default-features = false, features = ["crossterm"]`.
- **Binary size (task 6.2, Windows x86_64 release, 2026-11-04):** HEAD
  `89a97cb` (menu not linked) 16,178,176 bytes; with the menu and
  `ratatui` restricted to `crossterm` 20,785,152 bytes (+4.4 MiB). Most of
  that is the installer code `mmhmem setup` now reaches (agents, plan,
  check, release lookup); the same tree with `ratatui`'s default features
  is 20,797,440 bytes, so the restriction saves 12 KiB. With the
  restriction `termwiz`, `terminfo` and their dependencies left
  `Cargo.lock` (609 lines fewer).
- **Spike** (`tests/setup_update_spike.rs`, against an `httpmock` GitHub API
  mock):
  - a zipsign-signed `.zip` with a matching `SHA256SUMS` is installed;
  - an unsigned archive is refused with "could not read signatures";
  - one signed by another key, with "no matching key/signature pair found";
  - a wrong `SHA256SUMS` entry, with "checksum mismatch".

  In every refusal the installed file stays unchanged.

Verified on Windows only: `cargo build` (default and `otel`), `cargo clippy
--all-targets -D warnings` (default and `otel`), `cargo fmt --check`, and the
spike tests. **Pending:** Linux and macOS builds. This machine has no Linux
or macOS Rust targets, and WSL has no Rust toolchain. They are first proven
by CI (task 8.1), so task 1.4 stays open until then.
