# Brief: corpus-warmup-prune

## Problem
The token-savings corpus accrues only from live tool activity: a fresh install (or a new project) has an empty shunt cache and empty stats, so nothing is saved until agents organically read the big files. And after a big refactor, the content-addressed shunt cache accumulates stale rows (file gone or content changed) that are never served again and never removed.

## Current State
mmhmem ships six MCP tools (serve + local modes, parity by construction), a 160-test deterministic suite (97 pre-existing — the protected suite — plus 63 local-mode tests), and CLI surfaces `serve`/`stdio`/`stats`/`healthcheck`. The shunt cache is content-addressed (file, SHA-256, fingerprint) with no eviction. Breadcrumbs are auto-capped at 15 tool actions.

## Desired Outcome
- An agent-callable `shunt_warmup` MCP tool: pass an explicit list of files; each eligible file runs the real shunt path (worker delegation → cache row + real `shunt_delegated` stats row); already-cached → cache hit; below-threshold/security-critical → skipped with reason; missing → reported without failing the batch; batch cap 25; clear error without a worker; response summarizes counts and worker tokens. The SKILL.md playbook offers it ("new project? warm the key files"), still < 200 tokens.
- Pruning for the big-change case, on two surfaces: `mmhmem prune` CLI (stale-row removal: file missing or content hash changed; dry-run report; grouped counts; store-missing → message + exit 0) and a `shunt_prune` MCP tool with identical staleness semantics plus a dry_run preview argument. Prune touches only shunt-cache rows — never stats, observations, sessions, or breadcrumbs.

## Approach (user-confirmed 2026-11-01)
- Warmup input shape: explicit paths list (agent curates; cap bounds worker spend).
- Prune surfaces: CLI + MCP tool.
- Process: small follow-up spec (quick spec), TDD implementation.

## Scope
- **In**: `shunt_warmup` tool + skill line; `prune` CLI + `shunt_prune` tool; audit events for both; docs updates (README tool list, help text); tests. Shared handlers → both modes get both tools with identical semantics.
- **Out**: TTL/size-based cache eviction; synthetic or seeded statistics rows (honesty discipline: never present computed as measured); schema changes; auto-prune triggers; cross-machine anything.

## Constraints
- The pre-existing deterministic suite passes unmodified (it does not pin the tool count — verified; local-mode's own tests that pinned six tools are updated by this spec).
- Req 13 house caps apply (cognitive ≤ 15, ≤ 500 LOC/src file, ≤ 80 LOC/fn, ≤ 400 non-test LOC/task, tests ≤ 800 LOC/file).
- No new crates. English artifacts.

## Existing Spec Touchpoints
- **Extends**: `cross-agent-memory-server` (tool surface grows six → eight) and `local-mode` (its own tests updated where they pinned the six-tool list; storage/busy-timeout model reused).
- **Adjacent**: none.
