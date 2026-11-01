//! MCP protocol layer (design.md "mcp layer"; steering structure.md
//! `src/mcp/`): the Streamable HTTP server with bearer auth (task 10.1) and
//! the thin tool handlers (tasks 10.2/10.3). Handlers stay thin — all policy
//! lives in `services` / `storage`.
//!
//! Dependency rule (steering structure.md "Import Organization"): `mcp` may
//! import `services`, `storage`, `config`; never the reverse.

pub mod server;
