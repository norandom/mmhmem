//! The steps every agent check shares (reliable-installers task 4.2,
//! `.kiro/specs/reliable-installers/design.md` §6; Requirements 6.2–6.6).
//!
//! An agent's `check` builds [`Steps`] from the registration it read back
//! from the agent's own file (step 1), which runs the MCP check through
//! [`Env::probe_mcp`] (step 2). The agent then runs its own command (step 3),
//! the hook sample (step 4, [`hook_sample`]) and the config load (step 5)
//! with [`Steps::run`] / [`Steps::set`]; a step after a failure is skipped.
//! [`Steps::finish`] turns the outcomes and the record's pending user steps
//! into a [`CheckReport`] with the result lines of design.md §6.
//!
//! [`McpProbe`] and [`Clock`] are the injection points tests replace
//! (through [`Env::with_mcp_probe`] and [`Env::with_clock`]). Failures
//! ([`AgentFailure`]) hold fixed codes, program names and numbers only,
//! never agent output, so no secret printed by an agent CLI can reach a
//! result, a `Display` or a `Debug`.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::{
    DEFAULT_CHECK_TIMEOUT, EXPECTED_TOOLS, McpCheck, Registration, RegistrationError, check_mcp,
    text,
};
use crate::setup::agents::{
    AgentId, AgentRecord, CheckReport, CommandOutput, Env, Feature, Invocation, RunError,
};
use crate::setup::record::{
    CheckDetail, CheckResult, CheckStep, HookHarness, Os, StepOutcome, Target,
};

pub use super::hook::{SAMPLE_LINES, hook_sample};

/// The agent steps, in the order they are reported after the MCP steps.
pub const AGENT_STEPS: [CheckStep; 3] = [
    CheckStep::AgentCommand,
    CheckStep::HookSample,
    CheckStep::ConfigLoad,
];

/// Runs the MCP functional check on a registration (step 2).
pub trait McpProbe: Send + Sync {
    /// The result of starting `registration` and talking MCP to it.
    fn probe(&self, registration: &Registration) -> McpCheck;
}

/// The real probe: [`check_mcp`] with this release's tools and version and
/// the 20 s limit.
#[derive(Debug, Clone, Copy, Default)]
pub struct ServerProbe;

impl McpProbe for ServerProbe {
    fn probe(&self, registration: &Registration) -> McpCheck {
        check_mcp(
            registration,
            &EXPECTED_TOOLS,
            env!("CARGO_PKG_VERSION"),
            DEFAULT_CHECK_TIMEOUT,
        )
    }
}

/// Time for checks that wait (the OpenCode poll).
pub trait Clock: Send + Sync {
    /// Now.
    fn now(&self) -> Instant;
    /// Waits `duration`.
    fn sleep(&self, duration: Duration);
}

/// The system clock.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn sleep(&self, duration: Duration) {
        std::thread::sleep(duration);
    }
}

/// Why an agent's own program could not answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandProblem {
    /// It did not finish within this many seconds.
    TimedOut {
        /// The limit in whole seconds.
        seconds: u64,
    },
    /// It could not be started.
    CouldNotRun,
}

impl CommandProblem {
    /// The problem behind `error`.
    pub fn of(error: &RunError) -> Self {
        match error {
            RunError::TimedOut { after, .. } => Self::TimedOut {
                seconds: after.as_secs(),
            },
            _ => Self::CouldNotRun,
        }
    }
}

