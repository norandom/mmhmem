//! mmhmem owns its hook HANDLER, never the group it sits in (follow-up G1,
//! `.kiro/specs/reliable-installers/research.md` "2026-11-06 G1/G3"): the old
//! installers could leave mmhmem's handler inside a group that also holds the
//! user's handlers (`manage_hooks.ps1` "existing entry in a shared group
//! updated, no second group", "remove keeps the shared group and its other
//! handler"). Install, update, duplicates, removal, drift and adoption, for
//! Claude Code (`settings.json`) and Codex (`hooks.json`). Temporary homes
//! only.

use std::path::PathBuf;

use super::claude_code::ClaudeCode;
use super::codex::Codex;
use super::doubles::TestHome;
use super::{Agent, AgentId, Feature, Selection};
use crate::setup::adopt::{ScanEnv, adoption_plan, scan};
use crate::setup::plan::{self, Outcome, Plan};
use crate::setup::record::{
    Drift, InstallRecord, Os, Probe, RecordStore, UtcTimestamp, detect_drift,
};

/// One agent's hook file in a temporary home.
struct Rig {
    home: TestHome,
    store: RecordStore,
    record: InstallRecord,
    agent: Box<dyn Agent>,
    id: AgentId,
    file: PathBuf,
    runs: u32,
}

/// Which harness, with the strings its own handler is made of.
#[derive(Clone, Copy)]
enum Kind {
    Claude,
    Codex,
}

const OLD: &str = "/old/place/mmhmem";
const NEW: &str = "/opt/mmhmem/bin/mmhmem";

impl Kind {
    fn matcher(self) -> &'static str {
        match self {
            Self::Claude => "Read|Bash|PowerShell",
            Self::Codex => "^(Bash|shell)$",
        }
    }

    /// mmhmem's handler for `bin` as one line of JSON, the way the old
    /// installers wrote it.
    fn own(self, bin: &str) -> String {
        match self {
            Self::Claude => format!(
                "{{ \"type\": \"command\", \"command\": \"{bin}\", \"args\": [\"hook\", \"claude\"], \"timeout\": 10 }}"
            ),
            Self::Codex => format!(
                "{{ \"type\": \"command\", \"command\": \"{bin} hook codex\", \"timeout\": 10 }}"
            ),
        }
    }

    /// A user handler that merely mentions `hook` (and even mmhmem): never
    /// mmhmem's.
    fn lookalike(self) -> &'static str {
        match self {
            Self::Claude => {
                "{ \"type\": \"command\", \"command\": \"/usr/local/bin/audit\", \"args\": [\"hook\", \"claude\"] }"
            }
            Self::Codex => "{ \"type\": \"command\", \"command\": \"echo mmhmem hook codex\" }",
        }
    }

    fn rig(self) -> Rig {
        let home = TestHome::new(Os::Linux);
        let store = RecordStore::new(home.dir.path().join("state"));
        let (agent, id, file): (Box<dyn Agent>, _, _) = match self {
            Self::Claude => (
                Box::new(ClaudeCode::new(NEW)),
                AgentId::ClaudeCode,
                home.env.claude_config_dir().join("settings.json"),
            ),
            Self::Codex => (
                Box::new(Codex::new(NEW)),
                AgentId::Codex,
                home.env.codex_home().join("hooks.json"),
            ),
        };
        Rig {
            home,
            store,
            record: InstallRecord::new(),
            agent,
            id,
            file,
            runs: 0,
        }
    }
}

/// A user handler, kept byte for byte.
const MINE: &str = "{ \"type\": \"command\", \"command\": \"my-guard\", \"timeout\": 5 }";

/// `settings.json`/`hooks.json` with one `PreToolUse` group per entry of
/// `groups`, each `(matcher, handlers)`, one handler per line.
fn file_text(groups: &[(&str, Vec<String>)]) -> String {
    let groups: Vec<String> = groups
        .iter()
        .map(|(matcher, handlers)| {
            format!(
                "      {{\n        \"matcher\": \"{matcher}\",\n        \"hooks\": [\n          {}\n        ]\n      }}",
                handlers.join(",\n          ")
            )
        })
        .collect();
    format!(
        "{{\n  \"theme\": \"dark\",\n  \"hooks\": {{\n    \"PreToolUse\": [\n{}\n    ]\n  }}\n}}\n",
        groups.join(",\n")
    )
}

impl Rig {
    fn want(&self) -> Selection {
        Selection::new().with(self.id, &[Feature::ReadHook])
    }

    fn seed(&self, text: &str) {
        std::fs::create_dir_all(self.file.parent().unwrap()).unwrap();
        std::fs::write(&self.file, text).unwrap();
    }

    fn text(&self) -> String {
        std::fs::read_to_string(&self.file).unwrap()
    }

    fn apply(&mut self, changes: Vec<plan::Change>) {
        self.runs += 1;
        let run = UtcTimestamp::try_from(format!("2026-11-06T10:{:02}:00Z", self.runs)).unwrap();
        let report = plan::apply(&Plan { run, changes }, &self.store, &mut self.record);
        for agent in &report.agents {
            assert_eq!(agent.outcome, Outcome::Applied, "{:?}", agent.scope);
        }
    }

    fn install(&mut self) {
        let want = self.want();
        let changes = self
            .agent
            .plan(&self.home.env, &want, self.record.agents.get(&self.id))
            .expect("planned");
        self.apply(changes);
    }

    fn replan_is_empty(&self) -> bool {
        let rec = self.record.agents.get(&self.id);
        let changes = self.agent.plan(&self.home.env, &self.want(), rec).unwrap();
        changes.iter().all(|c| c.file != self.file)
    }

