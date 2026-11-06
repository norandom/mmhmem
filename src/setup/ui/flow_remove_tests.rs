//! Tests of the removal (task 7.2): a full install into a temporary home
//! seeded with the user's own settings (agent doubles, a fake user `PATH`,
//! never the registry), later user edits to shared files, then the removal
//! through the flow, the menu and the numbered fallback.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::*;
use crate::setup::agents::Feature;
use crate::setup::ui::app::{Key, Screen};
use crate::setup::ui::fixtures::{Fx, press, with_clis};
use crate::setup::ui::flow::{EXIT_CANCELLED, HomeAction, Newest, tidy};
use crate::setup::user_path::{FakeUserPath, PathValue, UserPathStore, ValueKind};
use text::remove as say;

/// The user's own files before mmhmem is installed.
const SEEDED: &[(&[&str], &str)] = &[
    (
        &[".claude.json"],
        "{\n  \"numStartups\": 3,\n  \"theme\": \"dark\"\n}\n",
    ),
    (
        &[".claude", "settings.json"],
        "{\n  \"model\": \"opus\"\n}\n",
    ),
    (
        &[".codex", "config.toml"],
        "# my codex settings\nmodel = \"o3\"\n\n[profiles.fast]\nmodel = \"o4-mini\"\n",
    ),
    (
        &[".config", "opencode", "opencode.json"],
        "{\n  \"$schema\": \"https://opencode.ai/config.json\",\n  \"theme\": \"dark\"\n}\n",
    ),
    (
        &[".config", "opencode", "AGENTS.md"],
        "# My rules\n\nBe nice.\n",
    ),
];

/// Edits the user makes after the install, in files mmhmem shares.
const LATER_EDITS: &[(&[&str], &str, &str)] = &[
    (
        &[".codex", "config.toml"],
        "model = \"o3\"",
        "model = \"o3-pro\"",
    ),
    (
        &[".config", "opencode", "opencode.json"],
        "\"dark\"",
        "\"light\"",
    ),
    (
        &[".config", "opencode", "AGENTS.md"],
        "Be nice.",
        "Be kind.",
    ),
];

const PROGRAM: &[u8] = b"the mmhmem program";

/// A user PATH value as the registry stores it (UTF-16LE, one NUL).
fn reg(text: &str) -> PathValue {
    let units: Vec<u16> = text.encode_utf16().chain([0]).collect();
    PathValue {
        kind: ValueKind::Expand,
        data: units.iter().flat_map(|u| u.to_le_bytes()).collect(),
    }
}

struct Home {
    fx: Fx,
    path: Arc<FakeUserPath>,
    original_path: Option<PathValue>,
    /// Every file in the home before the install.
    files_before: Vec<PathBuf>,
    _temp: tempfile::TempDir,
}

impl Home {
    fn file(&self, rel: &[&str]) -> PathBuf {
        rel.iter()
            .fold(self.fx.home.home().to_path_buf(), |p, s| p.join(s))
    }

    fn memories(&self) -> &Path {
        &self.fx.deps.memories.as_ref().expect("memories").dir
    }

    /// The user's files as they should be after a removal.
    fn expected(&self) -> BTreeMap<PathBuf, String> {
        let mut want: BTreeMap<PathBuf, String> = SEEDED
            .iter()
            .map(|(rel, text)| (self.file(rel), (*text).to_owned()))
            .collect();
        for (rel, from, to) in LATER_EDITS {
            let text = want.get_mut(&self.file(rel)).expect("seeded");
            *text = text.replace(from, to);
        }
        want
    }
}

/// Seeds the user's files, saved memories and a user PATH, installs every
/// feature but the read hook (it cannot start under the doubles) from a
/// bootstrap copy of the program, then makes the user's later edits.
fn installed() -> Home {
    installed_with(Some(reg(r"C:\Tools;%USERPROFILE%\bin")))
}

/// [`installed`] with this user PATH value before the install.
fn installed_with(original_path: Option<PathValue>) -> Home {
    let mut fx = with_clis();
    for (rel, text) in SEEDED {
        fx.write(rel, text);
    }
    let path = Arc::new(FakeUserPath::with(original_path.clone()));
    fx.deps.paths = path.clone();
    let temp = tempfile::tempdir().expect("temp");
    let program = temp.path().join("mmhmem");
    std::fs::write(&program, PROGRAM).expect("program");
    fx.deps.install_from = Some(program);
    let memories = fx.deps.memories.clone().expect("memories").dir;
    std::fs::create_dir_all(&memories).expect("store");
    std::fs::write(memories.join("mmhmem.db"), b"saved memories").expect("db");
    std::fs::write(memories.join("audit.jsonl"), b"{}\n").expect("audit");
    let files_before = files(fx.home.home(), &fx.deps.store.dir().join("backups"));
    install_everything(&fx);
    fx.deps.install_from = None;
    let home = Home {
        fx,
        path,
        original_path,
        files_before,
        _temp: temp,
    };
    for (rel, from, to) in LATER_EDITS {
        let file = home.file(rel);
        let text = std::fs::read_to_string(&file).expect("installed file");
        assert!(text.contains(from), "{file:?}");
        std::fs::write(&file, text.replace(from, to)).expect("user edit");
    }
    home
}

