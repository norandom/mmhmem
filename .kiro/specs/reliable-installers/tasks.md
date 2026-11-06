# Implementation Plan

All tasks are required. Each executable task is sized for approximately 1–3 hours.
Order: settle the open points, then the safe-write foundation, then agents and
the functional check, then release assets and bootstraps, then the menu,
update and removal, then the CI end-to-end gate, and finally migration and
deletion of the old scripts. Every commit must pass the project pre-commit gate
(fmt, size caps, clippy default and `otel`). New dependencies are pinned to an
exact version, as `tech.md` requires. Do not push `main` until task 9.2 is
released.

- [ ] 1. Settle the open points
- [x] 1.1 Confirm each agent's config format and check command
  - From each agent's current official documentation and its installed CLI, record the exact MCP server shape and file location for Claude Code (`claude mcp add/get/list/remove --scope user`), Claude Desktop (`claude_desktop_config.json`, MSIX and non-MSIX profiles), Codex (`config.toml` `[mcp_servers.*]`, `codex mcp list/get`, `hooks.json`) and OpenCode (`mcp.<name>` shape, `opencode.json` vs `.jsonc`, `opencode mcp list`).
  - Capture real output of each list/get command, connected and not connected, as test fixtures.
  - Done when `research.md` has a confirmed table with sources and versions, the OpenCode `mcp.servers` defect is confirmed or refuted, and the fixtures are committed under `tests/fixtures/setup/agents/`.
  - _Requirements: 3.1, 6.2_

- [x] 1.2 (P) Choose the side-effect-free check call
  - Check whether `mem_search_breadcrumbs` writes to the store or creates a session when called.
  - If it writes anything, add a read-only `status` tool that returns the version and store health, with tests.
  - Done when a test calls the chosen tool against a temporary store and shows the store's row counts and files are unchanged afterwards.
  - _Requirements: 6.1_

- [x] 1.3 (P) Trace the Windows `.tgz` report
  - Search the release history, `deploy/`, `scripts/`, workflow logs and agent marketplace/plugin build scripts (`build-marketplace.*`) for any `.tgz` or `.tar.gz` used on Windows.
  - Done when `research.md` names the source, or records that none exists in this repository and which external tool was the likely origin.
  - _Requirements: 2.8_

- [x] 1.4 Pin and vet the adopted libraries
  - Status 2026-11-05: the pinned set builds on Linux (musl), macOS (arm64 and x86_64) and Windows on GitHub: CI run 42012757538 and the release-target builds of Installer e2e run 42012757529 on 9bddd39, both green.
  - Add the libraries from `research.md` §6 with exact versions, including `self_update` with `default-features = false` and rustls, `archive-zip`, `archive-tar`, `compression-zip-deflate`, `compression-flate2` and `signatures`.
  - Check with `cargo tree -d` that no second `reqwest`, `rustls` or TLS stack is pulled in, and that the default release build and the `otel` build still pass the pre-commit gate.
  - Record each crate's version, licence and last release date in `research.md`.
  - Done when the pinned set builds on Linux, macOS and Windows, `cargo tree -d` shows no unexplained duplicates, and a spike updates a test binary from an `httpmock` release with a `zipsign`-signed archive and rejects an unsigned one.
  - _Requirements: 2.4, 8.3_

- [x] 2. Build the safe-write foundation
- [x] 2.1 Atomic write, backup and owner-only files
  - Implement `src/setup/fsx.rs`: atomic replace via a same-folder temp file and rename (`tempfile`), backup copy into the record's backup folder, and owner-only creation (Unix mode `0600` at open; Windows security descriptor granting only the current user SID at `CreateFileW`).
  - Done when tests show an interrupted write leaves the old file, backups are byte-identical, and a new key file is owner-only from creation on each OS (ACL read back on Windows, mode on Unix).
  - _Boundary: fsx_
  - _Requirements: 3.2, 3.3, 9.1_

