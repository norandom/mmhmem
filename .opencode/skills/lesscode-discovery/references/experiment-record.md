# Compression experiment record

Use one record per responsibility/family. Attach alternative results to the same record rather than counting them as independent candidate families. Use “not measured” for unknown quantities.

## Before the trial

- ID, date, repository revision, dirty-state/source hashes:
- User-authorized scope; read-only specimens:
- Selection: manual / systematic / synthetic; efficacy candidate / rejection control:
- Responsibility and complete owned source boundary:
- Family members and call sites, or single-operation replacement opportunity:
- Shared operation:
- Data variations:
- Required behavioral differences and unresolved differences:
- Hypothesis: code removed → replacement capability → behavior retained:
- Plausible alternatives, including keep original:
- Why worthwhile: expected deletions and all expected added support, with evidence:
- Functional boundary and unchanged baseline checks:
- Missing checks to add before rewriting; method/domain limits:
- Source/test/config/dependency accounting scope; overlap with other trials:
- Investigation budget and stopping condition:

## Observed results

| Alternative | Behavior evidence and limits | Production before/after | Added support/dependencies | Test impact | Readability/change impact | Disposition |
| --- | --- | --- | --- | --- | --- | --- |
| Original | | | | | | Baseline |

- Exact commands, exit codes, raw results, source hashes and complete diff:
- Solver model status, if used; emitted-code evidence separately:
- Baseline tests passed/failed/skipped; candidate tests on the same boundary:
- Unsupported or unknown behavior retained in the record:
- Full replacement counts; local versus project-wide delta:
- Adopted net reduction (zero when retained as an experiment):
- What this supports, what would disprove it, and what cannot generalize:

Do not count candidate-only savings, use test deletions to improve production metrics, or hide failures by removing an attempt from the record.
