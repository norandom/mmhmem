# Research & Design Decisions — shunt-content-tool

## Summary
- **Feature**: `shunt-content-tool`
- **Discovery Scope**: Extension of the validated shunt engine (light discovery) driven by an external consumer (mvibe proxy).
- **Key Findings**:
  - `ShuntEngine::evaluate` (`src/services/shunt.rs:170-205`) binds evaluation to a confined path and a filesystem read; stages 3–4 (`shunt`, `serve_cached`, `delegate`) are already path-agnostic except for the cache key and `cache::raw_chars(path)`.
  - The file cache (`shunt_file_cache`, `src/storage/schema.rs:63-71`) is keyed by `(file_path, content_sha256, query_fingerprint)`; prune classifies rows by file existence and content hash on disk (`src/services/prune.rs:40-80`). Content rows must not live there or prune would delete them as "missing".
  - The security-critical bypass is a path regex only (`src/services/shunt.rs:38-43`); content needs its own secret patterns. Audit redaction tests already enforce that secret patterns never reach disk (`tests/audit_redaction.rs`).
  - Tool count is pinned at nine in `tests/local_stdio.rs:176-208, 297` (agent-tool-routing Req 1.1); the annotation matrix is in `agent-tool-routing/design.md`.

## Research Log

### What the consumer sends (mvibe capture analysis, 2026-11-03)
- **Context**: Size the threshold, cache and worker payload.
- **Sources Consulted**: `../mvibe/.kiro/specs/proxy-econ-client/research.md`.
- **Findings**: File-read results dominate (80.7% of tool-result bytes); typical read output is 4–40 KB, line-numbered (`N: text`) with a header `Read file <path>, lines a-b`; a distinct result is resent ~12.6 times per conversation; 387 distinct contents across 258 captured requests.
- **Implications**: Cache hit rate will be high; cap of 5000 rows comfortably covers weeks of use; `SHUNT_MIN_LINES` (150) is a sensible shared threshold since the consumer additionally pre-filters by bytes. Maximum content size 1 MiB protects the worker call.

### Engine refactor seam
- **Context**: Reuse without duplication under the 500-LOC cap (`tests/loc_caps.rs`).
- **Sources Consulted**: `src/services/shunt.rs` (471 lines), `shunt_cache.rs`, `shunt_types.rs`, `stats.rs:90-115`, `worker.rs:130-146`.
- **Findings**: `delegate()` calls `cache::line_numbered_payload(&path)` and `cache::cache_store(db, path, sha, fp, ...)`; `serve_cached()` and `audit_shunt()` take a path for audit details. A `ShuntSource` enum (`File{path}` | `Content{hint}`) threaded through `Miss`/`ShuntAuditMeta` keeps one code path; `shunt.rs` is close to the cap, so the content entry point and secret patterns go in a new `src/services/shunt_content.rs`, and the content cache helpers in `shunt_cache.rs` (currently ~100 lines).
- **Implications**: File Structure Plan below; no change to `smart_read` behaviour.

### Worker payload for pre-numbered content
- **Context**: `BULK_READER_SYSTEM` expects line ranges `[L<start>-L<end>]`.
- **Findings**: OpenCode read output is already `N: text`; Claude Code's Read output is `N→text`. `line_numbered_payload` prefixes `L<n>: ` for files. For content, prefix the same way regardless of existing numbering: the worker then references the payload line numbers, which match the original line positions in the tool result 1:1, so the consumer's model can still correlate ranges with what it saw earlier.
- **Implications**: `summarize_file(payload, question)` reused unchanged; no new prompt.

### Statistics kinds
- **Context**: Separate kinds vs reuse.
- **Findings**: `token_stats.kind` is free text; the CLI groups by kind; `stats` reporting lists known kinds. Attribution per harness already exists (`agent_harness` column); the consumer identifies itself as `mvibe-proxy`.
- **Implications**: Reuse `shunt_delegated` and `shunt_cache_hit`; no reporting change. Audit `details.source = "content"` distinguishes the path.

