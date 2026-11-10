//! Tests of the check coordinator (reliable-installers task 4.2): stored
//! results, rollback of a failed check with files restored byte-identically
//! and the record put back, no rollback for pending user steps, re-checks
//! and agents the apply skipped. Temporary homes and the CLI doubles only.

use std::path::Path;

use super::*;
use crate::setup::agents::check_tests::{Fx, claude_get, codex_list, deny, fx};
use crate::setup::agents::doubles::Reply;
use crate::setup::agents::doubles_cli;
use crate::setup::agents::{Feature, Registry};
use crate::setup::plan::{Plan, apply};
use crate::setup::record::{CheckStep, PendingUserStep, StepOutcome};

const CLAUDE_SEED: &str = "{\n  \"numStartups\": 7,\n  \"mcpServers\": {}\n}\n";
const CODEX_SEED: &str = "# mine\nmodel = \"o3\"\n";
const OPENCODE_SEED: &str = "{\n  // keep me\n  \"theme\": \"dark\"\n}\n";

fn run_time() -> UtcTimestamp {
    UtcTimestamp::try_from("2026-11-04T11:00:00Z".to_owned()).expect("time")
}

/// Plans `want` for every agent of `registry`, applies it and runs the
/// checker; returns the apply report (after rollbacks) and the verdicts.
fn install_and_check(
    fx: &mut Fx,
    registry: &Registry,
    want: &Selection,
) -> (ApplyReport, VerifyReport) {
    let mut changes = Vec::new();
    for id in want.agents() {
        let agent = registry.get(id).expect("registered");
        let rec = fx.record.agents.get(&id);
        changes.extend(agent.plan(&fx.home.env, want, rec).expect("plan"));
    }
    let plan = Plan {
        run: run_time(),
        changes,
    };
    let mut applied = apply(&plan, &fx.store, &mut fx.record);
    let checker = Checker {
        registry,
        env: &fx.home.env,
        store: &fx.store,
    };
    let verdicts = checker.run(want, &mut applied, &mut fx.record);
    (applied, verdicts)
}

fn scripted() -> (Fx, Registry) {
    let fx = fx();
    for program in ["claude", "codex", "opencode"] {
        fx.home.install_program(program);
    }
    doubles_cli::all(&fx.home.runner, &fx.home.env);
    let registry = Registry::builtin(&fx.bin);
    (fx, registry)
}

fn read(file: &Path) -> Option<Vec<u8>> {
    std::fs::read(file).ok()
}

#[test]
fn passing_and_pending_checks_are_stored_without_rollback() {
    let (mut fx, registry) = scripted();
    let get = claude_get(&fx, "User config", "✔ Connected");
    let runner = &fx.home.runner;
    runner.on("claude", &["mcp", "get", "mmhmem"], vec![Reply::ok(&get)]);
    let list = codex_list(&fx, |_| {});
    runner.on("codex", &["mcp", "list", "--json"], vec![Reply::ok(&list)]);
    runner.on("mmhmem", &["hook", "codex"], vec![Reply::ok(&deny())]);
    let want = Selection::new()
        .with(AgentId::ClaudeCode, &[Feature::Mcp])
        .with(AgentId::Codex, &[Feature::Mcp, Feature::ReadHook]);
    let (applied, verdicts) = install_and_check(&mut fx, &registry, &want);
    assert!(verdicts.all_passed());
    assert!(verdicts.record_errors.is_empty());
    // Ten tools: shunt-content-tool Req 6.1 added shunt_content; the
    // 2026-11-10 restore removed the batch summary tool.
    assert_eq!(
        verdicts.lines(),
        [
            "✔ Claude Code — mmhmem connected, 10 tools",
            "✔ Codex — mmhmem connected, 10 tools",
            "● Codex — approve the mmhmem hook if you have not yet: open Codex, type /hooks, approve \"mmhmem read routing\"",
        ]
    );
    for v in &verdicts.agents {
        assert_eq!(v.rollback, Rollback::NotNeeded);
        assert_eq!(
            applied.agent(Scope::Agent(v.agent())).unwrap().outcome,
            Outcome::Applied
        );
    }
    let saved = fx.store.load().expect("load").expect("record");
    assert_eq!(saved, fx.record);
    let codex = &saved.agents[&AgentId::Codex];
    let last = codex.last_check.as_ref().expect("last check");
    assert_eq!(last.result, CheckResult::UserStepsPending);
    assert!(last.details.contains(&crate::setup::record::CheckDetail {
        step: CheckStep::HookSample,
        outcome: StepOutcome::Pass,
    }));
    assert_eq!(
        codex.pending_user_steps,
        [PendingUserStep::ApproveHookInCodex]
    );
    assert_eq!(codex.features.get(&Feature::ReadHook), Some(&true));
    assert_eq!(codex.features.get(&Feature::Skill), Some(&false));
    let claude = &saved.agents[&AgentId::ClaudeCode];
    assert_eq!(
        claude.last_check.as_ref().unwrap().result,
        CheckResult::Pass
    );
    assert!(fx.home.home().join(".codex/config.toml").exists());
}

