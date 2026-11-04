# Agent CLI fixtures (reliable-installers task 1.1)

Captured on Windows 11 on 2026-11-04 from Claude Code 2.1.302, codex-cli 0.166.0
and OpenCode 2.2.5. User names, other servers and environment values are
replaced with neutral placeholders; the structure, spacing and status glyphs
are as printed.

- `claude-code/mcp-list-connected.txt`: `claude mcp list`; status glyph is U+2714.
- `claude-code/mcp-get-connected.txt`: `claude mcp get mmhmem`.
- `codex/mcp-get.txt`: `codex mcp get mmhmem`. `codex mcp list` is a wide table;
  the check uses `codex mcp list --json` (output to be captured in task 3.4).
- `opencode/mcp-list-before-init.txt`: first `opencode mcp list` answer from a new
  directory instance or with `--standalone`, printed before MCP servers start.
- `opencode/mcp-list-connected.txt`: the same command once the instance is warm.
- `opencode/mcp-add-project-result.json`: the file OpenCode itself wrote for
  `opencode mcp add mmhmemprobe -- <bin> stdio` in an empty project folder.
