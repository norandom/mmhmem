# Implementation Plan

All tasks are required. Each executable task is sized for approximately 1–3
hours. Order: confirm the live Antigravity contract and capture fixtures,
then the hook adapter, then the setup agent, then text and flow, then
integration tests, documentation and the live validation on the owner's
machine. Every commit must pass the project pre-commit gate (fmt, size caps,
clippy with `-D warnings`). The install quality bar applies: only a
CI-verified install counts as done.

- [ ] 1. Confirm the Antigravity contract and capture fixtures
- [x] 1.1 Capture the live `agy` 1.5.3 facts
  - Outcome 2026-11-09: the file-read tool is `read_file` (args `Path`, `StartLine`, `EndLine`), not `view_file`; `{}` on stdout denies and `{"decision":"ask"}` passes (print mode); quotes in the command are kept literally, so the design moved to an unquoted absolute path with a Windows short-path fallback; two items stay unverified and are gated in task 5.3 (interactive `ask` prompts, tilde expansion order). Reviewer approved in round 2.
  - Record on the owner's Windows machine: `agy --version` output; `agy mcp list` with no servers and with mmhmem registered in `~/.gemini/config/mcp_config.json` (register by hand for the capture, then remove); the exact `PreToolUse` stdin Antigravity sends for a whole-file `view_file`, a ranged `view_file` and a `run_command` (a throwaway hook that writes stdin to a file); whether `{}` on stdout is accepted as "no opinion" or `{"decision":"ask"}` is needed; that `cmd /c "<abs path>" hook antigravity` runs with stdin attached.
  - Save the outputs as fixtures under `tests/fixtures/setup/agents/antigravity/` and `tests/fixtures/hook/antigravity/`, redacting conversation ids and user paths.
  - Update `research.md` of this spec (Pass output decision, `view_file` argument keys) and replace the Antigravity row of `.kiro/specs/reliable-installers/research.md` section 8 with the confirmed facts and a pointer here.
  - Done when the fixtures are committed, both research files state the confirmed keys and pass form with the version and date, and no open question remains in this spec's research.
  - _Requirements: 6.3_

- [ ] 2. Hook adapter for Antigravity
- [x] 2.1 Parse and render Antigravity `PreToolUse`
  - Add the Antigravity harness variant with the command-line name `antigravity`, its stable name and `agent_harness` value, and the parse and render branches.
  - Parse the captured payload shape: session from the conversation id, working folder from the first workspace path, the `read_file` tool only; `Path` (either slash style) and optional `StartLine`/`EndLine` from the arguments; anything else (including `call_mcp_tool` and `run_command`) or malformed is not a read.
  - Render a redirect as a deny with a reason that names the `smart_read` call; render a pass as `{"decision":"ask"}`; never auto-allow; the command always exits 0 and always prints one JSON object, because empty or invalid output blocks Antigravity's tool call.
  - Done when unit tests on the captured fixtures show whole-file → redirect request, ranged → ranged request, other tool → pass, oversized input → pass, and the hook CLI test runs `mmhmem hook antigravity` against a temp project at and below the threshold with the expected JSON on stdout and exit code 0 in every case.
  - _Boundary: Antigravity hook adapter_
  - _Requirements: 2.2, 2.3, 2.4_

- [x] 2.2 (P) Antigravity hook sample for the functional check
  - Build the Antigravity-shaped sample read the check feeds the hook and expect the deny or pass form from task 2.1.
  - Done when the check-hook unit test runs the sample for the Antigravity harness and reports pass, and a hook that prints nothing within the timeout is reported as not run.
  - _Boundary: Hook sample_
  - _Depends: 2.1_
  - _Requirements: 4.2_

- [ ] 3. Setup agent for Antigravity
- [x] 3.1 Detection by program and data folders
  - Detect Antigravity when the `agy` program is on PATH or any of the CLI, app or IDE data folders under the Gemini home exist; the bare Gemini home no longer counts.
  - Show the first line of `agy --version` as the version when the program is found.
  - Done when tests show: program found with version; folders only without version; Gemini home with only `settings.json` not found.
  - _Boundary: Antigravity agent_
  - _Requirements: 3.1, 3.2, 3.3_