/// Why an agent step failed. Fixed codes, program names and numbers only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentFailure {
    /// The agent's own command could not be run to the end.
    CommandFailed {
        /// The program's name (`claude`).
        program: &'static str,
        /// What went wrong.
        problem: CommandProblem,
    },
    /// The agent could not load its config after the change.
    ConfigNotLoaded,
    /// The agent does not list mmhmem.
    NotListed,
    /// Claude Code uses an mmhmem entry of another scope.
    WrongScope,
    /// The agent lists mmhmem but is not connected to it.
    NotConnected,
    /// mmhmem is switched off in the agent.
    Disabled,
    /// The agent starts another program or arguments for mmhmem.
    WrongCommand,
    /// The agent's answer could not be understood.
    UnexpectedAnswer,
    /// OpenCode did not report mmhmem connected in time.
    NotConnectedInTime {
        /// Whether mmhmem was listed at all.
        listed: bool,
        /// The limit in whole seconds.
        seconds: u64,
    },
    /// The hook let the sample read through.
    HookLetReadThrough,
    /// The hook answered something else than the redirect.
    HookWrongAnswer,
    /// The hook could not be run.
    HookNotRun {
        /// What went wrong.
        problem: CommandProblem,
    },
}

/// The outcome of one agent step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepResult {
    /// It passed.
    Pass,
    /// It did not run (not selected, or the agent has no such command).
    Skipped,
    /// It failed.
    Fail(AgentFailure),
}

/// The steps of one agent check, in order.
#[derive(Debug)]
pub struct Steps {
    agent: AgentId,
    mcp: McpCheck,
    outcomes: Vec<(CheckStep, StepOutcome)>,
    failure: Option<AgentFailure>,
    hook_inactive: bool,
}

impl Steps {
    /// Steps 1–2: the registration read back from the agent's file, and the
    /// MCP check of it (not run when the registration could not be read).
    pub fn start(
        env: &Env,
        agent: AgentId,
        registration: Result<Registration, RegistrationError>,
    ) -> Self {
        let mcp = match registration {
            Ok(registration) => env.probe_mcp(&registration),
            Err(error) => McpCheck::registration_failed(error),
        };
        Self {
            agent,
            mcp,
            outcomes: Vec::new(),
            failure: None,
            hook_inactive: false,
        }
    }

    /// Every step so far passed or was skipped.
    pub fn ok(&self) -> bool {
        self.mcp.passed() && self.failure.is_none()
    }

    /// Runs `step` only while [`Steps::ok`]; otherwise it stays skipped.
    pub fn run(&mut self, step: CheckStep, f: impl FnOnce() -> StepResult) {
        if self.ok() {
            let result = f();
            self.set(step, result);
        }
    }

    /// Step 4 (only call it when the read hook is selected): without a
    /// worker model the hook lets every read through by design, so the
    /// sample is skipped and the report says the hook stays inactive, which
    /// is never a failure. With a worker model the hook must redirect.
    pub fn hook(&mut self, env: &Env, binary: &Path, harness: HookHarness) {
        if !self.ok() {
            return;
        }
        if env.worker_configured() {
            let result = hook_sample(env, binary, harness);
            self.set(CheckStep::HookSample, result);
        } else {
            self.set(CheckStep::HookSample, StepResult::Skipped);
            self.hook_inactive = true;
        }
    }

    /// Records the outcome of `step`; the first failure is kept for the
    /// result line.
    pub fn set(&mut self, step: CheckStep, result: StepResult) {
        let outcome = match result {
            StepResult::Pass => StepOutcome::Pass,
            StepResult::Skipped => StepOutcome::Skipped,
            StepResult::Fail(failure) => {
                self.failure.get_or_insert(failure);
                StepOutcome::Fail
            }
        };
        self.outcomes.retain(|(s, _)| *s != step);
        self.outcomes.push((step, outcome));
    }

