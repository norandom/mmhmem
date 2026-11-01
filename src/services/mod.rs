//! Services layer (design.md "services layer"; steering structure.md
//! `src/services/`): the worker-model client (task 3.1); session bus,
//! shunt engine, boilerplate writer, memory, and stats land in later tasks.
//!
//! Dependency rule (steering structure.md "Import Organization"): services
//! may import `config` and `storage`, never `mcp`.

pub mod session_bus;
pub mod shunt;
pub mod worker;
