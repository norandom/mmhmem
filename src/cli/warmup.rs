//! `mmhmem warmup [<root>] [--root <dir>] [--dry-run] [--max-files <n>]`
//! (shunt-cache-efficiency tasks 6.1/6.2; design.md "Warmup CLI"; Req
//! 6.1-6.6): warm the shared summary cache for a whole repository.
//!
//! The candidates ([`super::warmup_select`]) go through the `shunt_warmup`
//! tool's own service, [`run_warmup`], in batches of
//! [`MAX_WARMUP_PATHS`] with the default question, on the local profile's
//! store, audit log and worker (`Config::load_local`, as `mmhmem stdio`
//! builds them): every file takes the real `smart_read` path, so cache rows,
//! audit records and statistics rows are the ones a live read writes. A
//! file already cached is a hit and costs no worker call, so a second run
//! costs nothing. The dry run lists the candidates and checks the cache
//! without touching a row; it opens the store read-only and writes nothing.
//! Without a worker, a run that would need one says so and exits 1 before
//! anything is written. No breadcrumbs: a warm-up is not an agent session.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::Context;

use super::warmup_select::{Candidate, DEFAULT_MAX_FILES, Rules, Selection, cap_message, select};
use crate::config::Config;
use crate::services::paths::AllowedRoots;
use crate::services::shunt::{ShuntEngine, ShuntError};
use crate::services::shunt_cache::{canonical_fingerprint, summary_cached};
use crate::services::shunt_content::DEFAULT_QUESTION;
use crate::services::shunt_types::CallCtx;
use crate::services::warmup::{MAX_WARMUP_PATHS, WarmupReport, run_warmup as warm_batch};
use crate::services::worker::WorkerClient;
use crate::storage::audit::{AuditWriter, sha256_hex};
use crate::storage::db::{DB_FILE_NAME, Db};

/// Printed on stderr for a bad `warmup` argument (exit 2).
pub const WARMUP_USAGE: &str =
    "usage: mmhmem warmup [<root>] [--root <dir>] [--dry-run] [--max-files <n>]";

/// The harness label of CLI warm-ups in audit and statistics records.
const CLI_HARNESS: &str = "cli";

/// Parsed `warmup` arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WarmupArgs {
    /// The repository root (positional or `--root`); `None`: the current folder.
    pub root: Option<PathBuf>,
    /// `--dry-run`: list and count, change nothing.
    pub dry_run: bool,
    /// `--max-files <n>`: candidates per run (at least 1).
    pub max_files: usize,
}

impl WarmupArgs {
    /// Parses the arguments after `warmup`; the error names the problem.
    pub fn parse<S: AsRef<str>>(args: &[S]) -> Result<Self, String> {
        let mut parsed = Self {
            root: None,
            dry_run: false,
            max_files: DEFAULT_MAX_FILES,
        };
        let mut args = args.iter().map(AsRef::as_ref);
        while let Some(arg) = args.next() {
            match arg {
                "--dry-run" => parsed.dry_run = true,
                "--max-files" => parsed.max_files = max_files(args.next())?,
                "--root" => parsed.set_root(args.next().ok_or("--root needs a folder")?)?,
                flag if flag.starts_with('-') => return Err(format!("unknown argument: {flag}")),
                root => parsed.set_root(root)?,
            }
        }
        Ok(parsed)
    }

    fn set_root(&mut self, root: &str) -> Result<(), String> {
        if self.root.is_some() {
            return Err(format!("one root only: {root}"));
        }
        self.root = Some(PathBuf::from(root));
        Ok(())
    }
}

fn max_files(value: Option<&str>) -> Result<usize, String> {
    match value.map(|v| v.parse::<usize>()) {
        Some(Ok(n)) if n > 0 => Ok(n),
        _ => Err("--max-files needs a whole number of at least 1".to_string()),
    }
}

/// Why a run stopped.
enum Failure {
    /// No worker and `uncached` of `total` candidates need one (Req 6.6).
    NoWorker {
        uncached: usize,
        total: usize,
    },
    Other(anyhow::Error),
}