    /// The report: every step's outcome (agent steps not run are skipped),
    /// the result and the lines. All passed with steps only the user can do
    /// in `rec` gives [`CheckResult::UserStepsPending`] and one `●` line per
    /// step.
    pub fn finish(self, rec: &AgentRecord) -> CheckReport {
        let mut details = self.mcp.details.clone();
        for step in AGENT_STEPS {
            let outcome = self
                .outcomes
                .iter()
                .find(|(s, _)| *s == step)
                .map_or(StepOutcome::Skipped, |(_, o)| *o);
            details.push(CheckDetail { step, outcome });
        }
        let label = self.agent.label();
        let notes = if self.ok() && self.hook_inactive {
            vec![format!("● {label} — {}", text::hook_inactive())]
        } else {
            Vec::new()
        };
        let (result, line, pending) = match (&self.failure, self.mcp.passed()) {
            (_, false) => (CheckResult::Fail, self.mcp.line(self.agent), Vec::new()),
            (Some(failure), true) => (
                CheckResult::Fail,
                format!(
                    "✘ {label} — {}. What you can do: {}.",
                    text::agent_reason(self.agent, failure),
                    text::agent_what_you_can_do(self.agent, failure)
                ),
                Vec::new(),
            ),
            (None, true) => {
                let pending: Vec<String> = rec
                    .pending_user_steps
                    .iter()
                    .map(|step| format!("● {label} — {}", text::pending_step(*step)))
                    .collect();
                let result = if pending.is_empty() {
                    CheckResult::Pass
                } else {
                    CheckResult::UserStepsPending
                };
                (result, self.mcp.line(self.agent), pending)
            }
        };
        CheckReport {
            agent: self.agent,
            ran: true,
            result,
            details,
            line,
            pending,
            notes,
        }
    }
}

/// The line shown after a failed check was rolled back.
pub fn restored_line(agent: AgentId) -> String {
    text::restored(agent)
}

/// The registration in `file`, read with `parse` from the bytes on disk. A
/// missing file has no entry; any other read error is unreadable.
pub fn read_registration(
    file: &Path,
    parse: impl FnOnce(&[u8]) -> Result<Registration, RegistrationError>,
) -> Result<Registration, RegistrationError> {
    match std::fs::read(file) {
        Ok(bytes) => parse(&bytes),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(RegistrationError::Missing),
        Err(_) => Err(RegistrationError::Unreadable),
    }
}

/// The file of the record's JSON entry at `pointer` (`["mcpServers",
/// "mmhmem"]`), if the record has one.
pub fn recorded_json_file(rec: &AgentRecord, pointer: &[&str]) -> Option<PathBuf> {
    rec.managed.iter().find_map(|entry| match &entry.target {
        Target::JsonEntry {
            file, pointer: p, ..
        } if p.iter().map(|k| k.as_str()).eq(pointer.iter().copied()) => Some(file.clone()),
        _ => None,
    })
}

/// Whether the record has the read hook switched on.
pub fn hook_selected(rec: &AgentRecord) -> bool {
    rec.features
        .get(&Feature::ReadHook)
        .copied()
        .unwrap_or(false)
}

/// Whether `shown` (as an agent prints or stores it) names `binary`.
/// Windows paths compare without case and with either slash.
pub fn same_program(shown: &str, binary: &Path, os: Os) -> bool {
    let want = binary.to_string_lossy();
    if os == Os::Windows {
        let norm = |s: &str| s.trim().replace('/', "\\").to_lowercase();
        norm(shown) == norm(&want)
    } else {
        shown.trim() == want
    }
}

/// Runs the agent's own `program` (found on the [`Env`]'s `PATH`) with
/// `args`, optionally in `dir`. `Err(Skipped)` when it is not on `PATH`,
/// `Err(Fail)` when it could not run to the end. The output stays with the
/// caller; it is never logged.
pub fn run_agent_command(
    env: &Env,
    program: &'static str,
    args: &[&str],
    dir: Option<&Path>,
    timeout: Duration,
) -> Result<CommandOutput, StepResult> {
    let Some(path) = env.find_program(program) else {
        return Err(StepResult::Skipped);
    };
    let mut invocation = Invocation::new(path, args).timeout(timeout);
    if let Some(dir) = dir {
        invocation = invocation.in_dir(dir);
    }
    env.run(&invocation).map_err(|error| match error {
        RunError::NotFound { .. } => StepResult::Skipped,
        other => StepResult::Fail(AgentFailure::CommandFailed {
            program,
            problem: CommandProblem::of(&other),
        }),
    })
}
