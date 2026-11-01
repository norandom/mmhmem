# Claude Code CLI (Req 8.4)

Remote Streamable HTTP MCP server with bearer-token auth (Req 1.1).

## Add (user scope)

```sh
claude mcp add --transport http --scope user mmhmem \
  http://mmhmem.internal:8443/mcp \
  --header "Authorization: Bearer ${MMHMEM_TOKEN}"
```

Omit `--scope user` for a project-local registration. Keep the token in an
environment variable — never in the command history or project files.

## Verify

```sh
claude mcp list          # mmhmem should show as connected
claude mcp get mmhmem    # shows url + headers (token still env-substituted)
```

## Local mode (no server; local-mode Req 1.2, 2.1, 2.5)

Claude Code can also spawn mmhmem directly as a stdio child. Same six
tools, same playbook — no token, no server, no network:

```sh
claude mcp add --transport stdio --scope user mmhmem -- mmhmem stdio
```

- Zero configuration: the store defaults to your per-user data dir
  (`~/.local/share/mmhmem` on Linux, `%APPDATA%\mmhmem` on Windows,
  `~/Library/Application Support/mmhmem` on macOS) and is created on
  first use.
- Every Claude Code session on the machine shares one per-user store:
  handoff, shunt cache, memory, and token stats (Req 5.1-5.5).
- Check your savings any time: `mmhmem stats` (or `mmhmem stats --json`).

## Playbook

Install the agent-side playbook so every session follows it:

```sh
mkdir -p ~/.claude/skills/unified-memory
cp deploy/skills/unified-memory/SKILL.md ~/.claude/skills/unified-memory/
```

TLS is expected to terminate at a reverse proxy in front of the daemon
(design.md "Security Considerations"); point the URL at your proxy hostname.
