# Research and Design Decisions

## Summary
- **Feature:** agent-tool-routing
- **Discovery scope:** Extension; integration-focused light discovery.
- Existing worker reads update stats successfully; observed Claude/OpenCode sessions used native reads without smart_read. Adoption is the primary investigated gap.
- Native automatically loaded instructions are needed alongside MCP discovery metadata. Claude rules avoid disrupting existing AGENTS.md discovery.
- Annotation claims must include audit/cache mutations and optional telemetry. Live harness adoption is a separate acceptance gate from deterministic service tests.

## Research Log

### Codebase and workflow
Consulted core steering, the approved requirements, `kiro-spec-design`, its light
discovery/synthesis/review rules, and design/research templates. Reused unified-memory
guidance for investigation/checkpointing. Research agents inspected current harness
documentation and pinned rmcp/source side effects; design synthesis remained in the
main context.

`src/mcp/tools.rs` defines nine tools with descriptions but no structured annotations.
`tests/local_stdio.rs` already checks enumeration and read/cache behavior. The skill
is generated into the marketplace package and checked for byte identity and a
200-estimated-token budget. Preserve proactive checkpoint contract checks.

`tests/e2e_measurement.rs` and `tests/e2e/mod.rs` offer opt-in CLI execution, timeouts,
JSON capture, fixtures, and usage extraction. They explicitly inject skill text and
do not prove automatically installed routing or natural tool selection. Reuse
patterns, not their acceptance conclusion; do not expand that measurement suite into
an unrelated routing framework.

### Annotation semantics and actual side effects
The [MCP ToolAnnotations schema](https://modelcontextprotocol.io/specification/2025-11-25/schema#toolannotations)
defines advisory hints; they are not permission grants. The pinned
[rmcp 3.8.0 API](https://docs.rs/rmcp/3.8.0/rmcp/model/struct.ToolAnnotations.html)
and local macro source support all four hint attributes without upgrading dependencies.

Source evidence: session state is upserted in `src/services/session_bus.rs`; cache
upserts replace summaries/timestamps in `src/services/shunt_cache.rs`; boilerplate
writes may overwrite files; prune deletes cache entries. Memory reads append audit
and breadcrumb records, and stats are additive rows. Therefore no tool is promised
read-only or idempotent. Destructive=false is reserved for additive memory operations.

Every tool is wrapped in tracing spans. Optional OTel builds can export to a
configured external endpoint (`src/telemetry.rs`). Static openWorld=true for all
nine conservatively covers supported configurations; it avoids claiming local-only
interaction for a Docker/OTel deployment. This sacrifices specificity, not safety.

### Harness loading and compatibility
Verified local versions during discovery: Claude 2.1.302 and OpenCode 2.2.5.
[Claude memory documentation](https://code.claude.com/docs/en/memory) describes global
and project rules. Creating CLAUDE.md can suppress AGENTS.md fallback; rules do not
trigger that suppression. Use unconditional rules without paths frontmatter.

[OpenCode V2 instructions](https://dev.opencode.ai/v2/docs/instructions/) use AGENTS.md
and combine global/ancestor instructions; unlike
[V1 rules](https://opencode.ai/docs/rules/), V2 has no CLAUDE.md fallback. Guard older
or unknown versions against suppressing existing instructions. Disabled or excluded
loading must remain the user's choice and be reported as an adoption limitation.

Current installers hardcode default home configuration paths. Sources for overrides:
[Claude environment variables](https://code.claude.com/docs/en/env-vars),
[OpenCode CLI](https://opencode.ai/docs/cli/), and
[OpenCode V2 configuration](https://dev.opencode.ai/v2/docs/cli/config).
Fail affected onboarding before writes if overrides make destinations inconsistent
or unprovable; do not partially support a relocated routing file while registration
and skills remain in the default root. Never print config-content environment values.

### Live acceptance
[Claude CLI reference](https://code.claude.com/docs/en/cli-reference) supports print,
stream JSON, bounded turns, and test-owned MCP configuration. Existing OpenCode
runner and local help support standalone JSON runs. Normal tools must remain
available, and prompts must not explicitly request the desired MCP tool.

Fresh synthetic fixtures permit extracting only tool order, versions, model,
instruction sources, worker count, and counters without private chat export.
Authentication is supplied by existing environment/profile mechanisms, never copied
into fixtures. Local authenticated runs must disclose ambient instructions; CI can
use isolated homes when credentials are supplied. Missing live prerequisites mean
NOT_VERIFIED. Same prompt alone does not establish the same cache question/key.

## Architecture Pattern Evaluation

| Option | Benefit | Limitation | Decision |
|---|---|---|---|
| Descriptions only | Minimal integration | Optional guidance already failed to produce adoption | Insufficient alone |
| Native instructions plus metadata | Available in ordinary tasks; protocol-native | Model compliance still needs observation | Selected |
| Hooks or proxy enforcement | Can force selection | Violates steering and approved boundary | Rejected |
| Custom policy/config framework | Broad extensibility | Expands scope and installer risk | Rejected |

## Design Decisions

### One owned-Markdown operation
Generalize Claude/OpenCode writes as plan/validate/atomic-commit of a single owned
section. Both OS implementations share markers and golden fixtures. This is a
specific reusable operation, not a general configuration management framework.
Native PowerShell avoids adding a Windows Python prerequisite; shell can reuse its
existing Python requirement. Preserve unrelated bytes, backups, and remove/dry-run
behavior; reject ambiguous ownership and concurrent changes.

### Adopt native capabilities
Use existing rmcp macros and harness instruction loading. No hooks, schema change,
new tool, or dependency upgrade is necessary. Keep server-side shunt/security policy
unchanged and preserve exact-read and explicit user-preference escape paths.

### Separate discovery, installation, and evidence
Metadata informs selection; installation distributes guidance; the evidence runner
observes outcomes. None claims authority over service policy. Static checks verify
distribution and contracts, while real harness events verify adoption. Missing live
evidence cannot be replaced by manually calling the desired endpoint.

## Risks and Mitigations
- User instruction damage: exact ownership parser, preflight, Unicode fixtures, backup and atomic replacement, no unowned file adoption.
- Harness version/path drift: record versions and effective sources, fail ambiguous overrides, rerun live gates on changes.
- Model ignores guidance: report failure with observed tool sequence; no universal compliance claim.
- Prompt cost: preserve portable skill budget, keep routing text concise, avoid duplicating the full checkpoint playbook.
- Cache identity drift: inspect actual key inputs and worker counts; distinguish inconclusive repeats from proven hits.
- Secret leakage: allowlist report fields, synthetic prompts/files, no raw configuration or authentication exports.

## Review Outcome
Design review covered all 20 acceptance criteria, boundaries, concrete files,
interfaces, ordering, failure behavior, and runtime prerequisites. Two local issues
were resolved: OTel-aware conservative hints and explicit cache-identity proof.
Design is ready for human review; tasks and implementation remain unapproved.
