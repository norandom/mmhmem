//! Storage layer (design.md "storage layer"; steering structure.md
//! `src/storage/`): schema/migrations and the WAL connection (task 2.1);
//! the redacting, hash-verifying, append-only JSONL audit writer (task 2.2).
//!
//! Dependency rule (steering structure.md "Import Organization"): storage
//! imports only `config`; it never imports `services` or `mcp`.

pub mod audit;
pub mod db;
pub mod schema;
