//! Tests of the agent trait, registry, environment and CLI doubles
//! (reliable-installers task 3.1).

use std::path::{Path, PathBuf};
use std::time::Duration;

use super::doubles::{FakeRunner, Reply, TestHome};
use super::doubles_cli;
use super::*;
use crate::setup::edit::json::JsonEditError;
use crate::setup::plan::Action;
use crate::setup::record::{CheckResult, Os, Scope, Target};

/// A tiny agent that writes one owned file twice (to exercise the chain).
struct FakeAgent(AgentId);

impl FakeAgent {
    fn file(env: &Env) -> PathBuf {
        env.home().join("fake").join("notes.txt")
    }

    fn change(chain: &mut FileChain, id: AgentId, file: &Path, add: &str) -> Change {
        let before = chain.bytes(file).expect("readable");
        let mut after = before.clone().unwrap_or_default();
        after.extend_from_slice(add.as_bytes());
        Change {
            scope: Scope::Agent(id),
            target: Target::File {
                file: file.to_path_buf(),
            },
            file: file.to_path_buf(),
            action: if before.is_some() {
                Action::Update
            } else {
                Action::Add
            },
            before,
            after: Some(after),
            owner_only: false,
            missing_parents: None,
            summary: format!("append {add}"),
        }
    }
}

impl Agent for FakeAgent {
    fn id(&self) -> AgentId {
        self.0
    }

    fn detect(&self, env: &Env) -> Detection {
        match env.find_program("fake") {
            Some(path) => Detection::Found {
                version: None,
                paths: vec![path],
            },
            None => Detection::NotFound {
                reason: "fake is not on PATH".to_owned(),
            },
        }
    }

    fn features(&self) -> &'static [Feature] {
        &[Feature::Skill, Feature::RecapCommand]
    }

    fn plan(
        &self,
        env: &Env,
        want: &Selection,
        _rec: Option<&AgentRecord>,
    ) -> Result<Vec<Change>, AgentProblem> {
        let file = Self::file(env);
        let mut chain = FileChain::new();
        for feature in self.features() {
            if want.wants(self.0, *feature) {
                let change = Self::change(&mut chain, self.0, &file, &format!("{feature:?}\n"));
                chain.push(change);
            }
        }
        Ok(chain.into_changes())
    }

    fn plan_remove(&self, _env: &Env, _rec: &AgentRecord) -> Vec<Change> {
        Vec::new()
    }
}

fn boxed(ids: &[AgentId]) -> Vec<Box<dyn Agent>> {
    ids.iter()
        .map(|id| Box::new(FakeAgent(*id)) as Box<dyn Agent>)
        .collect()
}

#[test]
fn test_home_is_isolated_from_the_real_user_folders() {
    let home = TestHome::new(Os::Windows);
    let root = home.dir.path();
    assert!(home.home().starts_with(root));
    assert_eq!(home.env.path_dirs(), [home.bin()]);
    assert!(home.env.dirs.appdata.as_ref().unwrap().starts_with(root));
    assert!(
        home.env
            .dirs
            .local_app_data
            .as_ref()
            .unwrap()
            .starts_with(root)
    );
    assert_eq!(home.env.os(), Os::Windows);
    assert_eq!(home.env.codex_home(), home.home().join(".codex"));
    assert_eq!(home.env.claude_config_dir(), home.home().join(".claude"));
    assert_eq!(home.env.config_home(), home.home().join(".config"));
    let linux = TestHome::new(Os::Linux);
    assert_eq!(linux.env.dirs.appdata, None);
    // `from_process` exists for `mmhmem setup`; tests only name it.
    let _real: fn() -> Option<Env> = Env::from_process;
}

#[test]
fn overrides_win_over_the_home_defaults() {
    let mut home = TestHome::new(Os::Linux);
    home.env.dirs.codex_home = Some(PathBuf::from("/x/codex"));
    home.env.dirs.claude_config_dir = Some(PathBuf::from("/x/claude"));
    home.env.dirs.xdg_config_home = Some(PathBuf::from("/x/config"));
    assert_eq!(home.env.codex_home(), PathBuf::from("/x/codex"));
    assert_eq!(home.env.claude_config_dir(), PathBuf::from("/x/claude"));
    assert_eq!(home.env.config_home(), PathBuf::from("/x/config"));
}

#[test]
fn programs_are_found_only_on_the_injected_path() {
    let home = TestHome::new(Os::current());
    assert_eq!(home.env.find_program("fake"), None);
    let installed = home.install_program("fake");
    let found = home.env.find_program("fake").expect("found");
    assert_eq!(found.file_name(), installed.file_name());
    assert!(found.starts_with(home.bin()));
    assert!(FakeAgent(AgentId::Codex).detect(&home.env).is_found());
    let mut no_path = TestHome::new(Os::current());
    no_path.env.dirs.user_path = None;
    assert_eq!(no_path.env.find_program("fake"), None);
}