- [x] 2.2 (P) JSON and JSONC entry editing
  - Implement `src/setup/edit/json.rs` on `jsonc-parser` CST: set, read and remove the value at a pointer, keeping comments, whitespace, key order, trailing commas, CRLF and a BOM if present.
  - Re-parse the result and reject output that does not parse.
  - Done when golden tests with comments, odd formatting, CRLF, BOM and deep nesting show add followed by remove is byte-identical to the input, and an invalid input is reported with line and column.
  - _Boundary: edit-json_
  - _Requirements: 3.1, 3.2, 3.4, 8.3_

- [x] 2.3 (P) TOML table editing
  - Implement `src/setup/edit/toml.rs` on `toml_edit`: set, read and remove one table, keeping comments and formatting.
  - Recognise and convert the old `# BEGIN mmhmem` marker block into a plain table, and detect a duplicate `[mcp_servers.mmhmem]`.
  - Done when golden tests show add/remove is byte-identical, the marker block converts cleanly, and invalid TOML is reported without changes.
  - _Boundary: edit-toml_
  - _Requirements: 3.1, 3.2, 3.4, 8.3_

- [x] 2.4 Install record with journaling
  - Implement `src/setup/record.rs`: the schema from the design, per-OS location, entry-level normalized hashes, `applying`/`done` states, atomic save, drift detection against disk, and the guarantee that no field can hold a secret.
  - Done when tests cover save/load, drift on a changed entry versus unrelated edits in the same file, recovery after a simulated crash at each step, and an unreadable record.
  - _Boundary: record_
  - _Depends: 2.1_
  - _Requirements: 5.1, 5.2, 5.3, 5.5, 5.7_

- [x] 2.5 Plan, apply and rollback
  - Implement `src/setup/plan.rs`: `Plan`/`Change` types, apply exactly the planned bytes in order with backup and journaling, per-agent rollback in reverse order, and a plain-language summary plus diff per change.
  - Done when tests show applied bytes equal planned bytes, a failure mid-agent restores that agent's files byte-identically and leaves other agents intact, and the record matches the disk afterwards.
  - _Depends: 2.1, 2.4_
  - _Requirements: 3.2, 3.5, 8.5_

- [x] 2.6 Record rebuild and adoption of old installs
  - When the record is missing or unreadable, scan each agent's known locations for mmhmem entries, files and the old Codex marker block, and produce an adoption plan that requires confirmation.
  - Done when tests seeded with homes produced by the old PowerShell and Bash installers yield the expected adoption plan and change nothing until it is confirmed.
  - _Depends: 2.3, 2.4, 2.5_
  - _Requirements: 5.4, 5.6_

- [x] 3. Integrate the agents
- [x] 3.1 Agent trait, environment and embedded assets
  - Rule for every agent module (from the 2.2/2.3 reviews): edit leaf keys of mmhmem's own entry (e.g. `mcp.servers.mmhmem.command`), never replace a whole user-visible object or array. Use the JSON array element functions for shared arrays (Claude Code `hooks.PreToolUse`), and keep user-added keys inside mmhmem's entry. Record `missing_parents` before a `set` so removal prunes only parents mmhmem created.
  - Implement `src/setup/agents/mod.rs` (trait, registry, `Env` with home, OS and `PATH`) and `src/setup/assets.rs`, embedding skills, commands, routing instructions and the OpenCode plugin moved to `assets/setup/`.
  - Add test doubles for the `claude`, `codex` and `opencode` CLIs that record arguments and edit files the way the confirmed real CLIs do.
  - Done when a test home can be constructed and each double replays the fixtures from task 1.1.
  - _Depends: 1.1_
  - _Requirements: 2.1, 8.1, 8.4_

