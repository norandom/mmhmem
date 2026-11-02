# Implementation Gap Analysis — corpus-warmup-prune

_Validated 2026-11-02 against the amended requirements (47 ACs, Req 1-10) and the codebase at `d5a11d7`. Fresh evidence gathered this session (file reads/greps cited inline)._

## Analysis Summary

- **The feature is a textbook hybrid (Option C)**: pure orchestration over the existing shunt engine for warmup, new focused modules (`warmup.rs`, `prune.rs`, `breadcrumbs.rs`, `mmhmem_file.rs`) behind established service/handler patterns. No engine refactor, no schema change.
- **All integration seams verified fresh**: `ShuntEngine::smart_read` exposes the full four-stage evaluation via `ReadOutcome` (`src/services/shunt_types.rs:91-118`); the local-profile config seam (`Config::load_local`/`from_env_local`, `src/config/profile.rs:33-81`) accepts closure-injected lookups, so a file fallback composes without touching the pure parser.
- **Three design-doc completeness gaps found** (warmup outcome buckets for `OutsideAllowedRoots`/worker failures; missing `src/config.rs` + `src/mcp/server.rs` entries in Modified Files; steering `structure.md` env-only rule conflict) — all fixable with small design/steering edits, none architectural.
- **One prior-session assumption corrected**: `tests/multiprocess.rs` has NO tool-count pin (verified — only `token_stats`/`sessions` asserts at lines 306/364). The only count pins live in `tests/local_stdio.rs` (test `stdio_handshake_lists_all_six_tools` at line 153 + `assert_eq!(names, 6)` at line 264-265).
- **Effort S-M, Risk Low** — every pattern (service + thin handler, installer copy seam, redaction, audit event + crumb) already exists and is test-covered.

## 1. Current State (verified assets)

| Asset | Evidence | Reuse for |
|---|---|---|
| Shunt evaluation: `ReadOutcome{Targeted, Direct{BypassReason}, Cached, Shunted}` | `src/services/shunt_types.rs:91-118` | Req 1 — warmup counts map 1:1 to these variants |
| `ShuntError{FileNotFound, OutsideAllowedRoots, Io, WorkerNotConfigured, Worker(String)}` | `src/services/shunt_types.rs:120-152` | Req 1.4/1.7 — error mapping below |
| `BypassReason{SecurityCritical, BelowThreshold}` with audit strings | `src/services/shunt_types.rs:7-23` | Req 1.4 skip reasons |
| Content-addressed cache helpers (`cache_lookup`/`cache_store`, fingerprint) | `src/services/shunt_cache.rs` | Req 1.2/1.3; prune scans the same table |
| `shunt_file_cache` PK `(file_path, content_sha256, query_fingerprint)` | `src/storage/schema.rs` | Req 3/4 staleness predicate = missing file or sha mismatch |
| `idx_breadcrumbs_proj(project, created_at_epoch DESC)` | `src/storage/schema.rs` | Req 10 — scan under this index; no new index needed |
| Local-profile config: `load_local()` → pure `from_env_local(&dyn Fn)` | `src/config/profile.rs:33-81` | Req 7 — merge seam |
| `present()`/`optional()` helpers; `ConfigError` enum | `src/config.rs` (enum at line 133) | Req 7 loader |
| `redacted_debug()` already masks `worker_api_key` (+ test) | `src/config.rs:228-251, 459-466` | Req 7.5 — redaction holds by construction for file-sourced keys; task adds a merged-config test |
| Tool handler conventions (`#[tool_router]`, `Parameters<T>`, `ok_text`/`err`, `with_span`) | `src/mcp/tools.rs` (348 LOC) | Req 1.1/4.1/10.1 handlers |
| Param struct pattern (derive `Deserialize+JsonSchema`, doc-comments = schema descriptions) | `src/mcp/params.rs` | Warmup/Prune/BreadcrumbSearch params |
| `AppState::build` service graph + `Services` struct | `src/mcp/server.rs:40-111` | Req 10 — one more service field + construct line |
| `SessionBus.breadcrumb()` + redaction; `MemoryService` audit+crumb parity | `src/services/session_bus.rs`, `src/services/memory.rs` | Req 10.5 per-call discipline |
| CLI runner pattern (read-only open, missing-store message, exit 0) | `src/cli/stats.rs` | Req 3 — `cli/prune.rs` mirrors it |
| Installer skill-copy seams (`install_skill`/`Set-Skill`, scope + dry-run + remove) | `deploy/install_local.{sh,ps1}` | Req 7.6 config write; Req 9 command-template copy |
| Cognitive-complexity clippy gate, size caps | existing CI config | Req 6 (nothing to build) |

