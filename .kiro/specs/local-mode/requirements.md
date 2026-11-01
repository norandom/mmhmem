# Requirements Document

## Project Description (Input)
mmhmem adopters run agentic harnesses (Claude Code, Antigravity, OpenCode, Claude Desktop) locally on Linux, Windows, and macOS, but today the token-efficiency layer — the smart_read/boilerplate_write shunt, the content-hash cache, and token statistics — exists only inside the mmhmem server: an adopter without the server gets nothing, and token-savings insight is only visible through the server's store. The same single Rust binary should additionally run in a local mode (`mmhmem stdio`): each harness spawns it as a stdio MCP server (rmcp `transport-io`) backed by a per-user shared store (~/.local/share/mmhmem or %APPDATA%\mmhmem) that serves ALL six existing tools — so two harnesses on the same machine share the shunt cache, sessions (same-machine handoff), memory, and token stats — plus a `mmhmem stats` CLI printing human-readable token-savings from the local store. The server remains unchanged as an optional upgrade for cross-machine access, central audit/compliance, central ops, and multi-user bearer auth; no serve-mode regression is acceptable.

## Requirements
<!-- Will be generated in /kiro-spec-requirements phase -->
