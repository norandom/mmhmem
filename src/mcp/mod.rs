//! MCP protocol layer (design.md "mcp layer"; steering structure.md
//! `src/mcp/`): the Streamable HTTP server with bearer auth (task 10.1) and
//! the thin tool handlers (tasks 10.2/10.3). Handlers stay thin — all policy
//! lives in `services` / `storage`.
//!
//! Dependency rule (steering structure.md "Import Organization"): `mcp` may
//! import `services`, `storage`, `config`; never the reverse.

pub mod params;
pub mod server;
pub mod stdio;
pub mod tools;

mod session_response;
mod tool_support;

mod instructions;

#[cfg(test)]
mod update_notice_tests;
