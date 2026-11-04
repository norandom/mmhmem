//! The `mmhmem setup` front end (reliable-installers requirement 4). Task 6.1
//! provides the entry point and the text; the full-screen menu (6.2: the
//! controller in `flow.rs`, the screens in `app.rs`), the numbered
//! fallback and the no-terminal run (6.3, `fallback.rs`) plug into
//! [`run`] through [`Mode`].

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

use crate::setup::adopt;
use crate::setup::agents::Env;
use crate::setup::record::{LocationInput, RecordStore, ReleaseTag};
use crate::setup::release::{self, LocalRelease, ReleaseSource};
use flow::{Deps, Flow, Newest, Releases};

/// Internal arguments of `mmhmem setup`; never shown in `--help`.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SetupArgs {
    /// Release tag the install script downloaded (`--bootstrap <tag>`).
    pub bootstrap: Option<String>,
    /// Local release artefacts for maintainers and CI (`--release-dir <dir>`):
    /// archives, `SHA256SUMS` and `SHA256SUMS.sig`, served to the normal
    /// release checks on a loopback port ([`LocalRelease`]).
    pub release_dir: Option<PathBuf>,
}

/// How the setup talks to the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Full-screen menu (task 6.2).
    Menu,
    /// Numbered prompts: `TERM=dumb` (task 6.3).
    Fallback,
    /// No terminal: install the standard choices (task 6.3).
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

/// Entry point of `mmhmem setup`. Returns the process exit code.
pub fn run(args: SetupArgs) -> i32 {
    if let Some(dir) = args.release_dir.as_ref().filter(|d| !d.is_dir()) {
        eprintln!("The release folder {} does not exist.", dir.display());
        return 1;
    }
    let Some(deps) = process_deps(&args) else {
        eprintln!("{}", text::NO_HOME);
        return 1;
    };
    match current_mode() {
        Mode::Menu => run_menu(deps),
        mode => run_fallback(Flow::load(deps), mode),
    }
}

/// The full-screen menu; falls back to the numbered prompts when the
/// terminal cannot enter raw mode or the alternate screen.
fn run_menu(deps: Deps) -> i32 {
    let source = deps.releases.source.clone();
    let flow = Flow::load(deps);
    match app::run_terminal(flow, Box::new(move || newest_release(&source))) {
        Ok((code, progress)) => {
            let lines = progress.map(|p| p.all_lines()).unwrap_or_default();
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
    fallback::run(&mut flow, &mut std::io::stdin().lock(), &mut out)
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
    Some(deps)
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
