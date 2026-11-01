# Brief: cross-agent-memory-server

## Problem
Security-conscious infrastructure operators running heterogeneous AI coding harnesses (Claude Code, Google Antigravity, OpenCode, Claude Desktop) suffer from (a) fragmented, per-workstation memory (claude-mem-style local plugins) with no lossless mid-task cross-agent handoff, (b) fail-open client-side lifecycle hooks that silently drop context, (c) rising I/O token costs from frontier models bulk-reading large files and generating boilerplate, and (d) SaaS/telemetry coupling in existing plugins. The pain: token spend of $200–$2,000+/dev/month and broken context continuity when switching agents mid-task.

## Current State
Greenfield. A research document (`Self-Hosted Cross-Agent Memory Solutions.md`) contains a full architectural analysis and a Python/FastMCP reference implementation. No code exists yet. No client hooks or MITM proxies are acceptable in the target design.

## Desired Outcome
A single self-hosted Linux daemon ("mmhmem") that agent harnesses connect to via remote Streamable HTTP MCP + portable SKILL.md, providing: live cross-agent session handoff, deterministic server-side I/O shunting with content-addressed caching, progressive-disclosure long-term memory, and a secret-redacted, tamper-evident JSONL audit trail rotatable with logrotate. Compiled language recommended (Rust; Go as fallback) — user asked for a suggestion.

## Approach
Build the daemon natively in a compiled language (recommended: Rust — memory safety for a network-facing security-grade daemon, single static binary for systemd, precedent of ai-memory in Rust; fallback: Go). Port the reference architecture from the research doc: MCP tool surface (session_resume_or_handoff, smart_read, boilerplate_write, mem_search, mem_get_observations, mem_record_observation), two-tier storage (live session bus + FTS long-term store + shunt cache), four-stage shunt evaluation with security-critical bypass and ~150-line threshold, SHA-256-keyed summary cache, worker-SLM endpoint for shunted reads/writes, audit middleware with redaction + record hashing, plus deploy artifacts (systemd unit, logrotate config, SKILL.md, harness connection snippets).

## Scope
- **In**: the daemon and its MCP tools; session bus + breadcrumbs; shunt engine + cache; progressive-disclosure memory store; audit log + redaction; worker model client; deployment artifacts (systemd, logrotate, SKILL.md, connection configs) **plus a Docker image containing only the dedicated binary and a docker-compose.yml**; token-savings statistics (server-computed estimates + optional client-reported deltas, per harness/per task over time); optional OTel export to a self-hosted OpenObserve; a documented data-lifecycle map; tests including an E2E measurement suite that drives a real harness (OpenCode + Mistral) via compose and asserts measured token savings. Scope is guarded by `breath.toml` in this directory.
- **Out**: the harness clients themselves; hosting the worker SLM; TLS certificate management; any web viewer UI; forking/patching claude-mem; real-time SIEM streaming (batch ship only); prompt-injection scanning/classification; any client SDK or hooks

## Boundary Candidates
- Protocol layer (MCP tools/auth) vs service layer (session/shunt/memory/audit) vs persistence
- Live session bus (ephemeral) vs long-term memory (durable) vs shunt cache (content-addressed)
- Daemon vs deployment/ops artifacts vs harness-side SKILL.md

## Out of Boundary
- Client-side hooks, LLM API reverse proxies, SaaS/telemetry, desktop UI, multi-machine replication/clustering (single-host daemon)

## Upstream / Downstream
- **Upstream**: agent harnesses speaking MCP Streamable HTTP + Agent Skills; a configurable worker SLM endpoint (local vLLM/Ollama or API); Linux host with systemd + logrotate
- **Downstream**: future OpenObserve batch ingestion; future multi-host sync; other tools consuming the JSONL audit

## Existing Spec Touchpoints
- **Extends**: none (greenfield)
- **Adjacent**: none

## Constraints
- No client-side hooks; no MITM proxy of LLM traffic; Bearer-token auth on the MCP endpoint
- Security-critical paths (auth/crypto/concurrency/...) must never be shunted to the worker model
- Worker SLM summaries must be cached by content hash so repeat cross-agent reads cost zero LLM calls
- Audit records must be redacted (keys, tokens, bearer headers, private tags) and hash-verifiable before hitting disk; logrotate must work without restarting the daemon
- Compiled language required (user request); recommendation: Rust, fallback Go
- Language of all spec artifacts: English
