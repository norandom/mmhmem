# Technology Stack

## Architecture

Single self-hosted daemon on a Linux host, exposed to agent harnesses as a remote MCP server over Streamable HTTP with Bearer-token auth, paired with a portable SKILL.md playbook installed in each harness. Storage is a local write-ahead-logged embedded store with a full-text index, plus append-only JSONL audit logs managed by logrotate. A configurable external worker model endpoint (local or API) handles shunted summarization/generation.

## Core Technologies

- **Language**: Rust — compiled, memory-safe, single binary (confirmed by user 2026-11-01; plain Linux static linking follows the distribution contract below; Go documented as fallback in spec research)
- **Serving**: async HTTP server exposing the MCP Streamable HTTP transport
- **Storage**: embedded SQL store with WAL journaling and FTS5 full-text search, plus Git-compatible plain-text state where useful
- **Audit**: append-only single-line JSON records, SHA-256 self-verifying, redacted before write

### Linux distribution contract

- The plain Linux CI release targets `x86_64-unknown-linux-musl` with exact
  Rust 1.98.1, baseline x86-64/SSE2 and a Linux 5.10 support floor. The canonical
  Dagger gate verifies one candidate on Debian 12 and Alpine 3.22, recording
  the actual shared host kernel and CPU separately from the support policy.
- Release status checked 2026-11-02: latest downloadable v0.2.2 is still
  glibc-dependent. Static CI candidates are verified, but public portability
  wording changes only after publication and latest-endpoint checksum
  confirmation. See `deploy/README.md` for the guarded draft transition.
- The OTel-enabled Docker image remains a GNU build. Production TLS policy
  is unchanged; reverse-proxy/load-balancer TLS belongs to the administrator,
  and TLS-specific tests are excluded from static-release acceptance.

## Key Libraries (Rust path)

- axum + tokio (HTTP/async runtime), rmcp (MCP server SDK, Streamable HTTP), rusqlite with bundled SQLite + fts5 feature, sha2, serde/serde_json, regex, reqwest (worker model calls)

## Development Standards

### Type Safety
- Rust: no `unsafe` outside narrow, reviewed modules; strict clippy lints (`-D warnings`)
- Go alternative: `go vet` clean, no ignored errors

### Code Quality
- rustfmt / gofmt enforced; conventional commits

### Testing
- Unit tests per module; integration tests spinning up the server on an ephemeral port and driving MCP calls end-to-end; audit-log redaction tests mandatory (secret patterns must never reach disk)

## Development Environment

### Required Tools
- Rust toolchain (stable) + cargo, or Go 1.2x+; SQLite CLI for forensics; logrotate on target host

### Common Commands
```bash
# Dev: cargo run   (or: go run ./cmd/server)
# Build: cargo build --release   (or: go build -o bin/mmhmem ./cmd/server)
# Test: cargo test   (or: go test ./...)
```

## Key Technical Decisions

- **Compiled language: Rust (recommended).** The daemon is network-facing, multi-tenant, and security-grade (path handling on untrusted input, secret redaction, concurrent sessions) — Rust's ownership model eliminates whole bug classes and yields one static binary for trivial systemd deployment. Precedent: ai-memory ships exactly this shape in Rust. Go is the fallback if team velocity outweighs memory-safety rigor; Zig rejected (immature HTTP/MCP/SQLite server ecosystem).
- **Protocol-native integration only**: MCP Streamable HTTP + Agent Skills; client hooks and LLM API proxies are explicitly rejected (fail-open hooks, OAuth breakage).
- **Shunt logic lives server-side**, not in client hooks, making it harness-neutral; security-critical path regex always bypasses the worker model; threshold tuned to ~150 lines from empirical median read sizes.
- **Two-tier storage**: ephemeral live session bus + durable progressive-disclosure store; content-addressed cache for shunted summaries.
- **No SaaS, no telemetry**: fully self-hosted; audit stays local (optional batch ship to self-hosted OpenObserve).

---
_Document standards and patterns, not every dependency_
