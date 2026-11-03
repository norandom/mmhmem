# Research: read-routing-hooks

Collected 2026-11-03. Adoption data from the user's local store and audit log;
Claude Code facts from the official docs (hooks reference, hooks guide,
permissions, tools reference at code.claude.com), gathered by a docs
research agent; Codex facts from the user's `~/.codex/config.toml` and public
Codex documentation excerpts.

## Adoption evidence

- `token_stats` holds three rows in total: codex `shunt_delegated` (4,030
  avoided est.), opencode `shunt_cache_hit` (3,999) and `shunt_delegated`
  (3,719).
- Audit events by day and harness: see `brief.md`. Claude Code on
  2026-11-03: 0 summary reads, 1 direct read, 40 resumes, 37 checkpoints.
- Exact slices (`IO_TARGETED_READ`) and direct reads (`IO_DIRECT_READ`) record
  audit events but no `token_stats` rows, which is why `mmhmem stats` cannot
  show them today.

## Shunt engine rules the hook must mirror

From `src/services/shunt.rs` (via `smart_read`): targeted when offset and
limit are set; direct when the path matches
`(?i)(auth|crypto|secret|token|rbac|jwt|tls|cert|mutex|lock|concurren|permission|sudo|pam)`
or the file has fewer than `shunt_min_lines` lines (default 150,
`SHUNT_MIN_LINES`); otherwise shunted, and shunting fails with
`WorkerNotConfigured` when no worker is set.

## Claude Code hook contract

- **Configuration:** `hooks.PreToolUse` holds matcher groups; a command
  handler takes `command`, `args`, `timeout`, `if`, `shell`. Matchers: exact
  names (`Read`), alternation (`Read|Bash`), or an unanchored regex.
- **Input (stdin JSON):** `session_id`, `transcript_path`, `cwd`,
  `permission_mode`, `hook_event_name`, `tool_name`, `tool_input`,
  `tool_use_id`. Read's `tool_input` has an absolute `file_path` (expanded
  before hooks run) and optional `offset` and `limit`.
- **Decisions:** exit 2 blocks with stderr as the reason, or JSON
  `hookSpecificOutput` with `permissionDecision` (`allow`, `deny`, `ask`,
  `defer`), `permissionDecisionReason` (shown to Claude for `deny`), and
  `additionalContext`. A hook cannot rewrite a Read into an MCP call; `deny`
  with a reason pointing to `smart_read` is the documented route.
- **Failure:** exit codes other than 0 and 2 proceed on PreToolUse (fail
  open). The default command timeout is 600 s; timeout behaviour on
  PreToolUse is not stated explicitly, so the hook sets a short `timeout`
  and the behaviour must be tested.
- **Read before Edit:** Edit requires a read in the current conversation; a
  read cut short by the PARTIAL notice does not count. Whether a deliberate
  offset/limit read counts, and whether an MCP read counts, is not
  documented. This must be tested: a redirect must not make Edit fail.
- **Locations and merging:** user `~/.claude/settings.json`, project
  `.claude/settings.json` and `.claude/settings.local.json`, managed
  settings, plugins. Entries merge across levels; identical handlers run
  once; matching hooks run in parallel; decision precedence is `deny` >
  `defer` > `ask` > `allow`.
- **Windows:** shell-form commands run in Git Bash, or PowerShell without
  it. Exec form (`command` plus `args`) passes arguments verbatim, so an
  absolute `.exe` path needs no quoting.
- **Repeat detection:** `session_id` and `tool_use_id` are provided; dedupe
  must be built by the hook (state keyed on session, tool and input hash,
  stored outside the project).
- **Trust and approval:** interactive sessions hold back hooks from every
  settings file until the workspace trust dialog for the folder is
  accepted; the dialog lists the hooks the settings would activate. `-p`
  and SDK sessions treat the folder as trusted.
- **Policy:** `disableAllHooks` exists as a settings key; managed settings
  can restrict permissions. No documented installer API to test policy, so
  the installer reads the settings files it can see.
