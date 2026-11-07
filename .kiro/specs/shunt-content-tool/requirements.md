# Requirements Document

## Project Description (Input)

An LLM-API proxy (mvibe, `../mvibe`) sees every resend of a file-read tool result across an agent conversation and wants mmhmem to turn such a result into a deterministic structural summary. mmhmem's shunt is path-based (`smart_read(file_path)` with allowed-root confinement), but the proxy holds a string, not a path, and the file may live outside mmhmem's roots or on another machine. Add one content-based MCP tool, `shunt_content`, that reuses the existing shunt engine (worker model, content-addressed cache, security-critical bypass, hash-only audit, honest statistics) for a caller-supplied string, with a bounded cache suited to ephemeral tool output. mmhmem stays protocol-native: it does not become a proxy and does not parse LLM API bodies.

## Introduction

The existing `smart_read` path evaluates a file: targeted slice, security-critical or below-threshold direct read, cache hit, or worker delegation with a cached summary. `shunt_content` is the same evaluation applied to content handed over by the caller: no filesystem access, no confinement, an optional source hint for the path-based security regex, and a separate cache table because tool output is ephemeral and must not be mistaken for a file the prune command can re-check on disk. Steering `tech.md` ("Protocol-native integration only", "Shunt logic lives server-side") is preserved: the consumer integrates over MCP, and all policy stays in mmhmem.

## Boundary Context

