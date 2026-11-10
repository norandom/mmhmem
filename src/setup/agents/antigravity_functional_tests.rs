//! Tests of Antigravity's functional check (antigravity-native-setup task
//! 3.5; Requirements 2.6, 4.1, 4.2 and 4.3): the registration read back from
//! the global server file, `agy mcp list` when the CLI is on PATH, and the
//! hook sample whenever the hook is selected and recorded. Temporary homes
//! and a fake `agy` only.

use std::sync::Arc;

use super::tests::{agent_record, run_apply, server_file, setup};
use super::*;
use crate::hook::Decision;
use crate::hook::adapters::{self, Harness};
use crate::setup::agents::doubles::{Reply, TestHome};
use crate::setup::agents::doubles_cli::{self, AGY_MCP_LIST_EMPTY};
use crate::setup::check::EXPECTED_TOOLS;
use crate::setup::check::agent::ServerProbe;
use crate::setup::record::{CheckResult, CheckStep, InstallRecord, StepOutcome, UtcTimestamp};

const ID: AgentId = AgentId::Antigravity;

/// Every step of a check, in report order.
const STEPS: [CheckStep; 7] = [
    CheckStep::Registration,
    CheckStep::Handshake,
    CheckStep::ToolsList,
    CheckStep::StatusTool,
    CheckStep::AgentCommand,
    CheckStep::HookSample,
    CheckStep::ConfigLoad,
];

const INACTIVE: &str = "● Antigravity — the read-routing hook is installed but stays inactive \
                        until a worker model is set up (choose \"Worker model\" in the menu)";

fn want(features: &[Feature]) -> Selection {
    Selection::new().with(ID, features)
}

fn both() -> [Feature; 2] {
    [Feature::Mcp, Feature::ReadHook]
}

/// `rec` with `want`'s features and pending steps, as the verify step
/// builds it before the check.
fn for_check(mut rec: AgentRecord, agent: &Antigravity, want: &Selection) -> AgentRecord {
    for feature in agent.features() {
        rec.features.insert(*feature, want.wants(ID, *feature));
    }
    rec.pending_user_steps = agent.pending_steps(want);
    rec
}

/// Plans and applies `features` as setup does; the record entry the check
/// gets.
fn installed(home: &TestHome, agent: &Antigravity, features: &[Feature]) -> AgentRecord {
    let want = want(features);
    let changes = agent.plan(&home.env, &want, None).expect("plan");
    let mut record = InstallRecord::new();
    run_apply(home, changes, &mut record);
    for_check(agent_record(&record), agent, &want)
}

/// `agy` on PATH, answering as captured (mmhmem listed).
fn with_agy(home: &TestHome) {
    home.install_program("agy");
    doubles_cli::agy(&home.runner);
}

/// The installed `mmhmem hook antigravity` prints `stdout`.
fn hook_prints(home: &TestHome, stdout: &str) {
    let args = ["hook", "antigravity"];
    home.runner.on("mmhmem", &args, vec![Reply::ok(stdout)]);
}

/// What the real hook prints for a redirect of the sample.
fn redirect() -> String {
    let file = Path::new("/work").join("mmhmem-sample.txt");
    let threshold = crate::config::DEFAULT_SHUNT_MIN_LINES;
    let lines = u64::from(threshold);
    let redirect = Decision::Redirect { lines };
    adapters::render(Harness::Antigravity, &redirect, &file, "p", threshold)
}

fn outcome(report: &CheckReport, step: CheckStep) -> StepOutcome {
    report
        .details
        .iter()
        .find(|d| d.step == step)
        .map(|d| d.outcome)
        .expect("every step is reported")
}

fn passed() -> String {
    format!(
        "✔ Antigravity — mmhmem connected, {} tools",
        EXPECTED_TOOLS.len()
    )
}

/// The record's `last_check.details` as saved.
fn saved_details(report: &CheckReport) -> Value {
    let at = UtcTimestamp::try_from("2026-11-10T09:00:00Z".to_owned()).unwrap();
    serde_json::to_value(report.last_check(at)).unwrap()["details"].clone()
}

/// Points mmhmem's `command` in the server file at `program`.
fn set_command(home: &TestHome, program: &Path) {
    let file = server_file(home);
    let text = std::fs::read_to_string(&file).unwrap();
    let value = json!(program.to_string_lossy());
    let text = crate::setup::edit::json::set(&text, &COMMAND, &value).unwrap();
    std::fs::write(&file, text).unwrap();
}

