# Implementation Plan

All tasks are required. Each executable task is sized for approximately 1–3
hours. Order: the cache key and migration first (everything else measures
against it), then the lookup policy, the hook range rule, the threshold,
the stats, the warm-up command, and the validation. Every commit must pass
the pre-commit gate (fmt, size caps, clippy default and `otel`); commit
from native PowerShell on the Windows host.

- [ ] 1. Content-keyed summary cache
- [x] 1.1 Canonical fingerprint and the shared table
  - Add `canonical_fingerprint()` (the default question's fingerprint) and treat an absent, empty or default `question` as canonical.
  - Add `last_path` to `shunt_content_cache`; implement `summary_lookup` (touching hits, last hit and last path) and `summary_store_and_evict` for kind `file_read`, reusing the content cache's cap and audited eviction.
  - Done when unit tests show: the same content under two paths yields one row and a hit on the second read with `last_path` updated; a custom question yields a separate row; eviction beyond the cap is audited with the count.
  - _Boundary: Summary cache_
  - _Requirements: 1.1, 1.2, 1.3, 1.4, 5.1, 5.2_

- [x] 1.2 Migration from the old file cache
  - Bump the schema version; on open of an older store, add the column if missing and copy every `shunt_file_cache` row into the shared table in one transaction; audit one `CACHE_MIGRATED` record with the row count; stop reading and writing the old table.
  - Done when a committed v2 store fixture opens at the new version, its rows are served without a worker call, the migration is audited exactly once, and reopening does not copy again.
  - _Boundary: Summary cache_
  - _Requirements: 1.5, 4.3_

- [ ] 2. Lookup policy and the question parameter
- [x] 2.1 Serve canonical first, custom answers on request
  - Implement the lookup order of the design flow: canonical for the default question; for a custom question serve a cached custom answer, else the canonical with the one-line note, producing the canonical when nothing is cached; produce and cache a custom answer only with `answer_question: true`.
  - Add `answer_question` to the `smart_read` tool and `ShuntRequest`; add `note` to the response; audit `question` and `served`; a served custom row counts as a cache hit.
  - Done when tests show: custom question without the flag → canonical plus note and no worker call; with the flag → one worker call and a custom row that is served next time; the canonical row survives both; audit fields present.
  - _Boundary: Lookup policy_
  - _Requirements: 2.1, 2.2, 2.3, 2.5_

- [x] 2.2 (P) Tool description and playbook wording
  - Rewrite the `smart_read` description and the `unified-memory` playbook line: omit the question for the shared summary; ask with `answer_question: true` only when the summary is not enough; mention `mmhmem warmup`.
  - Done when the playbook stays under its token cap test and the marketplace check passes, and the tool description test (if any) matches.
  - _Boundary: Lookup policy_
  - _Requirements: 2.4, 6.7_

- [ ] 3. Hook range rule
- [x] 3.1 Typed line ranges from every adapter
  - Introduce the line range type (start, optional end) and build it in the Claude Code, OpenCode, Antigravity and Codex/shell adapters from their own fields; forms that cannot be sized stay whole reads; unparsable values pass.
  - Done when adapter tests show each harness's ranged payload becomes the expected start/end, open-ended reads have no end, and the hook CLI tests still pass unchanged for whole reads.
  - _Boundary: LineRange + span rule_
  - _Requirements: 3.5_

- [x] 3.2 Span rule in the decision
  - Apply the span rule: a range at or above the threshold (or open-ended with enough lines remaining) follows the whole-file rules; a smaller span passes; an I/O error passes; audit the range on both outcomes.
  - Done when hook tests show, per harness: span of exactly the threshold → redirect; threshold minus one → pass; open-ended from line 2 of a long file → redirect; `tail -n 50` → pass; and the hook CLI tests cover a Claude `Read` with `limit: 2000` on a 400-line file (deny) and `offset: 10, limit: 100` (pass).
  - _Boundary: LineRange + span rule_
  - _Depends: 3.1_
  - _Requirements: 3.1, 3.2, 3.3_

- [x] 3.3 (P) Redirect texts
  - Rewrite the four harnesses' redirect texts: `smart_read` for the summary first; the exact-text fallback is a range under the threshold around the lines needed, naming the threshold; mention the `smart_read` slice where the harness has exact ranges; keep the 2-minute repeat hint last.
  - Done when render tests for all four harnesses assert the order and the threshold number, and the Antigravity reason stays ASCII.
  - _Boundary: Redirect texts_
  - _Requirements: 3.4, 3.6_

- [x] 4. Threshold 300
  - Set the default to 300; replace literals with the constant in hook and shunt tests and the check's hook sample; update `docs/configuration.md`, the README and installation guide sentences that name the threshold, the installer's worker-guide text if it does, and the steering note in `tech.md`.
  - Done when a grep for the old literal in `src/`, `docs/`, `README.md` and steering finds only historical spec text, the install-docs test passes, and a cached 200-line summary is still served when requested.
  - _Boundary: Threshold_
  - _Requirements: 4.1, 4.2, 4.3_

- [ ] 5. Stats: hit rate and follow-through
  - Count followed redirects (a `smart_read` record for the same file within 10 minutes of a redirect) alongside the existing activity counts; render `hit rate` and `follow-through` as percentages with rate labels, priority 3, `-` on a zero denominator, totals recomputed from the sums; `--json` exposes the counts.
  - Done when a seeded stats/audit test shows both columns with the expected percentages, the hidden-column hint still works when the table is too wide, and the existing snapshot tests are updated only where the new columns appear.
  - _Boundary: Stats rates_
  - _Requirements: 5.3, 5.4, 5.5, 5.6_

- [ ] 6. `mmhmem warmup`
- [ ] 6.1 Repository candidate selection
  - Resolve the root; list files with `git ls-files` when inside a repository, else a bounded walk; keep text files at or above the threshold; drop security-critical paths, paths outside the allowed roots and ignored files; stop at `--max-files` with the message.
  - Done when a unit test over a temp repository with large, small, ignored and secret files selects exactly the large non-ignored non-secret ones, and the cap message appears when exceeded.
  - _Boundary: Warmup CLI_
  - _Requirements: 6.1, 6.2, 6.4_

- [ ] 6.2 Batches, dry run and report
  - Push candidates through the existing warm-up service in batches of 25 with the default question; `--dry-run` lists candidates and the number of worker calls; one report with examined, hits, summaries, skipped by reason, failures and worker tokens; no worker configured → the existing message, exit 1, no change; wire the subcommand and the help text.
  - Done when the CLI test shows: first run summarises three files with a stub worker, the second run reports three hits and no worker call, the dry run writes nothing, and the no-worker case exits 1 with no rows.
  - _Boundary: Warmup CLI_
  - _Depends: 1.1, 6.1_
  - _Requirements: 6.1, 6.3, 6.4, 6.5, 6.6_

- [x] 7. Prune on the shared table
  - Point `mmhmem prune` and `shunt_prune` at rows of kind `file_read` in the shared table; stale when the last path is unknown or missing or its content hash differs; keep the dry run, grouped report and audit records as they are.
  - Done when prune tests pass on the new table with the three staleness cases and the report text is unchanged.
  - _Boundary: Summary cache_
  - _Depends: 1.2_
  - _Requirements: 7.1, 7.2, 7.3_

- [ ] 8. Validation
  - Run the full suite and the E2E_LLM measurement once over a repository in two sessions; record the hit rate and follow-through from `mmhmem stats`; on the owner's machine run `mmhmem warmup` on one repository, then one agent session, and record the stats row.
  - Done when the second session's hit rate is above 50 percent in the measurement, the live stats row shows a non-zero follow-through, and the evidence is in `validation.md`.
  - _Depends: 1.2, 2.1, 3.2, 4, 5, 6.2, 7_
  - _Requirements: 1.1, 3.1, 5.3, 5.4, 6.3_

## Implementation Notes

- 2026-11-10 (batch 1: tasks 1.1, 1.2, 2.1, 2.2, 7; one Opus implementer, controller inline review): file-read summaries live in `shunt_content_cache` (kind `file_read`) with a canonical fingerprint and `last_path`; schema v3 copies `shunt_file_cache` once (fixture `tests/fixtures/store/v2-file-cache.db`); stages 3–4 moved to `src/services/shunt_lookup.rs` (shunt.rs was at the 500-line cap). Accepted deviations: prune skips rows with a NULL `last_path` instead of pruning them, because `shunt_content` rows share the `file_read` kind and have no path; content summarised earlier only under custom questions gets a canonical summary on its first default read (the custom rows are kept and served). `CACHE_MIGRATED` is audited after the schema transaction commits. For batch 3: the playbook says `mmhmem warmup <repo>`, so the CLI must accept the root as a positional argument as well as `--root`; `deploy/instructions/mmhmem-routing.md` line 15 still says cache identity includes the question and must be reworded.
- 2026-11-10 (CI for batch 1, main 9406c7b): CI run 42857053522 failed once in the Windows job on `tests/routing_adoption.rs::bounded_child_drains_both_streams` (a 5-second timeout while a PowerShell child writes 200 KB to both streams; first failure of that job in two weeks of green runs, no process code in the batch) and passed on rerun; Installer e2e run 42857053280 passed. Treat that test as timing-sensitive on the Windows runner.
- 2026-11-10 (batch 2: tasks 3.1, 3.2, 3.3, 4; one Opus implementer, controller inline review, one remediation): `hook::LineRange` (`from_offset_limit`, `from_tail` encoded as start 0 + end n, `span_lines`), `range::span_at_or_above` and `scan_from` in `src/hook/range.rs`, shell ranges in `src/hook/shell_range.rs` (sed -n A,Bp / Ap, head -n N / -nN / -N, tail -n N / -n +N; `Get-Content` ranged forms stay unrecognised); audit `details.range` = {start,end} or {last}; redirect texts name the threshold and the smart_read slice; `DEFAULT_SHUNT_MIN_LINES = 300` with docs, README, installation guide, server docs, .env.example and steering updated; 14 tests resized from the constant, `shunt_content_fixture` pins `SHUNT_MIN_LINES=160` for its byte-compared corpus; `docs/concept.md` keeps its historical 150s. Rejected and reverted: serving a cached summary for a below-threshold file through smart_read (small files must return exact content); Req 4.3 is proven by a test that reads a 200-line file directly at 300 and from the cache at `SHUNT_MIN_LINES=150`.
