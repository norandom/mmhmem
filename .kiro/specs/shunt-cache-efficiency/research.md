# Research & Design Decisions

## Summary
- **Feature**: `shunt-cache-efficiency`
- **Discovery Scope**: Extension of `read-routing-hooks`, `corpus-warmup-prune` and the shunt service
- **Key Findings**:
  - The file cache key `(file_path, content_sha256, query_fingerprint)` defeats content addressing in practice: 105 distinct question fingerprints over 144 rows, 22 repeat summaries of unchanged content, 13 hits.
  - The hook passes any read with a range, whatever its size; 273 of 404 redirects were followed by a native ranged read of the same file within three minutes; 34 became summaries.
  - The existing `shunt_content_cache` table already has the right shape (content hash + question + kind, hits, last-hit LRU, cap, audited eviction); unifying the file read path on it needs a migration, not a new table.

## Research Log

### Why the cache misses (audit log of the owner's machine, 2026-11-10)
- **Context**: `mmhmem stats` showed 120 summaries and 11 cache hits across a week of use by Claude Code, Codex and OpenCode.
- **Sources Consulted**: `%APPDATA%\mmhmem\audit.jsonl` (`IO_SHUNT_DELEGATED`, `IO_SHUNT_CACHE_HIT`, `IO_TARGETED_READ`, `IO_DIRECT_READ`, `HOOK_*` records), `memory.db` tables `shunt_file_cache` and `shunt_content_cache` (read-only), `src/services/shunt_cache.rs`, `src/storage/schema.rs`. Analysis scripts in the session scratchpad (`stats/hook_seq.py`, `stats/cache_analysis.py`).
- **Findings**:
  - 144 summaries produced, 13 cache hits, 1,230 slices, 133 direct reads. 122 distinct content hashes; 17 hashes summarised more than once (22 wasted worker calls, 161,534 worker prompt tokens). Repeat gaps: median 6.4 hours; 1 of 22 under 10 minutes.
  - `shunt_file_cache` has 144 rows with 105 distinct `query_fingerprint`s; only 31 rows use the default question's fingerprint. One hash has six rows for six wordings. Two hashes also differ by path only (copies or worktrees).
  - `shunt_content_cache` (the `shunt_content` tool's cache) is empty: 0 rows.
  - The file cache has no `hits`/`last_hit` columns, no cap, no TTL; the content cache has all three (`SHUNT_CONTENT_CACHE_MAX_ROWS`, default 5000, LRU eviction audited as `CONTENT_CACHE_EVICT`).
  - Summarised files: min 152, median 310, p90 626, max 2,714 lines. Avoided-estimate 872,109 tokens against 1,305,021 worker prompt tokens: 0.67 avoided per worker input token.
  - Hook: 3,090 passes (2,303 `range`, 678 `short`, 78 `missing`, 21 `repeat`, 7 `security_critical`), 404 redirects. After a redirect on the same file within 180 s: 273 native ranged reads passed, 41 redirected again, 34 `smart_read` summaries, 3 slices, 30 nothing.
- **Implications**: the key must drop the question for the canonical summary and drop the path; the hook must size ranges; the threshold should move toward the median; the stats need the two rates.

### Code facts the design relies on
- `src/services/shunt_cache.rs`: `query_fingerprint(question) -> 16-char hash`; `cache_lookup(db, path, sha, fp)` / `cache_store` on `shunt_file_cache`; `content_cache_lookup(db, sha, fp, kind)` and `content_cache_store_and_evict(db, row, max_rows)` on `shunt_content_cache` (touch `last_hit_epoch`/`hits` on hit; evict oldest by `(last_hit_epoch, created_at_epoch)` beyond the cap, returning the evicted count).
- `src/services/shunt.rs`: stage 3–4 `shunt(ctx, path, sha, line_count, question)` → fingerprint → `cache_lookup` → worker `summarize_file(payload, question)` → `cache_store`; records `IO_SHUNT_CACHE_HIT` / `IO_SHUNT_DELEGATED` audit events and `token_stats` rows of kind `shunt_cache_hit` / `shunt_delegated`.
- `src/services/shunt_content.rs`: `DEFAULT_QUESTION = "Summarize exported symbols, signatures, and control flow with exact line ranges."` (line 38); the `kind` column's v1 value is `file_read`.
- `src/storage/schema.rs`: `SCHEMA_DDL` is `CREATE … IF NOT EXISTS` only; `apply` runs the DDL and stamps `PRAGMA user_version` when the on-disk version is lower; there is deliberately no migration framework, so a key change needs an explicit, idempotent migration step guarded by `user_version`.
- `src/hook/mod.rs`: `decide` returns `Pass(Range)` as soon as `req.range.is_some()`; `ReadRequest.range` is `Option<(u64, u64)>` with harness-specific meaning: Claude Code and OpenCode `(offset or 1, limit or 0)` where 0 means "to the end"; Antigravity `(StartLine or 1, EndLine or 0)`; Codex shell reads are recognised by `hook::shell::recognise` (`sed -n a,bp`, `head -n`, …) and carry a range in the same tuple. `scan` counts lines up to the threshold only.
- Redirect texts: `adapters.rs::render_claude`, `codex.rs::render`, `opencode.rs::render`, `antigravity.rs::render`; each says "For exact text, use <tool> with offset and limit / StartLine and EndLine".
- `src/config.rs`: `DEFAULT_SHUNT_MIN_LINES = 150` (`SHUNT_MIN_LINES`), `DEFAULT_SHUNT_CONTENT_CACHE_MAX_ROWS = 5000`. Docs: `docs/configuration.md` line 56; steering `tech.md` ("~150 lines from empirical median read sizes").
- `src/cli/stats.rs` aggregates `token_stats` by day × project × harness; `src/cli/stats_table.rs` has ten numeric columns with fit priorities; activity counts (summaries, cache hits, slices, direct, redirects) come from `src/cli/stats_activity.rs`.
- `src/services/warmup.rs::run_warmup(…, question, …)` implements the `shunt_warmup` tool (explicit paths, batch cap 25, allowed-roots confinement, security bypass, honest stats); `src/cli/prune.rs` and `shunt_prune` remove rows whose file is missing or whose content changed, keyed by path.

## Architecture Pattern Evaluation

| Option | Description | Strengths | Risks / Limitations | Notes |
|--------|-------------|-----------|---------------------|-------|
| A. Unify the file read on `shunt_content_cache` (kind `file_read`), key `(content_sha256, query_fingerprint, kind)`, canonical fingerprint for the default question | One table, LRU and hits already there | No new table; eviction, hits and audit exist; the content tool and file reads share summaries of identical content | Needs a one-time copy from `shunt_file_cache`; prune needs a path to check staleness | **Selected**, with a `last_path` column added for prune and audit |
| B. Change `shunt_file_cache`'s primary key in place | Minimal conceptual change | Keeps prune as is | SQLite cannot alter a PK; needs table rebuild anyway; keeps two caches with different policies | Rejected |
| C. Keep the key, normalise questions into intents | No schema change | Cheap | Free text cannot be classified reliably; still path-keyed; warm-up still question-bound | Rejected |

## Design Decisions

### Decision: Canonical summary keyed by content only; custom answers are additional rows
- **Context**: Req 1, 2. Agents word questions freely; the owner wants the shared summary to win.
- **Alternatives Considered**: (1) always answer the custom question (today: misses), (2) ignore the question entirely (loses a real capability), (3) serve the canonical summary unless the caller explicitly asks for a question-specific answer.
- **Selected Approach**: (3). `smart_read` gains an optional boolean `answer_question` (default false). Lookup for a non-default question: cached custom answer → serve; else cached canonical → serve with the one-line note; else produce the canonical (not the custom) and serve it with the note. With `answer_question: true` the worker answers the question and the answer is cached under `(sha, fingerprint(question), file_read)`; the canonical row is untouched.
- **Rationale**: maximises re-use across agents and keeps warm-up (which uses the default question) effective for everyone; a real question still has an explicit path.
- **Trade-offs**: an agent that asked a specific question gets a structural summary first; the note tells it how to get the answer. Two worker calls happen only when the agent insists.

### Decision: Path leaves the key, stays as `last_path`
- **Context**: Req 1.3, 7. Copies and worktrees should hit; prune checks files by path.
- **Selected Approach**: add `last_path TEXT` to `shunt_content_cache` (updated on store and on hit). Prune treats a row as stale when `last_path` is missing or its content hash differs; the LRU cap handles the rest.
- **Trade-offs**: a row whose last path moved but whose content still exists elsewhere may be pruned; it is cheap to re-create and the cap would evict it anyway.

### Decision: Migration is a guarded copy, not a framework
- **Context**: `schema.rs` has no migration framework by design.
- **Selected Approach**: bump `SCHEMA_VERSION`; `apply` runs, for an on-disk version below the new one, an idempotent step: `ALTER TABLE shunt_content_cache ADD COLUMN last_path` (guarded by `PRAGMA table_info`), then `INSERT OR IGNORE` every `shunt_file_cache` row as `(sha, fingerprint, 'file_read', summary, line_count, created, created, 0, file_path)`, then leave `shunt_file_cache` in place but unused (dropped in a later release). The copy is audited once.
- **Follow-up**: a test opens a v2 store fixture and verifies the copied rows are served.

### Decision: Range size rule in the hook
- **Context**: Req 3. `range` tuples mean different things per harness.
- **Selected Approach**: `ReadRequest.range` becomes `Option<LineRange { start: u64, end: Option<u64> }>` (inclusive, 1-based; `None` = to the end), built per adapter from its own fields. `decide`: no range → as today; range with `end`: span = end − start + 1; span ≥ threshold → whole-file rules; span < threshold → `Pass(Range)`. Range without `end`: scan from `start` with the same capped counter; remaining ≥ threshold → whole-file rules, else pass. Audit `details.range` records start/end for both outcomes.
- **Trade-offs**: a read of lines 1–299 of a 2,000-line file passes; agents can page through a large file in sub-threshold chunks, which is acceptable and visible in the stats.

### Decision: Threshold 300
- **Context**: Req 4; owner's choice. Median summarised file 310 lines; 0.67 payback on first use.
- **Selected Approach**: `DEFAULT_SHUNT_MIN_LINES = 300`; docs, steering and the installer's worker guide text updated; hook tests use the constant. Cached rows for 150–299-line files stay servable.

### Decision: Two rates in `mmhmem stats`
- **Context**: Req 5.3–5.5.
- **Selected Approach**: `hit rate` = cache_hits / (cache_hits + summaries), shown as a percentage with the `(rate)` label; `follow-through` = redirects followed within 10 minutes by any `smart_read` record for the same file (summary, cache hit, slice, direct) / redirects, computed in `stats_activity.rs` from the same records as the activity counts. Totals row shows the rates recomputed from the summed counts. Fit priority 3 (dropped first when the table is too wide), like the other activity columns.

### Decision: `mmhmem warmup` CLI on top of `run_warmup`
- **Context**: Req 6. `shunt_warmup` takes explicit paths (cap 25) and is agent-facing.
- **Selected Approach**: `src/cli/warmup.rs`: resolve the root; list candidates via `git ls-files -z` when the root is a repository (respects `.gitignore`), else a bounded walk; keep text files at or above the threshold, drop security-critical paths and paths outside the allowed roots; batch them through `run_warmup` with the default question; `--dry-run`, `--max-files <n>` (default 200), `--root <dir>`. One report in the `render_report` style.
- **Trade-offs**: counting lines before summarising reads every candidate once; acceptable for a one-off command.

## Risks & Mitigations
- Migration on a large store takes time at first open — the copy is one `INSERT … SELECT`; bounded by the old table's size (144 rows on the owner's machine).
- Agents keep passing questions out of habit — the note in the response and the playbook line steer them; `answer_question` makes intent explicit.
- Raising the threshold hides savings on 150–299-line files — visible in the new rates; the environment variable restores 150 per machine if wanted.
- The follow-through window (10 minutes) is a heuristic — labelled as such in the column header and documented.

## References
- `.kiro/specs/read-routing-hooks/design.md` — hook protocol and eligibility rules
- `.kiro/specs/corpus-warmup-prune/requirements.md` — `shunt_warmup` and prune contracts
- `.kiro/specs/shunt-content-tool/design.md` — the content cache, its LRU cap and audit
- Audit analysis scripts (session scratchpad, 2026-11-10): `stats/hook_seq.py`, `stats/cache_analysis.py`