## 2. Requirement-to-Asset Map (gaps tagged)

| Req | Need | Status |
|---|---|---|
| 1.1-1.8 | warmup orchestration + handler + params | **Missing**: `services/warmup.rs` NEW; handler thin. Constraint: per-file `smart_read` already audits/stats/crumbs — warmup must NOT double-audit; one batch-summary event (Req 1.8) on top |
| 1.4/1.5 | per-file outcome buckets incl. a failed count | **Constraint/gap in design**: `WarmupReport` lacks buckets for `OutsideAllowedRoots` (a denial, not a miss — requirements.md "Boundary Context" line: warmup honors the same path confinement) and per-file `Worker(String)` failures. See §3 recommendation |
| 2.1 | one playbook line | **Missing** (trivial): SKILL.md currently 4 steps, <200 tokens |
| 3.1-3.6 | stale scan + delete + audit + CLI | **Missing**: `services/prune.rs`, `cli/prune.rs` NEW; `Db` open read-write; busy-timeout contract already in `Config` |
| 4.1-4.3 | prune tool | **Missing** (thin): same service, `dry_run` arg |
| 5.1-5.4 | parity + updated own tests | **Constraint**: the two pins in `tests/local_stdio.rs:153,264-265` → nine; `tests/multiprocess.rs` has NO pin (verified) |
| 6.1-6.5 | caps enforcement | **Existing** — copy into tasks (already done) |
| 7.1-7.7 | `.mmhmem.json` loader + installer write + docs | **Missing**: `config/mmhmem_file.rs` NEW; `ConfigError::InvalidMmhmemFile` variant (enum in `src/config.rs:133`); `mod mmhmem_file` declaration → **design Modified Files omits `src/config.rs`** (gap D2) |
| 8.1-8.2 | session-start notice | **Missing** (thin): handler reads `state.config.worker_model`; response-shape tests exist in `local_stdio.rs` |
| 9.1-9.6 | recap command templates + installer wiring | **Missing**: `deploy/commands/{claude,opencode}/mmhmem-recap.md` + copy/remove/dry-run in both installers (mirror skill seam). Constraint: Claude Desktop + Antigravity get no command files (no user-defined command support) |
| 10.1-10.5 | breadcrumb search service + handler | **Missing**: `services/breadcrumbs.rs` NEW; handler thin. **Design Modified Files omits `src/mcp/server.rs`** (Services gains a field + build line — gap D2) and `src/services/mod.rs` (listed in tree, absent from Modified Files) |

**Research Needed (external)**: none — every dependency/contract was verifiable in-repo this session.

## 3. Key Findings (with recommendations — information, not decisions)

**F1 — Warmup error/outcome mapping (design refinement needed).**
Fresh evidence (`shunt_types.rs:120-152`): per-file failures arrive as `ShuntError`. Design currently names outcomes {delegated, cached, skipped_below_threshold, skipped_security, missing} and leaves two variants unmapped:
- `OutsideAllowedRoots` → recommended: count into a **`denied`** bucket (a denial is not a miss; requirements adjacent line mandates the same confinement rules; the shunt path already audits each denial).
- `Worker(String)` (call failure mid-batch) → recommended: count into a **`failed`** bucket and continue, so one flaky worker call cannot waste the tokens already spent on the rest of the batch; per-file reason in the response details. `WorkerNotConfigured` stays a whole-batch error (Req 1.7 — design already correct).
- AC 1.5's "failed count" is then rendered honestly from the missing+denied+failed buckets (each labeled).

