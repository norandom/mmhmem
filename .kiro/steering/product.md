# Product Overview

mmhmem is a self-hosted, cross-agent memory and I/O optimization server for AI coding harnesses. It replaces hook-bound local plugins (e.g. claude-mem) with a shared memory service that heterogeneous agents connect to via standard protocol-native interfaces. Optional, explicitly enabled read-routing hooks guide supported harnesses toward the MCP tools; memory and summaries work without them. No MITM API proxies or SaaS dependency.

Deployment model (confirmed 2026-11-01): client-server with the server OPTIONAL. Clients run locally on Linux, Windows, and macOS and each harness spawns `mmhmem stdio` — the full token-efficiency layer (shunt + cache + all six tools) against a per-user shared store, with local stats via `mmhmem stats`. The Linux container server is an optional upgrade: cross-machine access, central audit/compliance, central ops (logrotate, OTel), multi-user bearer auth. One binary, two transports, two config profiles.

## Core Capabilities

- Live cross-agent session memory: mid-task handoff between harnesses with structured working state plus an automatic breadcrumb trail of every tool action
- Deterministic server-side I/O shunting: large file reads are summarized by a cheap worker model and cached by content hash; security-critical code always bypasses shunting; boilerplate generation never consumes frontier-model tokens
- Progressive-disclosure long-term project memory: compact index search first, full details only for validated entries
- Security-grade auditability: every action recorded as a secret-redacted, hash-chained single-line JSON audit record on the local filesystem, rotatable with standard Unix tooling

## Target Use Cases

- An engineer switches mid-task from a terminal agent to a desktop IDE (or vice versa) and resumes with full in-flight context
- A security-conscious infrastructure operator wants per-agent memory and token economics without cloud telemetry or per-workstation silos, with optional hooks that respect harness policy and approval
- Teams spending heavily on agent I/O tokens want deterministic routing of bulk reads/writes to small models

## Value Proposition

One auditable, Unix-native daemon instead of N per-harness plugins: deterministic token savings, lossless cross-agent handoff, and a tamper-evident local audit trail — deployable as a single binary behind systemd.

---
_Focus on patterns and purpose, not exhaustive feature lists_