- [x] 3.2 (P) Claude Code
  - Decide (from the 2.5 review): `plan.rs` applies file bytes only. Either plan the `~/.claude.json` `mcpServers.mmhmem` edit as file bytes with `edit::json` (reviewable, journaled, rollback-safe), or treat `claude mcp add/remove` as a separate non-file step with its own record and rollback. Prefer file bytes unless 1.1 fixtures show Claude Code rewrites the file in a way that conflicts.
  - Detect the CLI, plan MCP registration through `claude mcp add --scope user`, the skill, the recap command, the delimited section in `CLAUDE.md` and the optional hook entry in `settings.json`, and plan removal of only those.
  - Done when temp-home tests cover install, re-plan with no changes, the hook on and off, removal leaving unrelated settings byte-identical, and an invalid `settings.json` skipping only Claude Code.
  - _Boundary: agent-claude-code_
  - _Depends: 2.2, 2.5, 3.1_
  - _Requirements: 3.1, 3.4, 7.3_

- [x] 3.3 (P) Claude Desktop
  - Detect MSIX and non-MSIX profiles. Several profiles become a choice for the user, not an abort.
  - Plan `mcpServers.mmhmem` and the skill folder, and add "restart Claude Desktop" as a pending user step.
  - Done when tests cover each profile layout, several profiles, install, removal and an invalid config.
  - _Boundary: agent-claude-desktop_
  - _Depends: 2.2, 2.5, 3.1_
  - _Requirements: 3.1, 3.4, 6.6, 7.3_

- [x] 3.4 (P) Codex
  - Plan `[mcp_servers.mmhmem]` with `toml_edit`, the skills, the recap prompt and the optional `hooks.json` entry. Mark hook approval as a pending user step.
  - Done when tests cover install, the old marker block, an existing duplicate table, removal and an invalid `config.toml`.
  - _Boundary: agent-codex_
  - _Depends: 2.3, 2.5, 3.1_
  - _Requirements: 3.1, 3.4, 6.6, 7.3_

- [x] 3.5 (P) OpenCode and Antigravity
  - OpenCode: plan the confirmed `mcp.servers.mmhmem` shape in the right one of `opencode.json` / `opencode.jsonc` (design §5 rule; report an entry present in both), plus the skill, the plural `commands/` folder and the optional plugin file. Adopt an existing correct entry written by the old installer instead of rewriting it.
  - Antigravity: detect it, and show the import snippet as a user step.
  - Done when tests cover both file names, comments kept in `.jsonc`, cleanup of the old entry, removal and an invalid file.
  - _Boundary: agent-opencode_
  - _Depends: 1.1, 2.2, 2.5, 3.1_
  - _Requirements: 3.1, 3.4, 7.3_

- [x] 4. Prove each agent works
- [x] 4.1 MCP functional check
  - Implement `src/setup/check.rs`: read back the registration as the agent will use it, start that exact command with the `rmcp` child-process client with a 20 s timeout, and run `initialize`, `tools/list` against the release's expected tools, and the tool chosen in 1.2.
  - Done when tests pass against the real binary and fail with plain reasons for a wrong path, a server that exits, a timeout and a missing tool.
  - _Depends: 1.2, 3.1_
  - _Requirements: 6.1, 6.5_

- [x] 4.2 Agent checks, hook sample and rollback on failure
  - Per agent, run its list/get command and parse the confirmed output, validate that the agent still loads its config, and run the hook sample when the hook is selected.
  - Wire a failed check to per-agent rollback, and store `last_check` and pending user steps in the record.
  - Done when tests with the CLI doubles cover pass, not connected, invalid config after write (rolled back) and hook mismatch, and the result lines match the design's wording.
  - _Depends: 2.5, 3.2, 3.3, 3.4, 3.5, 4.1_
  - _Requirements: 3.5, 6.2, 6.3, 6.4, 6.5, 6.6_

- [x] 5. Publish verifiable releases
- [x] 5.1 Archive assets per target
  - Change the release build to publish `mmhmem-<tag>-<target>.zip` for Windows and `.tar.gz` for Linux and macOS, plus the existing bare binaries for one transition release, and include all of them in `SHA256SUMS`.
  - Done when `publish-ci-release.ps1` tests show the asset set and that every asset is listed and re-verified after upload.
  - _Requirements: 2.3, 2.8_

