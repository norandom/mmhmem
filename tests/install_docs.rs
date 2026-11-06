//! Docs check for the install commands (reliable-installers tasks 9.1 and
//! 9.2): README.md shows the two release one-liners before any other install
//! text, user docs point at release URLs only and carry no checkout-based or
//! old-installer instructions, the release notes template starts with the
//! same one-liners, and the root `install.ps1` / `install.sh` stubs print
//! them without any download command.

use std::fs;
use std::path::{Path, PathBuf};

const WINDOWS: &str =
    "irm https://github.com/norandom/mmhmem/releases/latest/download/install.ps1 | iex";
const UNIX: &str =
    "curl -fsSL https://github.com/norandom/mmhmem/releases/latest/download/install.sh | sh";
const RELEASE_URLS: [&str; 2] = [
    "https://github.com/norandom/mmhmem/releases/latest/download/",
    "https://github.com/norandom/mmhmem/releases/download/",
];

fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn read(relative: &str) -> String {
    fs::read_to_string(root().join(relative))
        .unwrap_or_else(|e| panic!("{relative}: {e}"))
        .replace("\r\n", "\n")
}

/// The contents of every fenced code block, in order.
fn code_blocks(text: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut current: Option<Vec<&str>> = None;
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            match current.take() {
                Some(lines) => blocks.push(lines.join("\n").trim().to_owned()),
                None => current = Some(Vec::new()),
            }
        } else if let Some(lines) = current.as_mut() {
            lines.push(line);
        }
    }
    blocks
}

/// Docs written for people who install and use mmhmem. `docs/development.md`
/// is for contributors: it describes the old engine and the stubs that stay
/// until task 9.3, so it is checked for URLs only (see [`all_docs`]).
fn user_docs() -> Vec<PathBuf> {
    let mut docs: Vec<PathBuf> = all_docs()
        .into_iter()
        .filter(|p| !p.ends_with("docs/development.md"))
        .collect();
    for extra in [
        "deploy/README.md",
        ".marketplace/README.md",
        "deploy/skills/unified-memory/README.md",
        "plugins/unified-memory/skills/unified-memory/README.md",
    ] {
        docs.push(root().join(extra));
    }
    docs
}

/// README.md and every `docs/*.md`.
fn all_docs() -> Vec<PathBuf> {
    let mut docs = vec![root().join("README.md")];
    let mut dir: Vec<PathBuf> = fs::read_dir(root().join("docs"))
        .expect("docs/")
        .map(|e| e.expect("docs entry").path())
        .filter(|p| p.extension().is_some_and(|x| x == "md"))
        .collect();
    dir.sort();
    docs.extend(dir);
    docs
}

