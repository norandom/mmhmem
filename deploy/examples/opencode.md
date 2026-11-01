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

## Local mode (no server; local-mode Req 1.2, 2.1, 2.5)

OpenCode can spawn mmhmem directly as a stdio child — no token, no server,
no network. Either CLI:

```sh
opencode mcp add mmhmem --global --command mmhmem --args stdio
```

or hand-edit `~/.config/opencode/opencode.json` using `opencode-local.json`
next to this file: `type: "local"`, `command: ["mmhmem", "stdio"]`. The
per-user store (`~/.local/share/mmhmem`, `%APPDATA%\mmhmem`, or
`~/Library/Application Support/mmhmem` by OS) is created on first use and
shared by every OpenCode (and other harness) session on the machine. Check
savings with `mmhmem stats`.

## Playbook

Point the agent instructions at the shipped playbook:

```sh
mkdir -p ~/.config/opencode/skills/unified-memory
cp deploy/skills/unified-memory/SKILL.md ~/.config/opencode/skills/unified-memory/
```