impl From<anyhow::Error> for Failure {
    fn from(e: anyhow::Error) -> Self {
        Failure::Other(e)
    }
}

/// `mmhmem warmup` entrypoint: the report on stdout and exit 0; a missing
/// worker or any other failure on stderr and exit 1.
pub fn run_warmup(args: WarmupArgs) -> ExitCode {
    match run(&args) {
        Ok(report) => {
            println!("{report}");
            ExitCode::SUCCESS
        }
        Err(Failure::NoWorker { uncached, total }) => {
            eprintln!("mmhmem warmup: {}", ShuntError::WorkerNotConfigured);
            eprintln!(
                "{uncached} of {total} candidates have no cached summary; nothing was changed. \
                 Set worker_model in .mmhmem.json (or WORKER_MODEL) and run again."
            );
            ExitCode::FAILURE
        }
        Err(Failure::Other(e)) => {
            eprintln!("mmhmem warmup: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &WarmupArgs) -> Result<String, Failure> {
    let config = Config::load_local().context("loading the local configuration")?;
    let root = resolve_root(args.root.as_deref())?;
    let roots = AllowedRoots::from_config(&config);
    let rules = Rules {
        threshold: u64::from(config.shunt_min_lines),
        roots: &roots,
        max_files: args.max_files,
    };
    let selection = select(&root, &rules);
    let worker = WorkerClient::from_config(&config).ok();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("starting the warmup runtime")?;
    if args.dry_run {
        let uncached = rt.block_on(uncached_in_store(&config, &selection.files))?;
        return Ok(render_dry_run(
            &root,
            &selection,
            uncached,
            worker.is_some(),
        ));
    }
    if worker.is_none() && !selection.files.is_empty() {
        let uncached = rt.block_on(uncached_in_store(&config, &selection.files))?;
        if uncached > 0 {
            let total = selection.files.len();
            return Err(Failure::NoWorker { uncached, total });
        }
    }
    let report = if selection.files.is_empty() {
        WarmupReport::default()
    } else {
        rt.block_on(warm(&config, &root, worker, &selection.files))?
    };
    Ok(render_run(&root, &selection, &report))
}

/// The root as an absolute folder (`None`: the current folder).
fn resolve_root(arg: Option<&Path>) -> anyhow::Result<PathBuf> {
    let root = match arg {
        Some(root) => root.to_path_buf(),
        None => std::env::current_dir().context("reading the current folder")?,
    };
    let root = std::path::absolute(&root).with_context(|| root.display().to_string())?;
    anyhow::ensure!(root.is_dir(), "{} is not a folder", root.display());
    Ok(root)
}

/// Candidates without a shared summary in the store, checked read-only
/// without touching a row; no store means none is cached.
async fn uncached_in_store(config: &Config, files: &[Candidate]) -> anyhow::Result<usize> {
    let Some(db) = Db::open_read_only(&config.data_dir.join(DB_FILE_NAME))? else {
        return Ok(files.len());
    };
    let mut uncached = 0;
    for file in files {
        let cached = match std::fs::read(&file.abs) {
            Ok(bytes) => summary_cached(&db, &sha256_hex(&bytes), canonical_fingerprint())
                .await
                .map_err(|e| anyhow::anyhow!("checking the cache: {e}"))?,
            Err(_) => false,
        };
        uncached += usize::from(!cached);
    }
    Ok(uncached)
}

/// Every candidate through the warm-up service, one batch at a time.
async fn warm(
    config: &Config,
    root: &Path,
    worker: Option<WorkerClient>,
    files: &[Candidate],
) -> Result<WarmupReport, Failure> {
    let db = Db::open_for_config(config)?;
    let audit = AuditWriter::from_config(config);
    let project_dir = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let engine =
        ShuntEngine::new(db, audit.clone(), config.clone(), None, worker).with_project(project_dir);
    let project = project_label(root);
    let ctx = CallCtx {
        project: &project,
        harness: CLI_HARNESS,
        task: None,
    };
    let batches = files.len().div_ceil(MAX_WARMUP_PATHS);
    let mut total = WarmupReport::default();
    for (n, batch) in files.chunks(MAX_WARMUP_PATHS).enumerate() {
        let paths: Vec<String> = batch.iter().map(|c| c.abs.display().to_string()).collect();
        match warm_batch(&engine, &audit, &paths, DEFAULT_QUESTION, ctx).await {
            Ok(report) => total.absorb(report),
            // Only when a file changed after the cache check: the batches
            // before this one are warmed, so this is no "nothing changed".
            Err(e) => {
                let stop = format!("stopped at batch {} of {batches}", n + 1);
                return Err(anyhow::anyhow!("{e} ({stop})").into());
            }
        }
        eprintln!("mmhmem warmup: batch {} of {batches} done", n + 1);
    }
    Ok(total)
}

/// Audited project label: the root folder's name, as an agent started
/// there would name it.
fn project_label(root: &Path) -> String {
    root.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| CLI_HARNESS.to_string())
}

/// The one report of a run, in the `render_report` style: the selection's
/// and the service's outcomes merged, then the per-file details.
fn render_run(root: &Path, selection: &Selection, report: &WarmupReport) -> String {
    let skipped = &selection.skipped;
    let mut out = format!(
        "Repository warmup of {root}: {examined} files examined, {produced} summaries produced, \
         {hits} hits, skipped {below} below threshold, {not_text} not text, \
         {security} security-critical, {outside} outside allowed roots, \
         {missing} missing or unreadable; {failed} failed. \
         Worker tokens: {prompt} prompt + {completion} completion.",
        root = root.display(),
        examined = selection.examined,
        produced = report.delegated,
        hits = report.cached,
        below = skipped.below_threshold + report.skipped_below_threshold,
        not_text = skipped.not_text,
        security = skipped.security + report.skipped_security,
        outside = skipped.outside_roots + report.denied,
        missing = skipped.unreadable + report.missing,
        failed = report.failed,
        prompt = report.worker_prompt_tokens,
        completion = report.worker_completion_tokens,
    );
    if !report.notes.is_empty() {
        out.push_str("\nPer-file details:");
        for note in &report.notes {
            out.push_str("\n- ");
            out.push_str(note);
        }
    }
    push_cap(&mut out, selection);
    if report.delegated == 0 && report.failed == 0 && report.cached > 0 {
        out.push_str("\nsecond run: hits only");
    }
    out
}

/// The dry-run report: the candidates and the worker calls a run would make.
fn render_dry_run(root: &Path, selection: &Selection, uncached: usize, worker: bool) -> String {
    let skipped = &selection.skipped;
    let mut out = format!(
        "Repository warmup of {root} (dry run, nothing changed): {examined} files examined, \
         {candidates} candidates, skipped {below} below threshold, {not_text} not text, \
         {security} security-critical, {outside} outside allowed roots, \
         {missing} missing or unreadable.",
        root = root.display(),
        examined = selection.examined,
        candidates = selection.files.len(),
        below = skipped.below_threshold,
        not_text = skipped.not_text,
        security = skipped.security,
        outside = skipped.outside_roots,
        missing = skipped.unreadable,
    );
    if !selection.files.is_empty() {
        out.push_str("\nCandidates:");
        for file in &selection.files {
            out.push_str("\n- ");
            out.push_str(&file.rel.to_string_lossy().replace('\\', "/"));
        }
    }
    let cached = selection.files.len() - uncached;
    out.push_str(&format!(
        "\nwould summarise {uncached} files (worker calls); {cached} already cached"
    ));
    if uncached > 0 && !worker {
        out.push_str("\nno worker model is configured (WORKER_MODEL): a run would stop here");
    }
    push_cap(&mut out, selection);
    out
}

fn push_cap(out: &mut String, selection: &Selection) {
    if selection.capped {
        out.push('\n');
        out.push_str(&cap_message(selection.files.len()));
    }
}

#[cfg(test)]
#[path = "warmup_tests.rs"]
mod tests;
