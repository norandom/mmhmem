# Marketplace packaging

Canonical skill: `deploy/skills/unified-memory/SKILL.md`. Metadata and plugin
version: `.marketplace/skills.json`. Generated packages live in `plugins/`;
catalogs live in `.agents/plugins`, `.claude-plugin`, and `.gemini-plugin`.
All URLs point to norandom/mmhmem. The binaries use GitHub's latest release;
the skill catalog initially tracks main because v0.2.2 contains no plugins.

This follows the shared-template design in norandom/Skills (`lib.sh` and
`lib.ps1`, not `lib.psh`). Both builders use the same templates and metadata:

```sh
bash build-marketplace.sh --ref main
```

```powershell
.\build-marketplace.ps1 -Ref main
```

Bash builds require jq; PowerShell builds require PowerShell 5.1+. Commit the
generated packages and catalogs with source changes. Do not hand-edit them.
Plugin versions live outside SKILL.md so its frontmatter stays compatible
with the [Agent Skills specification](https://agentskills.io/specification).
The playbook retains its 200-token estimated budget (words times 1.3).

Validation on Windows uses `powershell -NoProfile -File scripts/test-install.ps1`.
Linux shell validation runs through `scripts/dagger-ci.ps1 -Call distribution`
locally, or the same Dagger function in CI. It tests Debian and Alpine
userlands, validates skill YAML, and checks manifest data and exact skill
contents against the PowerShell-generated files. JSON escaping differences
are normalized before comparison. Do not use Git Bash on Windows or run
these shell checks directly in WSL.

Before a release, build packages and include them in the tagged commit. Only
then pin catalogs with `bash build-marketplace.sh --release TAG` or
`.\build-marketplace.ps1 -Release TAG`. Both refuse tags missing plugin manifests.
Do not point the catalog at an older binary release that lacks the package.

The plugin is skill-only: MCP registration and platform-specific binaries
remain the responsibility of `mmhmem setup`. Catalog installation uses
the [Codex packaging format](https://developers.openai.com/plugins/build/plugins)
and [Claude marketplace format](https://code.claude.com/docs/en/plugin-marketplaces).
Gemini/Antigravity manifests mirror the sibling repository's format; a native
Gemini CLI extension installation is a separate format and is not advertised.
