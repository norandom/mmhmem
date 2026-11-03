# Brief: read-routing-hooks

## Problem

mmhmem saves frontier tokens only when an agent calls `smart_read` and gets
a summary of a large file. Agents rarely do. The audit log on the user's
workstation (2026-11-02 to 2026-11-03) shows:

| Day, harness | Summary reads | Exact slices | Direct reads | Resumes / checkpoints |
|---|---|---|---|---|
| Nov 2, OpenCode | 2 | 0 | 0 | 20 / 12 |
| Nov 3, Claude Code | 0 | 0 | 1 | 40 / 37 |
| Nov 3, Codex | 1 | 46 | 1 | 34 / 31 |
| Nov 3, OpenCode | 0 | 0 | 0 | 4 / 2 |

The whole store holds three savings rows (11,748 estimated tokens). The
Claude Code session of Nov 3 read dozens of large files with native tools.
The user reports the same picture on the server. The advisory routing rules
are ignored under load, as the concept document predicted for advisory
`CLAUDE.md` rules.

`mmhmem stats` also hides this: it shows only savings rows, so an empty
column cannot be told apart from an unused tool.

## User decision (2026-11-03)

"Intelligent, well chosen use of hooks": opt-in, per-harness hooks that
redirect only the reads a summary actually helps, plus adoption visibility
first so the effect can be measured.

## Approach direction (for design, not binding)

1. `mmhmem stats` shows per-harness read activity by route, including hook
   redirects, so a baseline exists before any hook is installed.
2. An opt-in Claude Code hook redirects full native reads of large,
   non-security-critical files to `smart_read`, mirroring the server's own
   routing rules, and passes everything else through.
3. The hook fails open, never blocks when no worker can summarize, and never
   deadlocks an agent that genuinely needs the whole file.
4. Codex has pre-tool hooks behind `[features] hooks = true` and approves each
   hook by content hash (`[hooks.state."<id>"]` with `trusted_hash`,
   `trusted_content_hash`, `enabled`; observed on the user's workstation,
   where the claude-mem plugin already installs Codex hooks). Codex is in
   scope, with its trust model respected. OpenCode follows if its plugin
   interface allows the same contract.
5. Hooks need consent to be accepted: opt-in per harness, user scope only,
   shown before installing, harness approval left to the harness, policy
   respected, stable entries so approvals survive updates, clean removal.

## Boundary Candidates

- Owned: the hook decision logic and its harness adapters, installer
  options to add and remove hooks, the adoption statistics, and hook audit
  events.
- Adjacent: cross-agent-memory-server Requirement 1.5 ("never require
  client-side shell lifecycle hooks"), the shunt engine's routing rules,
  agent-tool-routing (advisory guidance), the installers.
- Not owned: the shunt engine's behaviour, worker summaries, harness
  internals.
