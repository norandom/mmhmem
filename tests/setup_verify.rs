//! The installer's agent checks end to end (reliable-installers task 4.2;
//! Requirements 3.5 and 6.1–6.6): real Claude Code, Codex and OpenCode
//! configs written by `plan::apply` in a temporary home, pointing at the
//! real `mmhmem` binary. The MCP step starts that binary from the bytes on
//! disk and the hook sample runs the real `mmhmem hook <harness>`; only the
//! agent CLIs (`claude`, `codex`, `opencode`) are doubles, answering from
//! the files the plan wrote. A failed check restores every file.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use mmhmem::setup::agents::{
    AgentId, CommandOutput, CommandRunner, Env, Feature, Invocation, Registry, RunError, ScanEnv,
    Selection, SystemRunner,
};
use mmhmem::setup::check::Registration;
use mmhmem::setup::plan::{Outcome, Plan, apply};
use mmhmem::setup::record::{
    CheckResult, CheckStep, InstallRecord, Os, RecordStore, Scope, StepOutcome, UtcTimestamp,
};
use mmhmem::setup::verify::{Checker, Rollback};
use serde_json::{Value, json};

const BIN: &str = env!("CARGO_BIN_EXE_mmhmem");
const SECRET: &str = "verify-test-secret-3f0a";

struct Home {
    dir: tempfile::TempDir,
    env: Env,
    store: RecordStore,
}

impl Home {
    fn path(&self, rel: &str) -> PathBuf {
        rel.split('/')
            .fold(self.dir.path().join("home"), |p, s| p.join(s))
    }

    fn write(&self, rel: &str, text: &str) -> PathBuf {
        let file = self.path(rel);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, text).unwrap();
        file
    }

    fn data(&self) -> PathBuf {
        self.dir.path().join("data")
    }
}

/// The agent CLIs, answering from the files on disk; every other program
/// (the mmhmem hook) really runs, with a private store and a worker model
/// set so the hook may redirect.
struct Clis {
    home: PathBuf,
    data: PathBuf,
    codex_loads: bool,
}

impl Clis {
    fn read(&self, rel: &str) -> Vec<u8> {
        std::fs::read(rel.split('/').fold(self.home.clone(), |p, s| p.join(s))).unwrap()
    }

    fn ok(stdout: String) -> Result<CommandOutput, RunError> {
        Ok(CommandOutput {
            code: Some(0),
            stdout,
            stderr: String::new(),
        })
    }

    fn claude(&self) -> Result<CommandOutput, RunError> {
        let reg = Registration::from_claude_json(&self.read(".claude.json"), "mmhmem").unwrap();
        Self::ok(format!(
            "mmhmem:\n  Scope: User config (available in all your projects)\n  Status: ✔ \
             Connected\n  Type: stdio\n  Command: {}\n  Args: {}\n",
            reg.program.display(),
            reg.args.join(" ")
        ))
    }

    fn codex(&self) -> Result<CommandOutput, RunError> {
        if !self.codex_loads {
            return Ok(CommandOutput {
                code: Some(1),
                stdout: String::new(),
                stderr: "Error loading config.toml".to_owned(),
            });
        }
        let reg =
            Registration::from_codex_toml(&self.read(".codex/config.toml"), "mmhmem").unwrap();
        let list = json!([{"name": "mmhmem", "enabled": true, "transport": {"type": "stdio",
            "command": reg.program, "args": reg.args, "env": {"API_KEY": SECRET}}}]);
        Self::ok(list.to_string())
    }
}

impl CommandRunner for Clis {
    fn run(&self, invocation: &Invocation) -> Result<CommandOutput, RunError> {
        match invocation.program_name().as_str() {
            "claude" => self.claude(),
            "codex" => self.codex(),
            "opencode" => Self::ok("✓ mmhmem       connected\n".to_owned()),
            _ => {
                let real = invocation
                    .clone()
                    .env("AGENT_MEM_DATA_DIR", self.data.to_string_lossy())
                    .env("MMHMEM_NO_UPDATE_CHECK", "1")
                    .env("WORKER_MODEL", "verify-test-model")
                    .env("WORKER_API_URL", "")
                    .env("WORKER_API_KEY", "");
                SystemRunner.run(&real)
            }
        }
    }
}