fn install_everything(fx: &Fx) {
    let mut flow = fx.flow(Newest::Unknown);
    assert_eq!(flow.begin(&HomeAction::Install), Next::Agents);
    for row in flow.feature_rows() {
        let wanted = row.feature.is_some_and(|f| f != Feature::ReadHook);
        if wanted && !row.on {
            flow.toggle_feature(&row);
        }
    }
    flow.make_plan();
    assert_eq!(flow.confirm(), Confirmed::Done);
    assert_eq!(flow.exit_code(), 0, "{:?}", flow.progress());
    for (rel, text) in SEEDED.iter().skip(2) {
        let now = std::fs::read_to_string(fx.home.home().join(rel.join("/"))).unwrap();
        assert_ne!(now, *text, "{rel:?} was not shared with mmhmem");
    }
}

/// Every file under `dir`, except the setup backups.
fn files(dir: &Path, skip: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.starts_with(skip) {
            continue;
        }
        if path.is_dir() {
            out.extend(files(&path, skip));
        } else {
            out.push(path);
        }
    }
    out.sort();
    out
}

fn remove(home: &Home, delete_memories: bool) -> Flow {
    let mut flow = home.fx.flow(Newest::Unknown);
    assert!(flow.home().actions.contains(&HomeAction::Remove));
    assert_eq!(flow.begin(&HomeAction::Remove), Next::Review);
    if delete_memories {
        flow.toggle_memories();
    }
    assert_eq!(flow.confirm(), Confirmed::Done);
    flow
}

/// The user's files, the PATH and nothing else of mmhmem's are left.
fn assert_removed(home: &Home, flow: &Flow) {
    let progress = flow.progress().expect("results");
    assert_eq!(progress.code, 0, "{:?}", progress.lines);
    assert_eq!(
        progress.lines.last().map(String::as_str),
        Some(say::REMOVED)
    );
    for (file, text) in home.expected() {
        assert_eq!(std::fs::read_to_string(&file).unwrap(), text, "{file:?}");
    }
    assert_eq!(home.path.get(), home.original_path);
    assert!(!home.fx.deps.binary.parent().unwrap().exists());
    assert!(!home.fx.deps.binary.exists());
    assert!(!home.fx.deps.store.record_path().exists());
    assert!(flow.record().is_none());
    assert!(progress.lines.contains(&say::RECORD_DELETED.to_owned()));
}

#[test]
fn a_removal_leaves_the_users_files_and_path_as_they_were_and_keeps_the_memories() {
    let home = installed();
    let review = {
        let mut flow = home.fx.flow(Newest::Unknown);
        flow.begin(&HomeAction::Remove);
        flow.review()
    };
    assert_eq!(review.intro, [say::INTRO]);
    let items = review.items();
    let last = items.len() - 1;
    let path_at = items
        .iter()
        .position(|i| i.summary.starts_with("Take "))
        .unwrap();
    let program_at = items
        .iter()
        .position(|i| i.summary == say::CHANGE_PROGRAM)
        .unwrap();
    assert!(path_at < program_at && program_at == last - 1, "{review:?}");
    assert_eq!(items[last].summary, say::RECORD_FILE);
    assert_eq!(review.memories.as_ref().map(|m| m.on), Some(false));

    let flow = remove(&home, false);
    assert_removed(&home, &flow);
    let backups = home.fx.deps.store.dir().join("backups");
    // Also the Markdown file mmhmem created only for its section is gone.
    // The one file left is the Codex `hooks.json` mmhmem created for the
    // read-routing hook (on by default), emptied to `{}` (codex.rs).
    let hooks = home.fx.home.home().join(".codex").join("hooks.json");
    let mut expected = home.files_before.clone();
    expected.push(hooks.clone());
    expected.sort();
    let mut after = files(home.fx.home.home(), &backups);
    after.sort();
    assert_eq!(after, expected);
    let left: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&hooks).unwrap()).unwrap();
    assert_eq!(left, serde_json::json!({}));
    assert!(home.memories().join("mmhmem.db").exists());
    let lines = &flow.progress().unwrap().lines;
    let dir = tidy(home.fx.home.home(), &home.memories().display().to_string());
    assert!(lines.contains(&say::memories_kept(&dir)), "{lines:?}");
    assert!(
        lines
            .iter()
            .any(|l| l.starts_with("Copies of your settings"))
    );
}

