//! Test doubles for agent modules (reliable-installers task 3.1): a
//! [`TestHome`] over a temporary folder and a [`FakeRunner`] that replays
//! scripted CLI answers and records every call. The per-CLI scripts are in
//! `doubles_cli.rs`. Nothing here reads the real user folders or starts a
//! real agent.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use tempfile::TempDir;

use super::{CommandOutput, CommandRunner, Env, Invocation, RunError, ScanEnv};
use crate::setup::record::Os;

/// A side effect of a scripted call (the file a real CLI writes).
pub(crate) type Effect = Arc<dyn Fn(&Invocation) + Send + Sync>;

/// One scripted answer.
#[derive(Debug, Clone)]
pub(crate) enum Reply {
    /// The program ran and printed this.
    Output(CommandOutput),
    /// The program ran past its time limit.
    TimedOut,
}

impl Reply {
    /// Exit code 0 with `stdout`.
    pub(crate) fn ok(stdout: &str) -> Self {
        Self::exit(0, stdout, "")
    }

    /// Exit `code` with both streams.
    pub(crate) fn exit(code: i32, stdout: &str, stderr: &str) -> Self {
        Self::Output(CommandOutput {
            code: Some(code),
            stdout: stdout.to_owned(),
            stderr: stderr.to_owned(),
        })
    }
}

struct Rule {
    program: String,
    prefix: Vec<String>,
    replies: VecDeque<Reply>,
    effect: Option<Effect>,
}

/// Replays scripted answers by program name and longest matching argument
/// prefix. Replies are used in order; the last one repeats. An unscripted
/// call is [`RunError::NotFound`].
#[derive(Default)]
pub(crate) struct FakeRunner {
    rules: Mutex<Vec<Rule>>,
    calls: Mutex<Vec<Invocation>>,
}

impl FakeRunner {
    /// No scripts yet.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Scripts `program args-prefix…` (replacing an earlier script for the
    /// same prefix).
    pub(crate) fn on(&self, program: &str, prefix: &[&str], replies: Vec<Reply>) {
        self.script(program, prefix, replies, None);
    }

    /// As [`FakeRunner::on`], running `effect` on every matching call.
    pub(crate) fn on_with(
        &self,
        program: &str,
        prefix: &[&str],
        replies: Vec<Reply>,
        effect: Effect,
    ) {
        self.script(program, prefix, replies, Some(effect));
    }

    fn script(&self, program: &str, prefix: &[&str], replies: Vec<Reply>, effect: Option<Effect>) {
        assert!(!replies.is_empty(), "a script needs at least one reply");
        let prefix: Vec<String> = prefix.iter().map(|a| (*a).to_owned()).collect();
        let mut rules = self.rules.lock().expect("rules lock");
        rules.retain(|r| !(r.program == program && r.prefix == prefix));
        rules.push(Rule {
            program: program.to_owned(),
            prefix,
            replies: replies.into(),
            effect,
        });
    }

    /// Every call so far, in order.
    pub(crate) fn calls(&self) -> Vec<Invocation> {
        self.calls.lock().expect("calls lock").clone()
    }

    /// The arguments of every call of `program`, in order.
    pub(crate) fn args_of(&self, program: &str) -> Vec<Vec<String>> {
        self.calls()
            .into_iter()
            .filter(|c| c.program_name() == program)
            .map(|c| c.args)
            .collect()
    }
}

impl CommandRunner for FakeRunner {
    fn run(&self, invocation: &Invocation) -> Result<CommandOutput, RunError> {
        self.calls
            .lock()
            .expect("calls lock")
            .push(invocation.clone());
        let name = invocation.program_name();
        let (reply, effect) = {
            let mut rules = self.rules.lock().expect("rules lock");
            let rule = rules
                .iter_mut()
                .filter(|r| r.program == name && invocation.args.starts_with(&r.prefix))
                .max_by_key(|r| r.prefix.len())
                .ok_or_else(|| RunError::NotFound {
                    program: invocation.program.clone(),
                })?;
            let reply = if rule.replies.len() > 1 {
                rule.replies.pop_front().expect("non-empty")
            } else {
                rule.replies.front().cloned().expect("non-empty")
            };
            (reply, rule.effect.clone())
        };
        if let Some(effect) = effect {
            effect(invocation);
        }
        match reply {
            Reply::Output(out) => Ok(out),
            Reply::TimedOut => Err(RunError::TimedOut {
                program: invocation.program.clone(),
                after: invocation.timeout,
            }),
        }
    }
}

/// A temporary home with an injected environment: `home/` (with
/// `AppData/Roaming` and `AppData/Local` for Windows rules) and a `bin/`
/// folder that is the whole `PATH`.
pub(crate) struct TestHome {
    /// The temporary folder (deleted on drop).
    pub(crate) dir: TempDir,
    /// The environment agent modules get.
    pub(crate) env: Env,
    /// The runner inside `env`, for scripting and inspecting calls.
    pub(crate) runner: Arc<FakeRunner>,
}

impl TestHome {
    /// A fresh home for `os` rules with no agent installed.
    pub(crate) fn new(os: Os) -> Self {
        let dir = tempfile::tempdir().expect("temp dir");
        let home = dir.path().join("home");
        let bin = dir.path().join("bin");
        std::fs::create_dir_all(&home).expect("home");
        std::fs::create_dir_all(&bin).expect("bin");
        let mut dirs = ScanEnv::new(os, &home);
        if os == Os::Windows {
            dirs.appdata = Some(home.join("AppData").join("Roaming"));
            dirs.local_app_data = Some(home.join("AppData").join("Local"));
        }
        dirs.user_path = Some(vec![bin]);
        let runner = Arc::new(FakeRunner::new());
        let env = Env::new(dirs, runner.clone());
        Self { dir, env, runner }
    }

    /// The home folder.
    pub(crate) fn home(&self) -> &Path {
        self.env.home()
    }

    /// The `PATH` folder.
    pub(crate) fn bin(&self) -> PathBuf {
        self.dir.path().join("bin")
    }

    /// Puts an empty program called `name` on `PATH` (host executable rules)
    /// and returns its path.
    pub(crate) fn install_program(&self, name: &str) -> PathBuf {
        let file = if cfg!(windows) {
            self.bin().join(format!("{name}.exe"))
        } else {
            self.bin().join(name)
        };
        std::fs::write(&file, b"").expect("program file");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::Permissions::from_mode(0o755);
            std::fs::set_permissions(&file, mode).expect("chmod");
        }
        file
    }

    /// Writes `text` to `home/<rel>`, creating folders, and returns the path.
    pub(crate) fn write(&self, rel: &[&str], text: &str) -> PathBuf {
        let file = rel.iter().fold(self.home().to_path_buf(), |p, s| p.join(s));
        if let Some(parent) = file.parent() {
            std::fs::create_dir_all(parent).expect("parent folders");
        }
        std::fs::write(&file, text).expect("write file");
        file
    }
}