fn home(codex_loads: bool) -> Home {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let bin = dir.path().join("bin");
    let data = dir.path().join("data");
    for d in [&home, &bin, &data] {
        std::fs::create_dir_all(d).unwrap();
    }
    for name in ["claude", "codex", "opencode"] {
        let file = bin.join(if cfg!(windows) {
            format!("{name}.exe")
        } else {
            name.to_owned()
        });
        std::fs::write(&file, b"").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }
    let mut dirs = ScanEnv::new(Os::current(), &home);
    dirs.appdata = Some(home.join("AppData").join("Roaming"));
    dirs.local_app_data = Some(home.join("AppData").join("Local"));
    dirs.user_path = Some(vec![bin]);
    let clis = Clis {
        home,
        data,
        codex_loads,
    };
    let env = Env::new(dirs, Arc::new(clis)).with_worker_configured(true);
    let store = RecordStore::new(dir.path().join("state"));
    Home { dir, env, store }
}

/// The isolating environment the agents' entries carry, so the server the
/// check starts never touches the real store (as `tests/setup_check.rs`).
fn isolation(data: &Path) -> Value {
    json!({"AGENT_MEM_DATA_DIR": data, "WORKER_API_URL": "", "WORKER_MODEL": "",
        "WORKER_API_KEY": "", "MMHMEM_VERIFY_TEST_SECRET": SECRET})
}

/// Seeds the three agents' configs with an old mmhmem entry that carries
/// the isolating environment; returns the files and their seed text.
fn seed(home: &Home) -> Vec<(PathBuf, String)> {
    let env = isolation(&home.data());
    let claude = json!({"numStartups": 3, "mcpServers": {"mmhmem":
        {"type": "stdio", "command": "old-mmhmem", "args": ["stdio"], "env": env}}});
    let opencode = json!({"theme": "dark", "mcp": {"servers": {"mmhmem":
        {"type": "local", "command": ["old-mmhmem", "stdio"], "environment": env}}}});
    let mut codex = String::from(
        "model = \"o3\"\n\n[mcp_servers.mmhmem]\ncommand = \"old-mmhmem\"\nargs = [\"stdio\"]\n\n[mcp_servers.mmhmem.env]\n",
    );
    for (key, value) in env.as_object().unwrap() {
        codex.push_str(&format!("{key} = '{}'\n", value.as_str().unwrap()));
    }
    let seeds = [
        (".claude.json", format!("{claude:#}\n")),
        (".codex/config.toml", codex),
        (".config/opencode/opencode.json", format!("{opencode:#}\n")),
    ];
    seeds
        .into_iter()
        .map(|(rel, text)| (home.write(rel, &text), text))
        .collect()
}

fn selection() -> Selection {
    let features = [Feature::Mcp, Feature::ReadHook];
    Selection::new()
        .with(AgentId::ClaudeCode, &features)
        .with(AgentId::Codex, &features)
        .with(AgentId::OpenCode, &features)
}

fn install(
    home: &Home,
    registry: &Registry,
    record: &mut InstallRecord,
) -> mmhmem::setup::plan::ApplyReport {
    let want = selection();
    let mut changes = Vec::new();
    for id in want.agents() {
        let agent = registry.get(id).unwrap();
        changes.extend(agent.plan(&home.env, &want, None).unwrap());
    }
    let run = UtcTimestamp::now();
    apply(&Plan { run, changes }, &home.store, record)
}

