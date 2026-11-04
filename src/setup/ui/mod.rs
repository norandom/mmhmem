//! The `mmhmem setup` front end (reliable-installers requirement 4). Task 6.1
//! provides the entry point and the text; the full-screen menu (6.2: the
//! controller in `flow.rs`, the screens in `app.rs`), the numbered
//! fallback and the no-terminal run (6.3, `fallback.rs`) plug into
//! [`run`] through [`Mode`].
//!
//! **No terminal (amended Requirement 4.9).** Without a terminal only the
//! install script may change anything: `--bootstrap <tag>` installs the
//! standard choices; any other run (for example an AI agent running
//! `mmhmem setup` in a non-interactive shell) changes nothing, reads
//! nothing, prints [`text::remove::NEEDS_TERMINAL`] and exits with
//! [`EXIT_NEEDS_TERMINAL`] ([`refuse_without_terminal`]).
//!
//! **After an update** the new program is started in the same terminal and
//! its exit code is returned (`flow_update.rs`).

pub mod app;
pub mod fallback;
#[cfg(test)]
pub(crate) mod fixtures;
pub mod flow;
pub mod text;
#[cfg(test)]
mod text_tests;

use std::io::{IsTerminal, Write};
use std::path::PathBuf;
use std::sync::Arc;

use crate::setup::adopt;
use crate::setup::agents::Env;
use crate::setup::record::{LocationInput, RecordStore, ReleaseTag};
use crate::setup::release::{self, LocalRelease, ReleaseSource};
use flow::{Deps, Flow, Memories, Newest, Releases, Restart};

/// The exit code of a run without a terminal that was not started by the
/// install script: nothing was changed.
pub const EXIT_NEEDS_TERMINAL: i32 = 1;

/// Internal arguments of `mmhmem setup`; never shown in `--help`.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SetupArgs {
    /// Release tag the install script downloaded (`--bootstrap <tag>`).
    pub bootstrap: Option<String>,
    /// Local release artefacts for maintainers and CI (`--release-dir <dir>`):
    /// archives, `SHA256SUMS` and `SHA256SUMS.sig`, served to the normal
    /// release checks on a loopback port ([`LocalRelease`]).
    pub release_dir: Option<PathBuf>,
    /// This run was started by the previous program right after it updated
    /// itself to this release (`--restarted <tag>`): go straight to the
    /// review, and never restart again.
    pub restarted: Option<String>,
}

/// How the setup talks to the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Full-screen menu (task 6.2).
    Menu,
    /// Numbered prompts: `TERM=dumb` (task 6.3).
    Fallback,
    /// No terminal: install the standard choices when started by the
    /// install script, else change nothing (task 6.3, amended 4.9).
    NoTerminal,
}

impl SetupArgs {
    /// Parses the arguments after `setup`. `Err` holds the unknown argument.
    pub fn parse(args: &[String]) -> Result<Self, String> {
        let mut out = Self::default();
        let mut it = args.iter();
        while let Some(arg) = it.next() {
            let value = it.next();
            match (arg.as_str(), value) {
                ("--bootstrap", Some(v)) => out.bootstrap = Some(v.clone()),
                ("--release-dir", Some(v)) => out.release_dir = Some(PathBuf::from(v)),
                ("--restarted", Some(v)) => out.restarted = Some(v.clone()),
                _ => return Err(arg.clone()),
            }
        }
        Ok(out)
    }
}

/// Picks the interaction mode from the terminal facts.
pub fn detect_mode(stdin_tty: bool, stdout_tty: bool, term: Option<&str>) -> Mode {
    if !stdin_tty || !stdout_tty {
        Mode::NoTerminal
    } else if term.is_some_and(|t| t.eq_ignore_ascii_case("dumb")) {
        Mode::Fallback
    } else {
        Mode::Menu
    }
}

/// The mode for the real process.
pub fn current_mode() -> Mode {
    let term = std::env::var("TERM").ok();
    detect_mode(
        std::io::stdin().is_terminal(),
        std::io::stdout().is_terminal(),
        term.as_deref(),
    )
}

/// Without a terminal and not started by the install script: says to run
/// setup in a terminal window and returns [`EXIT_NEEDS_TERMINAL`]; `None`
/// when the run may go on.
pub fn refuse_without_terminal(mode: Mode, args: &SetupArgs, out: &mut dyn Write) -> Option<i32> {
    if mode != Mode::NoTerminal || args.bootstrap.is_some() {
        return None;
    }
    let _ = writeln!(out, "{}", text::remove::NEEDS_TERMINAL);
    Some(EXIT_NEEDS_TERMINAL)
}

