//! Tests for the pre-write check of `setup::plan` (task 2.5): a stale plan,
//! invalid new content, incoherent changes in one file, malformed changes
//! and the review diff. Temporary folders only.

use std::path::PathBuf;

use super::tests::{
    CLAUDE, CODEX, OPENCODE, SETTINGS, TOML_AFTER, TOML_BEFORE, change, env, file_target,
    hook_group, json_target, outcome, plan, read, toml_target, two_changes,
};
use super::*;
use crate::setup::edit;
use crate::setup::record::InstallRecord;

#[test]
fn stale_plan_writes_nothing_and_is_reported() {
    let e = env();
    let config = e.root.join("config.toml");
    std::fs::write(&config, TOML_BEFORE).unwrap();
    let other = e.root.join("SKILL.md");
    let plan = plan(vec![
        change(
            CODEX,
            toml_target(&config),
            Some(TOML_BEFORE.as_bytes()),
            Some(TOML_AFTER.as_bytes()),
        ),
        change(CLAUDE, file_target(&other), None, Some(b"skill\n")),
    ]);
    let edited = format!("{TOML_BEFORE}theme = \"light\"\n");
    std::fs::write(&config, &edited).unwrap();
    let mut record = InstallRecord::new();
    let report = apply(&plan, &e.store, &mut record);
    let stale = Skip::Stale {
        file: config.clone(),
    };
    assert_eq!(outcome(&report, CODEX), &Outcome::Skipped(stale.clone()));
    assert!(stale.to_string().contains("changed since you reviewed"));
    assert_eq!(read(&config).unwrap(), edited.as_bytes());
    assert!(record.entries(CODEX).is_empty());
    assert!(report.agent(CODEX).unwrap().backups().is_empty());
    assert_eq!(outcome(&report, CLAUDE), &Outcome::Applied);
}

#[test]
fn invalid_new_content_writes_nothing_for_that_agent() {
    let e = env();
    let skill = e.root.join("SKILL.md");
    let config = e.root.join("config.toml");
    let settings = e.root.join("settings.json");
    let strict = e.root.join("strict.json");
    std::fs::write(&strict, "{}\n").unwrap();
    let broken_toml = format!("{TOML_AFTER}[mcp_servers.mmhmem\n");
    let plan = plan(vec![
        change(CODEX, file_target(&skill), None, Some(b"# skill\n")),
        change(
            CODEX,
            toml_target(&config),
            None,
            Some(broken_toml.as_bytes()),
        ),
        change(
            CLAUDE,
            json_target(&settings, &["mcpServers", "mmhmem"]),
            None,
            Some(b"{\"mcpServers\": }"),
        ),
        change(
            OPENCODE,
            json_target(&strict, &["a"]),
            Some(b"{}\n"),
            Some(b"{ // note\n\"a\": 1 }\n"),
        ),
    ]);
    let mut record = InstallRecord::new();
    let report = apply(&plan, &e.store, &mut record);
    for (scope, file) in [(CODEX, &config), (CLAUDE, &settings), (OPENCODE, &strict)] {
        match outcome(&report, scope) {
            Outcome::Skipped(Skip::Invalid { file: f, .. }) => assert_eq!(f, file),
            other => panic!("{scope:?}: {other:?}"),
        }
    }
    let Outcome::Skipped(jsonc) = outcome(&report, OPENCODE) else {
        unreachable!()
    };
    assert!(jsonc.to_string().contains("plain JSON"), "{jsonc}");
    assert!(!skill.exists() && !config.exists() && !settings.exists());
    assert_eq!(read(&strict).unwrap(), b"{}\n");
    assert_eq!(record, InstallRecord::new());
    assert!(
        !e.store.dir().exists(),
        "no backup and no record were written"
    );
}

#[test]
fn incoherent_changes_in_one_file_are_refused() {
    let e = env();
    let settings = e.root.join("settings.json");
    std::fs::write(&settings, SETTINGS).unwrap();
    let (first, second) = two_changes(&settings);
    let mut gap = second.clone();
    gap.before = Some(SETTINGS.as_bytes().to_vec());
    let mut undo = second.clone();
    let mid = std::str::from_utf8(first.after.as_deref().unwrap()).unwrap();
    let removed = edit::json::remove(mid, &["mcpServers", "mmhmem"]).unwrap();
    undo.after = Some(
        edit::json::array_append(&removed, &["hooks", "PreToolUse"], &hook_group())
            .unwrap()
            .into_bytes(),
    );
    for (bad, words) in [(gap, "follow each other"), (undo, "undo an earlier")] {
        let mut record = InstallRecord::new();
        let report = apply(&plan(vec![first.clone(), bad]), &e.store, &mut record);
        let Outcome::Skipped(skip) = outcome(&report, CLAUDE) else {
            panic!("{report:?}")
        };
        assert!(skip.to_string().contains(words), "{skip}");
        assert_eq!(read(&settings).unwrap(), SETTINGS.as_bytes());
    }
}

#[test]
fn malformed_changes_are_refused() {
    let e = env();
    let settings = e.root.join("settings.json");
    std::fs::write(&settings, "{\"a\": 1}").unwrap();
    let delete_entry = change(
        CLAUDE,
        json_target(&settings, &["a"]),
        Some(b"{\"a\": 1}"),
        None,
    );
    let mut keeps = change(
        CLAUDE,
        json_target(&settings, &["a"]),
        Some(b"{\"a\": 1}"),
        Some(b"{\"a\": 2}"),
    );
    keeps.action = Action::Remove;
    let mut wrong_file = change(
        CLAUDE,
        json_target(&settings, &["a"]),
        Some(b"{\"a\": 1}"),
        Some(b"{\"a\": 2}"),
    );
    wrong_file.file = e.root.join("other.json");
    let path_entry = Change {
        target: Target::PathEntry {
            value: e.root.clone(),
        },
        ..wrong_file.clone()
    };
    for bad in [delete_entry, keeps, wrong_file, path_entry] {
        let mut record = InstallRecord::new();
        let report = apply(&plan(vec![bad.clone()]), &e.store, &mut record);
        assert!(
            matches!(
                outcome(&report, CLAUDE),
                Outcome::Skipped(Skip::Invalid { .. })
            ),
            "{bad:?}: {report:?}"
        );
    }
    assert_eq!(read(&settings).unwrap(), b"{\"a\": 1}");
}

#[test]
fn diff_is_unified_and_hides_what_it_must() {
    let file = PathBuf::from("/home/u/SKILL.md");
    let add = change(CODEX, file_target(&file), None, Some(b"one\ntwo\n"));
    let diff = add.diff();
    assert!(
        diff.contains("(new file)") && diff.contains("+one") && diff.contains("+two"),
        "{diff}"
    );
    let update = change(
        CODEX,
        file_target(&file),
        Some(b"one\ntwo\n"),
        Some(b"one\nthree\n"),
    );
    let diff = update.diff();
    assert!(
        diff.contains("-two") && diff.contains("+three") && diff.contains("@@"),
        "{diff}"
    );
    let same = change(CODEX, file_target(&file), Some(b"x\n"), Some(b"x\n"));
    assert_eq!(same.diff(), NO_DIFF);
    let binary = change(CODEX, file_target(&file), None, Some(&[0xff, 0x00, 0xfe]));
    assert_eq!(binary.diff(), BINARY_DIFF);
}
