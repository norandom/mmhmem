# unified-memory skill

`SKILL.md` is the portable agent playbook (Req 8.3): resume first, prefer
`smart_read` over raw reads of large files (offset/limit for edits), search the
memory index before fetching details, checkpoint before switching harnesses.

Token budget: 97 words ≈ **126 tokens** at the standard ~1.3 tokens/word
estimate — under the 200-token cap (Req 8.3). Re-count when editing
(`(Get-Content SKILL.md -Raw) -split '\s+'`); keep the four required
instructions and stay under budget.