#[test]
fn real_configs_and_the_real_server_pass_with_codex_approval_pending() {
    let home = home(true);
    seed(&home);
    let registry = Registry::builtin(Path::new(BIN));
    let mut record = InstallRecord::new();
    let mut applied = install(&home, &registry, &mut record);
    let checker = Checker {
        registry: &registry,
        env: &home.env,
        store: &home.store,
    };
    let verdicts = checker.run(&selection(), &mut applied, &mut record);
    let lines = verdicts.lines();
    // shunt-content-tool Req 6.1: eleven tools with shunt_content.
    assert_eq!(
        lines,
        [
            "✔ Claude Code — mmhmem connected, 11 tools",
            "✔ Codex — mmhmem connected, 11 tools",
            "● Codex — approve the mmhmem hook if you have not yet: open Codex, type /hooks, approve \"mmhmem read routing\"",
            "✔ OpenCode — mmhmem connected, 11 tools",
        ],
        "{verdicts:#?}"
    );
    for verdict in &verdicts.agents {
        assert_eq!(verdict.rollback, Rollback::NotNeeded);
        for step in [
            CheckStep::Handshake,
            CheckStep::StatusTool,
            CheckStep::HookSample,
        ] {
            let detail = verdict
                .report
                .details
                .iter()
                .find(|d| d.step == step)
                .unwrap();
            assert_eq!(
                detail.outcome,
                StepOutcome::Pass,
                "{:?} {step:?}",
                verdict.agent()
            );
        }
        assert!(!format!("{verdict:?}").contains(SECRET));
    }
    let saved = home.store.load().unwrap().unwrap();
    let codex = saved.agents[&AgentId::Codex].last_check.as_ref().unwrap();
    assert_eq!(codex.result, CheckResult::UserStepsPending);
    assert!(home.path(".codex/hooks.json").exists());
}

#[test]
fn a_failing_check_rolls_back_to_the_seeded_bytes() {
    let home = home(false);
    let seeds = seed(&home);
    let registry = Registry::builtin(Path::new(BIN));
    let mut record = InstallRecord::new();
    let before = record.clone();
    let mut applied = install(&home, &registry, &mut record);
    // Claude Code's entry now names a program that does not exist: the MCP
    // step reads the file, not the plan.
    let claude_json = home.path(".claude.json");
    let text = std::fs::read_to_string(&claude_json).unwrap();
    let bin = serde_json::to_string(BIN).unwrap();
    std::fs::write(
        &claude_json,
        text.replace(&bin, "\"missing-mmhmem-program\""),
    )
    .unwrap();
    let checker = Checker {
        registry: &registry,
        env: &home.env,
        store: &home.store,
    };
    let verdicts = checker.run(&selection(), &mut applied, &mut record);
    let claude = verdicts.agent(AgentId::ClaudeCode).unwrap();
    assert!(
        claude.lines()[0].starts_with("✘ Claude Code — mmhmem did not start: "),
        "{:?}",
        claude.lines()
    );
    let codex = verdicts.agent(AgentId::Codex).unwrap();
    assert!(codex.lines()[0].starts_with("✘ Codex — Codex could not load its settings"));
    for verdict in [claude, codex] {
        assert_eq!(verdict.rollback, Rollback::Restored);
        let restored = format!(
            "Restored your original {} settings.",
            verdict.agent().label()
        );
        assert_eq!(verdict.lines().last(), Some(&restored));
        let report = applied.agent(Scope::Agent(verdict.agent())).unwrap();
        assert_eq!(report.outcome, Outcome::RolledBack { reason: restored });
    }
    let opencode = verdicts.agent(AgentId::OpenCode).unwrap();
    assert_eq!(opencode.rollback, Rollback::NotNeeded);
    for (file, seed) in &seeds[..2] {
        assert_eq!(
            &std::fs::read_to_string(file).unwrap(),
            seed,
            "{}",
            file.display()
        );
    }
    assert!(!home.path(".codex/hooks.json").exists());
    assert_eq!(
        record.agents.keys().collect::<Vec<_>>(),
        [&AgentId::OpenCode]
    );
    assert_eq!(before.agents.len(), 0);
}
