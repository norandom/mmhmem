//! The agents' environment and command runner (reliable-installers task 3.1,
//! `.kiro/specs/reliable-installers/design.md` §5; Requirements 2.1 and 8.4).
//!
//! [`Env`] is everything an agent module may look at outside its own files:
//! the folders ([`ScanEnv`]: home, OS, `PATH`, `APPDATA`, `LOCALAPPDATA`,
//! `XDG_CONFIG_HOME`, `CLAUDE_CONFIG_DIR`, `CODEX_HOME`) and a
//! [`CommandRunner`] for the agents' own CLIs. Tests build it over a
//! temporary home with a replaying runner; only the real `mmhmem setup` calls
//! [`Env::from_process`].
//!
//! **No secrets in logs.** Agent CLIs can print secrets (`codex mcp list
//! --json` prints env values unmasked, research.md §8), so nothing here logs,
//! [`CommandOutput`]'s `Debug` shows only lengths and [`RunError`] names the
//! program, never its output.

use std::fmt;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, ExitStatus, Stdio};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use crate::setup::adopt::ScanEnv;
use crate::setup::check::agent::{Clock, McpProbe, ServerProbe, SystemClock};
use crate::setup::check::{McpCheck, Registration};
use crate::setup::record::Os;

/// The default time an agent CLI call may take (design.md §6 uses 20 s).
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(20);
/// At most this many bytes of each output stream are kept.
pub const MAX_OUTPUT: u64 = 1024 * 1024;
/// How long to wait for the output pipes after the program has exited.
const PIPE_GRACE: Duration = Duration::from_millis(500);
/// How often a running program is polled.
const POLL: Duration = Duration::from_millis(10);

/// One call of an agent CLI: program, arguments, folder and time limit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invocation {
    /// The program (a resolved path, or a bare name).
    pub program: PathBuf,
    /// The arguments.
    pub args: Vec<String>,
    /// The working folder (`None`: the caller's).
    pub cwd: Option<PathBuf>,
    /// Extra environment variables set for the program. Never put a secret
    /// here: invocations are recorded by test doubles and shown in `Debug`.
    pub envs: Vec<(String, String)>,
    /// The time limit; the program is killed after it.
    pub timeout: Duration,
    /// Bytes written to the program's standard input, which is then closed
    /// (`None`: no input). Never a secret, for the same reason as `envs`.
    pub stdin: Option<Vec<u8>>,
}

impl Invocation {
    /// A call of `program` with `args`, the caller's folder and
    /// [`DEFAULT_TIMEOUT`].
    pub fn new(program: impl Into<PathBuf>, args: &[&str]) -> Self {
        Self {
            program: program.into(),
            args: args.iter().map(|a| (*a).to_owned()).collect(),
            cwd: None,
            envs: Vec::new(),
            timeout: DEFAULT_TIMEOUT,
            stdin: None,
        }
    }

    /// Writes `bytes` to the program's standard input (never a secret).
    pub fn stdin(mut self, bytes: impl Into<Vec<u8>>) -> Self {
        self.stdin = Some(bytes.into());
        self
    }

    /// Adds one argument.
    pub fn arg(mut self, arg: impl Into<String>) -> Self {
        self.args.push(arg.into());
        self
    }

    /// Runs the program in `dir`.
    pub fn in_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.cwd = Some(dir.into());
        self
    }

    /// Sets one environment variable for the program (never a secret).
    pub fn env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.envs.push((key.into(), value.into()));
        self
    }

    /// Sets the time limit.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// The program's name without folder and extension, lower case
    /// (`C:\…\claude.cmd` → `claude`).
    pub fn program_name(&self) -> String {
        self.program
            .file_stem()
            .map(|s| s.to_string_lossy().to_lowercase())
            .unwrap_or_default()
    }
}

/// What a finished program printed. `Debug` shows only lengths.
#[derive(Clone, PartialEq, Eq)]
pub struct CommandOutput {
    /// The exit code (`None`: ended by a signal).
    pub code: Option<i32>,
    /// Standard output, lossily decoded as UTF-8.
    pub stdout: String,
    /// Standard error, lossily decoded as UTF-8.
    pub stderr: String,
}

impl CommandOutput {
    /// Exit code 0.
    pub fn success(&self) -> bool {
        self.code == Some(0)
    }
}

