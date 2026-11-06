//! The Codex hook command for binary paths with quotes and other shell
//! characters (follow-up G3, `.kiro/specs/reliable-installers/research.md`
//! "2026-11-06 G1/G3"). Codex runs the command through PowerShell on Windows
//! and through the POSIX shell elsewhere; setup must quote the path for that
//! shell (PowerShell also ends a single-quoted string at ’ ‘ ‚ ‛) and must
//! recognise its own entry by parsing the command, so a second run does not
//! add another entry and removal finds it. Temporary homes only.

use std::path::{Path, PathBuf};

use super::*;
use crate::setup::agents::doubles::TestHome;
use crate::setup::plan::{Outcome, Plan, apply};
use crate::setup::record::{InstallRecord, RecordStore, UtcTimestamp};

struct Rig {
    home: TestHome,
    store: RecordStore,
    record: InstallRecord,
    codex: Codex,
}

fn rig(os: Os, binary: &str) -> Rig {
    let home = TestHome::new(os);
    let store = RecordStore::new(home.dir.path().join("state"));
    Rig {
        home,
        store,
        record: InstallRecord::new(),
        codex: Codex::new(PathBuf::from(binary)),
    }
}

fn hook_only() -> Selection {
    Selection::new().with(ID, &[Feature::ReadHook])
}

impl Rig {
    fn hooks(&self) -> PathBuf {
        self.home.env.codex_home().join("hooks.json")
    }

    fn plan(&self) -> Result<Vec<Change>, AgentProblem> {
        let rec = self.record.agents.get(&ID);
        self.codex.plan(&self.home.env, &hook_only(), rec)
    }

    fn run(&mut self, changes: Vec<Change>) {
        let run = UtcTimestamp::try_from("2026-11-06T11:00:00Z".to_owned()).unwrap();
        let report = apply(&Plan { run, changes }, &self.store, &mut self.record);
        assert!(report.agents.iter().all(|a| a.outcome == Outcome::Applied));
    }

    fn install(&mut self) {
        let changes = self.plan().expect("planned");
        self.run(changes);
    }

    fn remove_all(&mut self) {
        let rec = self.record.agents.get(&ID).cloned().expect("recorded");
        let changes = self.codex.plan_remove(&self.home.env, &rec);
        self.run(changes);
    }

    /// The `command` strings of every hook handler in `hooks.json`.
    fn commands(&self) -> Vec<String> {
        let text = std::fs::read_to_string(self.hooks()).unwrap();
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        let groups = value["hooks"]["PreToolUse"].as_array().cloned();
        groups
            .unwrap_or_default()
            .iter()
            .flat_map(|g| g["hooks"].as_array().cloned().unwrap_or_default())
            .filter_map(|h| h["command"].as_str().map(str::to_owned))
            .collect()
    }

    fn seed(&self, command: &str) -> String {
        let text = format!(
            "{{\n  \"hooks\": {{\n    \"PreToolUse\": [\n      {{ \"matcher\": \"^(Bash|shell)$\", \"hooks\": [{{ \"type\": \"command\", \"command\": {}, \"timeout\": 10 }}] }}\n    ]\n  }}\n}}\n",
            serde_json::Value::String(command.to_owned())
        );
        std::fs::create_dir_all(self.hooks().parent().unwrap()).unwrap();
        std::fs::write(self.hooks(), &text).unwrap();
        text
    }
}

