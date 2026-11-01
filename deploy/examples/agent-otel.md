# Agent-side OTLP export to OpenObserve (Req 11.5)

mmhmem ships **no client code** for harness telemetry: agent-side tracing is
configured in the harness itself, sending directly to your self-hosted
OpenObserve. The server-side export (cargo feature `otel`, Docker image) is
separate — see `.env.example`.

## OpenCode

Use the community `opencode-plugin-otel` (targets OpenCode >= 2; exports LLM
and tool spans/metrics via OTLP):

```jsonc
// ~/.config/opencode/opencode.json
{
  "plugins": {
    "opencode-plugin-otel": "opencode-plugin-otel@2"
  }
}
```

Then point the plugin at OpenObserve in the harness process environment:

```sh
export OPENCODE_OTLP_ENDPOINT=http://openobserve.internal:4318
export OPENCODE_OTLP_PROTOCOL=http/protobuf
export OTEL_EXPORTER_OTLP_HEADERS='Authorization=Basic <your-openobserve-basic-auth>'
```

## Claude Code

Claude Code has built-in OpenTelemetry metrics export (no plugin needed):

```sh
export CLAUDE_CODE_ENABLE_TELEMETRY=1
export OTEL_EXPORTER_OTLP_ENDPOINT=http://openobserve.internal:4318
export OTEL_EXPORTER_OTLP_HEADERS='Authorization=Basic <your-openobserve-basic-auth>'
export OTEL_METRICS_EXPORTER=otlp
```

## Notes

- Keep credentials in the harness environment, never in committed configs.
- The JSONL audit trail on the mmhmem host remains the system of record
  (Req 11.3); harness-side traces are complementary observability.
- Antigravity/Claude Desktop do not expose OTLP export today; their activity
  is still covered by mmhmem's own optional OTel export and audit trail.