- **Security guidance:** hooks run with the user's full permissions;
  validate input, quote, use absolute paths, skip sensitive files. Parse
  stdin JSON with a real parser, never shell string handling.

## Codex hook model (observed)

- `~/.codex/config.toml` has `[features] hooks = true` and per-hook entries
  `[hooks.state."<plugin>:hooks/codex-hooks.json:pre_tool_use:0:0"]` with
  `trusted_hash`, `trusted_content_hash` and `enabled`, plus a
  `~/.codex/.hook-trust` directory. Codex approves each hook by content
  hash, so a changed hook entry needs approval again.
- The claude-mem plugin already installs Codex hooks for `pre_tool_use`,
  `post_tool_use`, `post_compact`, `session_start`, `user_prompt_submit`,
  `stop` and `subagent_start`. The mmhmem hook must coexist with it.
- Public excerpts say Codex loads hooks from `hooks.json` or inline
  `[hooks]` tables in `config.toml`, user-level and trusted project-level
  `.codex/` directories; a community guide says Codex PreToolUse supports
  `deny` but not `allow` or `ask`
  ([Codex advanced config](https://developers.openai.com/codex/config-advanced.md),
  [community guide](https://codex.danielvaughan.com/2026/04/15/codex-cli-hooks-complete-guide-events-policy-patterns/)).
  The official hooks page must be checked in design.

## Codex hook contract (official docs and source)

- **Configuration:** `hooks.json` or inline `[hooks]` in `config.toml`, in
  `~/.codex/` and `<repo>/.codex/`; all layers load, project hooks only when
  the project `.codex/` layer is trusted. Event, matcher group, handler
  `{"type":"command","command":…,"timeout":…}` with optional
  `commandWindows`. Matcher is a regex on `tool_name`. `[features] hooks =
  false` disables hooks ([hooks](https://learn.chatgpt.com/docs/hooks),
  [config reference](https://learn.chatgpt.com/docs/config-file/config-reference)).
- **Tools and input:** PreToolUse sees `Bash`, `apply_patch` (also matched by
  `Edit`/`Write`), MCP tools `mcp__server__tool` and local function tools.
  Stdin carries `session_id`, `cwd`, `turn_id`, `tool_name`, `tool_use_id`,
  `tool_input`; for Bash, `tool_input.command`. There is no native file-read
  tool and no file-path field: reads must be recognised in the command.
- **Decisions:** `hookSpecificOutput.permissionDecision: "deny"` with
  `permissionDecisionReason`, or exit 2 with stderr; `allow` supported;
  `ask` parsed but unsupported (the run fails and the tool runs).
- **Failure:** spawn error, timeout, other exit codes or invalid JSON mark
  the run failed and the tool proceeds: fail open, confirmed in
  [pre_tool_use.rs](https://github.com/openai/codex/blob/main/codex-rs/hooks/src/events/pre_tool_use.rs).
  Default timeout 600 s.
- **Trust:** `[hooks.state."<source>:<event>:<group>:<handler>"]` with
  `enabled`, `trusted_hash`, `trusted_content_hash`. `trusted_hash` is a
  SHA-256 of the normalised hook declaration (event, matcher, handler config
  including the command string), not of the executable; only plugin hooks
  hash bundle contents into `trusted_content_hash`. A new or changed hook is
  skipped with a startup warning until the user trusts it in `/hooks`
  ([discovery.rs](https://github.com/openai/codex/blob/main/codex-rs/hooks/src/engine/discovery.rs),
  [hook_config.rs](https://github.com/openai/codex/blob/main/codex-rs/config/src/hook_config.rs),
  [plugin_trust.rs](https://github.com/openai/codex/blob/main/codex-rs/hooks/src/engine/plugin_trust.rs)).
  An installer writing its own `trusted_hash` would defeat the review, so it
  must not. Because the binary is not hashed, a byte-identical declaration
  keeps its trust across mmhmem binary updates.
- **Policy:** `requirements.toml` `[features].hooks = false` forces hooks
  off; `allow_managed_hooks_only = true` drops user, project, session and
  plugin hooks.

## OpenCode plugin contract (source, v2.2.5)

- **Docs lag the release:** opencode.ai/docs/plugins still describes the v1
  API; v2 source differs, so a plugin written from the docs probably does
  not load.
- **Locations:** `~/.config/opencode/plugins/` and `.opencode/plugins/`
  (v2 also scans `plugin/`); npm plugins via config. A v2 module's default
  export is `{id, setup}` or `{id, effect}`
  ([source-directory.ts](https://github.com/anomalyco/opencode/blob/dev/packages/core/src/plugin/source-directory.ts),
  [module.ts](https://github.com/anomalyco/opencode/blob/dev/packages/core/src/plugin/module.ts)).
- **Interception:** `ctx.tool.hook("execute.before", e => …)`, event
  `{tool, sessionID, agent, messageID, id, input}`. Read tool input
  `{path, offset, limit}` (1-based, capped at 2000); shell tool `shell` with
  `{command, workdir?, timeout?}`
  ([tool.ts](https://github.com/anomalyco/opencode/blob/dev/packages/plugin/src/promise/tool.ts),
  [read.ts](https://github.com/anomalyco/opencode/blob/dev/packages/core/src/tool/plugin/read.ts),
  [shell.ts](https://github.com/anomalyco/opencode/blob/dev/packages/core/src/tool/plugin/shell.ts)).
- **Blocking:** throwing in `execute.before` fails that call with the
  message as a `Tool.Error`
  ([adapter.ts](https://github.com/anomalyco/opencode/blob/dev/packages/plugin/src/promise/adapter.ts)).
  The exact text the model sees is unconfirmed.
- **Failure:** any throw blocks, so the plugin must catch its own errors to
  fail open.
- **Trust and policy:** no plugin approval prompt; plugins run in-process
  with full privileges. A managed policy action `plugin.use` can deny a
  plugin (module.ts); the published policies page documents only
  `provider.use`.
- **External binaries:** the v2 plugin context has no Bun `$`; spawning a
  process from the plugin is presumably possible but unconfirmed.

## Per-agent summary

| Aspect | Claude Code | Codex | OpenCode (v2) |
|---|---|---|---|
| Mechanism | `PreToolUse` command hook | `PreToolUse` command hook, `[features] hooks` | in-process plugin, `execute.before` |
| Location (user scope) | `~/.claude/settings.json` | `~/.codex/hooks.json` or `[hooks]` in `config.toml` | `~/.config/opencode/plugins/` |
| Native read tool | `Read` with `file_path`, `offset`, `limit` | none; reads are shell commands | `read` with `path`, `offset`, `limit` |
| Shell tool | `Bash`, `PowerShell` | `Bash` (`tool_input.command`) | `shell` (`command`) |
| How to redirect | `deny` + reason, or exit 2 | `deny` + reason, or exit 2 | throw with the reason |
| On hook failure | proceeds for exits other than 0/2; timeout to test | proceeds (fail open, source-confirmed) | throw blocks: plugin must catch everything |
| Approval | workspace trust dialog gates all hooks | user trusts each hook in `/hooks`; hash of the declaration | none; installer consent is the only gate |
| Policy | `disableAllHooks`; managed settings | `requirements.toml`: `hooks = false`, `allow_managed_hooks_only` | managed `plugin.use` |
| Stays trusted across updates | entry unchanged | declaration byte-identical (binary not hashed) | n/a |
| Coexistence | all levels merge, `deny` wins | layers merge; claude-mem already hooks Codex | plugins compose |
| Decision code | `mmhmem hook claude` (exec form) | `mmhmem hook codex` | plugin calls the same decision (spawn `mmhmem hook opencode`, or a port; design decides) |