#[test]
fn the_memories_are_deleted_last_only_when_ticked() {
    let home = installed();
    let flow = remove(&home, true);
    assert_removed(&home, &flow);
    assert!(!home.memories().exists());
    let lines = &flow.progress().unwrap().lines;
    assert!(
        lines.contains(&say::MEMORIES_DELETED.to_owned()),
        "{lines:?}"
    );
}

#[test]
fn a_log_kept_elsewhere_is_deleted_with_its_copies_only_when_ticked() {
    let mut home = installed();
    let log = home.fx.home.home().join("logs").join("audit.jsonl");
    std::fs::create_dir_all(log.parent().unwrap()).unwrap();
    for name in ["audit.jsonl", "audit.jsonl.1", "audit.jsonl.2"] {
        std::fs::write(log.with_file_name(name), b"{}\n").unwrap();
    }
    std::fs::write(log.with_file_name("other.txt"), b"keep").unwrap();
    if let Some(m) = home.fx.deps.memories.as_mut() {
        m.log_file = log.clone();
    }
    remove(&home, true);
    assert!(!log.exists() && !log.with_file_name("audit.jsonl.2").exists());
    assert!(log.with_file_name("other.txt").exists());
}

#[test]
fn cancelling_at_the_review_changes_nothing() {
    let home = installed();
    let before = home.fx.digest();
    let mut flow = home.fx.flow(Newest::Unknown);
    assert_eq!(flow.begin(&HomeAction::Remove), Next::Review);
    flow.toggle_memories();
    assert!(flow.review().memories.unwrap().on);
    assert!(!flow.diff(0).is_empty());
    drop(flow);
    assert_eq!(home.fx.digest(), before);
    assert_eq!(
        home.path.get(),
        Some(
            home.original_path
                .as_ref()
                .unwrap()
                .with_folder(home.fx.deps.binary.parent().unwrap(),)
        )
    );
}

#[test]
fn an_unreadable_tool_file_skips_only_that_tool_and_keeps_mmhmem() {
    let home = installed();
    let opencode = home.file(&[".config", "opencode", "opencode.json"]);
    std::fs::write(&opencode, "{ not json").unwrap();
    let mut flow = home.fx.flow(Newest::Unknown);
    assert_eq!(flow.begin(&HomeAction::Remove), Next::Review);
    let review = flow.review();
    assert_eq!(review.problems.len(), 1, "{review:?}");
    assert!(review.problems[0].starts_with("OpenCode was skipped"));
    assert!(review.memories.is_none());
    assert!(
        review
            .items()
            .iter()
            .all(|i| i.summary != say::CHANGE_PROGRAM)
    );
    flow.toggle_memories();
    assert_eq!(flow.confirm(), Confirmed::Done);
    let progress = flow.progress().unwrap();
    assert_eq!(progress.code, 1);
    assert!(progress.lines.contains(&say::KEPT_FOR_SKIPPED.to_owned()));
    assert_eq!(std::fs::read_to_string(&opencode).unwrap(), "{ not json");
    let codex = home.file(&[".codex", "config.toml"]);
    assert_eq!(
        std::fs::read_to_string(&codex).unwrap(),
        home.expected()[&codex]
    );
    assert_eq!(std::fs::read(&home.fx.deps.binary).unwrap(), PROGRAM);
    assert!(home.memories().join("mmhmem.db").exists());
    let record = home.fx.deps.store.load().unwrap().expect("record kept");
    assert_eq!(
        record.agents.keys().copied().collect::<Vec<_>>(),
        [AgentId::OpenCode]
    );
    assert!(
        home.path
            .get()
            .unwrap()
            .contains(home.fx.deps.binary.parent().unwrap())
    );
}

#[test]
fn the_menu_removes_with_the_memories_option() {
    let home = installed();
    let mut app = home.fx.app(Newest::Unknown);
    let remove_at = app.flow().home().actions.len() - 1;
    press(&mut app, &vec![Key::Down; remove_at]);
    press(&mut app, &[Key::Enter]);
    assert_eq!(app.screen(), Screen::Review);
    let shown = crate::setup::ui::fixtures::render(&app);
    assert!(shown.contains(say::MEMORIES_OPTION), "{shown}");
    press(&mut app, &[Key::Space, Key::Enter]);
    assert_eq!(app.screen(), Screen::Progress);
    assert_removed(&home, app.flow());
    assert!(!home.memories().exists());
}

