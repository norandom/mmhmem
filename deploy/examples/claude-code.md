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

## Playbook

Install the agent-side playbook so every session follows it:

```sh
mkdir -p ~/.claude/skills/unified-memory
cp deploy/skills/unified-memory/SKILL.md ~/.claude/skills/unified-memory/
```

TLS is expected to terminate at a reverse proxy in front of the daemon
(design.md "Security Considerations"); point the URL at your proxy hostname.
