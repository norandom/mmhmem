# Implementation Plan — shunt-content-tool

All tasks are required unless marked `- [ ]*`. Sub-tasks are sized for 1–3 hours. File ownership follows the design's File Structure Plan; source files stay under the 500-line cap. Task-graph sanity review: `inline` (authoring session), verdict PASS. The consumer spec (`../mvibe/.kiro/specs/proxy-econ-client`) depends on the result contract fixed in task 1.1; do not change field names after it lands without a revalidation note.

## 1. Foundation: contract, storage and configuration

- [x] 1.1 Content types and shared source seam
  - Define the content request, kind, outcome and source types; thread the source through the engine's delegate, cached, audit, stat and breadcrumb helpers without changing file behaviour
  - Observable completion: the existing shunt, warmup and prune suites pass unchanged, and audit details for file reads now carry an explicit source label
  - _Requirements: 6.2_

- [x] 1.2 (P) Dedicated content cache table and helpers
  - Additive table and index per the design; lookup that touches last-use and hit count; store-and-evict in one transaction that deletes least-recently-used rows beyond the cap and returns the evicted count
  - Observable completion: storage tests show the table created on a pre-existing store, lookup/touch semantics, and eviction of exactly the oldest-hit rows beyond the cap
  - _Boundary: Content cache_
  - _Requirements: 4.1, 4.3, 4.5_

- [x] 1.3 (P) Cache cap configuration
  - Environment key with fail-fast validation and documented default; local-profile JSON key with the existing precedence rules
  - Observable completion: config tests cover default, valid override, invalid value error text, and local JSON precedence
  - _Boundary: Config key_
  - _Requirements: 4.3_

## 2. Core: evaluation and orchestration

- [x] 2.1 Content secret patterns and bypass evaluation
  - Implement the documented secret pattern set and the hint-path check; apply them before the threshold check; audit bypasses with reason and hash only; write no cache row and no statistics on bypass
  - Observable completion: a fixture corpus (PEM header, cloud key id, api-key assignment, bearer token, secret-bearing environment line, sensitive hint path) yields `security_critical`, a 149-line benign content yields `below_threshold`, and the audit lines contain hashes and reasons but no content
  - _Depends: 1.1_
  - _Boundary: shunt_content service_
  - _Requirements: 1.2, 1.5, 3.1, 3.2, 3.3, 3.4, 3.5_

- [x] 2.2 Cache-hit and delegation path for content
  - Line-number the content into the worker payload; call the existing summarize operation; store the summary, evict beyond cap, record the delegated statistics row before returning; serve cache hits with zero worker calls and a cache-hit statistics row; append breadcrumbs; map worker failures to the existing actionable errors with a hash-only audit and no cache row
  - Observable completion: with the stub worker, first call returns `summarized` with one worker request; second identical call returns `cached` with byte-identical summary and no worker request; statistics rows are visible from a separate connection before the call returns; timeout, unconfigured and HTTP-error stubs produce the documented errors and no cache rows
  - _Depends: 1.2, 1.3, 2.1_
  - _Boundary: shunt_content service_
  - _Requirements: 1.6, 2.2, 4.2, 4.3, 5.1, 5.2, 5.3, 5.4, 5.5_

## 3. Integration: MCP surface

- [x] 3.1 Tool handler, parameters, description and annotations
  - Register the tenth tool with the documented parameter schema; validate kind and size before calling the service; render exactly one text block holding the JSON result with absent fields omitted; map errors to actionable tool errors; wrap in the telemetry span
  - Observable completion: HTTP and stdio enumeration tests list ten tools with the conservative annotations; round trips over both transports and in local mode parse the JSON for summarized, cached and bypassed outcomes; invalid kind, empty and oversized content return the documented error texts and leave no rows behind
  - _Depends: 2.2_
  - _Requirements: 1.1, 1.3, 1.4, 2.1, 2.3, 2.4, 6.4_- [ ] 3.2 Update tool-count pins and adjacent suites
  - Update tests that pin nine tools to ten with a citation of this spec; assert prune (command and tool) leaves content rows untouched; extend audit redaction coverage to content bypasses
  - Observable completion: the whole suite passes, and the prune report still counts only file-cache rows after content rows exist
  - _Depends: 3.1_
  - _Requirements: 4.4, 6.1, 6.4_

## 4. Validation and house rules

- [ ] 4.1 Gates, help text and README
  - Run formatting, strict clippy, size caps and the full test suite; add the tool to the help text and the README tool list with the one-line distinction from `smart_read` and the cache cap setting
  - Observable completion: all gates green; README and help list ten tools; the consumer contract in the design matches the wire result byte for byte in an integration test fixture saved for the consumer's fake endpoint
  - _Requirements: 6.1, 6.2, 6.3_

- [ ]* 4.2 Live worker check
  - With a locally configured worker model, run one real content shunt on a 200-line benign file read and record latency, summary shape and the statistics delta in the spec's validation notes
  - Observable completion: figures recorded; no change to the deterministic gate
  - _Requirements: 5.1_

## Implementation Notes
- 2.2 review (2026-11-07) found `tests/loc_caps.rs`' skeleton blanker mishandles 0-hash `r"..."` raw strings (opening quote treated as own closer → Str state swallows newlines → skeleton collapses), so the 80-line function cap is NOT machine-enforced on files containing raw strings (shunt_content.rs, hook.rs, local_dirs.rs, stats_client.rs, ...). MANDATORY rider for task 4.1: fix the loc_caps lexer (correct `raw_closes` semantics for 0 hash counts) BEFORE certifying gates; until then function caps hold by manual review only (2.2 remediation manually verified all functions ≤ 80).
- tests/shunt_content_service.rs is at 747/800 lines — 3.x additions should go to a new test file or slim helpers, not extend it.
- Bypass crumbs use the AUDIT vocabulary (BypassReason::as_str: security_critical_bypass/below_line_threshold); the WIRE vocabulary in the JSON result (security_critical/below_threshold) is separate — do not conflate.
- shunt.rs and shunt_content.rs sit at 500/500 and 499/500 lines: new engine logic belongs in new files or must first extract helpers.