#[test]
fn claude_double_replays_get_and_records_arguments() {
    let home = TestHome::new(Os::Windows);
    doubles_cli::claude(&home.runner, &home.env);
    let program = home.install_program("claude");
    let out = home
        .env
        .run(&Invocation::new(&program, &["mcp", "get", "mmhmem"]))
        .expect("ran");
    assert!(out.success());
    assert_eq!(out.stdout, doubles_cli::CLAUDE_MCP_GET);
    assert!(out.stdout.contains("Status: \u{2714} Connected"));
    let list = home.env.run(&Invocation::new("claude", &["mcp", "list"]));
    assert_eq!(list.expect("ran").stdout, doubles_cli::CLAUDE_MCP_LIST);
    let version = home.env.run(&Invocation::new("claude", &["--version"]));
    assert!(version.expect("ran").stdout.starts_with("2.1.302"));
    assert_eq!(
        home.runner.args_of("claude"),
        [
            vec!["mcp", "get", "mmhmem"],
            vec!["mcp", "list"],
            vec!["--version"]
        ]
    );
    assert_eq!(home.runner.calls()[0].program, program);
}

#[test]
fn claude_double_add_and_remove_edit_claude_json() {
    let home = TestHome::new(Os::Windows);
    home.write(&[".claude.json"], r#"{"theme": "dark"}"#);
    doubles_cli::claude(&home.runner, &home.env);
    let args = [
        "mcp",
        "add",
        "--scope",
        "user",
        "--transport",
        "stdio",
        "mmhmem",
        "--",
        "C:\\b\\mmhmem.exe",
        "stdio",
    ];
    assert!(
        home.env
            .run(&Invocation::new("claude", &args))
            .expect("ran")
            .success()
    );
    let file = home.home().join(".claude.json");
    let value: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
    assert_eq!(value["theme"], "dark");
    assert_eq!(
        value["mcpServers"]["mmhmem"]["command"],
        "C:\\b\\mmhmem.exe"
    );
    assert_eq!(
        value["mcpServers"]["mmhmem"]["args"],
        serde_json::json!(["stdio"])
    );
    let remove = Invocation::new("claude", &["mcp", "remove", "mmhmem", "-s", "user"]);
    home.env.run(&remove).expect("ran");
    let value: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
    assert_eq!(value["mcpServers"], serde_json::json!({}));
    assert_eq!(value["theme"], "dark");
}

#[test]
fn codex_double_replays_list_json_and_edits_config_toml() {
    let home = TestHome::new(Os::Linux);
    doubles_cli::codex(&home.runner, &home.env);
    let list = Invocation::new("codex", &["mcp", "list", "--json"]);
    let out = home.env.run(&list).expect("ran");
    let parsed: serde_json::Value = serde_json::from_str(&out.stdout).expect("json");
    assert_eq!(parsed[0]["name"], "mmhmem");
    assert_eq!(parsed[0]["enabled"], true);
    let get = home
        .env
        .run(&Invocation::new("codex", &["mcp", "get", "mmhmem"]));
    assert_eq!(get.expect("ran").stdout, doubles_cli::CODEX_MCP_GET);
    let add = ["mcp", "add", "mmhmem", "--", "/b/mmhmem", "stdio"];
    home.env.run(&Invocation::new("codex", &add)).expect("ran");
    let file = home.env.codex_home().join("config.toml");
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(text.contains("[mcp_servers.mmhmem]"), "{text}");
    assert!(text.contains("command = \"/b/mmhmem\""), "{text}");
    assert!(text.contains("args = [\"stdio\"]"), "{text}");
    home.env
        .run(&Invocation::new("codex", &["mcp", "remove", "mmhmem"]))
        .expect("ran");
    assert!(!std::fs::read_to_string(&file).unwrap().contains("mmhmem"));
    assert_eq!(home.runner.args_of("codex").len(), 4);
}

#[test]
fn opencode_double_reproduces_the_list_race() {
    let home = TestHome::new(Os::Windows);
    doubles_cli::opencode(&home.runner, &home.env);
    let list = Invocation::new("opencode", &["mcp", "list"]).in_dir(home.home());
    let answers: Vec<String> = (0..3)
        .map(|_| home.env.run(&list).expect("ran").stdout)
        .collect();
    assert_eq!(answers[0].trim(), "No MCP servers configured");
    assert!(answers[1].contains("\u{2713} mmhmem       connected"));
    assert_eq!(answers[2], answers[1]);
    let calls = home.runner.calls();
    assert_eq!(calls.len(), 3);
    assert!(calls.iter().all(|c| c.cwd.as_deref() == Some(home.home())));
}

#[test]
fn opencode_double_add_writes_the_confirmed_shape() {
    let home = TestHome::new(Os::Linux);
    doubles_cli::opencode(&home.runner, &home.env);
    let project = home.dir.path().join("project");
    std::fs::create_dir_all(&project).unwrap();
    let add = ["mcp", "add", "mmhmemprobe", "--", "/b/mmhmem", "stdio"];
    home.env
        .run(&Invocation::new("opencode", &add).in_dir(&project))
        .expect("ran");
    let text = std::fs::read_to_string(project.join("opencode.json")).unwrap();
    let value: serde_json::Value = serde_json::from_str(&text).unwrap();
    let expected = serde_json::json!({"type": "local", "command": ["/b/mmhmem", "stdio"]});
    assert_eq!(value["mcp"]["servers"]["mmhmemprobe"], expected);
    let global = [
        "mcp",
        "add",
        "--global",
        "mmhmem",
        "--",
        "/b/mmhmem",
        "stdio",
    ];
    home.env
        .run(&Invocation::new("opencode", &global))
        .expect("ran");
    let file = home
        .env
        .config_home()
        .join("opencode")
        .join("opencode.json");
    let value: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(file).unwrap()).unwrap();
    assert_eq!(value["mcp"]["servers"]["mmhmem"]["type"], "local");
}