impl fmt::Debug for CommandOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CommandOutput")
            .field("code", &self.code)
            .field("stdout_len", &self.stdout.len())
            .field("stderr_len", &self.stderr.len())
            .finish()
    }
}

/// Why a program could not be run to the end. Names the program only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunError {
    /// The program does not exist (or is not on `PATH`).
    NotFound {
        /// The program as given.
        program: PathBuf,
    },
    /// The program could not be started or waited for.
    Failed {
        /// The program as given.
        program: PathBuf,
        /// The OS error kind.
        kind: io::ErrorKind,
    },
    /// The program did not finish in time and was stopped.
    TimedOut {
        /// The program as given.
        program: PathBuf,
        /// The time limit.
        after: Duration,
    },
}

impl fmt::Display for RunError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound { program } => write!(f, "{} was not found", program.display()),
            Self::Failed { program, kind } => {
                write!(f, "{} could not be run ({kind})", program.display())
            }
            Self::TimedOut { program, after } => write!(
                f,
                "{} did not finish within {} s and was stopped",
                program.display(),
                after.as_secs()
            ),
        }
    }
}

impl std::error::Error for RunError {}

/// Runs agent CLIs. Injected through [`Env`] so tests never start a real
/// agent.
pub trait CommandRunner: Send + Sync {
    /// Runs `invocation` to the end or until its time limit.
    fn run(&self, invocation: &Invocation) -> Result<CommandOutput, RunError>;
}

/// The real runner: `std::process` with a time limit, the invocation's
/// stdin (or none), both output
/// streams captured (at most [`MAX_OUTPUT`] bytes each) and never logged.
/// The program inherits this process's environment, as the agent CLI needs
/// it to find its own configuration.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemRunner;

impl CommandRunner for SystemRunner {
    fn run(&self, invocation: &Invocation) -> Result<CommandOutput, RunError> {
        let program = invocation.program.clone();
        let mut command = std::process::Command::new(&invocation.program);
        command
            .args(&invocation.args)
            .stdin(if invocation.stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(dir) = &invocation.cwd {
            command.current_dir(dir);
        }
        command.envs(invocation.envs.iter().map(|(k, v)| (k, v)));
        let mut child = command.spawn().map_err(|e| match e.kind() {
            io::ErrorKind::NotFound => RunError::NotFound {
                program: program.clone(),
            },
            kind => RunError::Failed {
                program: program.clone(),
                kind,
            },
        })?;
        let deadline = Instant::now() + invocation.timeout;
        feed(child.stdin.take(), invocation.stdin.clone());
        let stdout = drain(child.stdout.take());
        let stderr = drain(child.stderr.take());
        let status = wait_until(&mut child, deadline).map_err(|e| RunError::Failed {
            program: program.clone(),
            kind: e.kind(),
        })?;
        let Some(status) = status else {
            return Err(RunError::TimedOut {
                program,
                after: invocation.timeout,
            });
        };
        Ok(CommandOutput {
            code: status.code(),
            stdout: collect(&stdout, deadline),
            stderr: collect(&stderr, deadline),
        })
    }
}

/// Writes `bytes` to the program's stdin on its own thread and closes it,
/// so a program that does not read never blocks the runner.
fn feed(pipe: Option<std::process::ChildStdin>, bytes: Option<Vec<u8>>) {
    if let (Some(mut pipe), Some(bytes)) = (pipe, bytes) {
        thread::spawn(move || {
            use std::io::Write;
            let _ = pipe.write_all(&bytes);
        });
    }
}

/// Reads a pipe to the end on its own thread (so a full pipe never blocks
/// the program), keeping at most [`MAX_OUTPUT`] bytes.
fn drain<R: Read + Send + 'static>(pipe: Option<R>) -> mpsc::Receiver<Vec<u8>> {
    let (tx, rx) = mpsc::channel();
    if let Some(mut pipe) = pipe {
        thread::spawn(move || {
            let mut kept = Vec::new();
            let _ = (&mut pipe).take(MAX_OUTPUT).read_to_end(&mut kept);
            let _ = io::copy(&mut pipe, &mut io::sink());
            let _ = tx.send(kept);
        });
    }
    rx
}

/// The captured bytes of one pipe. A pipe a leftover child process keeps
/// open counts as empty after a short grace time.
fn collect(rx: &mpsc::Receiver<Vec<u8>>, deadline: Instant) -> String {
    let wait = deadline
        .saturating_duration_since(Instant::now())
        .max(PIPE_GRACE);
    rx.recv_timeout(wait)
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        .unwrap_or_default()
}