/// `id` failed, was rolled back with the plain reason, and `file` holds
/// exactly `seed` again.
fn assert_restored(
    verdicts: &VerifyReport,
    applied: &ApplyReport,
    id: AgentId,
    file: &Path,
    seed: &str,
) {
    let v = verdicts.agent(id).expect("checked");
    assert_eq!(v.report.result, CheckResult::Fail);
    assert_eq!(v.rollback, Rollback::Restored);
    let lines = v.lines();
    let label = id.label();
    assert!(lines[0].starts_with(&format!("✘ {label} — ")), "{lines:?}");
    let reason = format!("Restored your original {label} settings.");
    assert_eq!(lines.last(), Some(&reason));
    let outcome = &applied.agent(Scope::Agent(id)).expect("applied").outcome;
    assert_eq!(outcome, &Outcome::RolledBack { reason });
    assert_eq!(read(file).as_deref(), Some(seed.as_bytes()), "{id:?}");
}

#[test]
fn a_failed_check_restores_every_file_and_the_record() {
    let (mut fx, registry) = scripted();
    let claude_json = fx.home.write(&[".claude.json"], CLAUDE_SEED);
    let codex_toml = fx.home.write(&[".codex", "config.toml"], CODEX_SEED);
    let opencode = fx
        .home
        .write(&[".config", "opencode", "opencode.json"], OPENCODE_SEED);
    let runner = &fx.home.runner;
    let not_connected = claude_get(&fx, "User config", "✘ Failed to connect");
    runner.on(
        "claude",
        &["mcp", "get", "mmhmem"],
        vec![Reply::ok(&not_connected)],
    );
    let broken = Reply::exit(1, "", "Error loading config.toml");
    runner.on("codex", &["mcp", "list", "--json"], vec![broken]);
    let before = fx.record.clone();
    let want = Selection::new()
        .with(AgentId::ClaudeCode, &[Feature::Mcp, Feature::Skill])
        .with(AgentId::Codex, &[Feature::Mcp])
        .with(AgentId::OpenCode, &[Feature::Mcp]);
    let mut changes = Vec::new();
    for id in want.agents() {
        let agent = registry.get(id).unwrap();
        changes.extend(agent.plan(&fx.home.env, &want, None).expect("plan"));
    }
    let mut applied = apply(
        &Plan {
            run: run_time(),
            changes,
        },
        &fx.store,
        &mut fx.record,
    );
    // OpenCode's file no longer parses after the write.
    std::fs::write(&opencode, "{ broken").unwrap();
    let checker = Checker {
        registry: &registry,
        env: &fx.home.env,
        store: &fx.store,
    };
    let verdicts = checker.run(&want, &mut applied, &mut fx.record);
    assert!(!verdicts.all_passed());
    for (id, file, seed) in [
        (AgentId::ClaudeCode, &claude_json, CLAUDE_SEED),
        (AgentId::Codex, &codex_toml, CODEX_SEED),
        (AgentId::OpenCode, &opencode, OPENCODE_SEED),
    ] {
        assert_restored(&verdicts, &applied, id, file, seed);
    }
    let skill = fx
        .home
        .home()
        .join(".claude/skills/unified-memory/SKILL.md");
    assert!(
        !skill.exists(),
        "a new file of a rolled-back agent is removed"
    );
    assert_eq!(fx.record, before);
    let saved = fx.store.load().expect("load").expect("record");
    assert_eq!(saved, before);
}

#[test]
fn pending_user_steps_never_roll_back() {
    let (mut fx, registry) = scripted();
    let config = fx.home.home().join(".config").join("Claude");
    std::fs::create_dir_all(&config).unwrap();
    let want = Selection::new().with(AgentId::ClaudeDesktop, &[Feature::Mcp]);
    let (applied, verdicts) = install_and_check(&mut fx, &registry, &want);
    let v = verdicts.agent(AgentId::ClaudeDesktop).unwrap();
    assert_eq!(v.report.result, CheckResult::UserStepsPending);
    assert_eq!(v.rollback, Rollback::NotNeeded);
    let scope = Scope::Agent(AgentId::ClaudeDesktop);
    assert_eq!(applied.agent(scope).unwrap().outcome, Outcome::Applied);
    assert!(config.join("claude_desktop_config.json").exists());
    let rec = &fx.record.agents[&AgentId::ClaudeDesktop];
    assert_eq!(
        rec.pending_user_steps,
        [PendingUserStep::RestartClaudeDesktop]
    );
}

