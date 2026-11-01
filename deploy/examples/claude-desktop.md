# Claude Desktop (Req 8.4 — Custom Connectors)

Claude Desktop reaches remote MCP servers through **custom connectors**
(Settings → Extensions → Advanced settings → "Add custom connector"). Unlike
the CLI harnesses, Desktop connectors require a **public HTTPS** endpoint and
an OpenAPI schema, so mmhmem sits behind your TLS reverse proxy:

1. Terminate TLS for `https://mmhmem.example.com` at your proxy (Caddy, nginx,
   Traefik — cert issuance is out of boundary, design.md "Security
   Considerations") and forward to the daemon's `AGENT_MEM_HOST:AGENT_MEM_PORT`
   at `AGENT_MEM_MCP_PATH` (default `:8443/mcp`).
2. Host a minimal OpenAPI 3.0 schema describing the MCP endpoint, e.g.
   served by the same proxy at `https://mmhmem.example.com/openapi.json`.
3. In Claude Desktop: Settings → Extensions → Advanced settings →
   **Add custom connector**:
   - Manifest/OpenAPI URL: `https://mmhmem.example.com/openapi.json`
   - Authentication: API key → header name `Authorization`, value
     `Bearer <your AGENT_MEM_TOKEN>`.
4. Enable the connector for chats that should share the mmhmem workspace
   project.

## Playbook

```sh
mkdir -p ~/Library/Claude/skills/unified-memory     # macOS
cp deploy/skills/unified-memory/SKILL.md ~/Library/Claude/skills/unified-memory/
```

Claude Desktop runs on the user's machine, not the Linux host — it reaches
mmhmem over the network like any other harness; no hooks or local plugins are
installed (steering product.md: protocol-native interfaces only).
