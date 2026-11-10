//! Unit tests for `warmup.rs` (shunt-cache-efficiency task 6.2): argument
//! parsing and the two reports.
use super::*;
use crate::cli::warmup_select::{Listing, Skipped};

fn parse(args: &[&str]) -> Result<WarmupArgs, String> {
    WarmupArgs::parse(args)
}

#[test]
fn the_root_is_positional_or_a_flag_and_flags_come_in_any_order() {
    let want = WarmupArgs {
        root: Some(PathBuf::from("repo")),
        dry_run: true,
        max_files: 7,
    };
    assert_eq!(
        parse(&["repo", "--dry-run", "--max-files", "7"]),
        Ok(want.clone())
    );
    assert_eq!(
        parse(&["--max-files", "7", "--root", "repo", "--dry-run"]),
        Ok(want)
    );
    let defaults = parse(&[]).unwrap();
    assert_eq!(defaults.root, None, "the current folder");
    assert_eq!(defaults.max_files, DEFAULT_MAX_FILES);
    assert!(!defaults.dry_run);
}

#[test]
fn bad_arguments_are_errors() {
    for args in [
        &["--bogus"][..],
        &["--max-files"][..],
        &["--max-files", "0"][..],
        &["--max-files", "many"][..],
        &["--root"][..],
        &["a", "b"][..],
        &["--root", "a", "b"][..],
    ] {
        assert!(parse(args).is_err(), "{args:?}");
    }
}

fn selection(files: &[&str], capped: bool) -> Selection {
    Selection {
        listing: Listing::Git,
        files: files
            .iter()
            .map(|f| Candidate {
                rel: PathBuf::from(f),
                abs: Path::new("/r").join(f),
            })
            .collect(),
        examined: 9,
        skipped: Skipped {
            below_threshold: 4,
            not_text: 1,
            security: 1,
            outside_roots: 0,
            unreadable: 0,
        },
        capped,
    }
}

#[test]
fn the_run_report_merges_selection_and_service_counts() {
    let report = WarmupReport {
        delegated: 2,
        cached: 1,
        skipped_security: 1,
        worker_prompt_tokens: 1000,
        worker_completion_tokens: 60,
        notes: vec!["/r/x.rs: security-critical pattern (direct read)".into()],
        ..WarmupReport::default()
    };
    let out = render_run(Path::new("/r"), &selection(&["a", "b", "c"], true), &report);
    assert!(
        out.contains(
            "9 files examined, 2 summaries produced, 1 hits, skipped 4 below threshold, \
             1 not text, 2 security-critical, 0 outside allowed roots, \
             0 missing or unreadable; 0 failed. Worker tokens: 1000 prompt + 60 completion."
        ),
        "{out}"
    );
    assert!(
        out.contains("\nPer-file details:\n- /r/x.rs: security"),
        "{out}"
    );
    assert!(
        out.ends_with("\nstopped at 3 files; raise --max-files to continue"),
        "{out}"
    );
    assert!(!out.contains("second run"), "{out}");
}

#[test]
fn hits_only_is_said_when_nothing_was_produced_or_failed() {
    let sel = selection(&["a"], false);
    let hits = WarmupReport {
        cached: 1,
        ..WarmupReport::default()
    };
    assert!(render_run(Path::new("/r"), &sel, &hits).ends_with("\nsecond run: hits only"));
    let failed = WarmupReport {
        failed: 1,
        ..hits.clone()
    };
    assert!(!render_run(Path::new("/r"), &sel, &failed).contains("hits only"));
    let nothing = WarmupReport::default();
    assert!(!render_run(Path::new("/r"), &sel, &nothing).contains("hits only"));
}

#[test]
fn the_dry_run_lists_candidates_and_the_worker_calls() {
    let sel = selection(&["src/a.rs", "b.rs"], false);
    let out = render_dry_run(Path::new("/r"), &sel, 1, true);
    assert!(out.contains("(dry run, nothing changed): 9 files examined, 2 candidates"));
    assert!(out.contains("\nCandidates:\n- src/a.rs\n- b.rs\n"), "{out}");
    assert!(
        out.ends_with("would summarise 1 files (worker calls); 1 already cached"),
        "{out}"
    );
    let no_worker = render_dry_run(Path::new("/r"), &sel, 1, false);
    assert!(
        no_worker.contains("no worker model is configured"),
        "{no_worker}"
    );
    let all_cached = render_dry_run(Path::new("/r"), &sel, 0, false);
    assert!(!all_cached.contains("no worker model"), "{all_cached}");
}

#[test]
fn the_project_label_is_the_root_folder_name() {
    assert_eq!(project_label(Path::new("/work/mmhmem")), "mmhmem");
    assert_eq!(project_label(Path::new("/")), "cli");
}
