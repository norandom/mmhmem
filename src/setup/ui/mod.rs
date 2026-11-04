//! The `mmhmem setup` front end (reliable-installers requirement 4). Task 6.1
//! provides the entry point and the text; the full-screen menu (6.2: the
//! controller in `flow.rs`, the screens in `app.rs`) and the numbered
//! fallback (6.3) plug into [`run`] through [`Mode`].

pub mod app;
#[cfg(test)]
pub(crate) mod fixtures;
pub mod flow;
pub mod text;
#[cfg(test)]
mod text_tests;

use std::io::IsTerminal;
use std::path::PathBuf;

use crate::setup::agents::Env;
use crate::setup::record::{LocationInput, RecordStore, ReleaseTag};
use crate::setup::release::{self, ReleaseSource};
use flow::{Deps, Flow, Newest};

/// Internal arguments of `mmhmem setup`; never shown in `--help`.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SetupArgs {
    /// Release tag the install script downloaded (`--bootstrap <tag>`).
    pub bootstrap: Option<String>,
    /// Local release artefacts for maintainers and CI (`--release-dir <dir>`).
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
    match current_mode() {
        Mode::Menu => run_menu(),
        mode => run_fallback(None, mode, &args),
    }
}

/// The full-screen menu; falls back to the numbered prompts when the
/// terminal cannot enter raw mode or the alternate screen.
fn run_menu() -> i32 {
    let Some(deps) = process_deps() else {
        eprintln!("{}", text::NO_HOME);
        return 1;
    };
    let flow = Flow::load(deps);
    match app::run_terminal(flow, Box::new(newest_release)) {
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
        Err(app::TerminalUnavailable(flow)) => {
            run_fallback(Some(*flow), Mode::Fallback, &SetupArgs::default())
        }
    }
}

/// TODO(6.3): the numbered fallback and the no-terminal default
/// (`fallback.rs`) take over here, driving the same [`Flow`].
fn run_fallback(flow: Option<Flow>, mode: Mode, args: &SetupArgs) -> i32 {
    let _ = flow;
    println!("{}", text::TITLE_HOME);
    println!("{}", text::INTRO);
    match mode {
        Mode::Menu | Mode::Fallback => println!("{}", text::DUMB_TERMINAL),
        Mode::NoTerminal => println!("{}", text::NO_TERMINAL),
    }
    if let Some(tag) = &args.bootstrap {
        println!("Installing version {tag}.");
    }
    println!("{}", text::PLACEHOLDER);
    0
}

/// The real environment, record folder, binary and release of this run.
fn process_deps() -> Option<Deps> {
    let env = Env::from_process()?;
    let store = RecordStore::locate(&LocationInput::from_process()).ok()?;
    let version = ReleaseTag::try_from(format!("v{}", env!("CARGO_PKG_VERSION"))).ok()?;
    // TODO(7.1): install the program to its final place first; until then
    // the agents launch the recorded program, or this one.
    let recorded = store.load().ok().flatten().and_then(|r| r.binary);
    let binary = match recorded {
        Some(b) => b.path,
        None => std::env::current_exe().ok()?,
    };
    Some(Deps::new(env, store, binary, version))
}

/// The newest stable release, or [`Newest::Unknown`] when it cannot be
/// looked up (offline, rate limit, a platform without releases).
fn newest_release() -> Newest {
    release::Target::current()
        .and_then(|target| release::latest_release(&ReleaseSource::github(), target))
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