- [x] 5.2 (P) Sign checksums and publish attestations
  - Generate the ed25519 key pair with `zipsign`. Store the secret only in the GitHub `release` environment with required reviewers, and embed the public key list in the binary.
  - Sign each archive with `zipsign`, then compute `SHA256SUMS` over the signed archives, sign it as `SHA256SUMS.sig` with the same key, run the existing remote byte verification, and add `actions/attest-build-provenance` for every archive.
  - Done when a dry-run publish produces archives and a `SHA256SUMS.sig` that the embedded keys verify, and a modified archive or `SHA256SUMS` fails verification in a test.
  - _Requirements: 2.4_

- [x] 5.3 Render the bootstraps per release
  - Write `deploy/bootstrap/install.sh.tmpl` (POSIX sh) and `install.ps1.tmpl` (Windows PowerShell 5.1 compatible) as the design describes. Write `scripts/render-bootstraps.ps1` to fill in the tag, base URL and archive hashes, and publish both rendered scripts as release assets.
  - Done when shell tests (`sh`, `bash`, `dash`, PowerShell 5.1 and 7) show the scripts contain no agent logic, verify the hash, stop on a 404 or mismatch with plain messages and no changes, unpack with `tar` or `Expand-Archive`, start setup with `/dev/tty` on Unix, and clean up the temp folder.
  - _Depends: 5.1_
  - _Requirements: 1.1, 1.3, 2.1, 2.2, 2.3, 2.5, 2.7, 2.8, 8.2_

- [x] 5.4 Release verification inside setup
  - Implement `src/setup/release.rs` as a thin wrapper around `self_update`'s GitHub backend: latest-release lookup, asset choice by target triple, download, `zipsign` signature check with the embedded `verifying_keys`, and unpack. Add the detached `SHA256SUMS.sig` check (`zipsign-api` / `ed25519-dalek`), the archive hash check, and a re-hash of the installed binary for status. Run the blocking calls in `spawn_blocking`.
  - Refuse to build an updater with an empty key set (self_update silently disables signature checks then; research §11), and use `checksum_from_asset("SHA256SUMS")` for the hash check.
  - Done when tests against a local HTTP server (the `self_update` base URL pointed at `httpmock`) cover success, an empty key set, a bad archive signature, a bad `SHA256SUMS` signature, a bad hash, a missing asset and an unsupported platform, each leaving nothing changed.
  - _Depends: 1.4, 5.2_
  - _Requirements: 2.3, 2.4, 2.5, 2.6, 2.7_

- [x] 6. Build the menu
- [x] 6.1 Plain-language text and the `setup` entry point
  - Add `mmhmem setup` to the hand-parsed CLI in `src/main.rs` with no user-facing flags (only the internal `--bootstrap <tag>` and the maintainer `--release-dir <dir>`).
  - Put every user-facing string in `src/setup/ui/text.rs`, reviewed for users who don't know the project.
  - Done when `mmhmem setup` starts, the internal arguments are hidden from help, and a test asserts that no string in `text.rs` uses project-internal terms without an explanation.
  - _Requirements: 1.3, 1.4, 1.5_

- [x] 6.2 Full-screen menu
  - Set `ratatui` to `default-features = false, features = ["crossterm"]` and record the release binary size before and after (research §11).
  - Implement `src/setup/ui/app.rs` on `ratatui` + `crossterm`: Home with state from the record plus live detection and the newest release, Agents, Features (hook off by default), Review with diff, and Progress with check results. Show only applicable actions and keep a visible key legend.
  - Done when `TestBackend` snapshot tests cover each screen and state (fresh, installed, update available, drift, pending user step), and key-sequence tests show cancel or quit at every screen before confirm changes nothing.
  - _Depends: 2.5, 4.2, 6.1_
  - _Requirements: 4.1, 4.2, 4.3, 4.4, 4.5, 4.6, 4.7, 1.5_

