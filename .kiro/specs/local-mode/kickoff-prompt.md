# Kickoff prompt: mmhmem local-mode, early kiro spec phases

Paste this into a fresh agent session (or run it in the current one).

---

**Role.** You are the spec author in the mmhmem repo, running the kiro
spec-driven workflow under AGENTS.md.

**Task.** Run the early kiro spec phases for the `local-mode` feature:
`/kiro-spec-init local-mode`, then `/kiro-spec-requirements local-mode`.
A discovery brief already exists at `.kiro/specs/local-mode/brief.md` —
read it FIRST; it carries every user-confirmed decision. Stop after
requirements and present them for human review; do not proceed to design
without approval.

**Context (facts a fresh session cannot know).**
- Repo `D:\Source\mmhmem`, branch `main`, remote `origin`. The
  `cross-agent-memory-server` spec is validated and complete (25/25 tasks,
  97/97 tests, `spec.json` phase `validated`). Do not reopen it. local-mode
  EXTENDS it per the brief's "Existing Spec Touchpoints".
- All boundary decisions are already made and user-confirmed (2026-11-01);
  requirements must realize them, not re-open them: stdio MCP via rmcp
  `transport-io` (viability verified); shared per-user store so all six
  tools work locally incl. same-machine handoff; `mmhmem stats` CLI over
  the local token_stats; one binary, two transports, two config profiles.
- The user runs PARALLEL sessions: before writing anything, run
  `git status --porcelain` and `git log --oneline -5`; never overwrite
  another session's uncommitted files; one conventional commit per phase,
  push to origin.
- Foreground only, no subagents (user mandate; documented in
  `.kiro/specs/cross-agent-memory-server/handover.md`).

**Constraints.**
- English artifacts; EARS acceptance criteria with numeric IDs, in the
  style of the existing requirements.md (Req 13 caps are the house rule).
- No serve-mode regressions: the 97/97 suite must stay green; state this
  as an explicit requirement.
- Surface the adjacent expectation explicitly: local-mode's design
  supersedes the existing spec's single-connection Db invariant
  (multi-process-safe WAL, busy timeout); the server suite revalidates
  after that lands.
- Steering: `.kiro/steering/{product,tech,structure,dev-environment}.md`;
  the deployment model (server optional, clients local on lin/win/macos)
  is already updated in `product.md`.

**Example.** `.kiro/specs/cross-agent-memory-server/requirements.md` is the
living reference for structure, EARS phrasing, boundary context, and how
Req 13 caps appear in acceptance criteria.

**Output.** `spec.json` + a complete `requirements.md` under
`.kiro/specs/local-mode/` per the kiro templates, followed by a short
requirement-areas summary for review.
