# Validation Log — shunt-cache-efficiency

## 2026-11-10 — Feature validation (tasks 1–8) and live run on the owner's machine

### Verdict: GO

- All tasks `[x]`. Three implementation batches, each by one Opus
  implementer with controller inline review (owner's decision to save
  time); one remediation in batch 2 (below-threshold files must stay direct
  reads through `smart_read`).
- CI (`main`): 9406c7b batch 1 green after a Windows runner timing flake
  rerun (CI 42857053522, Installer e2e 42857053280); f6bae02 batch 2 failed
  the Linux Dagger gate on a pre-existing script hazard (see "CI incident")
  fixed in 3ce13eb; 7f26da5 batch 3 green (CI 42871457753, Installer e2e
  42871457823); 26635b0 release commit green (CI 42873063732, Installer e2e
  42873063713).
- Release: `v0.5.4` published 2026-11-10T18:35:29Z (run 42876249770,
  environment `release` approved), all thirteen assets present. The owner
  updated with `mmhmem update`; `mmhmem --version` reports 0.5.4.
- The `E2E_LLM=1` measurement suite was not run: owner decision (it is gated
  in CI by a repository variable and costs model tokens). The live run below
  stands in for it.

### Requirements coverage

| Req | Evidence |
|---|---|
| 1.1–1.4 | `tests/summary_cache.rs`: canonical row keyed by content; two paths one row and a hit; custom question separate row; no age expiry. Live: three `smart_read` calls from this Claude Code session on files warmed by the CLI (harness `cli`) were cache hits with no worker call. |
| 1.5 | `tests/shunt_cache_migration.rs` on the committed v2 fixture: opens at v3, rows served, `CACHE_MIGRATED` audited once. Owner's store migrated on first use after the update (warm-up dry run reported "1 already cached" from the old table's copy). |
| 2.1–2.5 | `tests/summary_cache.rs` policy tests; `tests/tool_routing.rs` and `tests/mcp_tools.rs` for the tool parameter and description; playbook line in `deploy/skills/unified-memory/SKILL.md` under the token cap (marketplace check PASS). |
| 3.1–3.6 | `src/hook/range_tests.rs`, adapter tests and `tests/hook_cli.rs`: Claude `limit: 2000` on a 400-line file denied, `offset 10 limit 100` passes, Antigravity `StartLine 1 EndLine 350` denied, Codex `sed -n 1,400p` denied, open-ended from line 2 denied, `tail -n 50` passes, unparsable range keeps whole-read behaviour; redirect texts name the threshold and the slice. |
| 4.1–4.3 | `DEFAULT_SHUNT_MIN_LINES = 300`; `tests/install_docs.rs` checks docs and `.env.example`; `tests/shunt.rs::summaries_cached_below_the_raised_threshold_are_kept_and_servable`. Live: the warm-up skipped 606 files as below threshold and summarised 138. |
| 5.1, 5.2 | `shunt_content_cache` hits/last-hit/`last_path`; cap `SHUNT_CONTENT_CACHE_MAX_ROWS` with `CONTENT_CACHE_EVICT` audit (`eviction_beyond_the_cap_is_audited_with_the_count`). |
| 5.3–5.6 | `mmhmem stats` live rows (below); `tests/stats_cli.rs` (75% / 50%, JSON carries `followed_redirects`, no rate fields); rates derived at render time only. |
| 6.1–6.7 | `tests/warmup_cli.rs` and the live runs below; playbook mentions `mmhmem warmup <repo>`; positional root supported. |
| 7.1–7.3 | `src/services/prune.rs` tests on the shared table (missing, changed, fresh, moved file kept, content-tool rows untouched). |

### Live run (owner's Windows 11 machine, mmhmem 0.5.4, Mistral worker)

- `mmhmem warmup --dry-run D:\Source\mmhmem`: "would summarise 138 files (worker calls); 1 already cached".
- `mmhmem warmup D:\Source\mmhmem` (first run, 470 s): "760 files examined, 138 summaries produced, 1 hits, skipped 606 below threshold, 5 not text, 10 security-critical, 0 outside allowed roots, 0 missing or unreadable; 0 failed. Worker tokens: 1018648 prompt + 79798 completion."
- Second run (1 s): "0 summaries produced, 139 hits … Worker tokens: 0 prompt + 0 completion. second run: hits only".
- Session reads: `smart_read` from Claude Code on `src/services/shunt.rs`, `src/setup/agents/antigravity.rs`, `src/hook/mod.rs` → "Cached Shunt Summary" each, estimated avoided 3,150 / 4,025 / 4,856 frontier input tokens, no worker call.
- `mmhmem stats` rows for 2026-11-10 (new columns `hit rate (rate)` and `follow-through (rate, 10 min)`):

| project | harness | summaries | cache hits | hit rate | redirects | follow-through |
|---|---|---|---|---|---|---|
| mmhmem | cli | 138 | 140 | 50% | 0 | - |
| mmhmem | claude-code | 0 | 3 | 100% | 16 | 0% |
| QOpenBB | codex | 6 | 3 | 33% | 6 | 83% |
| memguard_alpha | claude-code | 6 | 1 | 14% | 77 | 9% |

The `claude-code` follow-through of 0% for `mmhmem` reflects today's hook
checks (direct hook invocations and an `agy -p` run that answered with the
redirect and then tried a shell fallback); the 9% and 20% rows of other
projects are the pre-existing sidestep pattern that the range rule now
closes for new reads. Historic rows show the problem the spec addressed
(0% hit rates nearly everywhere).

### CI incident (batch 2)

`scripts/test-install-stub.sh` checked the README one-liner with
`tr -d '\015' < README.md | grep -qxF …` under `set -o pipefail`; `grep -q`
exits at the match and `tr` receives SIGPIPE on its second 4 KB stdio write,
so the pipeline returns 141. Reproduced in `debian:bookworm-slim` via Podman
on the batch-1 and batch-2 README alike, so the trigger was the base image's
buffering, not the README edit. Fixed in 3ce13eb by letting `grep` consume
the pipe. Reproduction recipe: `git archive HEAD` into a scratch folder,
mount it into the image, run the stage's three scripts.

### Open items

- The follow-through rate has no live sample above 0% for the hook's new
  range rule yet; the first agent sessions after the update will show it.
- `deploy/examples/mmhmem.env` is gitignored on the owner's machine and was
  edited locally in batch 2; it is not tracked and not part of the release.
