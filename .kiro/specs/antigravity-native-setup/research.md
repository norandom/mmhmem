# Research & Design Decisions

## Summary
- **Feature**: `antigravity-native-setup`
- **Discovery Scope**: Extension (corrects one agent integration inside the existing `mmhmem setup` architecture)
- **Key Findings**:
  - Antigravity 1.5.3 (CLI `agy` and IDE) loads global MCP servers from `~/.gemini/config/mcp_config.json` and global hooks from `~/.gemini/config/hooks.json`. The `~/.gemini/antigravity/` folder is data, not config.
  - Antigravity has a CLI (`agy mcp add|remove|list|enable|disable`, `agy plugin ...`, `agy --version`). The reliable-installers claim "no CLI or documented config file" is stale.
  - Antigravity hooks are `PreToolUse` groups keyed by a hook name, with a regex matcher on Antigravity tool names (`read_file`, `run_command`, `call_mcp_tool`, ...), camelCase JSON on stdin and a `decision` object on stdout. The shape differs from Claude Code and Codex, so a new hook adapter is needed.

## Research Log

### Where Antigravity reads MCP servers
- **Context**: The user pasted mmhmem into `~/.gemini/antigravity/mcp_config.json` on 2026-11-08 and `agy mcp list` still said "No MCP servers configured".
- **Sources Consulted**: installed docs `~/.gemini/antigravity-cli/builtin/skills/agy-customizations/docs/mcp_servers.md` (ships with `agy` 1.5.3); https://antigravity.google/docs/mcp; `agy mcp list` on this machine; file listing of `~/.gemini/` (2026-11-09).
- **Findings**:
  - Global: `~/.gemini/config/mcp_config.json`, root `{"mcpServers": {"<name>": {...}}}`; stdio entries use `command`, `args`, `env` (optional `cwd`). Same leaf shape as `claude_desktop_config.json`.
  - Workspace: `<workspace>/.agents/mcp_config.json`. Plugin: `<plugin>/mcp_config.json`.
  - On this machine `~/.gemini/config/mcp_config.json` is `{"mcpServers": {}}` (empty) and `~/.gemini/antigravity/mcp_config.json` holds the pasted mmhmem entry. The latter is not named by any current doc.
  - Tool-schema caches live in `~/.gemini/antigravity/mcp/<server>/` (app, IDE) and `~/.gemini/antigravity-cli/mcp/<server>/` (CLI); they are not configuration.
- **Implications**: Setup edits `~/.gemini/config/mcp_config.json` as a managed JSON entry, exactly as it does for Claude Desktop. The old location is detected only for stale-entry cleanup (Req 5.2).

### Antigravity CLI surface
- **Context**: reliable-installers research.md section 8 says Antigravity has no CLI.
- **Sources Consulted**: `agy --help`, `agy mcp --help`, `agy mcp add --help`, `agy plugin --help`, `agy --version` on Windows 11 (2026-11-09); https://antigravity.google/docs/cli/reference/.
- **Findings**:
  - `agy --version` prints `1.5.3`. The program is `agy.exe` in `%LOCALAPPDATA%\agy\bin` (on PATH). There is no program named `antigravity` on PATH.
  - `agy mcp add [--env K=V] [--type stdio|http] <name> <commandOrUrl> [args...]` adds or updates; `agy mcp remove <name>`; `agy mcp list` prints `No MCP servers configured.` when empty and a `NAME TYPE STATUS COMMAND/URL` table otherwise (both captured 2026-11-09 in `tests/fixtures/setup/agents/antigravity/`); `enable`/`disable`.
  - `agy plugin install <path|url|name@marketplace>`, `list [--json]`, `import [gemini|claude]`, `validate`.
  - The public CLI reference page does not list `agy mcp`; the installed binary has it. Treat the binary as authoritative and the subcommand as confirmation-only (Req 4.3), never as the only write path.
