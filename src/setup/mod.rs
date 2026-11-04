//! `mmhmem setup`: the user installer (reliable-installers spec,
//! `.kiro/specs/reliable-installers/design.md` "Source layout"). One shared
//! implementation for every platform: safe file writes, config edits, the
//! install record, plans, agent integrations, the functional check and the
//! menu.

pub mod adopt;
pub mod agents;
pub mod assets;
pub mod check;
pub mod edit;
pub mod fsx;
pub mod plan;
pub mod record;