- [x] 6.3 (P) Numbered fallback and no-terminal default
  - Implement `src/setup/ui/fallback.rs` with the same screens as numbered prompts, used when raw mode or the alternate screen is unavailable or `TERM=dumb`. Without a terminal, install the default selection, or stop without changes when no agent is detected.
  - Done when tests drive the fallback through install and cancel, and the no-terminal path installs the defaults with plain result lines.
  - _Boundary: ui-fallback_
  - _Depends: 6.1_
  - _Requirements: 4.8, 4.9_

- [x] 7. Update and removal
- [x] 7.1 Update flow
  - Let `self_update` replace the running binary (Windows file-in-use case included), record its hash and the `PATH` entry (`winreg` + `WM_SETTINGCHANGE` on Windows), re-plan all agents with the recorded selection, review, apply and check. An up-to-date install yields "Everything is current".
  - Done when tests update a temp-home install from a previous-release fixture to the current one, keep the user's choices, and show a second run reports no changes with byte-identical files.
  - _Depends: 5.4, 6.2_
  - _Requirements: 7.1, 7.2_

- [x] 7.2 (P) Removal flow
  - Reverse only recorded entries with the same parsers, remove the binary, the `PATH` entry and the record last, and keep the store unless "Also delete my saved memories" is ticked.
  - Done when tests show unrelated settings, including later user edits to shared files, are byte-identical after removal and the store survives by default.
  - _Depends: 6.2_
  - _Requirements: 7.3, 7.4_

- [x] 7.3 Update mode (owner decision 2026-11-06)
  - Add `mmhmem update` (plain questions, internal `--release-dir` and `--restarted` only), the "mmhmem vX is set up" home title with "Update to vN (keeps your choices)" first and selected, update mode for the install command run again in a terminal with a newer release, and ask only if needed after the update: an empty replan only runs the check and shows the results; otherwise the changes are listed and confirmed once.
  - Done when tests cover the argument parsing, no record, no terminal, an unknown newest release, up to date, a declined update, an accepted update with and without changes, the install command with a newer and with the same release, and the menu's restarted run skipping the Review; and the installer e2e `update` and `update-command` scenarios pass on GitHub.
  - Status 2026-11-06 (closed): the installer e2e workflow ran green on all five legs with both scenarios, on d7186d4 (run 42219236629) and again on 04b79d9 (run 42221440180, with CI 42221440231 green after a deterministic-test fix).
  - Status 2026-11-06: implemented with unit, flow, menu and CLI tests (`src/setup/ui/update.rs`, `update_tests.rs`); the `update` e2e scenario now expects the results instead of the Review, and the new `update-command` scenario runs `mmhmem update --release-dir` in the pseudo-terminal. Open until the installer e2e workflow runs green on GitHub (the scenarios never run locally).
  - _Depends: 7.1_
  - _Requirements: 7.5_

- [ ] 8. Gate releases on real installs
- [x] 8.1 End-to-end harness
  - Status 2026-11-05: done. Installer e2e run 42012757529 on 9bddd39 is green on ubuntu-latest, macos-15 (arm64), macos-15-intel and windows-latest under PowerShell 5.1 and 7, through every step.
  - Status 2026-11-04: workflow defined (actionlint clean) and every local step verified on Windows under PowerShell 7 and 5.1; open until the job runs green on GitHub after the first push. Phases that run the real installer refuse to run outside GitHub Actions unless MMHMEM_E2E_ALLOW_LOCAL is set.
  - Status 2026-11-05: first GitHub run (41883221749) reached the scenarios on every leg. Fixed: on Linux/macOS the server started by `serve` stopped answering once that step's pwsh exited (Start-Process's redirection is a pwsh-owned pipe; the next request's log line raised BrokenPipeError), and the next phase called it "taken by another program". The server now owns its log files (`scripts/e2e-http-server.py`), and `Start-Server` restarts its own unresponsive server (server.pid) and refuses only a foreign one. A failed leg prints `claude mcp get/list` (`-Phase agent-diagnostics`). Still open until a green run.
  - Write the CI job `.github/workflows/installer-e2e.yml`: build the commit's archives, render the bootstraps with a local base URL using the same renderer, serve them over local HTTP, and create a clean home seeded with agent configs that contain unrelated settings, comments and odd formatting. Install the real `claude`, `codex` and `opencode` CLIs from their official packages.
  - Drive the menu path with `expectrl` (pseudo-terminal on Unix, ConPTY on Windows), and serve assets with `httpmock` or `tiny_http`.
  - Done when the job runs green on `ubuntu-latest`, `macos-15` (arm64), `macos-15-intel` (x86_64; the macos-13/14 images were retired in 2026) and `windows-latest` (PowerShell 5.1 and 7) through the seeding step.
  - _Depends: 5.3_
  - _Requirements: 10.1, 10.2_

