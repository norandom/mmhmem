# Requirements Document

## Introduction
Engineers using Claude Code and OpenCode need ordinary file-inspection work to
use mmhmem's existing summary/cache path. A connected MCP server and an optional
memory skill have not produced that behavior in observed local sessions.

## Boundary Context
- In scope: descriptions and structured annotations for the existing MCP tools,
  discoverable portable guidance, automatically loaded routing instructions for
  Claude Code and OpenCode, reversible installer integration, and adoption tests.
- Out of scope: hooks, proxies, disabling native tools, new tools, bulk automatic
  warmup, storage/schema changes, savings formula changes, and universal claims
  that every model will follow instructions.
- Adjacent expectations: preserve checkpoint rules, existing permission and
  allowed-root boundaries, security-sensitive direct reads, and tool contracts.
  Shared metadata benefits other clients; automatic harness-guidance installation
  in this spec targets Claude Code and OpenCode.
- Success requires observed behavior in controlled real harness runs. Static
  instruction checks alone cannot establish adoption.

## Requirements

### Requirement 1: Discoverable tool selection and truthful annotations
**Objective:** As an agent user, I want tool metadata to explain when each tool is appropriate so that the agent can select it correctly.

#### Acceptance Criteria
1. When a client enumerates mmhmem tools, the server shall describe each tool's purpose, selection conditions, and relevant alternatives or limitations.
2. When smart_read is advertised, the server shall identify ordinary repository file inspection as its intended use and distinguish summary reads from exact targeted reads.
3. When structured annotations are supplied for an existing tool, the server shall make them consistent with that tool's actual behavior, including applicable mutations and external interactions.
4. If a tool combines read and write behavior or has cache, audit, statistics, or worker side effects, the metadata shall not make unsupported safety or idempotency claims.
5. The metadata shall preserve existing tool names and argument contracts and shall not present annotations as permission grants or guaranteed agent compliance.

### Requirement 2: Routing guidance available during ordinary work
**Objective:** As a Claude Code or OpenCode user, I want the reading guidance available without explicitly invoking a memory skill.

#### Acceptance Criteria
1. When a supported harness starts a fresh session after onboarding, the integration shall make file-routing guidance available through that harness's automatically loaded instruction mechanism without a user skill invocation.
2. When file inspection is needed and mmhmem is available and authorized, the guidance shall direct the agent to use smart_read before a full native or shell file read.
3. When exact source lines are required for an edit or verification, the guidance shall direct targeted reads and shall not treat a structural summary as exact source evidence.
4. If mmhmem or its worker is unavailable, the guidance shall provide a bounded fallback for authorized work without repeated failing calls, permission bypass, or false savings claims.
5. The guidance shall preserve native search, build, test, and edit workflows, existing checkpoint rules, and explicit user tool preferences while making the skill discoverable for file reading and token optimization.

### Requirement 3: Safe and reversible installation
**Objective:** As a user with existing harness settings, I want onboarding to add routing guidance without damaging my configuration.

#### Acceptance Criteria
1. When a selected supported harness is installed or updated, the installer shall add or update exactly one owned routing-instruction section while preserving unrelated user instructions and configuration.
2. When installation repeats or an update encounters content from either OS installer, the installer shall avoid duplicate sections and preserve Unicode content.
3. If existing content is ambiguous, malformed, or conflicts with safe ownership identification, the installer shall stop the affected operation before modifying it and explain the required correction.
4. When existing instruction content is changed, the installer shall retain a recoverable backup and prevent a failed write from leaving truncated content.
5. When removal or dry-run is requested, the installer shall respectively remove only owned guidance or leave the configuration unchanged; the completion message shall explain any session reload required for updated instructions to take effect.

### Requirement 4: Evidence of adoption and honest savings
**Objective:** As a user, I want evidence that installed guidance changes actual tool use and that displayed savings correspond to recorded work.

#### Acceptance Criteria
1. When a fresh Claude Code or OpenCode session receives an ordinary task to inspect an eligible large repository file, the acceptance run shall record smart_read use without naming that tool or explicitly invoking the skill in the task prompt.
2. When the acceptance run repeats an equivalent inspection of unchanged content with the same cache identity, the verification shall establish a cache hit, no new worker request for that hit, and the expected existing stats update.
3. When validating direct or targeted reads, session-only calls, or a failed worker request, the verification shall confirm that those operations do not falsely count summary/cache savings.
4. When evaluating adoption, the validation report shall record harness/model versions, effective instruction source, task prompt, observed tool sequence, and before/after counters while keeping credentials and private conversation content out of the report.
5. If a harness does not select smart_read, or a required live run cannot be performed, the report shall identify the observed failure or missing evidence and shall not declare adoption complete based only on static checks or a manually forced MCP call.

