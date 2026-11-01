# OpenCode (Req 8.4)

Two equivalent ways to connect; the token always comes from the environment
(`{env:MMHMEM_TOKEN}` is OpenCode's env substitution — shell `$VAR` is not
expanded inside JSON).

## CLI (writes the project config)

```sh
opencode mcp add mmhmem --global \
  --url http://mmhmem.internal:8443/mcp \
  --header 'Authorization=Bearer {env:MMHMEM_TOKEN}'
```

## Hand-edited config (`~/.config/opencode/opencode.json`)

See `opencode.json` next to this file. Fields:

- `type: "remote"` — MCP Streamable HTTP transport (Req 1.1).
- `oauth: false` — mmhmem authenticates with a bearer header, not OAuth.
- `codemode: false` — expose the six tools on the native tool list instead of
  grouping them under Code Mode.
- `headers` — sent on every request; `MMHMEM_TOKEN` must be set in the
  OpenCode process environment.

## Verify

```sh
opencode mcp list       # mmhmem should be connected, not "needs authentication"
```

## Playbook

Point the agent instructions at the shipped playbook:

```sh
mkdir -p ~/.config/opencode/skills/unified-memory
cp deploy/skills/unified-memory/SKILL.md ~/.config/opencode/skills/unified-memory/
```