    fn remove_all(&mut self) {
        let rec = self.record.agents.get(&self.id).cloned().expect("recorded");
        let changes = self.agent.plan_remove(&self.home.env, &rec);
        self.apply(changes);
    }

    fn hook_drift(&self) -> Drift {
        detect_drift(&self.record, &Probe::default())
            .into_iter()
            .find(|r| crate::setup::plan::target_file(&r.target) == Some(self.file.as_path()))
            .expect("the hook is recorded")
            .drift
    }
}

fn both() -> [Kind; 2] {
    [Kind::Claude, Kind::Codex]
}

#[test]
fn install_beside_a_user_group_with_the_same_matcher_adds_its_own_group() {
    for kind in both() {
        let mut r = kind.rig();
        let seed = file_text(&[(kind.matcher(), vec![MINE.to_owned()])]);
        r.seed(&seed);
        r.install();
        let text = r.text();
        assert!(text.starts_with(&seed[..seed.find("\n    ]").unwrap()]));
        assert_eq!(text.matches(MINE).count(), 1);
        assert_eq!(text.matches(NEW).count(), 1);
        assert!(r.replan_is_empty());
        r.remove_all();
        assert_eq!(r.text(), seed);
    }
}

#[test]
fn update_in_a_shared_group_changes_only_mmhmem_handler() {
    for kind in both() {
        let mut r = kind.rig();
        let handlers = vec![MINE.to_owned(), kind.own(OLD), kind.lookalike().to_owned()];
        let seed = file_text(&[(kind.matcher(), handlers)]);
        r.seed(&seed);
        r.install();
        // Only the binary path changed: no second group, user handlers kept.
        assert_eq!(r.text(), seed.replace(OLD, NEW));
        assert!(r.replan_is_empty());
        assert_eq!(r.hook_drift(), Drift::Unchanged);
    }
}

#[test]
fn remove_from_a_shared_group_keeps_the_user_handlers_byte_identical() {
    for kind in both() {
        let mut r = kind.rig();
        let own = kind.own(NEW);
        let seed = file_text(&[(
            kind.matcher(),
            vec![MINE.to_owned(), own.clone(), kind.lookalike().to_owned()],
        )]);
        r.seed(&seed);
        r.install();
        assert_eq!(r.text(), seed, "already up to date");
        // The user's handlers are not mmhmem's: editing them is not drift.
        r.seed(&seed.replace("my-guard", "my-guard --strict"));
        assert_eq!(r.hook_drift(), Drift::Unchanged);
        r.seed(&seed.replace("\"timeout\": 10", "\"timeout\": 99"));
        assert_ne!(r.hook_drift(), Drift::Unchanged);
        r.seed(&seed);
        r.remove_all();
        let expected = file_text(&[(
            kind.matcher(),
            vec![MINE.to_owned(), kind.lookalike().to_owned()],
        )]);
        assert_eq!(r.text(), expected);
    }
}

#[test]
fn duplicate_handlers_collapse_to_one_and_user_handlers_stay() {
    for kind in both() {
        let mut r = kind.rig();
        let seed = file_text(&[
            ("Bash", vec![MINE.to_owned(), kind.own(NEW)]),
            (kind.matcher(), vec![kind.own(OLD)]),
            (
                kind.matcher(),
                vec![kind.own(NEW), kind.lookalike().to_owned()],
            ),
        ]);
        r.seed(&seed);
        r.install();
        let text = r.text();
        assert_eq!(text.matches(NEW).count(), 1, "{text}");
        assert!(!text.contains(OLD));
        assert_eq!(text.matches(MINE).count(), 1);
        assert_eq!(text.matches(kind.lookalike()).count(), 1);
        assert!(r.replan_is_empty());
        r.remove_all();
        let text = r.text();
        assert!(!text.contains(NEW));
        assert_eq!(text.matches(MINE).count(), 1);
        assert_eq!(text.matches(kind.lookalike()).count(), 1);
    }
}

#[test]
fn handlers_that_only_mention_hook_are_never_touched() {
    for kind in both() {
        let mut r = kind.rig();
        let seed = file_text(&[(kind.matcher(), vec![kind.lookalike().to_owned()])]);
        r.seed(&seed);
        r.install();
        r.remove_all();
        assert_eq!(r.text(), seed);
    }
}

/// An old-installer home on Linux: the hook helper had put mmhmem's handler
/// into the user's group with the same matcher.
#[test]
fn adopting_an_old_shared_group_then_removing_keeps_the_user_handler() {
    for kind in both() {
        let mut r = kind.rig();
        let own = kind.own("/home/me/.local/bin/mmhmem");
        let seed = file_text(&[(kind.matcher(), vec![MINE.to_owned(), own])]);
        r.seed(&seed);
        let env = ScanEnv::new(Os::Linux, r.home.home());
        let run = UtcTimestamp::try_from("2026-11-06T09:00:00Z".to_owned()).unwrap();
        let plan = adoption_plan(&scan(&env), None, run);
        let report = plan::apply(&plan, &r.store, &mut r.record);
        assert!(report.agents.iter().all(|a| a.outcome == Outcome::Applied));
        assert!(r.record.agents.contains_key(&r.id), "the hook was adopted");
        assert_eq!(r.text(), seed);
        assert_eq!(r.hook_drift(), Drift::Unchanged);
        r.remove_all();
        let expected = file_text(&[(kind.matcher(), vec![MINE.to_owned()])]);
        assert_eq!(r.text(), expected);
    }
}