/// `(os, binary, the command setup must write)`.
fn cases() -> Vec<(Os, &'static str, &'static str)> {
    vec![
        (
            Os::Windows,
            "C:\\Users\\O'Brien\\AppData\\Local\\Programs\\mmhmem\\mmhmem.exe",
            "& 'C:\\Users\\O''Brien\\AppData\\Local\\Programs\\mmhmem\\mmhmem.exe' hook codex",
        ),
        (
            Os::Windows,
            "C:\\it's\\mmhmem.exe",
            "& 'C:\\it''s\\mmhmem.exe' hook codex",
        ),
        (
            Os::Windows,
            "C:\\a\u{2019}b\\mmhmem.exe",
            "& 'C:\\a\u{2019}\u{2019}b\\mmhmem.exe' hook codex",
        ),
        (
            Os::Windows,
            "C:\\q\u{2018}\u{201A}\u{201B}\\mmhmem.exe",
            "& 'C:\\q\u{2018}\u{2018}\u{201A}\u{201A}\u{201B}\u{201B}\\mmhmem.exe' hook codex",
        ),
        (
            Os::Windows,
            "C:\\Program Files\\$env x\\`b&c\\mmhmem.exe",
            "& 'C:\\Program Files\\$env x\\`b&c\\mmhmem.exe' hook codex",
        ),
        (
            Os::Linux,
            "/home/o'brien/.local/bin/mmhmem",
            "'/home/o'\"'\"'brien/.local/bin/mmhmem' hook codex",
        ),
        (
            Os::Linux,
            "/opt/my tools/$HOME/`x`&y/mmhmem",
            "'/opt/my tools/$HOME/`x`&y/mmhmem' hook codex",
        ),
        (
            Os::Linux,
            "/opt/a\u{2019}b\\c\"d/mmhmem",
            "'/opt/a\u{2019}b\\c\"d/mmhmem' hook codex",
        ),
    ]
}

#[test]
fn quoted_paths_install_once_and_are_removed() {
    for (os, binary, command) in cases() {
        let mut r = rig(os, binary);
        r.install();
        assert_eq!(r.commands(), [command], "{binary}");
        let again = r.plan().unwrap();
        assert!(again.is_empty(), "{binary}: {again:?}");
        r.install();
        assert_eq!(r.commands().len(), 1, "{binary}");
        r.remove_all();
        assert!(r.commands().is_empty(), "{binary}");
    }
}

#[test]
fn hand_edited_equivalent_quoting_is_recognised() {
    let cases = [
        (
            Os::Linux,
            "/opt/my tools/mmhmem",
            "\"/opt/my tools/mmhmem\" hook codex",
        ),
        (
            Os::Linux,
            "/opt/my tools/mmhmem",
            "/opt/my\\ tools/mmhmem   hook codex",
        ),
        (
            Os::Linux,
            "/home/o'brien/mmhmem",
            "'/home/o'\\''brien/mmhmem' hook codex",
        ),
        (
            Os::Windows,
            "C:\\Users\\O'Brien\\mmhmem.exe",
            "& \"C:\\Users\\O'Brien\\mmhmem.exe\" hook codex",
        ),
        (
            Os::Windows,
            "C:\\Users\\O'Brien\\mmhmem.exe",
            "&'C:\\Users\\O\u{2019}'Brien\\mmhmem.exe' hook codex",
        ),
        (
            Os::Windows,
            "C:\\Users\\me\\mmhmem.exe",
            "C:\\Users\\me\\mmhmem.exe hook codex",
        ),
    ];
    for (os, binary, command) in cases {
        let mut r = rig(os, binary);
        let seeded = r.seed(command);
        r.install();
        let text = std::fs::read_to_string(r.hooks()).unwrap();
        assert_eq!(text, seeded, "{command} was not recognised as {binary}");
        r.remove_all();
        assert!(r.commands().is_empty(), "{command}");
    }
}

#[test]
fn a_path_that_cannot_be_quoted_is_refused_before_writing() {
    for (os, binary) in [
        (Os::Linux, "/opt/a\nb/mmhmem"),
        (Os::Windows, "C:\\a\rb\\mmhmem.exe"),
    ] {
        let r = rig(os, binary);
        let err = r.plan().expect_err(binary);
        let message = err.to_string();
        assert!(message.contains("Codex"), "{message}");
        assert!(message.contains("control character"), "{message}");
        assert!(!Path::new(&r.hooks()).exists());
    }
}