### Result framing over MCP
- **Context**: rmcp 3.8.0 `CallToolResult`; consumer wants JSON.
- **Findings**: All existing tools return one `ContentBlock::text(markdown)`. rmcp 3.x also supports `structured_content`, but the consumer's minimal client parses text blocks; a single JSON object in the text block is unambiguous and version-proof.
- **Implications**: Requirement 2.1 (JSON object in one text block). `structured_content` may be set additionally if trivial; not required.

### Consulted skills and guidance
- `kiro-spec-requirements`, `kiro-spec-design`, `kiro-spec-tasks` rules; steering `product.md`, `tech.md` (protocol-native, server-side shunt logic, redaction tests mandatory), `structure.md` (services never import mcp), `dev-environment.md`.

## Architecture Pattern Evaluation

| Option | Description | Strengths | Risks / Limitations | Notes |
|---|---|---|---|---|
| Store content rows in `shunt_file_cache` with a synthetic path | Zero schema change | Minimal code | Prune deletes them as missing files; path semantics leak | Rejected |
| Dedicated `shunt_content_cache` table with LRU cap (selected) | Additive `CREATE TABLE IF NOT EXISTS` | Clean eviction policy; prune untouched; ephemeral data isolated | One more table; eviction query on insert | Selected |
| mmhmem parses LLM bodies itself | Single component | — | Violates steering (protocol-native only) | Rejected |
| Make `smart_read` accept inline content | No new tool | — | Overloads a path tool; annotations and routing text would mislead agents | Rejected |

## Design Decisions

### Decision: One shared evaluation path, two sources
- **Context**: LOC caps and non-regression.
- **Selected Approach**: Introduce `ShuntSource` in `shunt_types.rs`; `delegate`/`serve_cached`/`audit_shunt`/`record_stat` take the source; `shunt_content.rs` builds the source from the call and runs the same stages.
- **Trade-offs**: Small touch on `shunt.rs` internals; file-path behaviour covered by the existing suite, which must stay green.

### Decision: Content secret patterns live in mmhmem, hint regex reused
- **Context**: Steering mandates server-side policy.
- **Selected Approach**: `SECURITY_CRITICAL_RE` applied to `source_hint`; new `CONTENT_SECRET_RE` (PEM headers, `AKIA[0-9A-Z]{16}`, `(?i)(api[_-]?key|secret|token|password|passwd|pwd)\s*[:=]\s*\S{8,}`, `(?i)bearer\s+[a-z0-9._-]{16,}`, `(?i)^[A-Z_]*(SECRET|TOKEN|PASSWORD|KEY)[A-Z_]*=` per line). Any match ⇒ bypass, hash-only audit.
- **Rationale**: Same safety net for both entry points; the consumer memoizes the bypass so content is not resent.

### Decision: LRU eviction by `last_hit_epoch`
- **Context**: Ephemeral data; `corpus-warmup-prune` explicitly excluded TTL/size eviction for the file cache, not for a new content cache.
- **Selected Approach**: `SHUNT_CONTENT_CACHE_MAX_ROWS` (default 5000, env and `.mmhmem.json` for local profile); after each insert, delete rows beyond the cap ordered by `last_hit_epoch ASC, created_at_epoch ASC`; one audit event with the evicted count.
- **Trade-offs**: A regenerated summary after eviction may differ; the consumer's in-process memo keeps per-conversation determinism.

### Decision: Reuse statistics kinds; tag audit source
- See research log "Statistics kinds".

## Risks & Mitigations
- Worker latency on the consumer's warm path (not on inference): bounded by the existing 30 s summarize timeout; the consumer fails open.
- Content larger than the worker's context: 1 MiB hard cap; the worker error path reports it; no partial summaries.
- Tool-count pins in other specs' tests: updated here with citations (Req 6.1).
- Secret patterns false positives (e.g. a README mentioning `token=`): acceptable, results in a bypass, never a leak.

## References
- `../mvibe/.kiro/specs/proxy-econ-client/design.md` (consumer contract, `McpClient`).
- `.kiro/specs/corpus-warmup-prune/requirements.md` (prune semantics, stats honesty).
- `.kiro/specs/agent-tool-routing/design.md` (annotation matrix, tool enumeration tests).