- [x] 8.2 End-to-end scenarios
  - Status 2026-11-05: done. Installer e2e run 42012757529 on 9bddd39 passed every scenario on all five matrix entries. The manual run 42014205966 with `sabotage=wrong-hash` failed on all five: each bootstrap refused the archive ("does not match the checksum built into this installer, so it was NOT used") and the gate job failed.
  - Status 2026-11-04: every scenario is implemented (`scripts/e2e-scenarios.ps1`, comparison rules in `scripts/e2e-home-state.ps1`) and wired into the workflow with `E2E_SCENARIOS_ENFORCED: 'true'`; the Rust test-key feature `e2e-release-key` exists and the publisher refuses its marker. Open until the job runs green on GitHub: the scenarios run the real installer, which never runs locally. Decisions: the menu is driven by `ci/e2e/pty-driver` (portable-pty + vt100, one program for pty and ConPTY) rather than `expectrl`, because it gives a screen model instead of a byte stream and answers ConPTY's cursor-position request; "update from the previous release" uses the same commit built with the previous version number (only builds that trust the run's throwaway key can be updated to); Claude Code rewrites `~/.claude.json` itself, so that one file is compared parsed (see the header of `e2e-home-state.ps1`). Run the workflow by hand with `sabotage=wrong-hash` to see the gate fail.
  - Status 2026-11-05: in run 41883221749 both Windows legs passed cancel-at-review, download-404 and bad-hash, then the install rolled back on "Claude Code lists mmhmem but could not connect to it". Cause: outside Windows Terminal Claude Code prints `Status: √ Connected` (U+221A), and the check accepted only `✔` (research.md §8). The check now accepts the fallback tick; the expectation stays `agent_command=pass`. Still open until a green run.
  - Status 2026-11-05 (run 41994007615): install, assertions and rerun passed on every leg; the removal failed on all five because `mmhmem stdio`, kept running by the OpenCode service that setup's own `opencode mcp list` check had started, held the store (Windows: memory.db locked; Unix: -wal/-shm changed). Fixed in setup: the OpenCode check stops the service again when `opencode service status` said `stopped` before it, and never touches one that was already running (research.md §8 "Third e2e run"). The e2e now fails when mmhmem or that service still runs after install, rerun, update, invalid-config or removal, compares the memory store by its database content instead of the -wal/-shm bytes, counts OpenCode's service password file and Claude Code's mmhmem connection log as the agents' own, and on Windows fails when the removed program is left in %TEMP%. Still open until a green run.
  - Run the exact user one-liner with a pseudo-terminal (`script` / ConPTY) for the menu path and without a terminal for the default path. Require exit 0 and every agent passing.
  - Then run again (byte-identical home and "Everything is current"), update from the previous published release, remove, and compare the seeded files byte-for-byte.
  - Add the failure cases: cancel at Review, a 404 on the archive, a wrong hash, a bad signature on update, and an already-invalid `opencode.json`. Each must leave the home byte-identical.
  - Done when all scenarios pass on every matrix entry, and a deliberately broken commit (wrong hash rendered) fails the job.
  - _Depends: 7.1, 7.2, 8.1_
  - _Requirements: 10.3, 10.4_

