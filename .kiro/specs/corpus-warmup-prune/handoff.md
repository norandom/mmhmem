# handoff — corpus-warmup-prune (COMPLETE) + next steps

**Status 2026-11-02: SPEC COMPLETE — /kiro-validate-impl verdict GO.** All 30 tasks `[x]`, 47/47 AC coverage, full gates green (fmt, clippy -D warnings, 218 tests 0 failed incl. loc_caps house caps), protected-suite diff proof recorded in research.md. HEAD = a7e85b0 on main, all pushed.

## Shipped in this spec (commits 9cd27fe..a7e85b0)

- **Req 1 warmup**: `services/warmup.rs` — `run_warmup(engine, audit, paths ≤25, question, ctx)` over the REAL smart_read path; `WarmupReport{delegated, cached, skipped_below_threshold, skipped_security, missing, denied, failed, worker tokens, notes}`; `shunt_warmup` tool; one WARMUP audit event/batch; `ReadOutcome::Shunted` gained `usage: WorkerUsage`.
- **Req 3/4 prune**: `services/prune.rs` — `scan_stale` (preview) / `delete_stale` (exact stale rows by DISTINCT (file_path, content_sha256) pairs, one PRUNE event); `PruneService` handle; `mmhmem prune [--dry-run]` CLI; `shunt_prune` tool.
- **Req 7 .mmhmem.json**: `config/mmhmem_file.rs` loader (env > project > global > defaults per setting; invalid JSON fail-fast naming file; BOM tolerated; key redacted source-agnostically); installers WRITE the file (`-WorkerModel/-WorkerUrl/-WorkerApiKey`, `--worker-*`) with JSON-merge preserving other keys, BOM-less UTF8 on Windows; serve stays env-only; steering structure.md amended.
- **Req 8 notice**: session-start responses append the worker-unconfigured notice (gated on `AppState.worker_configured`); absent when configured; checkpoint branch untouched.
- **Req 9 recap**: `deploy/commands/{claude,opencode}/mmhmem-recap.md` templates; installers copy/remove them scope-matched (claude → `.claude/commands/`, opencode → `.opencode/command/`); desktop/antigravity none; playbook recap line (token-capped); README day-to-day.
- **Req 10 breadcrumb search**: `services/breadcrumbs.rs` — read-only LIKE-escaped substring search (tool/resource/summary), newest first, limit 20/100, optional harness filter, blank = list; `mem_search_breadcrumbs` tool; audit MEM_SEARCH_BREADCRUMBS + own crumb (mem_search parity).
- Tool surface: NINE tools; tests/local_stdio.rs count updated; playbook <200 tokens; README nine-tool list + warmup/prune/recap sections.

## NEXT SESSION (user-decided, not started)

1. **Real install (user decision from before: do it now that .mmhmem.json landed)**: `cargo build --release` FIRST (current target/release binary is STALE pre-local-mode), then run `deploy/install_local.ps1 -Global -Agent claude,opencode` (or all harnesses) with the user's Mistral worker settings — key goes into `~/.mmhmem.json`, NEVER committed. Mistral: model `mistral-small-2603`, URL `https://api.mistral.ai/v1/chat/completions`. ASK THE USER before touching their real global config; the key must come from them.
2. **adaptive-cache follow-up spec** (user-selected): draft via /kiro-spec-quick or step-by-step — schema v2 (`last_used_at_epoch` + optional `hit_count` on shunt_file_cache), byte-budget env var (e.g. AGENT_MEM_CACHE_MAX_BYTES ~32 MiB default), cost-aware eviction (recency × reuse × regeneration cost via line_count), per-project accounting decision (table has NO project column — path-prefix rule or v2 column), cache observability in `mmhmem stats` (rows, bytes, hit rate, evictions). Self-tuning budget explicitly REJECTED (non-deterministic). Sequenced AFTER corpus-warmup-prune (done — unblocked).
3. Optional: breadcrumb-search surfacing beyond the tool (e.g. deeper-than-15 resume) — not requested, skip unless asked.

## Standing mandates + gotchas

- Foreground only, no subagents; one conventional commit per task, push after each; English artifacts; house caps enforced BY TEST (tests/loc_caps.rs: ≤500 LOC/src, ≤80 LOC/fn, ≤800 LOC/test) — run it after every src change.
- Gate hygiene on Windows: `cmd /c "cargo test > file 2>&1"` then `findstr /C:"FAILED" file` (exit 1 = green); NEVER chain `%ERRORLEVEL%` in one cmd line (parse-time expansion); PS Select-String is case-INSENSITIVE ('FAILED' matches '0 failed'); PS 2>&1 wraps native stderr as NativeCommandError and can flip exit codes.
- Complex PS one-liners lose `$vars` → write a .ps1 under `C:\Users\mariu\AppData\Local\Temp\opencode\` and run via -File.
- rusqlite `with_conn` closures are `'static + Send`: clone req fields to owned Strings BEFORE the closure (E0521). Db is Clone; AuditWriter is Clone (shared handle).
- Cache keys use the CANONICAL path — seed tests via `std::fs::canonicalize` (Windows \\?\ prefix).
- Adding an enum field breaks exhaustive destructures across tests/ — grep tests/ for the variant first.
- WSL distro has NO python3: .sh json_merge falls back to manual instructions (honest, key withheld); python3 merge path unexercised locally (pre-existing code).
- Pre-existing flake: tests/multiprocess two-process handshake failed ONCE under full-suite parallel load; passed in every subsequent run.
- Never add .openidea/ (foreign). `.mmhmem.json` is gitignored.