/// Entry point of `mmhmem setup`. Returns the process exit code.
pub fn run(args: SetupArgs) -> i32 {
    let mode = current_mode();
    if let Some(code) = refuse_without_terminal(mode, &args, &mut std::io::stdout().lock()) {
        return code;
    }
    if let Some(dir) = args.release_dir.as_ref().filter(|d| !d.is_dir()) {
        eprintln!("The release folder {} does not exist.", dir.display());
        return 1;
    }
    let Some(deps) = process_deps(&args) else {
        eprintln!("{}", text::NO_HOME);
        return 1;
    };
    match mode {
        Mode::Menu => run_menu(deps),
        mode => run_fallback(Flow::load(deps), mode),
    }
}

/// After an update: the new program's exit code (`None`: no restart due).
fn restarted(flow: &Flow) -> Option<i32> {
    match flow.restart_now()? {
        Ok(code) => Some(code),
        Err(reason) => {
            eprintln!("{}", text::remove::restart_failed(&reason));
            Some(1)
        }
    }
}

/// The full-screen menu; falls back to the numbered prompts when the
/// terminal cannot enter raw mode or the alternate screen.
fn run_menu(deps: Deps) -> i32 {
    let source = deps.releases.source.clone();
    let flow = Flow::load(deps);
    match app::run_terminal(flow, Box::new(move || newest_release(&source))) {
        Ok((code, flow)) => {
            if let Some(code) = restarted(&flow) {
                return code;
            }
            let lines = flow.progress().map(|p| p.all_lines()).unwrap_or_default();
            for line in lines {
                println!("{line}");
            }
            if code == flow::EXIT_CANCELLED {
                println!("{}", text::CANCELLED);
            }
            code
        }
        Err(app::TerminalUnavailable(flow)) => run_fallback(*flow, Mode::Fallback),
    }
}

/// The numbered prompts (`TERM=dumb`, or the menu could not start) or the
/// run without a terminal: both drive the same [`Flow`] (`fallback.rs`),
/// with the same `--bootstrap` and `--release-dir` dependencies.
fn run_fallback(mut flow: Flow, mode: Mode) -> i32 {
    let mut out = std::io::stdout().lock();
    if mode == Mode::NoTerminal {
        flow.set_newest(Newest::Unknown);
        return fallback::run_default(&mut flow, &mut out);
    }
    let _ = writeln!(out, "{}", text::DUMB_TERMINAL);
    flow.set_newest(newest_within(flow.release_source(), NEWEST_WAIT));
    let code = fallback::run(&mut flow, &mut std::io::stdin().lock(), &mut out);
    drop(out);
    restarted(&flow).unwrap_or(code)
}

/// How long the numbered prompts wait for the newest-release lookup.
const NEWEST_WAIT: std::time::Duration = std::time::Duration::from_secs(3);

/// The newest release, or [`Newest::Unknown`] when the lookup takes longer
/// than `wait`.
fn newest_within(source: ReleaseSource, wait: std::time::Duration) -> Newest {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(newest_release(&source));
    });
    rx.recv_timeout(wait).unwrap_or(Newest::Unknown)
}

/// The real environment, record folder, program, `PATH` store and release
/// source of this run.
///
/// The agents always launch the program at its permanent place: the
/// recorded one, else the standard place for this OS
/// ([`adopt::installed_binary`]). The running program is copied there
/// (a reviewed change) when it was started by the bootstrap
/// (`--bootstrap <tag>`, from a temporary folder that is deleted
/// afterwards) or when nothing is there yet.
///
/// The memory store is the local profile's (`Config::load_local`, which
/// honours the same environment and `.mmhmem.json` as `mmhmem stdio`).
/// A run started with `--restarted` never restarts again.
fn process_deps(args: &SetupArgs) -> Option<Deps> {
    let env = Env::from_process()?;
    let store = RecordStore::locate(&LocationInput::from_process()).ok()?;
    let own = ReleaseTag::try_from(format!("v{}", env!("CARGO_PKG_VERSION"))).ok()?;
    let version = args
        .bootstrap
        .clone()
        .and_then(|t| ReleaseTag::try_from(t).ok())
        .unwrap_or(own);
    let recorded = store.load().ok().flatten().and_then(|r| r.binary);
    let binary = recorded.map_or_else(|| adopt::installed_binary(&env.dirs), |b| b.path);
    let running = std::env::current_exe().ok();
    let install_from = running
        .filter(|r| !same_file(r, &binary) && (args.bootstrap.is_some() || !binary.exists()));
    let mut deps = Deps::new(env, store, binary, version);
    deps.install_from = install_from;
    if let Some(dir) = &args.release_dir {
        deps.releases = Releases::local(LocalRelease::serve(dir).ok()?);
    }
    deps.memories = crate::config::Config::load_local()
        .ok()
        .map(|c| Memories::from_config(&c));
    deps.restart = restart_of(args);
    Some(deps)
}

