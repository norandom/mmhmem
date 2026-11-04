//! Tests of the agent checks (reliable-installers task 4.2): per agent a
//! pass, the agent not seeing mmhmem, a config that no longer loads, the
//! hook sample, the OpenCode race and pending user steps. Temporary homes,
//! the CLI doubles and the fake MCP probe and clock only.

use std::path::PathBuf;

use serde_json::{Value, json};

use super::doubles::{Reply, TestHome};
use super::doubles_cli::{self, CLAUDE_MCP_GET, CODEX_MCP_LIST_JSON};
use super::{Agent, AgentId, AgentRecord, CheckReport, Feature, Selection, claude_code, codex};
use crate::setup::check::Registration;
use crate::setup::plan::{Outcome, Plan, apply};
use crate::setup::record::{
    CheckResult, CheckStep, InstallRecord, Os, RecordStore, StepOutcome, UtcTimestamp,
};

/// The program path in the captured fixtures.
const FIXTURE_BIN: &str = r"C:\Users\user\AppData\Local\Programs\mmhmem\mmhmem.exe";
const SECRET: &str = "sk-codex-check-secret-91c4";

pub(crate) struct Fx {
    pub(crate) home: TestHome,
    pub(crate) store: RecordStore,
    pub(crate) record: InstallRecord,
    pub(crate) bin: PathBuf,
}

pub(crate) fn fx() -> Fx {
    let home = TestHome::new(Os::Linux);
    let store = RecordStore::new(home.dir.path().join("state"));
    let bin = home.bin().join("mmhmem");
    Fx {
        home,
        store,
        record: InstallRecord::new(),
        bin,
    }
}

impl Fx {
    /// Plans and applies `want` for `agent`; every part must apply.
    pub(crate) fn install(&mut self, agent: &dyn Agent, want: &Selection) {
        let rec = self.record.agents.get(&agent.id());
        let changes = agent.plan(&self.home.env, want, rec).expect("plan");
        let run = UtcTimestamp::try_from("2026-11-04T10:00:00Z".to_owned()).expect("time");
        let report = apply(&Plan { run, changes }, &self.store, &mut self.record);
        assert!(report.agents.iter().all(|a| a.outcome == Outcome::Applied));
    }

    /// The agent's check on its record entry with `want`'s features and
    /// pending steps (as the coordinator builds it).
    pub(crate) fn check(&self, agent: &dyn Agent, want: &Selection) -> CheckReport {
        let id = agent.id();
        let mut rec: AgentRecord = self.record.agents.get(&id).cloned().unwrap_or_default();
        for f in agent.features() {
            rec.features.insert(*f, want.wants(id, *f));
        }
        rec.pending_user_steps = agent.pending_steps(want);
        agent.check(&self.home.env, &rec)
    }

    pub(crate) fn bin_text(&self) -> String {
        self.bin.to_string_lossy().into_owned()
    }
}

pub(crate) fn outcome(report: &CheckReport, step: CheckStep) -> StepOutcome {
    report
        .details
        .iter()
        .find(|d| d.step == step)
        .map(|d| d.outcome)
        .expect("every step is reported")
}

pub(crate) fn want(id: AgentId, features: &[Feature]) -> Selection {
    Selection::new().with(id, features)
}

/// The captured `claude mcp get` with `scope`, `status` and our program.
pub(crate) fn claude_get(fx: &Fx, scope: &str, status: &str) -> String {
    CLAUDE_MCP_GET
        .replace(FIXTURE_BIN, &fx.bin_text())
        .replace("User config", scope)
        .replace("✔ Connected", status)
}

/// The captured `codex mcp list --json` pointing at our program, with a
/// secret env value as the real CLI prints it.
pub(crate) fn codex_list(fx: &Fx, edit: impl FnOnce(&mut Value)) -> String {
    let mut list: Value = serde_json::from_str(CODEX_MCP_LIST_JSON).expect("fixture");
    list[0]["transport"]["command"] = json!(fx.bin_text());
    list[0]["transport"]["env"] = json!({"OPENAI_API_KEY": SECRET});
    edit(&mut list);
    list.to_string()
}