- [x] 3.2 (P) MCP registration in Antigravity's global server file
  - Plan the managed server entry (command and arguments) in the global Antigravity server file, creating the file with the needed parents when missing, keeping user keys inside the entry and everything else byte-identical.
  - Plan removal that takes out only the managed keys and the parents this install created.
  - Done when golden tests on a fixture with other servers, comments and odd formatting show add followed by remove is byte-identical, a missing file is created with the parent count recorded, and re-planning an unchanged, recorded entry yields no change.
  - _Boundary: Antigravity agent_
  - _Requirements: 1.1, 1.2, 1.3, 5.3_

- [ ] 3.3 (P) Read-routing hook in Antigravity's global hook file
  - Plan the whole named mmhmem hook (pre-tool-use group matching exactly the `read_file` tool, one command handler with a 10-second timeout) in the global Antigravity hook file, creating the file when missing.
  - Build the command as unquoted whitespace-free tokens: the binary's absolute path followed by `hook antigravity`; when the path contains whitespace, use the Windows 8.3 short form if the OS returns a whitespace-free one, otherwise report the agent problem for the hook feature and still plan the MCP entry (Antigravity keeps quotes literally and a failing hook blocks every tool).
  - Plan removal of that named hook only.
  - Done when golden tests on a fixture with another user hook show only the mmhmem key is added and removed, the file stays byte-identical otherwise, the command is the unquoted absolute path followed by `hook antigravity`, a Windows path with a space becomes its short form in a test using a temp folder with a space, and a path that cannot be shortened yields the problem without a hook change.
  - _Boundary: Antigravity agent_
  - _Depends: 2.1_
  - _Requirements: 2.1, 2.5_

- [ ] 3.4 Migration from the snippet-era install and stale-location cleanup
  - When the record manages the old snippet file next to the binary, plan its removal; stop emitting the import step and drop it from records on save while still reading old records.
  - When the no-longer-read Antigravity data-folder server file holds a mmhmem entry whose command is the mmhmem program, plan the removal of only that entry with the one-line explanation; ignore a missing, unreadable or foreign entry.
  - Done when tests show: a snippet-era record yields the two new entries plus the snippet removal; an old record with the import step saves without it; a stale file with a mmhmem entry yields exactly one removal change and other servers in it are untouched; a stale file whose `mmhmem` entry points elsewhere yields nothing.
  - _Boundary: Antigravity agent, Antigravity check_
  - _Depends: 3.2, 3.3_
  - _Requirements: 5.1, 5.2, 5.4_

- [ ] 3.5 Functional check from Antigravity's own file
  - Read the registration back from the global server file bytes, run the standard steps with that command, run the hook sample when the hook feature is on, and confirm with the Antigravity CLI's server listing when the program is present (skipped with a reason otherwise).
  - Remove the plan-derived check and the snippet constants from the agent.
  - Keep the existing "hook installed but inactive until a worker model is set up" line for Antigravity when its hook feature is on and no worker is configured.
  - Done when tests with a fake `agy` show `agent_command` pass with the populated fixture and skipped without the program, a server file naming a wrong command fails the registration step with the existing reason text, and the inactive-hook line appears for Antigravity without a worker.
  - _Boundary: Antigravity check_
  - _Depends: 2.2, 3.2_
  - _Requirements: 2.6, 4.1, 4.2, 4.3_

- [ ] 4. Text and flow
- [ ] 4.1 Remove the snippet from screens, results and status
  - Replace the Antigravity next step with "start a new session", delete the snippet and import texts and the Antigravity special case in the apply flow, and remove the import pending-step text while tolerating the deprecated step in old records.
  - Refresh the affected UI snapshots.
  - Done when `grep -ri snippet src/setup` finds no Antigravity reference, the snapshot tests pass, and the results screen for Antigravity shows the generic next step.
  - _Boundary: Text and flow_
  - _Requirements: 1.4, 1.5, 6.2_

- [ ] 5. Validation
- [ ] 5.1 Integration tests for install, update and removal
  - Setup CLI test from a clean test home with a fake `agy`: both entries written, check passes, record lists both managed entries and no pending step.
  - Update test from a snippet-era record plus a stale data-folder server file: new entries present, snippet file and stale entry gone, record has no import step.
  - Removal test: both entries gone, other servers and hooks byte-identical, check reports nothing left to undo. Rollback test: a failing handshake restores the original bytes of both files.
  - Done when all four scenarios pass in `cargo test` on Linux and Windows CI.
  - _Depends: 3.4, 3.5, 4.1_
  - _Requirements: 1.1, 1.2, 2.1, 2.5, 4.1, 4.3, 4.4, 5.1, 5.2, 5.3, 5.4_

