//! CLI layer (local-mode Req 1.1-1.4; design.md "CLI dispatch" and
//! "Stats CLI"): subcommand runners that do not belong to the daemon path.
//! The arg matching itself stays in `main.rs` (thin arms, one runner each);
//! this module owns the `stats` surface and future local-mode commands.
//!
//! `mmhmem stats` is READ-ONLY (Req 8.5/8.6): it opens the per-user store
//! with `Db::open_read_only`, never creates it, and exits 0 with a clear
//! message when no statistics exist yet.

pub mod stats;

/// Printed when the store has no statistics yet (Req 8.5): empty is not an
/// error — a fresh local install simply has nothing to report yet.
pub const NO_STATS_MESSAGE: &str =
    "No token-savings statistics recorded yet. Use mmhmem from a harness (stdio \
     or serve mode), then run `mmhmem stats` again.";