#[test]
fn a_failed_recheck_without_changes_stores_the_failure() {
    let (mut fx, registry) = scripted();
    let list = codex_list(&fx, |_| {});
    let runner = fx.home.runner.clone();
    runner.on("codex", &["mcp", "list", "--json"], vec![Reply::ok(&list)]);
    let want = Selection::new().with(AgentId::Codex, &[Feature::Mcp]);
    let (_, first) = install_and_check(&mut fx, &registry, &want);
    assert!(first.all_passed());
    runner.on("codex", &["mcp", "list", "--json"], vec![Reply::ok("[]")]);
    let config = fx.home.home().join(".codex/config.toml");
    let installed = read(&config);
    let (applied, again) = install_and_check(&mut fx, &registry, &want);
    assert!(applied.agents.is_empty(), "nothing to change");
    let v = again.agent(AgentId::Codex).unwrap();
    assert_eq!(v.rollback, Rollback::NothingToRestore);
    assert_eq!(v.lines().len(), 1);
    assert_eq!(read(&config), installed);
    let last = fx.record.agents[&AgentId::Codex]
        .last_check
        .clone()
        .unwrap();
    assert_eq!(last.result, CheckResult::Fail);
}

#[test]
fn agents_the_apply_skipped_are_not_checked() {
    let (mut fx, registry) = scripted();
    let want = Selection::new().with(AgentId::Codex, &[Feature::Mcp]);
    let changes = registry
        .get(AgentId::Codex)
        .unwrap()
        .plan(&fx.home.env, &want, None)
        .unwrap();
    fx.home
        .write(&[".codex", "config.toml"], "changed = true\n");
    let mut applied = apply(
        &Plan {
            run: run_time(),
            changes,
        },
        &fx.store,
        &mut fx.record,
    );
    let checker = Checker {
        registry: &registry,
        env: &fx.home.env,
        store: &fx.store,
    };
    let verdicts = checker.run(&want, &mut applied, &mut fx.record);
    assert!(verdicts.agents.is_empty());
    assert!(fx.home.probe.seen().is_empty());
}

fn hooked_codex(worker: bool, hook_answer: &str) -> (Fx, Registry, ApplyReport, VerifyReport) {
    let (mut fx, registry) = scripted();
    fx.home.env = fx.home.env.clone().with_worker_configured(worker);
    let list = codex_list(&fx, |_| {});
    let runner = &fx.home.runner;
    runner.on("codex", &["mcp", "list", "--json"], vec![Reply::ok(&list)]);
    runner.on("mmhmem", &["hook", "codex"], vec![Reply::ok(hook_answer)]);
    let want = Selection::new().with(AgentId::Codex, &[Feature::Mcp, Feature::ReadHook]);
    let (applied, verdicts) = install_and_check(&mut fx, &registry, &want);
    (fx, registry, applied, verdicts)
}

#[test]
fn a_hook_without_a_worker_model_is_skipped_noted_and_kept() {
    let (fx, _registry, applied, verdicts) = hooked_codex(false, "");
    let v = verdicts.agent(AgentId::Codex).unwrap();
    assert_eq!(v.report.result, CheckResult::UserStepsPending);
    assert_eq!(v.rollback, Rollback::NotNeeded);
    let hook = v
        .report
        .details
        .iter()
        .find(|d| d.step == CheckStep::HookSample);
    assert_eq!(hook.unwrap().outcome, StepOutcome::Skipped);
    assert_eq!(
        v.lines(),
        [
            "✔ Codex — mmhmem connected, 10 tools",
            "● Codex — approve the mmhmem hook if you have not yet: open Codex, type /hooks, approve \"mmhmem read routing\"",
            "● Codex — the read-routing hook is installed but stays inactive until a worker model is set up (choose \"Worker model\" in the menu)",
        ]
    );
    assert!(fx.home.runner.args_of("mmhmem").is_empty(), "no sample ran");
    let scope = Scope::Agent(AgentId::Codex);
    assert_eq!(applied.agent(scope).unwrap().outcome, Outcome::Applied);
    let rec = &fx.record.agents[&AgentId::Codex];
    assert!(rec.managed.iter().any(|e| matches!(
        e.target,
        crate::setup::record::Target::JsonArrayElement { .. }
    )));
    assert_eq!(rec.features.get(&Feature::ReadHook), Some(&true));
    assert!(fx.home.home().join(".codex/hooks.json").exists());
}

#[test]
fn a_wrong_hook_answer_with_a_worker_model_fails_and_rolls_back() {
    let (fx, _registry, applied, verdicts) = hooked_codex(true, "{\"decision\":\"pass\"}");
    let v = verdicts.agent(AgentId::Codex).unwrap();
    assert_eq!(v.report.result, CheckResult::Fail);
    assert_eq!(v.rollback, Rollback::Restored);
    assert!(v.report.notes.is_empty());
    let scope = Scope::Agent(AgentId::Codex);
    assert!(matches!(
        applied.agent(scope).unwrap().outcome,
        Outcome::RolledBack { .. }
    ));
    assert!(!fx.home.home().join(".codex/hooks.json").exists());
    assert!(!fx.record.agents.contains_key(&AgentId::Codex));
}