/// The `PreToolUse` deny the real hook prints for Claude Code and Codex.
pub(crate) fn deny() -> String {
    json!({"hookSpecificOutput": {"hookEventName": "PreToolUse", "permissionDecision": "deny",
        "permissionDecisionReason": "mmhmem: big. Call smart_read instead."}})
    .to_string()
}

fn claude_fx() -> (Fx, claude_code::ClaudeCode) {
    let mut fx = fx();
    fx.home.install_program("claude");
    doubles_cli::claude(&fx.home.runner, &fx.home.env);
    let agent = claude_code::ClaudeCode::new(&fx.bin);
    fx.install(&agent, &want(AgentId::ClaudeCode, &[Feature::Mcp]));
    (fx, agent)
}

#[test]
fn claude_code_passes_with_user_scope_connected_and_the_installed_program() {
    let (fx, agent) = claude_fx();
    let get = claude_get(&fx, "User config", "✔ Connected");
    fx.home
        .runner
        .on("claude", &["mcp", "get", "mmhmem"], vec![Reply::ok(&get)]);
    let report = fx.check(&agent, &want(AgentId::ClaudeCode, &[Feature::Mcp]));
    assert_eq!(report.result, CheckResult::Pass);
    assert_eq!(report.line, "✔ Claude Code — mmhmem connected, 10 tools");
    assert!(report.pending.is_empty());
    assert_eq!(outcome(&report, CheckStep::AgentCommand), StepOutcome::Pass);
    assert_eq!(outcome(&report, CheckStep::ConfigLoad), StepOutcome::Pass);
    assert_eq!(
        outcome(&report, CheckStep::HookSample),
        StepOutcome::Skipped
    );
    let seen = fx.home.probe.seen();
    assert_eq!(seen, [Registration::new(&fx.bin, &["stdio"])]);
    let calls = fx.home.runner.calls();
    let get_call = calls
        .iter()
        .find(|c| c.args[..2] == ["mcp", "get"])
        .unwrap();
    assert_eq!(get_call.cwd.as_deref(), Some(fx.home.home()));
}

#[test]
fn claude_code_failures_say_what_is_wrong_and_what_to_do() {
    let (fx, agent) = claude_fx();
    let other = fx.bin.with_file_name("old-mmhmem");
    let cases = [
        (
            claude_get(&fx, "User config", "✘ Failed to connect"),
            "✘ Claude Code — Claude Code lists mmhmem but could not connect to it. \
             What you can do: restart Claude Code, then run the installer again.",
        ),
        (
            claude_get(
                &fx,
                "Local config (private to you in this project)",
                "✔ Connected",
            ),
            "✘ Claude Code — Claude Code uses an mmhmem entry from a project or local \
             setting instead of your user settings. What you can do: remove that entry with \
             claude mcp remove mmhmem -s local (or -s project inside that project), then run \
             the installer again.",
        ),
        (
            claude_get(&fx, "User config", "✔ Connected")
                .replace(&fx.bin_text(), &other.to_string_lossy()),
            "✘ Claude Code — Claude Code starts a different program for mmhmem than the one \
             this installer put in place. What you can do: run the installer again.",
        ),
        (
            "something else\n".to_owned(),
            "✘ Claude Code — Claude Code answered in a form this installer does not \
             understand. What you can do: update Claude Code, then run the installer again.",
        ),
    ];
    for (get, line) in cases {
        fx.home
            .runner
            .on("claude", &["mcp", "get", "mmhmem"], vec![Reply::ok(&get)]);
        let report = fx.check(&agent, &want(AgentId::ClaudeCode, &[Feature::Mcp]));
        assert_eq!(report.result, CheckResult::Fail);
        assert_eq!(report.line, line);
        assert_eq!(outcome(&report, CheckStep::AgentCommand), StepOutcome::Fail);
    }
}

