# Brief: agent-tool-routing

## Problem
Claude Code and OpenCode users connect mmhmem but continue reading files through
built-in tools, leaving the savings features unused.

## Current State
Local metadata-only investigation on 2026-11-03 found 21 built-in reads in each
harness. OpenCode recorded no mmhmem calls; Claude Code used session memory but
no smart reads. Installed skills existed but neither history showed a skill
invocation. A real diagnostic smart_read recorded 4,030 estimated avoided input
tokens immediately in the installed CLI, confirming the worker and shared stats
path. These observations describe this machine, not all harness installations.

Tool descriptions explain mechanics. Server instructions only say to prefer
smart_read. Skill discovery metadata emphasizes memory. Structured MCP tool
annotations are absent. Automatically loaded routing guidance is not installed.

## Desired Outcome
Ordinary file-inspection tasks use mmhmem appropriately without the user naming
the tool. Exact reads and safe fallbacks remain available. Fresh Claude Code and
OpenCode sessions demonstrate actual adoption.

## Approach
Use explicit tool-selection descriptions, truthful MCP annotations, discoverable
skills, and safely managed automatically loaded harness guidance. This follows
the direction agreed in conversation. Description-only changes have lower
installation impact but leave the observed guidance-loading gap. Interception
hooks could enforce routing but conflict with project policy and are excluded.

## Scope
- In: metadata, routing guidance, safe installation/removal, real harness checks.
- Out: hooks, proxies, native-tool disabling, cache algorithm and stats redesign.

## Boundary Candidates
- MCP tool discovery and behavioral hints.
- Portable skill and harness instruction distribution.
- Installer ownership and preservation of user configuration.
- Adoption verification using existing breadcrumbs and counters.

## Upstream / Downstream
- Upstream: cross-agent-memory-server, local-mode, corpus-warmup-prune.
- Adjacent: completed proactive-session-checkpoints; preserve its save rules.
- Downstream: agents choosing tools in ordinary work.

## Constraints
No new service or hook. Preserve existing permissions, allowed roots, redaction,
size limits, direct-read safety paths, and playbook budget. Documentation loading
and MCP annotation semantics must be verified against current official sources
during design. Annotations are hints, not permission grants or routing enforcement.