- [x] 8.3 Block publishing on the gate
  - Status 2026-11-06: proven by the v0.5.0 release (run 42189177772, tag on a1de88f with green CI 42187133907 and installer e2e 42187133942): the publish job waited for approval of the `release` environment, built and checked the draft, attested the four archives (`gh attestation verify` passes for each, signer `release.yml@refs/tags/v0.5.0`) and published it. A fresh download verifies: every asset matches `SHA256SUMS`, `SHA256SUMS.sig` and all four archive signatures verify with the key in `RELEASE_PUBLIC_KEYS`, a one-byte change fails, and the bootstraps carry the archive hashes.
  - Status 2026-11-05: implemented, open until it runs on GitHub. `scripts/publish-ci-release.ps1` now requires, besides main CI, the newest `push` run of `installer-e2e.yml` on `main` for the tag commit to be completed and successful with its `installer-e2e` job green (waiting up to 30 min, like the CI wait); `workflow_dispatch` runs (the only kind that can carry `sabotage=wrong-hash`) and `pull_request` runs never count. The check runs in the script, not via a second `workflow_run` trigger, so all three entry points (tag push, CI completion, dispatch) pass the same fail-closed gate before any artifact download or release write. The build step leaves the release a draft and checks its exact asset set; attestation follows; a last step (`-PublishDraft`, no signing key) re-checks the tag, both runs and the asset set, then publishes. Proven locally (pwsh 7 and PowerShell 5.1): `test-publish-ci-release.ps1` shows the publisher refuses with no, a red, an unfinished or only a `workflow_dispatch` installer run, or a red aggregate job, with zero `gh` calls, and proceeds to the artifact download when both are green; the draft is published only with an exact, fully uploaded asset set, and release.yml orders draft, attest, publish. Not proven until a real tag release: the GitHub API answers in that shape for these queries, the step outputs reach the later steps, the attestation of the draft's archives, and publishing the draft.
  - Make the release workflow require a green `installer-e2e` run for the exact tag commit, in the same way it already requires main CI.
  - Follow-up from the 5.2 review: the provenance attestation currently runs after the release is published. Keep the release a draft until signing, upload verification and attestation all succeed, then publish it as the last step.
  - Done when `test-publish-ci-release.ps1` shows publishing refuses a commit without a green installer run.
  - _Depends: 8.2_
  - _Requirements: 10.5_

- [ ] 9. Migrate and remove the old installers
- [x] 9.1 README and release page
  - Status 2026-11-06: done. `README.md` (6a02d2e) and the new release notes template `.github/release-notes.md` start with the two release one-liners, then one paragraph on what the installer does and the `gh attestation verify` line; `scripts/publish-ci-release.ps1` creates the draft with `--notes-file .github/release-notes.md --generate-notes` (checked by `scripts/test-publish-ci-release.ps1`). `docs/installation.md` is rewritten for the new installer; `docs/configuration.md`, `docs/usage.md`, `deploy/README.md` and `.marketplace/README.md` no longer describe the old installer or its flags. The docs check is `tests/install_docs.rs` (full `cargo test` in CI, and the pre-commit hook): the first two code blocks and the first install text in README.md are exactly the two one-liners, every `install.ps1`/`install.sh` URL in README.md and `docs/*.md` is a `github.com/norandom/mmhmem/releases/...` URL, user docs contain no `raw.githubusercontent.com/.../HEAD/install`, `install_local`, `bash -s --` or old installer flags, README.md and `docs/installation.md` contain no `git clone`/`cargo build`/`cargo install`, the release notes start with the one-liners, and the root stubs print the same one-liners. `docs/development.md` (contributors) and `docs/server.md`'s `git clone` (the optional Docker server, not an install of mmhmem) are deliberately outside the old-instructions check.
  - Put the two one-liners first in `README.md` and in the release notes template, followed by one plain paragraph on what the installer does and the optional `gh attestation verify` command. Remove all checkout-based install instructions from user docs.
  - Done when a docs check asserts the one-liners are the first install text and point at release URLs only.
  - _Requirements: 1.1, 1.2, 2.2_