- **Implications**: Detection keys on `agy` (Req 3.1) and reads the version from `agy --version` (Req 3.3). The check uses `agy mcp list` to confirm visibility when present.

### Antigravity hooks contract
- **Context**: Req 2 needs a `PreToolUse` hook for file reads.
- **Sources Consulted**: installed docs `.../agy-customizations/docs/hooks.md`; https://antigravity.google/docs/hooks.
- **Findings**:
  - File: `hooks.json` in a customization root. Global: `~/.gemini/config/hooks.json`. Hooks placed in `~/.gemini/antigravity-cli/settings.json` are not loaded.
  - Format: top-level map of **hook name** to spec. Spec: optional `enabled`, and event keys `SessionStart`, `PreToolUse`, `PostToolUse`, `PreInvocation`, `PostInvocation`, `Stop`. `PreToolUse` is grouped: `[{ "matcher": "<regex>", "hooks": [ { "type": "command", "command": "<shell string>", "timeout": <s> } ] }]`.
  - Matcher is a regex on the tool name. The docs' examples (`view_file`, `run_command`, `write_to_file`, `grep_search`) are illustrative; the live names are below. Tool names come from the step type with `CORTEX_STEP_TYPE_` stripped and lowercased.
  - The command is one string run via `cmd /c` on Windows and `sh -c` elsewhere, working directory = the folder holding `hooks.json`; `~` is expanded. Default timeout 30 s.
  - Stdin: `{"conversationId", "workspacePaths": [...], "transcriptPath", "artifactDirectoryPath", "modelName", "lastUserInput", "toolCall": {"name", "args": {...}}, "stepIdx"}` (camelCase).
  - Stdout for `PreToolUse`: `{"decision": "allow"|"deny"|"ask"|"force_ask", "reason"?, "permissionOverrides"?, "overwrite"?}`. `deny` is a hard block with `reason` shown to user and agent.