#[test]
fn unscripted_calls_are_not_found_and_timeouts_replay() {
    let runner = FakeRunner::new();
    let err = runner.run(&Invocation::new("claude", &["mcp", "get", "mmhmem"]));
    assert!(matches!(err, Err(RunError::NotFound { .. })));
    runner.on("codex", &["mcp"], vec![Reply::TimedOut]);
    runner.on(
        "codex",
        &["mcp", "list"],
        vec![Reply::exit(1, "", "bad config")],
    );
    let slow = Invocation::new("codex", &["mcp", "get"]).timeout(Duration::from_secs(3));
    let err = runner.run(&slow).unwrap_err();
    assert_eq!(
        err.to_string(),
        "codex did not finish within 3 s and was stopped"
    );
    let out = runner
        .run(&Invocation::new("codex", &["mcp", "list"]))
        .unwrap();
    assert_eq!((out.code, out.stderr.as_str()), (Some(1), "bad config"));
    assert_eq!(runner.calls().len(), 3);
}

#[test]
fn command_output_debug_hides_the_text() {
    let out = CommandOutput {
        code: Some(0),
        stdout: "OPENAI_API_KEY=sk-secret".to_owned(),
        stderr: String::new(),
    };
    let shown = format!("{out:?}");
    assert!(!shown.contains("secret"), "{shown}");
    assert!(shown.contains("stdout_len: 24"), "{shown}");
}

#[test]
fn registry_sorts_into_display_order_and_rejects_duplicates() {
    let mut reversed = DISPLAY_ORDER;
    reversed.reverse();
    let registry = Registry::new(boxed(&reversed)).expect("no duplicates");
    assert_eq!(registry.ids(), DISPLAY_ORDER);
    assert_eq!(
        registry.get(AgentId::Codex).map(|a| a.id()),
        Some(AgentId::Codex)
    );
    let dup = Registry::new(boxed(&[
        AgentId::Codex,
        AgentId::ClaudeCode,
        AgentId::Codex,
    ]));
    assert_eq!(dup.unwrap_err().to_string(), "Codex is registered twice");
    let builtin = Registry::builtin().ids();
    let mut sorted = builtin.clone();
    sorted.sort_by_key(|id| id.display_position());
    assert_eq!(builtin, sorted);
}

#[test]
fn agent_ids_round_trip_through_key_and_serde() {
    for (position, id) in DISPLAY_ORDER.into_iter().enumerate() {
        assert_eq!(AgentId::from_key(id.key()), Some(id));
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, format!("\"{}\"", id.key()));
        assert_eq!(serde_json::from_str::<AgentId>(&json).unwrap(), id);
        assert_eq!(id.display_position(), position);
    }
    assert_eq!(AgentId::from_key("vim"), None);
    assert_eq!(AgentId::OpenCode.label(), "OpenCode");
}

#[test]
fn selection_tracks_features_per_agent() {
    let mut want = Selection::new().with(AgentId::Codex, &[Feature::Mcp, Feature::Skill]);
    want.select(AgentId::ClaudeCode, Feature::ReadHook);
    assert!(want.wants(AgentId::Codex, Feature::Skill));
    assert!(!want.wants(AgentId::Codex, Feature::ReadHook));
    assert_eq!(want.agents(), [AgentId::ClaudeCode, AgentId::Codex]);
    want.deselect(AgentId::ClaudeCode, Feature::ReadHook);
    assert_eq!(want.agents(), [AgentId::Codex]);
    let mut record = InstallRecord::new();
    let rec = record.agents.entry(AgentId::OpenCode).or_default();
    rec.features.insert(Feature::Mcp, true);
    rec.features.insert(Feature::ReadHook, false);
    let from = Selection::from_record(&record);
    assert_eq!(
        from.features(AgentId::OpenCode)
            .into_iter()
            .collect::<Vec<_>>(),
        [Feature::Mcp]
    );
}