#[test]
fn claude_code_not_listed_and_settings_that_do_not_load() {
    let (fx, agent) = claude_fx();
    let missing = Reply::exit(1, "", "No MCP server found with name: mmhmem\n");
    fx.home
        .runner
        .on("claude", &["mcp", "get", "mmhmem"], vec![missing]);
    let report = fx.check(&agent, &want(AgentId::ClaudeCode, &[Feature::Mcp]));
    assert!(
        report
            .line
            .contains("does not list mmhmem among its MCP servers")
    );
    assert_eq!(outcome(&report, CheckStep::ConfigLoad), StepOutcome::Pass);
    let broken = Reply::exit(1, "", "Invalid settings in ~/.claude.json\n");
    fx.home
        .runner
        .on("claude", &["mcp", "get", "mmhmem"], vec![broken]);
    let report = fx.check(&agent, &want(AgentId::ClaudeCode, &[Feature::Mcp]));
    assert_eq!(report.result, CheckResult::Fail);
    assert!(report.line.starts_with(
        "✘ Claude Code — Claude Code could not load its settings after the change. \
         What you can do: open Claude Code"
    ));
    assert_eq!(outcome(&report, CheckStep::ConfigLoad), StepOutcome::Fail);
}

#[test]
fn claude_code_hook_sample_needs_the_redirect() {
    let (mut fx, agent) = claude_fx();
    let hooked = want(AgentId::ClaudeCode, &[Feature::Mcp, Feature::ReadHook]);
    fx.install(&agent, &hooked);
    let get = claude_get(&fx, "User config", "✔ Connected");
    fx.home
        .runner
        .on("claude", &["mcp", "get", "mmhmem"], vec![Reply::ok(&get)]);
    fx.home
        .runner
        .on("mmhmem", &["hook", "claude"], vec![Reply::ok(&deny())]);
    let report = fx.check(&agent, &hooked);
    assert_eq!(report.result, CheckResult::Pass, "{}", report.line);
    assert_eq!(outcome(&report, CheckStep::HookSample), StepOutcome::Pass);
    let call = fx.home.runner.calls().into_iter().last().expect("hook ran");
    assert_eq!(call.program, fx.bin);
    let event: Value = serde_json::from_slice(call.stdin.as_deref().unwrap()).unwrap();
    assert_eq!(event["tool_name"], "Read");
    assert_eq!(event["hook_event_name"], "PreToolUse");
    let sample = PathBuf::from(event["tool_input"]["file_path"].as_str().unwrap());
    assert_eq!(sample.parent(), call.cwd.as_deref());
    assert!(!sample.exists(), "the sample folder is removed afterwards");
    fx.home
        .runner
        .on("mmhmem", &["hook", "claude"], vec![Reply::ok("")]);
    let report = fx.check(&agent, &hooked);
    assert_eq!(report.result, CheckResult::Fail);
    assert!(
        report
            .line
            .contains("the read-routing hook let a long file read through")
    );
    assert_eq!(outcome(&report, CheckStep::HookSample), StepOutcome::Fail);
}

#[test]
fn claude_code_without_its_program_skips_the_agent_steps() {
    let mut fx = fx();
    let agent = claude_code::ClaudeCode::new(&fx.bin);
    fx.install(&agent, &want(AgentId::ClaudeCode, &[Feature::Mcp]));
    let report = fx.check(&agent, &want(AgentId::ClaudeCode, &[Feature::Mcp]));
    assert_eq!(report.result, CheckResult::Pass);
    assert_eq!(
        outcome(&report, CheckStep::AgentCommand),
        StepOutcome::Skipped
    );
    assert_eq!(
        outcome(&report, CheckStep::ConfigLoad),
        StepOutcome::Skipped
    );
}

pub(crate) fn codex_fx(features: &[Feature]) -> (Fx, codex::Codex) {
    let mut fx = fx();
    fx.home.install_program("codex");
    doubles_cli::codex(&fx.home.runner, &fx.home.env);
    let agent = codex::Codex::new(&fx.bin);
    fx.install(&agent, &want(AgentId::Codex, features));
    (fx, agent)
}

fn codex_lists(fx: &Fx, reply: Reply) {
    fx.home
        .runner
        .on("codex", &["mcp", "list", "--json"], vec![reply]);
}

