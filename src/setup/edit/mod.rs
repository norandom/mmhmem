//! Format-preserving config edits (design.md §4): every edit keeps comments,
//! whitespace and key order, and its result is re-parsed before use.

pub mod json;
pub mod markdown;
pub mod toml;
