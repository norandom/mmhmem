//! The functional check's result as the install record keeps it (task 2.4,
//! design.md §3 and §6; Requirement 5.5): the overall result, each step's
//! outcome and when the check ran.

use serde::{Deserialize, Serialize};

use super::UtcTimestamp;

/// The overall result of a functional check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckResult {
    /// Everything passed.
    Pass,
    /// Something failed.
    Fail,
    /// Passed, but user steps are pending.
    UserStepsPending,
}

/// One step of the functional check (design.md §6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckStep {
    /// Read back the registration.
    Registration,
    /// Start the server and run `initialize`.
    Handshake,
    /// `tools/list`.
    ToolsList,
    /// The `status` tool.
    StatusTool,
    /// The agent's own check command.
    AgentCommand,
    /// The hook sample.
    HookSample,
    /// The agent's config load.
    ConfigLoad,
}

/// The outcome of one check step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepOutcome {
    /// The step passed.
    Pass,
    /// The step failed.
    Fail,
    /// The step did not run.
    Skipped,
}

/// One step's outcome in [`LastCheck::details`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckDetail {
    /// Which step.
    pub step: CheckStep,
    /// How it went.
    pub outcome: StepOutcome,
}

/// The last functional check of an agent (Requirement 5.5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LastCheck {
    /// When it ran.
    pub at: UtcTimestamp,
    /// The overall result.
    pub result: CheckResult,
    /// Per-step outcomes.
    #[serde(default)]
    pub details: Vec<CheckDetail>,
}