**F2 — Loader merge approach (Req 7.1-7.3).**
`from_env_local` is pure (closure-injected lookup, `profile.rs:41`) and collapses absent-env → defaults via `optional()`. Recommended: compose a **merged lookup closure** in `load_local()` — `env(key).or_else(project-file setting).or_else(global-file setting)` for the three `WORKER_*` keys — so `from_env_local` stays byte-identical, the precedence (env > project > global > defaults) falls out of the closure chain, and the pure parser's test surface is untouched. `MmhmemFile::load()` runs first and fails fast with `InvalidMmhmemFile(path)` on bad JSON (Req 7.3). Serve profile (`from_env`) is never touched (Req 7.4). Redaction: `redacted_debug()` masks `worker_api_key` regardless of source (`config.rs:250`) — holds by construction for file-sourced keys; task 8.1 adds the merged-config redaction test.

**F3 — Design Modified Files completeness (gap D2).**
The list must add: `src/config.rs` (`mod mmhmem_file;` + re-export, `ConfigError::InvalidMmhmemFile`), `src/mcp/server.rs` (`Services` gains the breadcrumb-search service + one build line), and `src/services/mod.rs` (module declarations). All are one-to-few-line touches; the directory tree already shows the new files correctly.

**F4 — Steering conflict (gap D3, owner-flagged).**
`structure.md:54` "Config is environment-only; no config files parsed at runtime" conflicts with Req 7 (local profile only). Recommended steering amendment: environment-only stays true for serve; the local profile additionally reads worker settings from `.mmhmem.json` (global + project, owner decision 2026-11-01). Also `structure.md:16` enumerates the six tools — genericize per the steering's own "document patterns, not exhaustive feature lists" principle.

**F5 — Test-pin truth (corrects prior-session note).**
Only `tests/local_stdio.rs` pins the tool count (two places, lines 153 and 264-265). `tests/multiprocess.rs` has no count pin — the design's "untouched" claim is confirmed, the handoff's gotcha was imprecise.

## 4. Implementation Approach Options

**Option A (extend only)** — squeeze warmup/prune/search into existing modules. Rejected: `shunt.rs` (467 LOC) and `session_bus.rs` (491 LOC) already sit near the 500-LOC house cap; batching logic in `tools.rs` would fatten the thin-handler layer against the layering rule.

**Option B (all new components)** — new engine for warmup, separate prune pipeline, standalone breadcrumb store. Rejected: duplicates the four-stage evaluation, cache, audit, stats, and crumb plumbing; violates "policy lives in services" reuse and doubles the test surface for no benefit.

**Option C (hybrid — design's current shape)** — new focused modules (`services/warmup.rs`, `services/prune.rs`, `services/breadcrumbs.rs`, `config/mmhmem_file.rs`) orchestrating existing engines, plus thin handlers/params/CLI runner and installer seams. ✅ Recommended: honors the layered dependency rule (`mcp → services → storage`), keeps every existing file under its cap, maximizes reuse of verified plumbing, and matches every neighboring pattern (e.g. `MemoryService` + `mem_search`).

## 5. Complexity & Risk

- **Effort: M** (3-7 days) — nine tools' worth of surface, but all shared plumbing verified; the only network path (worker) already exists and is integration-tested.
- **Risk: Low** — extend established patterns; deterministic test discipline throughout; no schema change, no migration, no new external dependency; installers are shell scripts already exercised on host PowerShell + WSL.

## 6. Recommendations for the implementation phase

1. Adopt the closure-composition loader (F2) — smallest diff, zero serve-path change.
2. Extend `WarmupReport` with `denied` + `failed` buckets per F1 before implementing Major 1 (keeps the render honest for AC 1.4/1.5).
3. Fix design Modified Files (F3) and amend steering `structure.md` (F4) before `/kiro-impl`.
4. Implement Major 8 first (loader) — the real install depends on it (owner decision, handoff).
