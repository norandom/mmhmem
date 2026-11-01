---
name: lesscode-discovery
description: Find and test behavior-preserving code compression by identifying complete responsibilities, comparing existing capabilities and shared implementations, and measuring whole replacements. Use for grown-code simplification and empirical less-code trials.
---

# Less code, same behavior

Drive the experiment from a plausible code reduction, then choose the tool. This is an experimental agent procedure, not an automatic analyzer or a claim of proven compression effectiveness.

Respect the user's editing scope. A specimen may be read-only. Discovery does not authorize editing it. When simplification is authorized, carry the selected trial through validation without inventing another approval step. Preserve existing user work.

## Find the unit worth replacing

Establish one repository-wide installed-capability inventory per source snapshot before local trials: runtime/framework/library APIs and constraints, separately from build, test, analyzer and research dependencies. Reuse that inventory across responsibilities; do not rediscover dependencies per function or introduce competing frameworks locally. Bind supplied inventory to the source snapshot and manifest/lock references. Refresh affected entries when those inputs or cited contracts change. Proposed dependency additions/upgrades need a repository-level decision within the user's authorized scope and explicit integration-cost accounting.

Start at a complete implementation responsibility: parsing a record, assembling a request, reconciling two inventories, applying a transition, formatting a result. Trace its helpers, callers and tests. A file, module, high-complexity function or solver-compatible signature is only a locator.

Look for either:
- A family of implementations repeating the same operation with explainable variations.
- A substantial single custom operation already supplied by the language, an installed dependency or framework, or a simpler known representation.

For each plausible candidate, state **which maintained code could disappear and what would take over its behavior**. If that cannot be stated concretely, retain it as an observation rather than starting a rewrite. Do not force a small, already-direct function into a shared kernel merely because a tool accepts it.

## Separate shared behavior from differences

Make a small member matrix with source locations. For each member, record:
- The common operation and relevant input/output contract.
- Differences that are data: names, constants, field lists, mappings, thresholds.
- Differences that are behavior: validation, ordering, errors, side effects, lifetime, numeric semantics, extension contracts.
- Apparent differences whose significance is not yet established.

AST resemblance and matching signatures are leads, not equivalence. Do not turn unknown behavior into configuration or hide executable strategies in a supposedly static data table. Keep useful field names and types.

## Admit a compression hypothesis before selecting tools

Write: “Replace [specific owned implementation] with [specific existing capability or simpler representation], retaining [observable differences], so [specific code and support] becomes unnecessary.”

Rank candidates by evidenced removal opportunity, clarity of the boundary, and validation feasibility. Prefer a substantial repeated burden with an existing replacement over a convenient toy example. Record uncertainty instead of assigning an invented quality score. A control chosen to test rejection must be labeled a control, not presented as an efficacy trial.

Consider only plausible alternatives, always retaining the original as a valid choice:

| Mechanism | Evidence needed before a trial |
| --- | --- |
| Delete or inline | No required external contract, lifecycle, policy or effect is lost; sharing is not duplicated accidentally. |
| Reuse existing implementation | Callers require the same operation; different error, identity or ordering rules are accounted for. |
| Native/framework/library replacement | Concrete API and semantic compatibility, including adapters and dependency cost. |
| Direct idiomatic expression or representation | The complete operation becomes clearer and shorter, not merely denser. |
| Shared kernel plus data | Multiple members share behavior; variations are representable as data; table, wrappers and dispatch can plausibly cost less than the removed family. |

Do not require a pattern, class, table, branch-depth ceiling or solver. Multiple implementations are necessary to justify factoring a family, but not to justify replacing one large custom operation with an existing capability.

## Freeze the trial, then verify at the right level

Use [the experiment record](references/experiment-record.md) before editing the candidate. Record the source identity, members, selection rationale, observable boundary, proposed alternatives, checks, cost scope and bounded investigation budget. Label retrospective records honestly; do not claim preregistration after seeing results.

Choose verification from the behavior:
- Use unchanged existing tests plus targeted differential/property checks for ordinary code and library substitutions.
- Add missing boundary tests against the original before refactoring. Preserve invalid inputs, exception details where contractual, ordering, side effects, serialization and lifecycle behavior. Demonstrate a meaningful test can catch a relevant defect when practical.
- Use symbolic verification/CEGIS only for a faithful supported semantic model and a justified sketch. Record domain, extraction limits, solver status and emission evidence separately. Unknown, timeout or incomplete extraction is not equivalence.
- Require appropriate integration checks for framework, state or I/O changes. If the boundary cannot be validated, report that limit instead of removing it from the denominator.

Do not weaken tests to accommodate a candidate. If a test asserts implementation detail rather than required behavior, identify that distinction and preserve an equivalent behavior check before changing it. Tests have their own footprint; they are not the default reduction target.

## Measure the whole replacement and decide

For native Python/TypeScript comparisons, read [the native command recipe](references/native-workflow.md). It covers scope/inventory review, frozen paired checks, explicit artifact roots and the verified ATS replay. The CLI consumes supplied evidence; it does not run tests or discover replacement candidates. Use the existing specialist process for any other verification; this recipe adds no solver or MCP integration.

Count unique maintained source in the affected responsibility before and after, including residual old code, new helpers, tables, wrappers, adapters and required configuration. Show production, tests and research apparatus separately. Disclose new dependency/integration burden; do not pretend third-party implementation has zero cost or count all its source as owned code. Report physical lines, logical statements and aggregate decisions with their definitions. Bytes and compressed size are not compression-benefit criteria. Retain whole-project totals when claiming project-level reduction.

For factoring, assess the concrete break-even condition:

`removed family code > kernel + data + wrappers + adapters + residual implementation`

This is an accounting check, not a readability score or a formal lower bound. Count overlapping families only once in aggregate savings. Estimates stay labeled until replaced by actual candidate measurements.

Adopt only when behavior evidence passes, the complete replacement provides useful reduction, and review finds clear names, understandable control flow and no unjustified new indirection. Passing tests is not enough; fewer lines is not enough. Retain the original when evidence or usefulness is insufficient. State whether a result was modeled, tested, reviewed or unverified; do not collapse these into “proved.”

## Produce credible evidence

Keep every examined candidate's disposition: not a compression hypothesis, deferred/unsupported, behavior failure, validation incomplete, equivalent but not useful, or adopted. Report denominators separately: source inventoried, candidates examined, families trialed, alternatives validated and replacements adopted. Tool-interface coverage is not compression potential.

Keep manual samples and synthetic controls separate from real-repository results. Repeated cases and repeated test runs are not independent experiments. Report accepted net reduction, exclusions, test changes and remaining uncertainties; do not manufacture a general success rate from heterogeneous trials. Use maintenance/change exercises when claiming reduced maintenance effort.

If a trial fails, retain the reason and reconsider the hypothesis. Extend a tool only when a worthwhile candidate demonstrates a specific missing capability. Do not build an analysis framework to avoid selecting and comparing real code.

## Local project entry point

Read references/local-measurement.md before measuring this Rust repository.