- **Confirmed 2026-11-09 (task 1.1, live capture with agy 1.5.3, fixtures in `tests/fixtures/hook/antigravity/`)**:
  - The file-read tool is named **`read_file`**, not `view_file` (the docs' example names are illustrative). Arguments: `Path` (absolute; backslashes in most calls, forward slashes in one), and for a ranged read `StartLine` and `EndLine` as integers. `run_command` carries `CommandLine`, `WaitMsBeforeAsync`, `toolSummary`. MCP tool calls arrive as `call_mcp_tool` with `ServerName`, `ToolName`, `Arguments`, so the matcher must be exactly `^read_file$` or the hook would deny `smart_read` itself.
  - `workspacePaths[0]` is the workspace folder (forward slashes); `conversationId` identifies the session; `modelName`, `stepIdx`, `lastUserInput`, `transcriptPath`, `artifactDirectoryPath` are also present.
  - **`{}` on stdout is a denial**: the agent saw "tool call denied by pre-tool hook:" with an empty reason. `{"decision":"ask"}` lets a `read_file` proceed in print mode (answer came back correctly). So the pass form is `{"decision":"ask"}`; `allow` is never used.
  - A deny with a reason reaches the agent as `Encountered error in tool execution: tool call denied by pre-tool hook: <reason>`.
  - **A hook that exits non-zero or prints invalid JSON blocks the tool call** with an error naming the hook (observed: `JSON hook "jsonhook__<name>_PreToolUse_0_0" failed: command failed: exit status 64 ...`, and `failed to unmarshal result ... via protojson`). A timeout was not observed but is assumed to behave the same. The hook must therefore always exit 0 with valid JSON, even on internal errors.
  - **Command tokenization on Windows**: the command string is split on whitespace and quotes are passed literally (a quoted path reached PowerShell as `'"D:/…/capture.ps1"'` and failed; an `echo {"decision":"ask"}` command produced stdout with the quotes backslash-escaped, `{\"decision\":\"ask\"}`, which protojson rejected at position 1:2). Quoting cannot be used to protect spaces. `~` was expanded in the first token (`~/agy-capture-tmp/capture.cmd` ran) and not in a later one (`-File ~/…/capture.ps1` failed). Whether that expansion happens before or after the split is **unverified** (the capture home has no space), so the `~/` form is not relied on. The docs' "`~` is expanded" and "run via `cmd /c`" are only partly right.
  - Named hooks for one event ran in sequence in the capture, and a failing one blocked the tool (inferred from runs 0, 5 and 6). The global `hooks.json` applied to a second, concurrent CLI session on the machine (observed); the docs say the IDE reads the same file.
  - `agy mcp add mmhmem <bin> stdio` writes `~/.gemini/config/mcp_config.json` with the keys `args`, `command`, `disabled: false` (alphabetical in the capture); `agy mcp list` prints a space-aligned table (`NAME TYPE STATUS COMMAND/URL`); `agy mcp remove` left `{"mcpServers": {}}`. The agent addressed the mmhmem server by name (`call_mcp_tool` with `ServerName: mmhmem`, `ToolName: status` and `smart_read`), which shows the registration was loaded; those calls were denied by the capture hook, so a completed tool round-trip was not observed.
- **Implications**: A new `Harness::Antigravity` adapter: parse `toolCall.name == "read_file"`, `Path` (either slash style) and optional `StartLine`/`EndLine` from `toolCall.args`, `cwd` from `workspacePaths[0]`, session from `conversationId`; render redirect as `{"decision":"deny","reason": <redirect text>}` and pass as `{"decision":"ask"}`; any internal error renders the pass form and exits 0. Setup owns one whole named hook (`mmhmem-read-routing`) in `~/.gemini/config/hooks.json` with matcher `^read_file$`. The hook command is written as unquoted whitespace-free tokens: the binary's absolute path; when that path contains whitespace, Windows' 8.3 short form is used if it is whitespace-free, otherwise setup reports an agent problem for the hook feature instead of writing a hook that would block every read.

### Detection folders on this machine
- **Context**: Req 3.
- **Sources Consulted**: `ls ~/.gemini`, `%LOCALAPPDATA%`, `%APPDATA%` (2026-11-09).
- **Findings**: `~/.gemini/antigravity-cli/` (CLI data, settings.json, cli.log), `~/.gemini/antigravity/` (app data), `~/.gemini/antigravity-ide/` (IDE data), `~/.gemini/config/` (shared config: `config.json`, `mcp_config.json`, `plugins/`). `~/.gemini/GEMINI.md` can be written by other tools (claude-mem did here), so `~/.gemini` alone proves nothing. `%LOCALAPPDATA%\Antigravity` and `%APPDATA%\Antigravity` hold the Electron app.
- **Implications**: Detection = `agy` on PATH, or any of `~/.gemini/antigravity-cli`, `~/.gemini/antigravity`, `~/.gemini/antigravity-ide` is a directory.

### Existing install on the owner's machine
- **Context**: Req 5.
- **Sources Consulted**: `%LOCALAPPDATA%\mmhmem\install.json` (2026-11-09).
- **Findings** (also confirmed 2026-11-09 that the pasted entry in `~/.gemini/antigravity/mcp_config.json` is invisible to `agy mcp list`): `agents.antigravity` has `features: {mcp: true}`, `managed: []` (the snippet file was never recorded as managed because the owner's install predates the 0.5.1 fix), `pending_user_steps: ["import_antigravity_snippet"]`, `last_check.result: user_steps_pending` with registration/handshake/tools_list/status_tool "pass" (read from the snippet, not from Antigravity). Other installs (0.5.1+) may have a `Target::File` entry for `antigravity-mcp.json`.
- **Implications**: Migration must handle both shapes: a record with the snippet file managed, and one with nothing managed. The `import_antigravity_snippet` step must still deserialize from old records and then be dropped.

## Architecture Pattern Evaluation

| Option | Description | Strengths | Risks / Limitations | Notes |
|--------|-------------|-----------|---------------------|-------|
| A. Edit `~/.gemini/config/mcp_config.json` and `hooks.json` directly (managed JSON entries) | Same mechanism as Claude Desktop and Codex `hooks.json` | Works for IDE-only installs without `agy`; byte-preserving edits, backups, drift detection and removal come free from `edit::json` and the record | Depends on the documented path staying stable | **Selected** |
| B. Shell out to `agy mcp add` / `agy mcp remove` | Let Antigravity write its own file | Immune to path changes | Fails for IDE-only machines; no hook equivalent; writes outside mmhmem's backup/drift tracking; `agy mcp` is undocumented on the public reference page | Used read-only (`agy mcp list`) in the check |
| C. Ship everything as the `unified-memory` plugin (`mcp_config.json` + `hooks.json` at plugin root) | One folder under `~/.gemini/config/plugins/` | Single unit, namespaced | Hook command needs the absolute binary path baked in at install time; plugin lifecycle (enable/disable in `config.json`) is a second state machine; marketplace distribution is a separate concern | Out of boundary; revisit in a plugin spec |

## Design Decisions

### Decision: Direct file edits, CLI only for confirmation
- **Context**: Req 1, 4.3, 5.
- **Alternatives Considered**: A, B, C above.
- **Selected Approach**: A. The check additionally runs `agy mcp list` when `agy` is present and looks for the mmhmem line.
- **Rationale**: Reuses the proven safe-write foundation; identical semantics to Claude Desktop; works for the IDE without the CLI.
- **Trade-offs**: If Antigravity moves the global path again, detection of the stale location must be extended; mitigated by Req 5.2 pattern and the research record (Req 6.3).
- **Follow-up**: done 2026-11-09 (task 1.1): `tests/fixtures/setup/agents/antigravity/agy-mcp-list-mmhmem.txt`.

### Decision: One whole named hook, not a shared array group
- **Context**: Req 2.5. Claude Code and Codex share `hooks.PreToolUse` arrays with the user, which needs `hook_entry.rs` merging.
- **Selected Approach**: mmhmem owns the top-level key `mmhmem-read-routing` in `~/.gemini/config/hooks.json` as a `Target::JsonEntry` with pointer `["mmhmem-read-routing"]`. Other hook names are never touched.
- **Rationale**: Antigravity's format gives each hook its own name, so ownership is exact without matchers.
- **Trade-offs**: A user who renames the key loses tracking; drift detection reports the entry as missing and re-plan re-adds it.

### Decision: Hook command as unquoted absolute path, short form for whitespace
- **Context**: Antigravity splits the command string on whitespace and keeps quotes literally (confirmed 2026-11-09, task 1.1), so the Codex-style double-quoted path breaks the hook and, because a failing hook blocks every tool call, would make Antigravity unusable.
- **Alternatives Considered**:
  1. Double-quoted absolute path (Codex style) — fails on this build.
  2. `~/` relative program token — `~` was expanded in the first token, but whether expansion happens before or after the whitespace split is unverified; if before, every user whose home folder has a space would get a hook that blocks all tools. Rejected until verified.
  3. Absolute path, and the Windows 8.3 short form when the path has whitespace — testable without Antigravity; 8.3 names are on by default on the system volume, and their absence is detectable.
- **Selected Approach (revised 2026-11-09 after review)**: 3. The command is `<absolute path> hook antigravity`. On Windows a path with whitespace is replaced by `GetShortPathNameW`'s result when that is whitespace-free; otherwise, and on Unix with whitespace, setup yields `AgentProblem::HookCommandUnrepresentable` for the hook feature (clear message, MCP still installed). Timeout 10 s, matcher `^read_file$`.
- **Trade-offs**: Users on volumes with 8.3 names disabled and a space in their path get no hook; they are told why. The `~/` form may be adopted later once verified with a home folder containing a space (Risks).

### Decision: Pass output
- **Context**: Docs mark `decision` as required but `allow` would bypass permission prompts, which the hook must not change.
- **Selected Approach (confirmed 2026-11-09, task 1.1, in print mode)**: `{"decision": "ask"}` on pass. `{}` was observed to deny the call, so it is never emitted. `allow` is never emitted. Every internal error path (bad input, no worker, serialization) also renders `{"decision": "ask"}` and exits 0, because a non-zero exit or invalid JSON blocks the tool.
- **Open (Risks)**: the installed hooks.md defines `ask` as "Prompt the user for permission (respects Always Allow cache)". In `agy -p` the read proceeded without a prompt, and a concurrent interactive CLI session kept working while receiving `ask`, but it was not confirmed that no extra prompt appeared. Task 5.3 verifies this interactively; if `ask` adds a prompt to every `read_file`, the only remaining non-denying form is `allow`, and that choice (auto-approving a read that Antigravity already runs without prompting) must be decided explicitly before the hook ships.

### Decision: Stale-location cleanup is a planned change, not a prompt
- **Context**: Req 5.2; the review screen already is the confirmation step.
- **Selected Approach**: When `~/.gemini/antigravity/mcp_config.json` has `mcpServers.mmhmem` whose `command` file name is `mmhmem` or `mmhmem.exe`, plan a `Remove` of that entry (JSON entry removal, parents pruned only if mmhmem created them, which it never did, so none) with the summary "Antigravity no longer reads this file; remove the mmhmem entry". It is not recorded as managed afterwards.

## Risks & Mitigations
- ~~`view_file` argument names differ from the guess~~ — resolved 2026-11-09: the tool is `read_file` with `Path`, `StartLine`, `EndLine`; fixtures captured.
- ~~Antigravity rejects `{}` on pass~~ — resolved 2026-11-09: `{}` denies; the pass form is `{"decision":"ask"}`.
- A hook failure blocks every Antigravity tool call — mitigated by: exit 0 with valid JSON on every path, the `^read_file$` matcher, the unquoted whitespace-free command form, and the functional check running the hook sample before the install is reported as passed.
- **Unverified: `~` expansion order.** The `~/` program form worked for a home without spaces; whether a home folder with a space survives the whitespace split is unknown, so the form is not used. Verify with a test account whose profile path has a space before adopting it.
- **Unverified: `ask` in interactive sessions.** `{"decision":"ask"}` passed reads in print mode only. If it adds a permission prompt interactively, the pass form must change (see Decision: Pass output). Checked in task 5.3.
- The capture's global hook intercepted the owner's concurrent Antigravity CLI session (denied calls during run 4, failing hooks during runs 0 and 5); the owner was told during the session. `~/.gemini/config/hooks.json` did not exist before the capture and now holds `{}` at the owner's request, so setup will see a pre-existing file there on this machine.
- IDE and CLI diverge on the global path in a future release — mitigated by Req 6.3 and the stale-location cleanup pattern.
- File-size and function-size caps (500 lines per file, 80 per function, pre-commit gate) — the Antigravity agent is split into `antigravity.rs` (agent, detection, plan) and `antigravity_check.rs` (check, migration helpers) like Codex.

## References
- https://antigravity.google/docs/mcp — global and workspace MCP config paths and schema
- https://antigravity.google/docs/hooks — hooks.json format, events, I/O contract
- https://antigravity.google/docs/plugins — plugin layout (hooks.json and mcp_config.json at plugin root)
- https://antigravity.google/docs/cli/reference/ — CLI reference (does not list `agy mcp`)
- Installed docs: `~/.gemini/antigravity-cli/builtin/skills/agy-customizations/docs/{hooks,mcp_servers,plugins,json_configs}.md` (agy 1.5.3)
- `.kiro/specs/reliable-installers/research.md` section 8 — the stale Antigravity row this spec supersedes
- `.kiro/specs/read-routing-hooks/design.md` — adapter contract the new harness variant follows