- [ ] 5.2 (P) Installer end-to-end scenario and documentation
  - Add the Antigravity scenario (fake `agy` on PATH) to the installer e2e workflow on the Windows and Linux runners.
  - Update `README.md`, `docs/installation.md` and the Antigravity example under `deploy/examples/` so they say setup configures Antigravity itself and the user starts a new session; remove the "no command line" sentence.
  - Done when the e2e workflow is green with the new scenario and the install-docs test passes against the updated text.
  - _Depends: 5.1_
  - _Requirements: 1.5, 6.1, 6.2_

- [ ] 5.3 Live validation on the owner's machine
  - Build the release artefacts locally and run `mmhmem setup` against the existing install on the owner's Windows machine (the snippet-era record), accept the review screen, and record the outcome.
  - In an interactive `agy` session (owner present), confirm that a `read_file` of a short file answered with `{"decision":"ask"}` raises no permission prompt that was not there without the hook; if it does, stop and record the finding for an explicit decision on the pass form.
  - Done when `agy mcp list` names mmhmem, a new `agy` session lists the mmhmem tools, a whole-file `read_file` of a file above the threshold is answered with the redirect while a ranged read proceeds, the interactive prompt check passed, `install.json` matches the data model in design.md, and the evidence (commands and output) is recorded in `validation.md` of this spec.
  - _Depends: 5.2_
  - _Requirements: 1.1, 1.5, 2.1, 2.2, 4.3, 5.1, 5.2, 5.4_

## Implementation Notes

- 2026-11-09 (task 1.1): the auto-mode classifier refused to spawn a subagent whose prompt drove `agy` with permission prompts skipped and installed a global hook; the controller ran the capture itself. A global `~/.gemini/config/hooks.json` fires in every Antigravity session on the machine, so live hook experiments must be short and announced to the owner.
- 2026-11-09 (task 2.1): `src/lib.rs` `--help` text still lists the hook harnesses as "(claude, codex, opencode)"; task 4.1 (text and flow) must add `antigravity`.
- 2026-11-09 (task 2.1): the build host is tight on memory; use `cargo nextest run --build-jobs 2`. `setup::running::tests::a_running_copy_is_seen_and_this_process_is_not` is flaky under parallel nextest (passes with `-j 1`) and is unrelated to this spec.
- 2026-11-09 (task 2.1): the pre-commit gate fails when `git commit` is run from the Git Bash tool (the installer stub test cannot invoke pwsh with `-File`); commit from native PowerShell.
- 2026-11-09 (task 2.1 review): the hook repeat-window key is the path as sent; Antigravity sends `Path` in both slash styles, so a repeat in the other style is redirected again instead of passing. Non-blocking; normalise separators in the key if it shows up in task 5.3.
- 2026-11-09 (task 2.2): `Steps::hook` in `check_agent.rs` skips the hook sample when no worker model is configured. For Antigravity a broken hook blocks every `read_file`, so task 3.5 must run the sample even without a worker (expecting `{"decision":"ask"}`), either by calling `hook_sample` directly or by extending `Steps` with an opt-in. The sample uses the check's fresh session id (not a literal) so re-runs never hit the hook's 2-minute repeat window.
- 2026-11-09 (task 3.2 review): `plan` currently removes everything recorded when Mcp is deselected; once ReadHook exists (3.3), map each recorded entry to its feature as `claude_desktop.rs` does, so deselecting one feature removes only its entries.
- 2026-11-09 (task 3.2 review): removing mmhmem over an entry written by `agy mcp add` leaves `"mmhmem": {"disabled": false}` (user keys stay). Task 5.3 must check how `agy mcp list` and the IDE show an entry without a command and raise a spec decision if it shows as broken.
- 2026-11-09 (task 3.2 review): `remove_entry` only removes the old snippet file when it sits next to the current binary; task 3.4 should match the recorded `Target::File` by file name `antigravity-mcp.json` so a moved binary still migrates. `REMOVE_INSTRUCTION`/`removal_instruction()` are now dead apart from tests; drop in 4.1. antigravity.rs is at 331/500 lines and its tests at 436/500: 3.3/3.4 should split into `antigravity_check.rs` + tests as the design plans.
