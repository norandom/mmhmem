//! Storage layer (design.md "storage layer"; steering structure.md
//! `src/storage/`): schema/migrations and the WAL connection (task 2.1);
//! the JSONL audit writer lands in task 2.2.
//!
//! Dependency rule (steering structure.md "Import Organization"): storage
//! imports only `config`; it never imports `services` or `mcp`.

pub mod db;
pub mod schema;
