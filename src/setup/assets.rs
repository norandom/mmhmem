//! Files the agents install, embedded in the binary (reliable-installers
//! task 3.1, `.kiro/specs/reliable-installers/design.md` "Source layout";
//! Requirements 2.1 and 8.1).
//!
//! The sources stay under `deploy/` while the old installers still copy them
//! (until task 9.3 moves them to `assets/setup/`); the bytes embedded here
//! are exactly those files.

/// One embedded file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Asset {
    /// The file name it is installed under.
    pub file_name: &'static str,
    /// Its source in the repository, relative to the repository root.
    pub source: &'static str,
    /// The contents.
    pub text: &'static str,
}

impl Asset {
    /// The contents as bytes, for a [`crate::setup::plan::Change`].
    pub fn bytes(&self) -> &'static [u8] {
        self.text.as_bytes()
    }
}

macro_rules! asset {
    ($file_name:literal, $source:literal) => {
        Asset {
            file_name: $file_name,
            source: $source,
            text: include_str!(concat!("../../", $source)),
        }
    };
}

/// The `unified-memory` skill (`SKILL.md`) for every agent with skills.
pub const UNIFIED_MEMORY_SKILL: Asset = asset!("SKILL.md", "deploy/skills/unified-memory/SKILL.md");
/// The Codex `mmhmem-recap` skill (`SKILL.md`).
pub const CODEX_RECAP_SKILL: Asset =
    asset!("SKILL.md", "deploy/codex/skills/mmhmem-recap/SKILL.md");
/// The Claude Code `/mmhmem-recap` command.
pub const CLAUDE_RECAP_COMMAND: Asset =
    asset!("mmhmem-recap.md", "deploy/commands/claude/mmhmem-recap.md");
/// The OpenCode `/mmhmem-recap` command.
pub const OPENCODE_RECAP_COMMAND: Asset = asset!(
    "mmhmem-recap.md",
    "deploy/commands/opencode/mmhmem-recap.md"
);
/// The Codex `mmhmem-recap` prompt.
pub const CODEX_RECAP_PROMPT: Asset =
    asset!("mmhmem-recap.md", "deploy/commands/codex/mmhmem-recap.md");
/// The read-routing instructions (the body of the delimited section).
pub const ROUTING_INSTRUCTIONS: Asset =
    asset!("mmhmem-routing.md", "deploy/instructions/mmhmem-routing.md");
/// The OpenCode read-routing plugin.
pub const OPENCODE_READ_PLUGIN: Asset = asset!(
    "mmhmem-read-routing.js",
    "deploy/opencode/mmhmem-read-routing.js"
);

/// Every embedded file.
pub const ALL: [Asset; 7] = [
    UNIFIED_MEMORY_SKILL,
    CODEX_RECAP_SKILL,
    CLAUDE_RECAP_COMMAND,
    OPENCODE_RECAP_COMMAND,
    CODEX_RECAP_PROMPT,
    ROUTING_INSTRUCTIONS,
    OPENCODE_READ_PLUGIN,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assets_are_non_empty_and_match_the_deploy_files_byte_for_byte() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        for asset in ALL {
            assert!(!asset.text.trim().is_empty(), "{} is empty", asset.source);
            let on_disk = std::fs::read(root.join(asset.source)).expect(asset.source);
            assert_eq!(
                asset.bytes(),
                on_disk.as_slice(),
                "{} differs",
                asset.source
            );
            assert!(asset.source.ends_with(asset.file_name), "{}", asset.source);
        }
    }

    #[test]
    fn every_source_is_embedded_once() {
        let mut sources: Vec<&str> = ALL.iter().map(|a| a.source).collect();
        sources.sort_unstable();
        sources.dedup();
        assert_eq!(sources.len(), ALL.len());
    }
}
