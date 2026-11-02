//! CLI layer (local-mode Req 1.1-1.4; design.md "CLI dispatch" and
//! "Stats CLI"): subcommand runners that do not belong to the daemon path.
//! The arg matching itself stays in `main.rs` (thin arms, one runner each);
//! this module owns the `stats` surface and future local-mode commands.
//!
//! `mmhmem stats` is READ-ONLY (Req 8.5/8.6): it opens the per-user store
//! with `Db::open_read_only`, never creates it, and exits 0 with a clear
//! message when no statistics exist yet.

pub mod prune;
pub mod stats;

use std::process::ExitCode;

use anyhow::Context;

use crate::config::Config;
use crate::storage::db::{Db, DB_FILE_NAME};

/// Printed when the store has no statistics yet (Req 8.5): empty is not an
/// error — a fresh local install simply has nothing to report yet.
pub const NO_STATS_MESSAGE: &str =
    "No token-savings statistics recorded yet. Use mmhmem from a harness (stdio \
     or serve mode), then run `mmhmem stats` again.";

pub use prune::run_prune;

/// `mmhmem stats [--json]` entrypoint (Req 1.3, 8.1-8.6): read-only report
/// from the local profile's per-user store; exits 0 even with no data.
pub fn run_stats(json: bool) -> ExitCode {
    match build_report(json) {
        Ok(report) => {
            println!("{report}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("mmhmem stats: {e:#}");
            ExitCode::FAILURE
        }
    }
}

/// Resolves the report (or the no-statistics message) without mutating
/// anything: the store is opened read-only and is never created (Req
/// 8.5/8.6).
fn build_report(json: bool) -> anyhow::Result<String> {
    let config = Config::load_local()?;
    let store_path = config.data_dir.join(DB_FILE_NAME);
    let Some(db) = Db::open_read_only(&store_path)? else {
        return Ok(NO_STATS_MESSAGE.to_string());
    };
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("starting the stats runtime")?;
    let rows = rt.block_on(stats::aggregate(&db))?;
    if rows.is_empty() {
        return Ok(NO_STATS_MESSAGE.to_string());
    }
    Ok(if json {
        stats::render_json(&rows)
    } else {
        stats::render_table(&rows)
    })
}
