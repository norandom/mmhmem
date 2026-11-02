//! `mmhmem prune [--dry-run]` (corpus-warmup-prune Req 3): stale-only cache
//! pruning over the local profile's per-user store. A missing store is not
//! an error — clear message, exit 0 (the stats contract). The scan/delete
//! logic is the service's; the CLI only parses, opens, and prints, so the
//! tool and the CLI behave identically by construction.

use std::process::ExitCode;

use anyhow::Context;

use crate::config::Config;
use crate::services::prune::{delete_stale, render_report, scan_stale};
use crate::services::shunt_types::CallCtx;
use crate::storage::audit::AuditWriter;
use crate::storage::db::{Db, DB_FILE_NAME};

/// Printed when the store does not exist yet (Req 3.4).
pub const NO_STORE_MESSAGE: &str =
    "No mmhmem store found yet — nothing to prune. Use mmhmem from a harness first.";

/// `mmhmem prune [--dry-run]` entrypoint (Req 3.1-3.4).
pub fn run_prune(dry_run: bool) -> ExitCode {
    match run(dry_run) {
        Ok(msg) => {
            println!("{msg}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("mmhmem prune: {e:#}");
            ExitCode::FAILURE
        }
    }
}

/// Resolves the report (or the no-store message); deleting only without
/// `dry_run`.
fn run(dry_run: bool) -> anyhow::Result<String> {
    let config = Config::load_local()?;
    let store_path = config.data_dir.join(DB_FILE_NAME);
    if !store_path.is_file() {
        return Ok(NO_STORE_MESSAGE.to_string());
    }
    let db = Db::open_at(&store_path)?;
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("starting the prune runtime")?;
    let report = if dry_run {
        rt.block_on(scan_stale(&db))?
    } else {
        let audit = AuditWriter::from_config(&config);
        let project = project_label();
        let ctx = CallCtx {
            project: &project,
            harness: "cli",
            task: None,
        };
        rt.block_on(delete_stale(&db, &audit, ctx))?
    };
    Ok(render_report(report, dry_run))
}

/// Audited project label for CLI runs: the current directory's name (the
/// project being pruned), falling back to "cli".
fn project_label() -> String {
    std::env::current_dir()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "cli".to_string())
}