#[test]
fn agy_listing_mmhmem_passes_the_agent_command_step() {
    let (home, agent) = setup();
    let rec = installed(&home, &agent, &[Feature::Mcp]);
    with_agy(&home);
    let report = agent.check(&home.env, &rec);
    assert_eq!(report.result, CheckResult::Pass, "{}", report.line);
    assert_eq!(report.lines(), [passed()]);
    assert_eq!(outcome(&report, CheckStep::AgentCommand), StepOutcome::Pass);
    assert_eq!(
        outcome(&report, CheckStep::HookSample),
        StepOutcome::Skipped
    );
    let calls = home.runner.calls();
    assert_eq!(home.runner.args_of("agy"), [["mcp", "list"]]);
    assert_eq!(calls[0].cwd.as_deref(), Some(home.home()));
    let seen = home.probe.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].program, agent.binary);
    assert_eq!(seen[0].args, ["stdio"]);
    let details = saved_details(&report);
    let step = json!({"step": "agent_command", "outcome": "pass"});
    assert!(details.as_array().unwrap().contains(&step), "{details}");
}

#[test]
fn an_agy_listing_without_mmhmem_fails_naming_antigravitys_list() {
    let (home, agent) = setup();
    let rec = installed(&home, &agent, &[Feature::Mcp]);
    with_agy(&home);
    let list = |reply| home.runner.on("agy", &["mcp", "list"], vec![reply]);
    let not_listed = "✘ Antigravity — Antigravity does not list mmhmem among its MCP servers. \
                      What you can do: restart Antigravity, then run the installer again.";
    // A server whose command merely contains "mmhmem" is not mmhmem's line.
    let foreign = "NAME   TYPE   STATUS   COMMAND/URL\n\
                   other  stdio  enabled  C:\\tools\\mmhmem.exe stdio\n";
    for listing in [AGY_MCP_LIST_EMPTY, foreign] {
        list(Reply::ok(listing));
        let report = agent.check(&home.env, &rec);
        assert_eq!(report.result, CheckResult::Fail, "{listing}");
        assert_eq!(report.line, not_listed, "{listing}");
        assert_eq!(outcome(&report, CheckStep::AgentCommand), StepOutcome::Fail);
        assert!(report.notes.is_empty(), "{:?}", report.notes);
    }
    // agy that fails: Antigravity could not load its settings.
    list(Reply::exit(1, "", "agy-stderr-text"));
    let report = agent.check(&home.env, &rec);
    assert_eq!(
        report.line,
        "✘ Antigravity — Antigravity could not load its settings after the change. What you \
         can do: open Antigravity to see which setting it rejects, fix it, then run the \
         installer again."
    );
    assert!(!format!("{report:?}").contains("agy-stderr-text"));
    // agy that never answers.
    list(Reply::TimedOut);
    let report = agent.check(&home.env, &rec);
    assert_eq!(report.result, CheckResult::Fail);
    assert!(
        report
            .line
            .starts_with("✘ Antigravity — the agy program did not answer within "),
        "{}",
        report.line
    );
}

#[test]
fn without_agy_the_listing_is_skipped_and_the_report_says_why() {
    let (home, agent) = setup();
    let rec = installed(&home, &agent, &[Feature::Mcp]);
    // Scripted but not on PATH: never run.
    doubles_cli::agy(&home.runner);
    let report = agent.check(&home.env, &rec);
    assert_eq!(report.result, CheckResult::Pass, "{}", report.line);
    assert_eq!(
        report.lines(),
        [
            passed(),
            "● Antigravity — Antigravity CLI not found; checked the config file only".to_owned(),
        ]
    );
    assert_eq!(
        outcome(&report, CheckStep::AgentCommand),
        StepOutcome::Skipped
    );
    assert!(home.runner.calls().is_empty());
    let details = saved_details(&report);
    let step = json!({"step": "agent_command", "outcome": "skipped"});
    assert!(details.as_array().unwrap().contains(&step), "{details}");
}

#[test]
fn the_check_starts_what_the_server_file_names_not_the_plan() {
    let (home, agent) = setup();
    let rec = installed(&home, &agent, &[Feature::Mcp]);
    let other = home.bin().join("other").join("mmhmem.exe");
    set_command(&home, &other);
    agent.check(&home.env, &rec);
    let seen: Vec<PathBuf> = home.probe.seen().into_iter().map(|r| r.program).collect();
    assert_eq!(seen, [other]);
}

#[test]
fn a_server_file_naming_a_wrong_program_fails_the_registration_step() {
    let (home, agent) = setup();
    let rec = installed(&home, &agent, &both());
    with_agy(&home);
    hook_prints(&home, &redirect());
    let wrong = home.bin().join("gone").join("mmhmem.exe");
    set_command(&home, &wrong);
    // The real MCP check: that program is not there, so nothing starts.
    let env = home.env.clone().with_mcp_probe(Arc::new(ServerProbe));
    let report = agent.check(&env, &rec);
    assert_eq!(report.result, CheckResult::Fail);
    assert_eq!(
        report.line,
        format!(
            "✘ Antigravity — mmhmem did not start: the program at \"{}\" does not exist. What \
             you can do: run the installer again.",
            wrong.display()
        )
    );
    for step in STEPS {
        let want = if step == CheckStep::Registration {
            StepOutcome::Fail
        } else {
            StepOutcome::Skipped
        };
        assert_eq!(outcome(&report, step), want, "{step:?}");
    }
    assert!(report.notes.is_empty(), "{:?}", report.notes);
    assert!(home.runner.calls().is_empty(), "nothing else runs");
}