/// How this run starts the new program after an update.
fn restart_of(args: &SetupArgs) -> Restart {
    let mut again = Vec::new();
    if let Some(dir) = &args.release_dir {
        let dir = std::path::absolute(dir).unwrap_or_else(|_| dir.clone());
        again.extend(["--release-dir".to_owned(), dir.display().to_string()]);
    }
    let resumed = args
        .restarted
        .clone()
        .and_then(|t| ReleaseTag::try_from(t).ok());
    Restart {
        exec: args
            .restarted
            .is_none()
            .then(|| Arc::new(flow::spawn) as flow::Exec),
        args: again,
        resumed,
    }
}

/// Whether two paths name the same file (also through links).
fn same_file(a: &std::path::Path, b: &std::path::Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

/// The newest stable release, or [`Newest::Unknown`] when it cannot be
/// looked up (offline, rate limit, a platform without releases).
fn newest_release(source: &ReleaseSource) -> Newest {
    release::Target::current()
        .and_then(|target| release::latest_release(source, target))
        .map_or(Newest::Unknown, |latest| Newest::Known(latest.tag))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parses_internal_arguments() {
        let a = SetupArgs::parse(&v(&["--bootstrap", "v1.0.0", "--release-dir", "d"])).unwrap();
        assert_eq!(a.bootstrap.as_deref(), Some("v1.0.0"));
        assert_eq!(a.release_dir, Some(PathBuf::from("d")));
        assert_eq!(SetupArgs::parse(&[]).unwrap(), SetupArgs::default());
    }

    #[test]
    fn parses_the_restart_argument() {
        let a = SetupArgs::parse(&v(&["--restarted", "v9.9.9"])).unwrap();
        assert_eq!(a.restarted.as_deref(), Some("v9.9.9"));
        assert_eq!(
            SetupArgs::parse(&v(&["--restarted"])),
            Err("--restarted".into())
        );
    }

    #[test]
    fn without_a_terminal_only_the_install_script_may_go_on() {
        let plain = SetupArgs::default();
        let mut out = Vec::new();
        let code = refuse_without_terminal(Mode::NoTerminal, &plain, &mut out);
        assert_eq!(code, Some(EXIT_NEEDS_TERMINAL));
        let text = String::from_utf8(out).unwrap();
        assert_eq!(text.trim_end(), text::remove::NEEDS_TERMINAL);
        assert!(text.contains("run: mmhmem setup"), "{text}");
        let restarted = SetupArgs::parse(&v(&["--restarted", "v1.0.0"])).unwrap();
        assert!(refuse_without_terminal(Mode::NoTerminal, &restarted, &mut Vec::new()).is_some());
        let bootstrap = SetupArgs::parse(&v(&["--bootstrap", "v1.0.0"])).unwrap();
        let mut out = Vec::new();
        assert_eq!(
            refuse_without_terminal(Mode::NoTerminal, &bootstrap, &mut out),
            None
        );
        assert!(out.is_empty());
        for mode in [Mode::Menu, Mode::Fallback] {
            assert_eq!(refuse_without_terminal(mode, &plain, &mut Vec::new()), None);
        }
    }

    #[test]
    fn a_restarted_run_never_restarts_again() {
        let first = restart_of(&SetupArgs::parse(&v(&["--release-dir", "rel"])).unwrap());
        assert!(first.exec.is_some() && first.resumed.is_none());
        assert_eq!(first.args[0], "--release-dir");
        assert!(
            PathBuf::from(&first.args[1]).is_absolute(),
            "{:?}",
            first.args
        );
        let again = restart_of(&SetupArgs::parse(&v(&["--restarted", "v9.9.9"])).unwrap());
        assert!(again.exec.is_none());
        assert_eq!(
            again.resumed.map(|t| t.as_str().to_owned()),
            Some("v9.9.9".into())
        );
    }

    #[test]
    fn rejects_unknown_and_incomplete_arguments() {
        assert_eq!(SetupArgs::parse(&v(&["--bogus"])), Err("--bogus".into()));
        assert_eq!(
            SetupArgs::parse(&v(&["--bootstrap"])),
            Err("--bootstrap".into())
        );
    }

    #[test]
    fn detects_the_mode() {
        assert_eq!(detect_mode(true, true, Some("xterm")), Mode::Menu);
        assert_eq!(detect_mode(true, true, None), Mode::Menu);
        assert_eq!(detect_mode(true, true, Some("dumb")), Mode::Fallback);
        assert_eq!(detect_mode(false, true, Some("xterm")), Mode::NoTerminal);
        assert_eq!(detect_mode(true, false, None), Mode::NoTerminal);
    }
}
