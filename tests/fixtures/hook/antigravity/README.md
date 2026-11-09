# Antigravity `PreToolUse` hook payloads

Captured 2026-11-09 from Antigravity CLI (`agy`) 1.5.3 on Windows 11 with a
throwaway global hook (`~/.gemini/config/hooks.json`, matcher `.*`) whose
command appended stdin to a log. Each file is one stdin payload, exactly as
received (one JSON object, camelCase keys), with the Windows user name
replaced by `<user>`, the conversation id by `<conversation-id>` and the
scratch workspace by `D:/work/ws`.

Observed facts the hook adapter relies on (see
`.kiro/specs/antigravity-native-setup/research.md`):

- The file-read tool is `read_file`, not `view_file`. Arguments: `Path`
  (absolute; sent with backslashes in most calls and with forward slashes in
  one), optional `StartLine` and `EndLine` (integers) for a ranged read.
- `run_command` carries `CommandLine`, `WaitMsBeforeAsync`, `toolSummary`.
- MCP calls arrive as `call_mcp_tool` with `ServerName`, `ToolName`,
  `Arguments`; a matcher must not catch them or `smart_read` itself is
  blocked.
- Stdout `{}` is treated as a denial ("tool call denied by pre-tool hook:").
  `{"decision":"ask"}` lets the read proceed in print mode. A deny shows the
  agent `Encountered error in tool execution: tool call denied by pre-tool
  hook: <reason>`. A hook that exits non-zero or prints invalid JSON blocks
  the tool call with an error.
