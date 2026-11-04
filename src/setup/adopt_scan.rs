//! Reading and checking the files of the adoption scan (task 2.6).
//!
//! Each file is read once. A JSON/JSONC file must parse; a TOML file must
//! parse and, when it holds the old `# BEGIN mmhmem` block, the markers must
//! match (the block is then converted in memory with
//! `edit::toml::convert_marker_block`). Whether an artefact is there is
//! decided with the same `observe_in` the drift check uses, so the scan and
//! the later record agree. Nothing is written.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

use super::table::Spec;
use super::{AgentScan, Artefact, ArtefactKind, Format, Origin, Problem, ProblemKind};
use crate::setup::edit;
use crate::setup::plan::target_file;
use crate::setup::record::{AgentId, KeySegment, Observation, Target, observe_in};

/// What reading one file gave.
enum Loaded {
    Absent,
    Failed(ProblemKind),
    Ready { before: Vec<u8>, after: Vec<u8> },
}

/// What one spec found in its file.
enum Outcome {
    Nothing,
    Found(Box<Artefact>),
    Foreign,
    Problem(Problem),
}

/// Check every spec against the disk, grouped by agent in agent order.
pub(super) fn scan_agents(specs: Vec<Spec>) -> Vec<AgentScan> {
    let mut loaded: BTreeMap<PathBuf, Loaded> = BTreeMap::new();
    let mut agents: BTreeMap<AgentId, AgentScan> = BTreeMap::new();
    for spec in specs {
        let Some(file) = target_file(&spec.target).map(Path::to_path_buf) else {
            continue;
        };
        let content = loaded
            .entry(file.clone())
            .or_insert_with(|| load(&spec.target, &file));
        let scan = agents.entry(spec.agent).or_insert_with(|| AgentScan {
            agent: spec.agent,
            artefacts: Vec::new(),
            problems: Vec::new(),
            foreign: Vec::new(),
        });
        match evaluate(&spec, &file, content) {
            Outcome::Nothing => {}
            Outcome::Found(artefact) => scan.artefacts.push(*artefact),
            Outcome::Foreign => scan.foreign.push(file),
            Outcome::Problem(problem) if !scan.problems.contains(&problem) => {
                scan.problems.push(problem);
            }
            Outcome::Problem(_) => {}
        }
    }
    agents
        .into_values()
        .filter(|a| !a.artefacts.is_empty() || !a.problems.is_empty() || !a.foreign.is_empty())
        .collect()
}

fn unexpected(reason: &str) -> Loaded {
    Loaded::Failed(ProblemKind::Unexpected(reason.to_owned()))
}

/// Read `file` and check it parses as its target's format.
fn load(target: &Target, file: &Path) -> Loaded {
    let before = match std::fs::read(file) {
        Ok(bytes) => bytes,
        Err(e)
            if matches!(
                e.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
            ) =>
        {
            return Loaded::Absent;
        }
        Err(e) => return Loaded::Failed(ProblemKind::Unreadable(e.kind().to_string())),
    };
    let text_format = match target {
        Target::JsonEntry { .. } | Target::JsonArrayElement { .. } => Some(Format::Json),
        Target::TomlTable { .. } => Some(Format::Toml),
        _ => None,
    };
    let Some(format) = text_format else {
        return Loaded::Ready {
            after: before.clone(),
            before,
        };
    };
    let Ok(text) = std::str::from_utf8(&before) else {
        return unexpected("is not UTF-8 text");
    };
    let after = match (format, target) {
        (Format::Json, _) => edit::json::dialect(text)
            .map(|_| text.to_owned())
            .map_err(|_| ProblemKind::NotValid(Format::Json)),
        (Format::Toml, Target::TomlTable { key, .. }) => toml_after(text, key),
        (Format::Toml, _) => Ok(text.to_owned()),
    };
    match after {
        Ok(after) => Loaded::Ready {
            after: after.into_bytes(),
            before,
        },
        Err(kind) => Loaded::Failed(kind),
    }
}

/// The TOML text as adopted: valid, with the old marker lines removed.
fn toml_after(text: &str, key: &[KeySegment]) -> Result<String, ProblemKind> {
    edit::toml::validate(text).map_err(|_| ProblemKind::NotValid(Format::Toml))?;
    if !edit::toml::has_marker_block(text) {
        return Ok(text.to_owned());
    }
    edit::toml::convert_marker_block(text, &segments(key)).map_err(|e| {
        ProblemKind::Unexpected(format!("has mmhmem marker lines that do not match ({e})"))
    })
}

fn segments(path: &[KeySegment]) -> Vec<&str> {
    path.iter().map(KeySegment::as_str).collect()
}

fn evaluate(spec: &Spec, file: &Path, loaded: &Loaded) -> Outcome {
    let (before, after) = match loaded {
        Loaded::Absent => return Outcome::Nothing,
        Loaded::Failed(kind) => {
            return Outcome::Problem(Problem {
                file: file.to_path_buf(),
                kind: kind.clone(),
            });
        }
        Loaded::Ready { before, after } => (before, after),
    };
    if spec.signature.is_some_and(|sig| !before.starts_with(sig)) {
        return Outcome::Foreign;
    }
    let problem = |reason: &str| {
        Outcome::Problem(Problem {
            file: file.to_path_buf(),
            kind: ProblemKind::Unexpected(reason.to_owned()),
        })
    };
    match observe_in(&spec.target, after) {
        Observation::Missing => Outcome::Nothing,
        Observation::Unreadable(reason) => problem(&reason),
        Observation::Present(_) if !entry_is_object(&spec.target, after) => {
            problem("holds an mmhmem entry that is not a settings table")
        }
        Observation::Present(_) => Outcome::Found(Box::new(Artefact {
            kind: spec.kind,
            target: spec.target.clone(),
            file: file.to_path_buf(),
            origin: origin(spec, before, after),
            before: before.clone(),
            after: after.clone(),
        })),
    }
}

/// Whether the entry a JSON/TOML target names is an object (a table).
fn entry_is_object(target: &Target, bytes: &[u8]) -> bool {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return true;
    };
    match target {
        Target::JsonEntry { pointer, .. } => edit::json::get(text, &segments(pointer))
            .ok()
            .flatten()
            .is_none_or(|v| v.is_object()),
        Target::TomlTable { key, .. } => edit::toml::get_item_json(text, &segments(key))
            .ok()
            .flatten()
            .is_none_or(|v| v.is_object()),
        _ => true,
    }
}

/// The old installers' signatures: the Codex marker block, the plugin
/// header (checked in `evaluate`) and the routing section's prefix line.
fn origin(spec: &Spec, before: &[u8], after: &[u8]) -> Origin {
    let marked = match spec.kind {
        ArtefactKind::McpEntry => before != after,
        ArtefactKind::ReadPlugin => true,
        ArtefactKind::RoutingSection => {
            let text = String::from_utf8_lossy(before);
            text.contains("<!-- mmhmem:routing:prefix=")
        }
        _ => false,
    };
    if marked {
        Origin::OldInstaller
    } else {
        Origin::Unknown
    }
}