- **In scope**: the `shunt_content` tool in serve and local modes over both transports; its argument and result contract; content-based security-critical bypass (hint path regex plus content secret patterns); below-threshold bypass; a dedicated bounded content cache with least-recently-used eviction; audit, breadcrumb and statistics records with the existing honesty rules; tests, help text, README tool list, and updates to this repository's tests that pin the tool count.
- **Out of scope**: any parsing of chat-completion bodies or tool-call correlation (consumer's job); new worker prompts beyond reusing the bulk-reader prompt; changes to `smart_read`, `shunt_warmup` or `shunt_prune` semantics; TTL eviction for the file cache; summarizing kinds other than file reads in v1 (the `kind` argument is validated, not extended); playbook or routing-instruction changes (agents keep using `smart_read`; this tool is for programmatic consumers).
- **Adjacent expectations**: the consumer sends only historical file-read results and keeps summaries byte-stable on its side; it treats a tool error as fail-open; it never sends credentials as content on purpose and accepts that mmhmem's content secret patterns are a safety net, not a guarantee. The `agent-tool-routing` spec's nine-tool enumeration assertions become ten-tool assertions here. The `corpus-warmup-prune` prune command must not treat content-cache rows as stale files.

## Requirements

### Requirement 1: Content-based shunt tool
**Objective:** As a programmatic MCP consumer holding a tool-result string, I want one tool that returns a structural summary for it, so that I can reuse mmhmem's shunt without a file path.

#### Acceptance Criteria
1. The mmhmem binary shall expose a `shunt_content` tool that accepts the project, the calling harness, the content string, a `kind` discriminator, an optional source hint and an optional question, in both serve and local modes and over both the stdio and Streamable HTTP transports.
2. When `kind` is the file-read kind, the tool shall evaluate the content with the same ordering as a shunted read: security-critical bypass first, then below-threshold bypass, then cache lookup, then worker delegation.
3. If `kind` is not a supported value, the tool shall return an actionable error naming the supported values and shall record no cache, statistics or breadcrumb entry.
4. If the content is empty or exceeds a documented maximum size, the tool shall return an actionable error and shall not call the worker.
5. The tool shall never read, write or confine filesystem paths on behalf of this call; the source hint is a label only.
6. When no question is supplied, the tool shall use one fixed default question so that identical content from different callers shares one cache identity.

### Requirement 2: Result contract
**Objective:** As the consumer, I want a machine-readable result, so that I can act on it without parsing prose.

#### Acceptance Criteria
1. The tool shall return exactly one text content block containing a single JSON object with: `outcome` (one of `summarized`, `cached`, `bypassed`), `summary` (string when summarized or cached, otherwise absent), `bypass_reason` (string when bypassed, otherwise absent), `content_sha256`, `line_count`, `avoided_input_tokens_est`, `latency_ms`, and `worker_model` (when the worker was called).
2. When the result is a cache hit, the `summary` bytes shall be identical to the bytes stored at delegation time.
3. The tool shall not include the content, the source hint's directory structure beyond what the caller supplied, or any credential in the result beyond the summary text produced by the worker.
4. When the tool is listed, its description shall state that it is for programmatic consumers with content in hand, that summaries are not exact evidence, and that cache, audit and statistics are written; its annotations shall follow the existing conservative matrix (not read-only, destructive because cache rows can be replaced or evicted, not idempotent, open world).

### Requirement 3: Bypass rules for content
**Objective:** As a security-conscious operator, I want secrets and sensitive code never sent to the worker through this tool, so that the content path is at least as safe as the file path.

#### Acceptance Criteria
1. When the source hint matches the existing security-critical path pattern, the tool shall bypass with reason `security_critical` without calling the worker or writing a cache row.
2. When the content matches a documented set of secret patterns (at least: PEM private-key headers, bearer or API-key assignments, cloud access-key identifiers, environment assignments whose key contains secret, token, password or key), the tool shall bypass with reason `security_critical` without calling the worker or writing a cache row.
3. When the content has fewer lines than the configured shunt threshold, the tool shall bypass with reason `below_threshold` without calling the worker.
4. The tool shall audit every bypass with the reason and the content hash, never the content.
5. The tool shall not add savings statistics for a bypass.

### Requirement 4: Bounded content cache
**Objective:** As an operator, I want the content cache to stay small, so that ephemeral tool output cannot grow the store without limit.

#### Acceptance Criteria
1. The tool shall cache summaries in a dedicated content cache keyed by content hash, question fingerprint and kind, separate from the file cache.
2. When a cached summary exists for the same key, the tool shall return it without calling the worker, update its last-use time, and count a cache hit.
3. The content cache shall hold at most a configured number of rows (default documented) and, when the cap is exceeded after an insert, shall evict the least recently used rows until the cap holds; eviction shall be audited as a count, not per row.
4. The prune command and the `shunt_prune` tool shall leave the content cache untouched and shall report only file-cache rows, as today.
5. Startup on an existing store shall add the content cache without altering existing tables or data, and the pre-existing suite shall pass unchanged.

### Requirement 5: Honest accounting and audit
**Objective:** As the maintainer of the statistics discipline, I want this tool to record only real events with labelled estimates, so that reports stay truthful.

#### Acceptance Criteria
1. When the worker is called, the tool shall record one delegated-shunt statistics row with the labelled estimate, the worker usage, the project and the harness, committed before the response returns.
2. When a cache hit is served, the tool shall record one cache-hit statistics row with the labelled estimate.
3. Every outcome shall write one audit event carrying the outcome, the content hash, the kind, the line count and the latency, and never the content or the summary.
4. Every non-error outcome shall append one breadcrumb naming the tool, the outcome and the hash prefix.
5. If the worker is not configured, times out or fails, the tool shall return the existing actionable worker error, shall audit a worker error with the hash, and shall write no cache row and no savings statistics.

### Requirement 6: Non-regression and house rules
**Objective:** As the maintainer, I want the new surface to arrive without breaking existing guarantees or size caps.

#### Acceptance Criteria
1. The pre-existing deterministic suite shall pass unmodified; tests that pin the tool count shall be updated from nine to ten and shall cite this spec.
2. Source files shall respect the existing 500-line and function-size caps; the shunt engine shall be extended through a shared evaluation path rather than duplicated.
3. The help text and README tool list shall include the new tool with a one-line description that distinguishes it from `smart_read`.
4. Integration tests shall exercise the tool over both transports in serve mode and in local mode, including the error, bypass, delegation and cache-hit outcomes with a stub worker and no network.