/// Waits for the program; kills it at `deadline` (`Ok(None)`).
fn wait_until(child: &mut Child, deadline: Instant) -> io::Result<Option<ExitStatus>> {
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(Some(status));
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Ok(None);
        }
        thread::sleep(POLL);
    }
}

/// The environment an agent module works in (design.md §5).
#[derive(Clone)]
pub struct Env {
    /// Home, OS, `PATH` and the per-agent folder overrides.
    pub dirs: ScanEnv,
    runner: Arc<dyn CommandRunner>,
    mcp: Arc<dyn McpProbe>,
    clock: Arc<dyn Clock>,
    worker: Option<bool>,
}

impl fmt::Debug for Env {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Env")
            .field("dirs", &self.dirs)
            .finish_non_exhaustive()
    }
}

impl Env {
    /// An environment over `dirs` whose agent CLIs run through `runner`;
    /// the MCP check starts the real server ([`ServerProbe`]) and waiting
    /// uses the system clock.
    pub fn new(dirs: ScanEnv, runner: Arc<dyn CommandRunner>) -> Self {
        Self {
            dirs,
            runner,
            mcp: Arc::new(ServerProbe),
            clock: Arc::new(SystemClock),
            worker: None,
        }
    }

    /// The same environment with a fixed answer to "is a worker model set
    /// up?" (tests), instead of the hook's own config lookup.
    pub fn with_worker_configured(mut self, configured: bool) -> Self {
        self.worker = Some(configured);
        self
    }

    /// Whether a worker model is set up, decided exactly as `mmhmem hook`
    /// decides it ([`crate::cli::hook::worker_configured`]).
    pub fn worker_configured(&self) -> bool {
        self.worker
            .unwrap_or_else(crate::cli::hook::worker_configured)
    }

    /// The same environment with another MCP check (tests).
    pub fn with_mcp_probe(mut self, probe: Arc<dyn McpProbe>) -> Self {
        self.mcp = probe;
        self
    }

    /// The same environment with another clock (tests: no real waiting).
    pub fn with_clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = clock;
        self
    }

    /// Runs the MCP functional check on `registration`.
    pub fn probe_mcp(&self, registration: &Registration) -> McpCheck {
        self.mcp.probe(registration)
    }

    /// The clock agent checks wait with.
    pub fn clock(&self) -> &dyn Clock {
        self.clock.as_ref()
    }

    /// The real environment of this process with the [`SystemRunner`].
    /// Only `mmhmem setup` itself calls this; tests never do.
    pub fn from_process() -> Option<Self> {
        Some(Self::new(ScanEnv::from_process()?, Arc::new(SystemRunner)))
    }

    /// Which OS rules apply.
    pub fn os(&self) -> Os {
        self.dirs.os
    }

    /// The user's home folder.
    pub fn home(&self) -> &Path {
        &self.dirs.home
    }

    /// The folders on `PATH` (empty when not known).
    pub fn path_dirs(&self) -> &[PathBuf] {
        self.dirs.user_path.as_deref().unwrap_or(&[])
    }

    /// `XDG_CONFIG_HOME`, or `~/.config`.
    pub fn config_home(&self) -> PathBuf {
        self.dirs.config_home()
    }

    /// `CLAUDE_CONFIG_DIR`, or `~/.claude`.
    pub fn claude_config_dir(&self) -> PathBuf {
        self.dirs.claude_config_dir()
    }

    /// `CODEX_HOME`, or `~/.codex`.
    pub fn codex_home(&self) -> PathBuf {
        self.dirs.codex_home()
    }

    /// Finds `name` in [`Env::path_dirs`] only (never the process `PATH`),
    /// with the host's executable rules (`PATHEXT` on Windows).
    pub fn find_program(&self, name: &str) -> Option<PathBuf> {
        let dirs = self.path_dirs();
        if dirs.is_empty() {
            return None;
        }
        let joined = std::env::join_paths(dirs).ok()?;
        which::which_in(name, Some(joined), self.home()).ok()
    }

    /// Runs one agent CLI call through the injected runner.
    pub fn run(&self, invocation: &Invocation) -> Result<CommandOutput, RunError> {
        self.runner.run(invocation)
    }
}
