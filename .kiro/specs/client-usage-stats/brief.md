# Brief: client-usage-stats

## Problem

`mmhmem stats` never shows the frontier token counts that harnesses report.
The column meant for them always prints 0 on real data, and the numbers it
would show are labelled as "avoided" tokens although they are tokens spent.
The tool contract is also ambiguous about whether a report is a running
total or a delta, so a harness that reports at every checkpoint could be
counted several times.

## Evidence (see research.md)

- `StatsCollector::record_client_usage` stores `prompt_tokens`,
  `completion_tokens` and `source` only in the row's `details` JSON and
  writes 0 to `avoided_input_tokens_est`.
- `cli/stats.rs` sums `avoided_input_tokens_est` for `kind = client_reported`,
  so the client column is always 0 for rows written by the server.
- Both stats CLI tests insert 700 directly into `avoided_input_tokens_est`
  for a client-reported row, which hides the mismatch.
- The tool description and `params.rs` call the payload frontier "totals";
  `stats.rs` calls it "deltas". cross-agent-memory-server Req 10.4 says
  "harness-reported frontier prompt/completion tokens"; local-mode Req 8.2
  sums client-reported values as "frontier input tokens avoided".

## Approach direction (for design, not binding)

1. Read client-reported prompt and completion tokens from the stored
   details, so existing rows become visible without a migration.
2. Report them as frontier tokens spent, in their own labelled columns,
   never as tokens avoided.
3. Define a report as the harness's cumulative usage for one workspace task
   and source, and count only the latest report per task and source.
4. State that contract in the tool description and the skill guidance.

## Boundary Candidates

- Owned: the stats aggregation and rendering in `src/cli/stats.rs`, the
  `client_usage` wording in the tool description, `params.rs` and the
  unified-memory skill, and the stats tests.
- Adjacent: cross-agent-memory-server Req 10.4 and local-mode Req 8.2, 8.4,
  whose wording this spec amends; the E2E measurement suite, which writes one
  report per task.
- Not owned: how harnesses obtain their usage numbers, the server-side
  estimate rows, worker token accounting, OTel counters, the audit format.