#[test]
fn a_fake_agent_chains_changes_per_file_and_has_no_check_yet() {
    let home = TestHome::new(Os::Linux);
    let agent = FakeAgent(AgentId::Codex);
    let file = FakeAgent::file(&home.env);
    home.write(&["fake", "notes.txt"], "start\n");
    let want = Selection::new().with(AgentId::Codex, &[Feature::Skill, Feature::RecapCommand]);
    let changes = agent.plan(&home.env, &want, None).expect("planned");
    assert_eq!(changes.len(), 2);
    assert_eq!(changes[0].before.as_deref(), Some(&b"start\n"[..]));
    assert_eq!(changes[1].before, changes[0].after);
    assert_eq!(
        changes[1].after.as_deref(),
        Some(&b"start\nSkill\nRecapCommand\n"[..])
    );
    assert!(changes.iter().all(|c| c.file == file));
    let report = agent.check(&home.env, &AgentRecord::default());
    assert!(!report.ran);
    assert_eq!(report.result, CheckResult::Fail);
    assert_eq!(report.line, format!("✘ Codex — {CHECK_NOT_IMPLEMENTED}"));
    let at = UtcTimestamp::now();
    assert_eq!(report.last_check(at.clone()).at, at);
    assert!(
        agent
            .plan_remove(&home.env, &AgentRecord::default())
            .is_empty()
    );
}

#[test]
fn file_chain_reports_unreadable_and_non_text_files() {
    let home = TestHome::new(Os::Linux);
    let mut chain = FileChain::new();
    assert_eq!(chain.bytes(&home.home().join("absent")).unwrap(), None);
    let binary = home.home().join("bin.dat");
    std::fs::write(&binary, [0xff, 0xfe, 0x00]).unwrap();
    let err = chain.text(&binary).unwrap_err();
    assert_eq!(
        err.to_string(),
        format!("{} is not UTF-8 text", binary.display())
    );
    let dir_err = read_optional(home.home()).unwrap_err();
    assert!(matches!(dir_err, AgentProblem::Unreadable { .. }));
}

#[test]
fn agent_problems_name_files_never_contents() {
    let file = PathBuf::from("settings.json");
    let problem = AgentProblem::Json {
        file: file.clone(),
        error: JsonEditError::RootNotObject,
    };
    assert_eq!(
        problem.to_string(),
        "settings.json: the top-level JSON value is not an object"
    );
    let dup = AgentProblem::DuplicateEntry {
        files: vec![
            PathBuf::from("opencode.json"),
            PathBuf::from("opencode.jsonc"),
        ],
    };
    assert_eq!(
        dup.to_string(),
        "mmhmem is set up in more than one file: opencode.json, opencode.jsonc; keep only one"
    );
    let profiles = AgentProblem::SeveralProfiles {
        profiles: vec![PathBuf::from("a"), PathBuf::from("b")],
    };
    assert_eq!(
        profiles.to_string(),
        "more than one profile was found: a, b; choose one"
    );
}

/// Spawned by the system-runner test; sleeps only when asked to.
#[test]
#[ignore = "helper for system_runner_runs_programs_and_stops_them_at_the_limit"]
fn runner_sleep_helper() {
    if std::env::var_os("MMHMEM_RUNNER_SLEEP").is_some() {
        std::thread::sleep(Duration::from_secs(30));
    }
}

#[test]
fn system_runner_runs_programs_and_stops_them_at_the_limit() {
    let me = std::env::current_exe().expect("test binary");
    let list = Invocation::new(&me, &["--list", "--format", "terse", "--ignored"]);
    let out = SystemRunner.run(&list).expect("ran");
    assert!(out.success(), "{out:?}");
    assert!(out.stdout.contains("runner_sleep_helper"));
    let sleep = Invocation::new(&me, &["--ignored", "--exact"])
        .arg("setup::agents::tests::runner_sleep_helper")
        .env("MMHMEM_RUNNER_SLEEP", "1")
        .timeout(Duration::from_millis(300));
    let err = SystemRunner.run(&sleep).unwrap_err();
    assert!(matches!(err, RunError::TimedOut { .. }), "{err:?}");
    let missing = Invocation::new(me.with_file_name("no-such-program-mmhmem"), &[]);
    assert!(matches!(
        SystemRunner.run(&missing),
        Err(RunError::NotFound { .. })
    ));
}
