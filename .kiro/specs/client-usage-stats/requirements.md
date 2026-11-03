# Requirements Document

## Project Description (Input)

`mmhmem stats` never shows the frontier token counts that harnesses report
through `client_usage`. The server stores those counts only in each row's
details, while the stats command sums a numeric column that is always 0 for
those rows. The column is also labelled as tokens avoided, although the
numbers are tokens the frontier model spent. Finally, the tool contract does
not say whether a report is a running total or a delta, so repeated reports
could be counted several times. Fix the reporting so client-reported usage
appears, correctly labelled and counted once, without a data migration.
Findings are in `research.md`. The user requested this work through the Kiro
specification workflow, with its requirements, design and tasks approval
gates.

## Introduction

This specification governs how `mmhmem stats` reads and presents
client-reported frontier usage, and how the `client_usage` payload is
described to harnesses. Server-side estimates, worker token accounting and
the way client reports are stored stay as they are.

## Boundary Context

- **In scope:** aggregation and presentation of client-reported rows in
  `mmhmem stats` (table and `--json`), the counting rule for repeated
  reports, the wording of the `client_usage` contract in the
  `session_resume_or_handoff` tool description, its parameter documentation
  and the unified-memory skill guidance, and the stats tests.
- **Out of scope:** how harnesses obtain their usage numbers, the stored row
  format and the `token_stats` schema, server-side estimate rows, worker
  token columns, OTel counters, the audit event format, and serve-mode HTTP
  behaviour.
- **Adjacent expectations:** this spec clarifies cross-agent-memory-server
  Requirement 10.4 (a report is cumulative per workspace task and source) and
  amends local-mode Requirements 8.2 and 8.4 (client-reported values are
  frontier tokens spent, not avoided, and the JSON fields change). The E2E
  measurement suite (cross-agent-memory-server Requirement 12.2) sends one
  report per task and source and keeps working unchanged.
- **Data:** client-reported rows already in a store must be reported
  correctly by the fixed command, without rewriting them.

## Requirements

### Requirement 1: Client-reported usage is reported

**Objective:** As a developer checking token savings, I want `mmhmem stats`
to show the frontier usage my harness reported, so that I can compare real
frontier spend with the server's savings estimates.

#### Acceptance Criteria

1. When `mmhmem stats` aggregates the store, the mmhmem stats command shall report, per day, project and harness, the client-reported frontier prompt tokens and frontier completion tokens carried by the counted client usage reports.
2. When the store contains client-reported rows written before this change, the mmhmem stats command shall report their prompt and completion tokens without modifying or migrating those rows.
3. If a client-reported row lacks a numeric prompt or completion value, the mmhmem stats command shall count the missing value as 0 and still report the other value.
4. The mmhmem stats command shall keep client-reported values out of the estimated tokens-avoided totals, and keep estimated values out of the client-reported totals.
5. The mmhmem stats command shall continue to open the store read-only.

### Requirement 2: Correct labelling

**Objective:** As a developer reading the stats, I want every number labelled
for what it is, so that measured spend is never confused with estimated
savings.

#### Acceptance Criteria

1. The mmhmem stats command shall label client-reported values as frontier tokens spent as reported by the harness, and shall never label them as tokens avoided.
2. The mmhmem stats command shall keep labelling server-computed values as estimates.
3. When `--json` is requested, the mmhmem stats command shall list the distinct report sources that contributed to each aggregate row's client-reported values.

### Requirement 3: Each report is counted once

**Objective:** As a developer whose harness reports usage at every
checkpoint, I want repeated reports counted once, so that the totals match
what my harness actually spent.

#### Acceptance Criteria

1. The mmhmem server shall describe a client usage report, in the `session_resume_or_handoff` tool description, its parameter documentation and the unified-memory skill guidance, as the harness's cumulative frontier usage for the identified workspace task at the time of the call.
2. When several client usage reports exist for the same project, harness, workspace task and source, the mmhmem stats command shall count only the most recent report.
3. When client usage reports differ in workspace task or source, the mmhmem stats command shall count each of them.
4. The mmhmem stats command shall attribute a counted report to the UTC day of that report.
5. The mmhmem server shall keep storing and auditing every client usage report unchanged.

### Requirement 4: Output contract

**Objective:** As someone scripting against `mmhmem stats --json`, I want a
stable, explicit contract for the new values, so that my tooling reads the
right numbers.

#### Acceptance Criteria

1. When `--json` is requested, the mmhmem stats command shall replace the `avoided_input_tokens_client_reported` field with `frontier_prompt_tokens_client_reported` and `frontier_completion_tokens_client_reported`, in every day row and in the totals.
2. The mmhmem stats command shall keep every other existing JSON field name and meaning unchanged.
3. The mmhmem stats command shall show the two client-reported values as two table columns in place of the former client-reported avoided column, and shall include them in the TOTAL row.

### Requirement 5: Verification against the real write path

**Objective:** As a maintainer, I want the stats tests to use rows shaped as
the server writes them, so that this class of mismatch cannot hide again.

#### Acceptance Criteria

1. The stats test suite shall create client-reported rows through the same server code path that records them in production, rather than by inserting values into numeric columns.
2. The stats test suite shall cover repeated reports for one task and source, reports for different tasks and sources, a report with a missing value, and rows written in the pre-change shape.