#[test]
fn codex_passes_and_its_env_values_never_appear() {
    let (fx, agent) = codex_fx(&[Feature::Mcp]);
    codex_lists(&fx, Reply::ok(&codex_list(&fx, |_| {})));
    let report = fx.check(&agent, &want(AgentId::Codex, &[Feature::Mcp]));
    assert_eq!(report.result, CheckResult::Pass);
    assert_eq!(report.line, "✔ Codex — mmhmem connected, 10 tools");
    assert_eq!(outcome(&report, CheckStep::ConfigLoad), StepOutcome::Pass);
    let disabled = codex_list(&fx, |l| l[0]["enabled"] = json!(false));
    let other = codex_list(&fx, |l| l[0]["transport"]["args"] = json!(["serve"]));
    let cases = [
        (disabled, "mmhmem is switched off in Codex"),
        ("[]".to_owned(), "Codex does not list mmhmem"),
        (other, "Codex starts a different program for mmhmem"),
        ("{\"secret\": 1".to_owned(), "Codex answered in a form"),
    ];
    for (list, reason) in cases {
        codex_lists(&fx, Reply::exit(0, &list, SECRET));
        let report = fx.check(&agent, &want(AgentId::Codex, &[Feature::Mcp]));
        assert_eq!(report.result, CheckResult::Fail);
        assert!(
            report.line.starts_with(&format!("✘ Codex — {reason}")),
            "{}",
            report.line
        );
        for shown in [format!("{report:?}"), report.lines().join("\n")] {
            assert!(!shown.contains(SECRET), "{shown}");
        }
    }
    let calls = format!("{:?}", fx.home.runner.calls());
    assert!(!calls.contains(SECRET));
}

#[test]
fn codex_config_that_does_not_load_fails_both_steps() {
    let (fx, agent) = codex_fx(&[Feature::Mcp]);
    codex_lists(&fx, Reply::exit(1, "", "Error: config.toml line 3"));
    let report = fx.check(&agent, &want(AgentId::Codex, &[Feature::Mcp]));
    assert_eq!(report.result, CheckResult::Fail);
    assert_eq!(
        report.line,
        "✘ Codex — Codex could not load its settings after the change. What you can do: \
         open Codex to see which setting it rejects, fix it, then run the installer again."
    );
    assert_eq!(outcome(&report, CheckStep::AgentCommand), StepOutcome::Fail);
    assert_eq!(outcome(&report, CheckStep::ConfigLoad), StepOutcome::Fail);
}

#[test]
fn codex_hook_passes_with_its_approval_pending_and_a_wrong_answer_fails() {
    let features = [Feature::Mcp, Feature::ReadHook];
    let (fx, agent) = codex_fx(&features);
    codex_lists(&fx, Reply::ok(&codex_list(&fx, |_| {})));
    fx.home
        .runner
        .on("mmhmem", &["hook", "codex"], vec![Reply::ok(&deny())]);
    let report = fx.check(&agent, &want(AgentId::Codex, &features));
    assert_eq!(report.result, CheckResult::UserStepsPending);
    assert_eq!(
        report.lines(),
        [
            "✔ Codex — mmhmem connected, 10 tools",
            "● Codex — approve the mmhmem hook if you have not yet: open Codex, type /hooks, approve \"mmhmem read routing\"",
        ]
    );
    let event = fx.home.runner.calls().into_iter().last().unwrap();
    let event: Value = serde_json::from_slice(event.stdin.as_deref().unwrap()).unwrap();
    assert_eq!(event["tool_name"], "Bash");
    let wrong = json!({"decision": "pass"}).to_string();
    fx.home
        .runner
        .on("mmhmem", &["hook", "codex"], vec![Reply::ok(&wrong)]);
    let report = fx.check(&agent, &want(AgentId::Codex, &features));
    assert_eq!(report.result, CheckResult::Fail);
    assert!(report.pending.is_empty());
    assert!(
        report
            .line
            .contains("the read-routing hook gave an answer Codex")
    );
}