#[test]
fn a_missing_or_unreadable_server_file_fails_without_its_text() {
    let (home, agent) = setup();
    with_agy(&home);
    let rec = for_check(AgentRecord::default(), &agent, &want(&[Feature::Mcp]));
    let none = "the agent's settings have no entry for mmhmem";
    let cases = [
        (None, none),
        (Some(r#"{"mcpServers": {"other": {"command": "x"}}}"#), none),
        (
            Some("{\"mcpServers\": SECRET-TEXT-7f3 not json"),
            "the agent's settings file could not be read",
        ),
    ];
    for (text, reason) in cases {
        if let Some(text) = text {
            home.write(&[".gemini", "config", "mcp_config.json"], text);
        }
        let report = agent.check(&home.env, &rec);
        assert_eq!(report.result, CheckResult::Fail, "{text:?}");
        assert_eq!(
            report.line,
            format!(
                "✘ Antigravity — mmhmem did not start: {reason}. What you can do: run the \
                 installer again."
            )
        );
        assert!(!format!("{report:?}").contains("SECRET-TEXT"));
        assert_eq!(outcome(&report, CheckStep::Registration), StepOutcome::Fail);
        assert_eq!(
            outcome(&report, CheckStep::AgentCommand),
            StepOutcome::Skipped
        );
    }
    assert!(home.probe.seen().is_empty());
    assert!(home.runner.calls().is_empty());
}

#[test]
fn the_hook_sample_runs_without_a_worker_and_the_inactive_line_stays() {
    let (home, agent) = setup();
    let rec = installed(&home, &agent, &both());
    with_agy(&home);
    let no_worker = home.env.clone().with_worker_configured(false);
    hook_prints(&home, adapters::fail_open_output(Harness::Antigravity));
    let report = agent.check(&no_worker, &rec);
    assert_eq!(report.result, CheckResult::Pass, "{}", report.line);
    assert_eq!(report.lines(), [passed(), INACTIVE.to_owned()]);
    assert_eq!(outcome(&report, CheckStep::HookSample), StepOutcome::Pass);
    assert_eq!(home.runner.args_of("mmhmem"), [["hook", "antigravity"]]);
    let details = saved_details(&report);
    let step = json!({"step": "hook_sample", "outcome": "pass"});
    assert!(details.as_array().unwrap().contains(&step), "{details}");

    // An answer that would block every read_file fails, with no note.
    hook_prints(&home, "{}");
    let report = agent.check(&no_worker, &rec);
    assert_eq!(report.result, CheckResult::Fail);
    assert_eq!(
        report.line,
        "✘ Antigravity — the read-routing hook could not be run. What you can do: run the \
         installer again."
    );
    assert_eq!(outcome(&report, CheckStep::HookSample), StepOutcome::Fail);
    assert!(report.notes.is_empty(), "{:?}", report.notes);

    // With a worker model the hook must redirect; no inactive line.
    hook_prints(&home, &redirect());
    let report = agent.check(&home.env, &rec);
    assert_eq!(report.result, CheckResult::Pass, "{}", report.line);
    assert_eq!(report.lines(), [passed()]);
    assert_eq!(outcome(&report, CheckStep::HookSample), StepOutcome::Pass);
}

#[test]
fn a_selected_hook_that_was_left_out_is_skipped_with_a_reason() {
    let (home, agent) = setup();
    // Only the server entry was written and recorded: the hook was left out.
    let rec = installed(&home, &agent, &[Feature::Mcp]);
    let rec = for_check(rec, &agent, &want(&both()));
    with_agy(&home);
    hook_prints(&home, adapters::fail_open_output(Harness::Antigravity));
    let left_out = "● Antigravity — the read-routing hook was left out, so it was not checked";
    for worker in [true, false] {
        let env = home.env.clone().with_worker_configured(worker);
        let report = agent.check(&env, &rec);
        assert_eq!(report.result, CheckResult::Pass, "{}", report.line);
        assert_eq!(report.lines(), [passed(), left_out.to_owned()], "{worker}");
        assert_eq!(
            outcome(&report, CheckStep::HookSample),
            StepOutcome::Skipped
        );
    }
    assert!(home.runner.args_of("mmhmem").is_empty());
}