- [ ] 9.2 Transition release
  - Status 2026-11-06 (later): the root `install.ps1` / `install.sh` are now stubs. They print that the installer moved, the new one-liner for that OS, that it installs the latest signed release and opens `mmhmem setup`, and `mmhmem update` (v0.5.1+) / `mmhmem setup` (v0.5.0) for updates; they ignore every old option, download and change nothing, and report exit code 2 (`install.ps1` piped into `iex` sets `$LASTEXITCODE` to 2 and never calls `exit`). Checked offline by `scripts/test-install-stub.ps1` (pwsh 7 and PowerShell 5.1: `-File`, `irm | iex`, in-process with download commands stubbed) and `scripts/test-install-stub.sh` (sh, bash, dash, busybox sh as available: file, `| sh`, `| bash -s -- <options>`), both run by `scripts/test-install.*`, so by the pre-commit hook, the Windows CI job and the Dagger `distribution` and `static_installer` stages; the old release-installer coverage there now runs `scripts/fixtures/release-install.*` pinned to a tag. Releases: v0.5.0 (release run 42189177772, installer e2e 42187133942) and v0.5.1 (release run 42226523811 on e41238b, installer e2e 42224460436) passed the installer gate and are published; v0.5.1 is `latest` and ships `install.ps1` / `install.sh`. The installer e2e runs the release one-liners (`irm <url>/install.ps1 | iex`, `curl -fsSL <url>/install.sh | sh`) with the rendered bootstraps on all five legs, green on the last five pushes to `main` (42219236629 on d7186d4 through 42227750000 on 6a02d2e). Still open: (1) the stubs reach users only when this commit is pushed to `main` (old HEAD URLs serve `main`); (2) the e2e serves the bootstraps from a local server, so an install with the published `github.com/norandom/mmhmem/releases/latest/download/...` one-liners on each matrix platform is not yet recorded. For (2), `.github/workflows/published-install.yml` now exists (evidence, not a gate; no secrets): after each successful "Publish release" run that published the latest release, weekly and by hand, it runs the README one-liners exactly as written, without a terminal, in the seeded throwaway home on the same five legs, and asserts with the e2e's own checks (`scripts/e2e-run.ps1 -Phase published-release | published-install | assert-published`, `scripts/e2e-published.ps1`): `SHA256SUMS.sig` verified with the key in `src/setup/release_keys.rs`, the archive against `SHA256SUMS` and its signature, exit 0, the record names the latest tag, the installed program is the one in that archive, Claude Code, Codex and OpenCode recorded with the default features (memory and the read-routing hook) and an OK check, seeded files unchanged, nothing left running. It needs `main` to carry it before it can run; 9.2 closes when it has run green on all five legs.
  - Replace the root `install.ps1` / `install.sh` with stubs that print the new one-liner and exit without downloading. Publish the first release with `mmhmem setup`, then push `main`.
  - Done when the release passes the installer gate and the published one-liners install it on all matrix platforms.
  - _Depends: 8.3, 9.1_
  - _Requirements: 2.2_

- [ ] 9.3 Delete the replaced scripts
  - In the following release, delete `lib.ps1`, `lib.sh`, `deploy/install_local.*`, `deploy/manage_*`, `deploy/*_onboarding.*`, `deploy/check_worker.*`, `deploy/install_binary.ps1`, the root stubs, and their `scripts/test-*` / `test_*.py` files. Before deleting, port any still-relevant test case to the Rust suites.
  - Done when the repository has no installer logic outside `src/setup/` and `deploy/bootstrap/`, `rg` finds no reference to the deleted files, and CI including the installer gate is green.
  - _Depends: 9.2_
  - _Requirements: 8.6_
