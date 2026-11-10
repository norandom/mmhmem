# Validation Log — antigravity-native-setup

## 2026-11-10 — Feature validation (tasks 1.1–5.3) and live run on the owner's machine

### Verdict: GO (one owner confirmation outstanding, see "Open")

- All tasks `[x]` except 5.3, whose last item (interactive prompt check)
  waits for the owner to confirm the session used was `agy`.
- Full suite on the release commit: CI run 42819787157 and Installer e2e run
  42819787087 on `43375f8` (v0.5.3) green; earlier runs 42793963503 /
  42793963495 on `09d03c4` green with the new Antigravity e2e phase on all
  five runners (ubuntu, macos-15, macos-15-intel, windows PowerShell 5.1,
  windows pwsh 7). The only non-success job is the gated `E2E_LLM`
  measurement, skipped by design.
- Release: `v0.5.3` published 2026-11-10T15:16:54Z through the release
  pipeline (plan + publish jobs green, environment `release` approved),
  assets: `install.ps1`, `install.sh`, four archives, `mmhmem.exe`,
  `mmhmem-linux-x86_64`, two macOS binaries, `SHA256SUMS`, `SHA256SUMS.sig`,
  `verification.json`.

### Requirements coverage

| Req | Evidence |
|---|---|
| 1.1–1.3 | `~/.gemini/config/mcp_config.json` written by `mmhmem update` on the owner's machine (3.2 unit tests; `setup_cli_antigravity` scenario 1; e2e phase). Owner's run shows "+ Let Antigravity use mmhmem (adds mmhmem to Antigravity's server list in ~\.gemini\config\mcp_config.json)". |
| 1.4, 1.5 | Update output ends "Antigravity: start a new session." with no snippet step; `tests/install_docs.rs` and `text_tests` forbid snippet/import/paste wording. |
| 2.1, 2.5 | `hooks.json` holds exactly `mmhmem-read-routing` → `PreToolUse[{matcher "^read_file$", hooks[{type command, command "<abs>\mmhmem.exe hook antigravity", timeout 10}]}]` (3.3 golden tests; live file identical). |
| 2.2–2.4 | Live hook call with the whole-file payload → `{"decision":"deny","reason":"… call smart_read … agent_harness \"antigravity\" …"}`, ranged payload → `{"decision":"ask"}`, both exit 0; `agy -p` whole-file read was redirected and a ranged read answered "line" (audit records: redirect `large_file`, pass `range`, twice). |
| 2.6 | 3.5 unit test `the_hook_sample_runs_without_a_worker_and_the_inactive_line_stays`. |
| 3.1–3.3 | 3.1 unit tests; owner's machine detected via `agy` (version 1.5.3). |
| 4.1–4.3 | Live record: `last_check.result = pass`, details registration/handshake/tools_list/status_tool/agent_command/hook_sample all `pass`, config_load skipped; `agy mcp list` prints `mmhmem  stdio  enabled  C:\Users\<user>\AppData\Local\Programs\mmhmem\mmhmem.exe stdio`. |
| 4.4 | `tests/setup_verify.rs` rollback tests (both Antigravity files restored). |
| 5.1, 5.2 | Owner's `mmhmem update`: "- Remove mmhmem from Antigravity (changes …\antigravity-mcp.json)" and "- Antigravity no longer reads this file; remove the mmhmem entry (~\.gemini\antigravity\mcp_config.json)"; afterwards no `antigravity-mcp.json` next to the binary and the stale file is `{"mcpServers": {}}`. |
| 5.3 | `setup_cli_antigravity` removal scenario (both entries out, other servers/hooks byte-identical). |
| 5.4 | Live record: `features {mcp: true, read_hook: true}`, two `json_entry` managed entries, `pending_user_steps: []`, release `v0.5.3`. |
| 6.1–6.3 | README, `docs/installation.md`, `tests/install_docs.rs`; research.md of this spec and the Antigravity row in reliable-installers research §8. |

### Live run detail (owner's Windows 11 machine, agy 1.5.3)

Pre-state (16:17): mmhmem 0.5.2, record snippet-era with the 0.5.1 snippet
file managed and `import_antigravity_snippet` pending, both `config/`
files empty, stale pasted entry present, `agy mcp list` empty.

Owner ran `mmhmem update` (→ v0.5.3, three Antigravity changes, backups to
`~\AppData\Local\mmhmem\backups\2026-11-10T15-27-06Z`, "✔ Antigravity —
mmhmem connected, 11 tools"), then `mmhmem setup` → "Change agents and
features" to switch the read hook on (the update keeps recorded choices).

Verification 16:30 (`scratchpad/live-5.3/verify.sh`): see the table above;
raw log `02-verify-163048.txt` in the session scratchpad.

Note for the headless `agy -p` whole-file run: after the hook's deny the
agent tried a `run_command` fallback, which headless mode auto-denies; the
redirect itself is in the audit log. The ranged read in the same mode
proceeded, so the `ask` answer does not block reads in print mode.

### Open

- Interactive `ask` behaviour: the owner reported reading a small file in an
  interactive session with no new permission prompt. The pasted transcript
  looks like Claude Code's display, so it must be confirmed that the session
  was `agy`. If it was, the last open research item is closed.
- Not exercised live: `agy mcp disable mmhmem` display and the entry left
  as `{"disabled": false}` after a removal over an `agy mcp add`-written
  entry (3.2/3.5 review suggestions). Both are informational for a later
  spec decision and would modify the owner's config, so they were skipped.
- The `~/` hook command form and 8.3 paths with `~` stay unverified; the
  owner's path has no spaces, so the absolute-path form is what runs.

### Cross-task integration

- Dependency direction holds: `setup` reaches the hook only through the
  public adapter surface; `src/hook/antigravity.rs` imports nothing from
  `setup`.
- Record shape matches design "Data Models" exactly on the live machine.
- The only out-of-boundary code change in the feature is the small
  `Agent::feature_problems` / review-screen `left_out` extension (3.3),
  accepted by its reviewer and recorded in the task notes.
