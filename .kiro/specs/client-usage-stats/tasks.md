# Implementation Plan

All tasks are required. Each executable task is sized for approximately 1–3 hours.
File ownership follows the approved design's File Structure Plan. No task changes
how client usage reports are written or stored. Every commit must pass the
project pre-commit gate (fmt, size caps, clippy on default and `otel` features).

- [x] 1. Reduce client-reported rows to per-day totals
- [x] 1.1 Build the client-row reduction module with its unit tests
  - Read every client-reported row with a read-only query and parse its stored details into prompt tokens, completion tokens and source, never failing on missing, non-numeric, negative or malformed values.
  - Keep only the latest report per project, harness, workspace task and source, by newest timestamp and then by later row.
  - Sum the kept reports per UTC day, project and harness, with the sorted distinct sources behind each total.
  - Declare the new module next to the existing stats module without changing the command's behaviour yet.
  - Done when unit tests show complete, partial, malformed and empty details parsed as specified, repeated reports for one key counted once with ties broken by row order, different tasks and sources each counted, and each kept report placed on its own UTC day.
  - _Requirements: 1.1, 1.2, 1.3, 1.5, 3.2, 3.3, 3.4_

- [x] 2. Report client usage and state the contract
- [x] 2.1 Rework the stats aggregation, table and JSON output around the new client totals
  - Exclude client-reported rows from the estimate aggregation, merge estimates with the client totals by day, project and harness, and fill zeros on the side a key lacks.
  - Replace the former client-reported avoided value with frontier prompt and frontier completion values plus the source list, keeping every other JSON field and the row ordering unchanged.
  - Widen the table to the two client-reported frontier columns with spent-token labels, and include both in the TOTAL row.
  - Update the module's unit tests to the new contract with client rows in the shape the server writes.
  - Done when unit tests confirm the merge, the separation of estimates from client values, the exact table headers and the exact JSON field names in day rows and totals.
  - _Depends: 1.1_
  - _Requirements: 1.1, 1.4, 2.1, 2.2, 2.3, 4.1, 4.2, 4.3_

- [x] 2.2 (P) State the cumulative report contract wherever harnesses read it
  - Describe a client usage report as the harness's cumulative frontier usage for the identified workspace task in the session tool description, its parameter documentation and the stats service documentation.
  - Add the cumulative-task-total wording to the unified-memory skill within its token budget, and regenerate the packaged plugin copy with the marketplace build script.
  - Add dated amendment notes pointing to this spec under cross-agent-memory-server Requirement 10.4 and local-mode Requirements 8.2 and 8.4.
  - Done when the routing, marketplace and checkpoint guidance checks pass with the packaged skill byte-identical to the canonical one, and existing tool-description tests still pass.
  - _Boundary: contract wording_
  - _Requirements: 3.1, 3.5_

- [x] 3. Verify against the production write path
- [x] 3.1 Rewrite the stats command integration tests on rows written by the server code
  - Write client-reported rows through the same collector the server uses, adjusting a row's timestamp afterwards only where a day boundary is needed.
  - Cover three cumulative reports for one task and source, two tasks and two sources, a report on an earlier day, a hand-written row with a missing value, rows already present before the change, and the source list in the JSON output.
  - Confirm the command leaves the store file unchanged and that every report is still stored and audited.
  - Run the full deterministic suite and strict clippy on default and `otel` features.
  - Done when the integration tests pass with only the latest report per key counted, the full suite and both clippy passes are green, and no test inserts client values into numeric columns.
  - _Depends: 2.1, 2.2_
  - _Requirements: 1.2, 1.3, 1.5, 2.3, 3.2, 3.3, 3.4, 3.5, 5.1, 5.2_

## Implementation Notes

- Task 1.1: `ClientReport` also carries `day`, computed by SQLite's `date(ts_epoch_ms / 1000, 'unixepoch')` so client days match the estimate query exactly (the crate has no date library); per-day summing is the public `client_days` function. Reviewed inline by the parent.
- Task 2.1: the stats unit tests moved to `src/cli/stats_tests.rs` (`#[path]` module, as `session_bus_tests.rs`) to keep `stats.rs` under the 500-line cap. The two `tests/stats_cli.rs` assertions on the old JSON field and table label were updated in the same commit so the suite stays green; task 3.1 rewrites that file on production-written rows. Reviewed inline by the parent.
- Task 3.1: full `cargo test` and strict clippy (default and `otel`) green on 2026-11-03; the hand-written row for the missing-value case carries values only in `details`. Reviewed inline by the parent.