#[test]
fn the_numbered_fallback_removes_and_keeps_the_memories() {
    let home = installed();
    let mut flow = home.fx.flow(Newest::Unknown);
    let remove_at = flow.home().actions.len();
    let script = format!("{remove_at}\nm\nm\ny\n");
    let mut out = Vec::new();
    let code = crate::setup::ui::fallback::run(&mut flow, &mut script.as_bytes(), &mut out);
    let out = String::from_utf8(out).unwrap();
    assert_eq!(code, 0, "{out}");
    assert!(
        out.contains(&format!("m) [x] {}", say::MEMORIES_OPTION)),
        "{out}"
    );
    assert!(out.trim_end().ends_with(say::REMOVED), "{out}");
    assert_removed(&home, &flow);
    assert!(home.memories().join("mmhmem.db").exists());
}

#[test]
fn quitting_the_fallback_at_the_removal_review_changes_nothing() {
    let home = installed();
    let before = home.fx.digest();
    let mut flow = home.fx.flow(Newest::Unknown);
    let script = format!("{}\nm\nd 1\nq\n", flow.home().actions.len());
    let code = crate::setup::ui::fallback::run(&mut flow, &mut script.as_bytes(), &mut Vec::new());
    assert_eq!(code, EXIT_CANCELLED);
    assert_eq!(home.fx.digest(), before);
}

#[test]
fn a_path_value_ending_in_a_separator_comes_back_byte_identical() {
    let home = installed_with(Some(reg(r"C:\Tools;%USERPROFILE%\bin;")));
    let flow = remove(&home, false);
    assert_removed(&home, &flow);
    assert_eq!(home.path.get(), home.original_path);
}

#[test]
fn a_path_value_mmhmem_created_is_deleted_again() {
    let home = installed_with(None);
    let folder = home.fx.deps.binary.parent().unwrap().to_path_buf();
    assert!(home.path.get().is_some_and(|v| v.contains(&folder)));
    let flow = remove(&home, false);
    assert_removed(&home, &flow);
    assert_eq!(home.path.get(), None);
}

#[test]
fn a_created_value_the_user_added_to_is_kept_without_the_folder() {
    let home = installed_with(None);
    let folder = home.fx.deps.binary.parent().unwrap().to_path_buf();
    let mine = home.path.get().unwrap().with_folder(Path::new(r"C:\mine"));
    home.path.write(Some(&mine)).unwrap();
    remove(&home, false);
    assert_eq!(home.path.get(), Some(mine.without_folder(&folder, false)));
    assert_eq!(
        home.path.get().unwrap().entries(),
        [PathBuf::from(r"C:\mine")]
    );
}

#[test]
fn content_the_user_added_to_a_file_mmhmem_created_is_kept() {
    let home = installed();
    let rules = home.file(&[".claude", "rules", "mmhmem.md"]);
    let installed_text = std::fs::read_to_string(&rules).unwrap();
    std::fs::write(&rules, format!("# My own rules\n\n{installed_text}")).unwrap();
    let flow = remove(&home, false);
    assert_eq!(flow.exit_code(), 0, "{:?}", flow.progress());
    // Exactly the user's bytes before mmhmem's section.
    assert_eq!(
        std::fs::read_to_string(&rules).unwrap(),
        "# My own rules\n\n"
    );
}

#[test]
fn a_program_folder_holding_other_files_is_kept() {
    let home = installed();
    let folder = home.fx.deps.binary.parent().unwrap().to_path_buf();
    std::fs::write(folder.join("my-tool"), b"mine").unwrap();
    let flow = remove(&home, false);
    assert_eq!(flow.exit_code(), 0, "{:?}", flow.progress());
    assert!(!home.fx.deps.binary.exists());
    assert_eq!(std::fs::read(folder.join("my-tool")).unwrap(), b"mine");
}

#[test]
fn a_program_folder_that_existed_before_the_install_is_kept() {
    let mut fx = with_clis();
    for (rel, text) in SEEDED {
        fx.write(rel, text);
    }
    let folder = fx.deps.binary.parent().unwrap().to_path_buf();
    std::fs::create_dir_all(&folder).unwrap();
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("mmhmem"), PROGRAM).unwrap();
    fx.deps.install_from = Some(temp.path().join("mmhmem"));
    install_everything(&fx);
    fx.deps.install_from = None;
    let mut flow = fx.flow(Newest::Unknown);
    flow.begin(&HomeAction::Remove);
    flow.confirm();
    assert_eq!(flow.exit_code(), 0, "{:?}", flow.progress());
    assert!(!fx.deps.binary.exists() && folder.is_dir());
}
