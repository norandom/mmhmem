# Antigravity CLI (`agy`) 1.5.3 outputs

Captured 2026-11-09 on Windows 11 from `agy.exe` 1.5.3
(`%LOCALAPPDATA%\agy\bin\agy.exe`), with the Windows user name replaced by
`<user>`.

- `agy-version.txt`: `agy --version`.
- `agy-mcp-list-empty.txt`: `agy mcp list` with `~/.gemini/config/mcp_config.json`
  holding `{"mcpServers": {}}`.
- `agy-mcp-list-mmhmem.txt`: `agy mcp list` after
  `agy mcp add mmhmem <bin> stdio`. Columns are space-aligned; the mmhmem line
  contains the server name, `stdio`, `enabled` and the command with its
  arguments.
- `mcp_config-after-agy-mcp-add.json`: the exact file `agy mcp add` wrote to
  `~/.gemini/config/mcp_config.json` (keys sorted, two-space indent, adds
  `"disabled": false`). `agy mcp remove mmhmem` left `{"mcpServers": {}}`.

The CLI and the IDE read the same global file. The older
`~/.gemini/antigravity/mcp_config.json` is not read (a mmhmem entry there is
ignored by `agy mcp list`).