fn relative(path: &Path) -> String {
    path.strip_prefix(root())
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

#[test]
fn readme_starts_with_the_two_release_one_liners() {
    let readme = read("README.md");
    let blocks = code_blocks(&readme);
    assert!(
        blocks.len() >= 2,
        "README.md has fewer than two code blocks"
    );
    assert_eq!(blocks[0], WINDOWS, "README.md's first code block");
    assert_eq!(blocks[1], UNIX, "README.md's second code block");
    let first_install = readme
        .lines()
        .find(|l| {
            [
                "irm ",
                "iwr ",
                "curl ",
                "wget ",
                "install.ps1",
                "install.sh",
                "git clone",
                "cargo ",
            ]
            .iter()
            .any(|w| l.contains(w))
        })
        .expect("README.md has install text");
    assert_eq!(
        first_install, WINDOWS,
        "the first install text in README.md"
    );
    let install = readme
        .find("## Install")
        .expect("README.md has an Install section");
    assert!(readme.find(WINDOWS).expect("one-liner") > install);
}

#[test]
fn install_urls_in_docs_are_release_urls_only() {
    for path in all_docs().iter().chain(user_docs().iter()) {
        let text = fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", relative(path)));
        for word in text.split(|c: char| c.is_whitespace() || "`()<>[]\"'".contains(c)) {
            if !(word.contains("/install.ps1") || word.contains("/install.sh")) {
                continue;
            }
            assert!(
                RELEASE_URLS.iter().any(|u| word.starts_with(u)),
                "{}: install URL is not a release URL: {word}",
                relative(path)
            );
        }
    }
}

#[test]
fn user_docs_have_no_checkout_or_old_installer_instructions() {
    // Old PowerShell flags start with a space, so "Multi-Agent " is not one.
    let forbidden = [
        "raw.githubusercontent.com/norandom/mmhmem/HEAD/install",
        "install_local",
        "bash -s --",
        ".\\install.ps1",
        "-OutFile install.ps1",
        "--hooks ",
        " -Hooks ",
        "--remove-hooks",
        " -RemoveHooks",
        "--worker-model",
        " -WorkerModel",
        "--agent ",
        " -Agent ",
    ];
    for path in user_docs() {
        let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", relative(&path)));
        for bad in forbidden {
            assert!(!text.contains(bad), "{} contains {bad:?}", relative(&path));
        }
    }
    // A checkout is for the optional server (docs/server.md) and for
    // contributors, never for installing mmhmem.
    for doc in ["README.md", "docs/installation.md"] {
        let text = read(doc);
        for bad in ["git clone", "cargo build", "cargo install"] {
            assert!(!text.contains(bad), "{doc} contains {bad:?}");
        }
    }
}

#[test]
fn release_notes_template_starts_with_the_one_liners() {
    let notes = read(".github/release-notes.md");
    let blocks = code_blocks(&notes);
    assert!(
        blocks.len() >= 2,
        "release notes have fewer than two code blocks"
    );
    assert_eq!(blocks[0], WINDOWS);
    assert_eq!(blocks[1], UNIX);
    assert!(notes.contains("gh attestation verify <archive> --repo norandom/mmhmem"));
    assert!(notes.contains("mmhmem setup") && notes.contains("mmhmem update"));
}

/// The stub's code lines without the printed one-liner and comments.
fn stub_code(text: &str, one_liner: &str) -> String {
    text.lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .map(|l| l.replace(one_liner, ""))
        .collect::<Vec<_>>()
        .join("\n")
}

fn has_word(text: &str, word: &str) -> bool {
    text.match_indices(word).any(|(i, _)| {
        let before = text[..i].chars().next_back();
        let after = text[i + word.len()..].chars().next();
        let edge = |c: Option<char>| c.is_none_or(|c| !(c.is_ascii_alphanumeric() || c == '-'));
        edge(before) && edge(after)
    })
}

#[test]
fn root_stubs_print_the_readme_one_liners_and_download_nothing() {
    let ps1 = read("install.ps1");
    assert!(
        ps1.lines()
            .any(|l| l.trim() == format!("Write-Host '  {WINDOWS}'")),
        "install.ps1 does not print the Windows one-liner"
    );
    assert!(ps1.contains("$global:LASTEXITCODE = 2") && ps1.contains("exit 2"));
    let code = stub_code(&ps1, WINDOWS).to_ascii_lowercase();
    for word in [
        "invoke-webrequest",
        "invoke-restmethod",
        "iwr",
        "irm",
        "webclient",
        "httpclient",
        "start-bitstransfer",
        "curl",
        "wget",
    ] {
        assert!(!has_word(&code, word), "install.ps1 uses {word}");
    }
    let sh = read("install.sh");
    assert!(sh.starts_with("#!/bin/sh\n"), "install.sh must be POSIX sh");
    assert!(
        sh.lines().any(|l| l == format!("  {UNIX}")),
        "install.sh does not print the Unix one-liner"
    );
    assert!(sh.trim_end().ends_with("exit 2"));
    let code = stub_code(&sh, UNIX);
    for word in ["curl", "wget", "fetch", "nc", "python", "python3"] {
        assert!(!has_word(&code, word), "install.sh uses {word}");
    }
}
