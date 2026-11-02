# unified-memory skill

`SKILL.md` is the portable agent playbook (Req 8.3): resume first, prefer
`smart_read` over raw reads of large files (offset/limit for edits), search the
memory index before fetching details, checkpoint before switching harnesses.

Token budget: keep the complete skill below **200 estimated tokens** at the
standard ~1.3 tokens/word estimate (Req 8.3). Re-count when editing
(`(Get-Content SKILL.md -Raw) -split '\s+'`); keep the four required
instructions and stay under budget.

The canonical skill is packaged by the root marketplace builders. Plugin
versions belong in `.marketplace/skills.json`, not unsupported top-level
SKILL.md fields. See the repository README for release and marketplace installs.
