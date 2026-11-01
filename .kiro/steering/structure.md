# Project Structure

## Organization Philosophy

Layered daemon: protocol layer (MCP tools) → service layer (session bus, shunt engine, memory store, audit) → storage/persistence. Each layer has a single responsibility and is testable in isolation.

## Directory Patterns

### `cmd/` (or `src/main.rs`)
**Location**: `/cmd/server/`  
**Purpose**: entrypoint, config loading from environment, wiring, graceful shutdown  
**Example**: parses `AGENT_MEM_*` env vars, opens storage, starts HTTP listener

### `src/mcp/` (protocol layer)
**Location**: `/src/mcp/`  
**Purpose**: MCP tool definitions and handlers (session handoff, smart_read, boilerplate_write, mem_search, mem_get_observations, mem_record_observation); thin — delegates to services  
**Example**: `smart_read` handler validates auth context, calls shunt service, returns markdown

### `src/services/`
**Location**: `/src/services/`  
**Purpose**: session bus, shunt engine, memory store, audit middleware — the core logic  
**Example**: `shunt.rs` implements the four-stage read evaluation and content-hash cache

### `src/storage/`
**Location**: `/src/storage/`  
**Purpose**: schema/migrations, connection handling (WAL), FTS index sync, JSONL audit writer  
**Example**: `audit.rs` owns redaction filter + atomic append + record hashing

### `deploy/`
**Location**: `/deploy/`  
**Purpose**: systemd unit, logrotate config, SKILL.md playbook, example harness connection configs (claude mcp add, mcp_config.json snippets)  
**Example**: `/deploy/logrotate.d/agent-memory`

### `tests/`
**Location**: `/tests/`  
**Purpose**: end-to-end MCP integration tests and redaction/security tests  
**Example**: spins server on ephemeral port, runs full handoff + shunt + audit scenario

## Naming Conventions

- **Files/modules**: snake_case (Rust) / snake_case packages (Go)
- **MCP tools**: snake_case verbs (`session_resume_or_handoff`, `smart_read`)
- **Env vars**: `AGENT_MEM_*` prefix for server config; `SHUNT_MIN_LINES`, `WORKER_*` for tuning
- **Audit event types**: `SCREAMING_SNAKE_CASE` (e.g. `IO_SHUNT_DELEGATED`, `SESSION_CHECKPOINT`)

## Import Organization

- Layered dependency rule: `mcp` → `services` → `storage`; never the reverse. `storage` must not depend on `services`.

## Code Organization Principles

- Tool handlers stay thin; all policy (thresholds, security bypass regex, redaction) lives in services/storage and is unit-testable without HTTP
- Every state mutation goes through the audit writer — no direct writes outside it
- Config is environment-only; no config files parsed at runtime

---
_Document patterns, not file trees. New files following patterns shouldn't require updates_
