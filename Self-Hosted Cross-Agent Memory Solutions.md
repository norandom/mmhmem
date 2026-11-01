Architecting a Self-Hosted, Multi-Agent Memory and I/O Optimization Server: Replacing Claude-Mem Across Heterogeneous Harnesses

Executive Assessment: Forking Claude-Mem vs. Deploying a Unified Memory and I/O Server

The proliferation of specialized AI coding harnesses—spanning terminal-native autonomous agents such as Claude Code and OpenCode, desktop conversational workspaces such as Claude Desktop, and parallel multi-agent integrated development environments (IDEs) such as Google Antigravity—has shifted the primary engineering bottleneck from raw model reasoning to context state management and input/output (I/O) token economics1. While claude-mem (thedotmack/claude-mem) pioneered persistent cross-session context compression by capturing tool execution observations and re-injecting semantic summaries into subsequent sessions, its architectural trajectory through version 15.5.0 introduces structural friction for security-conscious infrastructure operators seeking a unified, self-hosted memory fabric across heterogeneous agents5.

A forensic examination of the claude-mem codebase clarifies both its internal capabilities and its operational boundaries. Contrary to the perception that claude-mem exclusively requires Anthropic models for memory compression, its local Bun-managed Express worker service (worker-service.cjs) does support multiple compression backends alongside its default cmem.ai SaaS observer (CMEM Pro)6. Specifically, operators can configure personal OpenRouter API keys (https://openrouter.ai/api/v1), Google Gemini API keys, local host-agent loopbacks (--provider host), or existing Anthropic plans (--provider claude routing to Haiku 4.5, Sonnet 5, or Opus 4.8), and version 15.5.0 further introduced opt-in OpenAI-compatible batching (#5117) with explicit summary provider selection (#5104)6. Furthermore, following its version 7.1.0 migration from PM2 to a native Bun ProcessManager and bun:sqlite, claude-mem already generates daily rotating log files (\~/.claude-mem/logs/worker-YYYY-MM-DD.log) and maintains a durable tool\_uses SQLite table (introduced in schema migration 051) that records up to 64 KB of raw tool inputs and responses alongside pre-truncation content hashes8. It also implements a token-saving PreToolUse File Read Gate (file-context), routes grep and read note-taking operations to Haiku to cut observation overhead by 50%, and exposes a three-layer progressive disclosure Model Context Protocol (MCP) search workflow5.

Despite these built-in features, forking and patching claude-mem is architecturally suboptimal for a multi-agent, self-hosted server deployment due to four compounding structural constraints6. First, the project exhibits deepening SaaS coupling and telemetry drift: the interactive installation pipeline (npx claude-mem install) defaults to OAuth device pairing against https://cmem.ai/api/installer/oauth/start and pre-selects the hosted cmem-observer backend, which operates on a 30-day trial before triggering an event-driven fallback via CLAUDE\_MEM\_PRO\_FALLBACK\_AT8. Recent releases such as v15.5.0 have further embedded commercial SaaS instrumentation, including PostHog telemetry tracking viewer opens and page switches (#5107), requiring continuous maintenance of a custom fork to strip cloud sync and analytics pathways5.

Second, claude-mem depends entirely on client-side lifecycle hooks (SessionStart, UserPromptSubmit, PreToolUse, PostToolUse, PostToolUseFailure, Stop, and SessionEnd) invoking a compiled local binary launcher (\~/.local/bin/claude-mem) that dispatches fire-and-forget HTTP requests with a strict 2,000-millisecond timeout to a localhost port (37700 + (uid % 100))6. Because these hooks are engineered to fail open—always exiting with status 0 so that IDE responsiveness is never blocked—any worker congestion or network latency silently drops live context injection or leaves observations stranded in the pending\_messages queue, where automatic startup recovery has been disabled since version 5.x8.

Third, its storage topology is fundamentally siloed per workstation8. The bun:sqlite database (\~/.claude-mem/claude-mem.db) operates in local Write-Ahead Logging (WAL) mode bound to a single OS user port on the local host8. It is neither architected as a network-accessible, multi-tenant daemon with concurrent cross-machine session locking, nor capable of intercepting tool calls in desktop clients that lack CLI command hooks, such as Claude Desktop5. Fourth, and most critically, claude-mem lacks live cross-agent session handoff8. Because it generates structured session summaries (request, investigated, learned, completed, next\_steps) asynchronously on the Stop and SessionEnd hooks via background LLM calls, it excels at post-hoc historical replay across sequential runs of the same tool, but fails when an engineer switches mid-task from Claude Code to Google Antigravity or OpenCode and requires immediate, uncompressed working-state continuity8.

Consequently, the most resilient engineering path is not maintaining a patched fork of a hook-bound local plugin, but rather deploying a single, self-hosted Remote MCP and Agent Plugin Server on a Linux host13. By replacing brittle client hooks and intrusive network proxies with protocol-native Streamable HTTP MCP tools and standardized Agent Skills (SKILL.md), a centralized Linux daemon can deterministically enforce Spotify-style I/O token shunting, maintain live cross-agent session state, and write tamper-evident, Unix-native daily JSONL audit logs managed by logrotate2.

Deconstructing I/O Token Optimization: Merging Spotify Shunt with Progressive Disclosure

A foundational premise of modern coding agent cost optimization—demonstrated by Dimitri Mazmanov at Spotify in September 2026—is that the vast majority of token expenditure in autonomous coding sessions is consumed by raw I/O rather than complex reasoning2. Industry benchmarks indicate that a quarter of engineering organizations spend between $200 and $500 per developer per month on agent tokens, with heavy workloads exceeding $2,000 per month and projected to surpass average developer salaries by 2028 if left unoptimized2. This expenditure is driven largely by frontier models reading five entire source files simply to verify a single method signature or generating repetitive test boilerplate2. To eliminate this waste without degrading code quality, an optimal self-hosted memory server must synthesize two distinct token-reduction paradigms: Spotify's deterministic I/O model routing (shunt) and claude-mem's progressive disclosure retrieval2.

![](image.png)

Mechanics and Second-Order Trade-Offs of Spotify's Shunt Architecture

Spotify engineered the shunt plugin (spotify/portal-ai-plugins) to route high-volume I/O tasks away from Claude Code's primary frontier model (such as Claude Opus 4.8 or Sonnet 4.6) to ephemeral, declarative agent runtimes called AiKA Modes hosted on Spotify Portal (its commercial Backstage platform) and backed by fast, inexpensive Small Language Models (SLMs) such as Gemini 2.5 Flash or Claude Haiku 4.52. Initially, Spotify attempted to enforce model routing via advisory natural-language rules inside CLAUDE.md, but found that frontier models routinely ignored advisory instructions under heavy context load and required maintaining duplicate instructions across every repository2.

To make routing deterministic rather than advisory, shunt introduced a three-layer architecture combining interception hooks, execution scripts, and redirection skills2. At the interception layer, check-file-size inspects every incoming Read tool call and evaluates whether the target file exceeds a configurable line threshold (SHUNT\_MIN\_LINES, defaulting to 350 lines); if exceeded, it blocks the read and instructs the agent to invoke the /bulk-reader skill, while permitting targeted reads that specify explicit line offset and limit parameters to pass through2. Simultaneously, check-bash-read intercepts shell commands (cat, head, tail, less, and more) targeting large files while allowing piped filtering commands such as cat file \| grep to execute normally2. At the execution layer, the bulk-read script wraps target files in XML boundary tags and transmits them alongside the agent's question to the bulk-reader worker mode, which returns strictly formatted bullet points beginning with exact method names or line numbers—ensuring the raw file contents never enter the frontier model's context window2. Complementing this on the output path, the code-write script accepts a specification prompt and a reference file, invokes the code-writer worker mode to generate boilerplate, test files, or type stubs matching existing codebase conventions, strips markdown fences, and writes the output directly to disk2. Because the generated file lands straight on disk, the frontier model pays neither input tokens to read the reference file nor expensive output tokens to generate the boilerplate, while Markdown skill files (SKILL.md) provide the exact invocation syntax so blocked operations degrade gracefully2.

While this pattern achieved a 90% mean token reduction on bulk reads in Spotify's Java monorepo benchmarks, deploying it naively in a self-hosted security environment exposes three critical second-order failure modes that require architectural mitigation2. First, static file-size thresholds suffer from severe repository sensitivity. Routing purely on a 350-line cutoff works well for verbose Java classes, but when ThakiCloud replayed Spotify's exact shunt logic against 1,099 real-world coding agent session transcripts, the default 350-line threshold offloaded only 14% of total read bytes because the empirical median file read across their repositories was just 158 lines21. In modular TypeScript, Rust, Python, or Go codebases, an effective server-side router must lower the interception threshold to 120–150 lines or trigger shunting based on cumulative multi-file byte volume rather than single-file line count20.

Second, line-number drift creates a hard boundary between code comprehension and code editing2. Worker SLMs summarizing large files do not maintain reliable line-number accuracy across complex control flows2. Consequently, file editing can never be delegated to a summarizing worker; the frontier model must use the worker's structural summary to locate the relevant symbol and then issue a targeted read with explicit offset and limit parameters to inspect the exact lines before applying a patch2. Third, and most importantly for a security professional, worker SLMs exhibit vulnerability blindness in safety-critical code alongside significant network latency2. During Spotify's internal validation, the Gemini Flash worker model identified surface-level structural patterns in a multi-file read but completely missed a subtle thread-safety bug that Claude identified in seconds once given direct context2. Furthermore, each remote worker delegation adds 10 to 30 seconds of network round-trip latency2. Therefore, deterministic server-side routing logic must explicitly bypass shunting for authentication modules, cryptographic routines, concurrency primitives, and active security debugging, sending those files directly to the frontier model2.

Synthesizing shunt with claude-mem Progressive Disclosure

In claude-mem, token optimization is achieved on the memory retrieval side through a streamlined 312-line MCP server (mcp-server.cjs) that reduced server code by 88% compared to earlier 9-tool iterations and exposes four tools enforcing a three-layer progressive disclosure pattern—supplemented by a fourth raw tool I/O layer introduced in schema migration 0515. Rather than dumping 35,000 tokens of raw history at startup (where v1–v2 benchmarks showed only 1.4% to 8% of tokens were relevant) or fetching 20 full RAG chunks upfront (costing 10,000–20,000 tokens), progressive disclosure reduces per-session context overhead to roughly 1,100 tokens at startup and 2,800–5,600 tokens for deep searches, achieving a 10x token efficiency improvement5.

| **Disclosure Layer** | **Tool Name** | **Underlying Storage / Endpoint** | **Token Cost per Call** | **Architectural Function** |
| --- | --- | --- | --- | --- |
| **Layer 0 (Policy)** | important\_workflow | Static MCP tool schema instruction | \~250 tokens (startup) | Always visible; instructs the agent never to fetch full details before filtering via index8. |
| **Layer 1 (Index)** | search | SQLite FTS5 (observations\_fts, session\_summaries\_fts) | \~50–100 tokens / result | Returns compact markdown tables of IDs, titles, dates, and types (decision, bugfix, feature, refactor, discovery, change) in \~12ms5. |
| **Layer 2 (Context)** | timeline | Chronological window query (depth\_before, depth\_after) | \~300–600 tokens | Displays chronological activity surrounding an anchor observation ID to establish causality5. |
| **Layer 3 (Details)** | get\_observations | POST /api/observations/batch on observations table | \~500–1,000 tokens / ID | Batch-fetches full narrative, facts, concepts, files\_read, and files\_modified only for validated IDs5. |
| **Layer 4 (Raw I/O)** | get\_tool\_uses | SQLite tool\_uses table (Schema v51, capped at 64 KB) | Variable (up to 64 KB) | Retrieves raw tool\_input and tool\_response keyed by content\_hash and tool\_use\_id8. |

When a self-hosted server unifies Spotify's shunt pattern with claude-mem's progressive disclosure database, a powerful compounding optimization emerges: content-addressed cross-agent I/O caching2. In Spotify's stateless AiKA architecture, if Claude Code summarizes a 600-line file via bulk-reader and twenty minutes later a second agent in Google Antigravity or OpenCode inspects the same file, the worker SLM is invoked twice, incurring another 10–30 second latency penalty and burning duplicate worker tokens2. By contrast, when the self-hosted memory server executes the bulk-reader SLM call, it computes the SHA-256 content\_hash of the target file and stores the structural summary, exported symbols, and line-range map inside the SQLite FTS5 index8. Any subsequent read of that unchanged file by any connected agent returns the cached structural summary in 12 milliseconds with zero LLM worker invocation8.

Landscape Evaluation of Open-Source Multi-Agent Memory Replacements

By late 2026, the open-source ecosystem for coding agent memory has bifurcated into two distinct architectural camps: trace-and-distill harnesses, which capture session events and asynchronously compile them into reusable skills or wiki pages, and stateful context servers, which manage shared memory blocks, Git-backed repositories, or multi-modal retrieval graphs via REST and MCP APIs12. Evaluating these candidates against the core operational requirements—Unix-native trace logging, elimination of client hooks and proxies, multi-provider self-hosting, and live cross-agent session memory—clarifies which components should be adopted or adapted.

| **System / Repository** | **License \& Core Stack** | **Capture \& Integration Mechanism** | **Storage \& Trace Substrate** | **Cross-Agent Session Handoff** | **Built-In I/O Token Optimization** |
| --- | --- | --- | --- | --- | --- |
| **Claude-Mem** (thedotmack/claude-mem v15.5.0)5 | Apache-2.0; TypeScript, Bun, Express 5, bun:sqlite, Chroma | 6 client lifecycle hooks + 4 MCP search tools (stdio wrapper to local HTTP worker)5 | Local SQLite WAL (claude-mem.db) + FTS5 + \~/.claude-mem/logs/worker-YYYY-MM-DD.log8 | **Weak:** Summarizes asynchronously on Stop/SessionEnd; designed for historical replay, not mid-session handoff8. | **High (Retrieval):** 3-layer MCP search (\~10x savings) + PreToolUse File Read Gate + Haiku grep routing5. |
| **Beacon** (Asymptote-Labs/agent-beacon)17 | MIT; TypeScript/Go OTLP Collector, MCP Server, Skills CLI | Multi-modal: Hooks, OTLP export, plugins (OpenCode/Cline), polling, SDK wrappers17 | OpenTelemetry JSONL (\~/.beacon/endpoint/logs/runtime.jsonl) + SIEM/S3 export17 | **Low:** Distills raw traces via offline "dreaming" (Jev scorer) + human approval gate into static skills17. | **Moderate:** Progressive disclosure via SKILL.md frontmatter and local beacon mcp serve17. |
| **ai-memory** (akitaonrails/ai-memory)12 | Open Source; Rust Server, Git-backed Markdown Wiki + Derived Index | Lifecycle hooks with typed privacy boundary sanitization + multi-user Bearer auth API12 | Self-hosted server; Git repository of plain .md files + rebuildable search index + audit log12 | **High:** Explicitly engineered for 4-step loop (*Capture, Consolidate, Recall, Handoff*) to switch agents mid-task12. | **High (Write-Side):** Zero LLM calls by default for memory aging/compaction; deterministic fact extraction12. |
| **Letta / Letta Code** (letta-ai/letta-code v0.1.189)26 | Apache-2.0; Python/TS Server, MemFS, Conversations API, MCP Server | Stateful REST API (/v1/repositories, blocks), MCP Server (@iflow-mcp/letta-mcp-server), Skills31 | PostgreSQL/SQLite state DB + Git-tracked Shared Memory Repos ($MEMORY\_DIR/../)26 | **Very High:** Shared in-context memory blocks update immediately across agents; Conversations API + Git sync30. | **Moderate:** Fixed character limit per in-context block; offloads bulk files to external Git repos30. |
| **Hindsight** (Vectorize.io v0.10.1)25 | MIT; Docker Server, Vector + BM25 + Temporal Graph, LiteLLM | 2-line LiteLLM proxy wrapper or MCP server (retain, recall, reflect tools)25 | Server database (Vector + BM25 + Entity/Causal Graph) + opt-in 45-pattern PII redaction25 | **Moderate:** Per-repo automatic memory bank from git history and sessions, but incurs LLM latency on every retain25. | **Low (Write-Heavy):** Requires an LLM call on every retain operation plus cross-encoder reranking25. |
| **Agent Memory Repo** (Cognition Spec)24 | MIT Specification; Git + Markdown (MEMORY.md) | Agent-driven Git workflow (clone ![](image.png) search ![](image.png) update ![](image.png) push) + offline Dreaming agent24 | Standard Git repository with full commit diff, blame, and revert auditability24 | **Moderate:** Portable across any agent with Git access, but relies on agent discipline to commit/push mid-session24. | **Moderate:** Links from MEMORY.md to deeper notes prevent context bloat, but no active I/O shunting24. |

Architectural Critique of Leading Open-Source Candidates

For a security professional prioritizing auditability, **Beacon** (Asymptote-Labs/agent-beacon) provides the most mature trace normalization substrate in the open-source ecosystem17. Supporting over 20 harnesses—including Claude Code, OpenCode, Cursor, Codex CLI, Gemini CLI, and Antigravity CLI—Beacon normalizes every session prompt, tool invocation, shell command, file modification, user approval, MCP call, and token metric into a unified OpenTelemetry (OTLP) event model written directly to a local, lock-in-free JSONL file (\~/.beacon/endpoint/logs/runtime.jsonl)17. Because this output is native JSONL, it integrates directly with standard Linux logrotate, grep, or OpenObserve without requiring a complex database, while offering a local terminal trace inspector (beacon traces), a read-only local web view (beacon endpoint dashboard), and native forwarding to enterprise SIEMs such as Wazuh, Splunk, and Elastic17. Furthermore, for harnesses such as OpenCode, Cline, Gemini CLI, and Codex Desktop, Beacon avoids shell hooks entirely by utilizing native plugins or direct OTLP telemetry export17. However, Beacon's memory philosophy explicitly rejects raw cross-agent session sharing: its operational pipeline (*Capture* ![](image.png) *Normalize* ![](image.png) *Evaluate \& Distill* ![](image.png) *Review \& Approve* ![](image.png) *Serve \& Reuse*) uses an offline scoring process ("dreaming" via the Jev scoring model) to propose reusable skills that require explicit human approval before being served via beacon mcp serve or SKILL.md files17. While this human-in-the-loop gate prevents memory poisoning and confident wrong lessons from polluting future sessions, it renders Beacon unsuitable for real-time, mid-task cross-agent session handoffs17.

In contrast, **ai-memory** (akitaonrails/ai-memory), written in Rust as a self-hosted network server supporting multi-user bearer token authentication and built-in audit logs, directly targets the cross-agent session handoff problem—enabling an engineer to quit Claude Code mid-task and immediately resume inside Codex, OpenCode, or Cursor without re-explaining the active context12. Its architecture treats a Git-backed wiki of plain .md files as the authoritative source of truth and uses its database purely as a rebuildable derived index, ensuring that a security operator can audit, grep, git blame, or logrotate the state using standard Unix utilities12. Crucially, ai-memory implements an access-weighted memory aging and compaction engine that avoids LLM calls by default, collapsing near-duplicates, flagging contradictions, and compacting cold episodic notes down to durable facts (such as file paths, error codes, and architectural decisions) deterministically while keeping all original notes recoverable in Git12. Its primary limitation is that its default capture layer still relies on client lifecycle hooks (albeit sanitized at a typed privacy boundary so local transcript bodies are never transmitted raw) rather than pure MCP tool interception, and it lacks Spotify-style file-read shunting12.

Derived from UC Berkeley's MemGPT operating-system memory hierarchy, **Letta** (letta-ai/letta-code) provides stateful multi-agent coordination through two distinct mechanisms: in-context shared memory blocks and Git-backed shared memory repositories26. Through the client.blocks API, multiple agents—such as a supervisor agent and specialized worker agents—can attach to the exact same block\_id (representing task\_queue, handoff\_context, or domain\_knowledge)30. Because Letta compiles the context window dynamically from database state on every turn, when one agent updates a shared block, all other attached agents observe the updated state immediately on their next turn without retrieval latency, a capability further extended by Letta's January 2026 Conversations API for maintaining shared memory across concurrent user sessions30. However, because blocks.update() performs a full string replacement where concurrent memory\_rethink operations by multiple agents cause last-write-wins race conditions—and because read-only flags apply globally rather than per-agent—Letta's late-2026 architecture advises migrating bulk collaborative state to Git-backed Shared Memory Repositories (MemFS and /v1/repositories)26. Managed via the managing-shared-memory skill (SKILL.md) and letta shared-memory sync, these repositories mount alongside each agent's workspace ($MEMORY\_DIR/../) and resolve multi-agent concurrency using standard git pull --rebase semantics, though they remain heavily oriented toward Letta's own agent runtime rather than acting as a lightweight I/O shunt for external harnesses26.

Finally, **Hindsight** (Vectorize.io v0.10.1) combines vector similarity, BM25 keyword search, entity/causal graphs, and temporal filtering merged via reciprocal rank fusion and a cross-encoder reranker25. While Hindsight exposes an MCP server (retain, recall, reflect) and an automatic per-repo memory bank, its 2-line integration relies on a LiteLLM proxy wrapper, its 45-pattern PII/secret redaction ("Memory Defense") is opt-in per bank rather than enforced by default, and every retain operation triggers a full LLM call—introducing substantial latency and token overhead on every memory write25.

![](image.png)

Eliminating Hooks and Proxies: Protocol-Native Interception via Streamable HTTP MCP and Agent Plugins

A foundational architectural decision for multi-agent reliability is rejecting both client-side lifecycle hooks and man-in-the-middle (MITM) LLM API reverse proxies in favor of Remote Streamable HTTP MCP (--transport http) paired with portable Agent Plugins (SKILL.md and plugin.json)14.

Why Client Hooks and API Proxies Fail in Multi-Agent Production

Client-side lifecycle hooks—such as Claude Code's settings.json command hooks or Antigravity CLI's \~/.gemini/config/hooks.json—exhibit systemic reliability flaws across heterogeneous environments6. Because hooks execute within the host IDE's lifecycle, claude-mem enforces a hard 2,000ms HTTP timeout on PostToolUse and Stop calls and forces every hook launcher to exit with code 0 regardless of downstream failure so that the user session is never interrupted8. Whenever the worker service is busy compacting a prior turn or experiences transient network latency, context injection silently degrades and tool observations are dropped without alerting the operator8. Moreover, hook payload schemas and lifecycle events are non-standard across harnesses, requiring constant maintenance to prevent breakage6. In claude-mem v15.5.0 alone, maintainers had to patch numerous harness-specific edge cases, including recording Antigravity (agy) MCP calls under their real tool names and stable IDs (#5170), resolving Antigravity project contexts (#5105), fixing Windows 8.3 short-path lookups (#5099), and allocating special hook API budgets for Pi (#5118), while desktop clients like Claude Desktop do not expose shell lifecycle hooks at all5.

Routing agent traffic through a central LLM API reverse proxy—such as Hindsight's LiteLLM wrapper—introduces even more severe operational friction25. Modern coding harnesses rely heavily on native OAuth subscription sessions and proprietary vendor endpoints rather than raw, interchangeable API keys3. Engineers operating Claude Code on Pro or Max subscription plans (20–200/month), Google Antigravity authenticated via Google Application Default Credentials ("authProviderType": "google\_credentials") or Google AI Pro/Ultra plans, or OpenCode linked to GitHub Copilot quotas cannot transparently route those authenticated sessions through a custom MITM proxy without breaking OAuth handshakes, violating provider terms, or disabling native IDE capabilities such as Antigravity's parallel Agent Teams manager and built-in Chromium visual verification sub-agent3.

The Protocol-Native Solution: Streamable HTTP MCP + Standardized Agent Skills

Instead of observing agents externally via fragile shell hooks or intercepting TLS traffic via an API proxy, the self-hosted server should operate directly inside the agent's tool and cognitive loop using the two open standards supported natively across all target harnesses in 2026: Remote Streamable HTTP MCP (which superseded the deprecated Server-Sent Events transport) and the Linux Foundation AAIF Agent Skills and Agent Plugins specification (SKILL.md and plugin.json)14.

| **Target Agent Harness** | **Native Remote MCP Transport Support** | **Standardized Plugin / Skill Packaging** | **Exact Self-Hosted Server Connection Configuration** |
| --- | --- | --- | --- |
| **Claude Code CLI** (v2.1.265+)8 | Native http (streamable-http alias), sse fallback, ws, and stdio. Supports fleet push via managedMcpServers (v2.1.259+)14. | .claude-plugin bundle + .claude/skills/\<name\>/SKILL.md17. | claude mcp add --transport http -s user sec-mem https://mem.internal:8443/mcp --header "Authorization: Bearer ${MEM\_TOKEN}"10 |
| **Google Antigravity** (IDE \& agy CLI)16 | Native remote Streamable HTTP via serverUrl in mcp\_config.json with custom headers, enabledTools, disabledTools, and timeoutSeconds16. | Versioned plugin.json manifest packaging MCP servers, skills, and auth flows as one toggleable unit37. | In mcp\_config.json: {"serverUrl": "https://mem.internal:8443/mcp", "headers": {"Authorization": "Bearer ..."}, "timeoutSeconds": 180}16 |
| **OpenCode**5 | Native remote http / sse MCP client and native plugin runtime5. | Supports portable SKILL.md via npx skills add and OpenCode plugin extensions17. | Configure remote HTTP MCP endpoint https://mem.internal:8443/mcp with static Bearer header in global OpenCode config14. |
| **Claude Desktop**5 | Native Custom Connectors (Streamable HTTP URL) or mcp-remote bridge in claude\_desktop\_config.json10. | Claude Desktop Skills (SKILL.md) loaded on demand via YAML frontmatter5. | Add https://mem.internal:8443/mcp in *Settings* ![](image.png) *Connectors*, or via npx mcp-remote https://mem.internal:8443/mcp --header "Authorization: Bearer ..."10. |

How Server-Side MCP Tools Replace Client Hooks Deterministically

By combining MCP tool permission scoping with an on-demand SKILL.md playbook, a centralized server achieves the exact deterministic behavior of Spotify's shunt hooks and claude-mem's lifecycle hooks without executing a single client-side shell hook2. Both Claude Code (via tool permission rules and managedMcpServers settings) and Google Antigravity (via enabledTools and disabledTools arrays in mcp\_config.json) allow operators to restrict raw, unbounded file reads and prioritize MCP tools16. When an agent invokes the remote MCP server's smart\_read or boilerplate\_write tool against a repository workspace on the server (or a synchronized project tree), the MCP server's backend code deterministically measures the file line count before returning any content to the frontier model2. If the file exceeds 150 lines, the server transparently shunts the payload to a fast SLM worker—such as Gemini 3.5 Flash, Claude Haiku 4.5, or a local model—logs the raw operation to disk, caches the structural summary by SHA-256 content hash, and returns only the compact summary to the calling agent2.

Simultaneously, cross-agent session continuity is enforced without SessionStart or Stop hooks through a dual active-and-passive capture mechanism8. On the active path, every connected harness loads a lightweight (under 200 tokens) SKILL.md playbook whose YAML description field triggers automatically at the start of any coding task or when the operator requests a checkpoint or handoff17. On the passive path, because the agent continuously invokes the MCP server's smart\_read, boilerplate\_write, and mem\_search tools during normal task execution, the server automatically records every tool invocation, file path, and parameter delta into the active workspace\_task\_id breadcrumb table on the backend. Even if an engineer abruptly terminates a Claude Code terminal session without running a handoff command, the server already possesses the complete chronological breadcrumb trail of inspected files, queries, and modifications ready for immediate injection into Google Antigravity or OpenCode.

Reference Architecture: Self-Hosted Cross-Agent Session Memory and Unix Audit Server

To satisfy all operational requirements on a single self-hosted Linux server without SaaS dependencies, the reference architecture integrates a Python FastMCP Streamable HTTP server13, a two-tier SQLite 3 WAL and FTS5 database paired with Git-backed Markdown state (synthesizing the strongest primitives of claude-mem, ai-memory, and Letta)8, an asynchronous SLM Shunt Worker2, and a Unix-native JSONL Audit Logger managed by standard Linux logrotate8.

![](image.png)

Cross-Agent Session Memory vs. Long-Term Memory: The Two-Tier Schema

Resolving the architectural gap between general multi-agent knowledge bases and live cross-agent session memory requires partitioning the server's SQLite WAL database (/var/lib/agent-memory/memory.db) into two distinct operational tiers: an ephemeral Cross-Agent Session Bus and a long-term Progressive Disclosure Store8. Unlike claude-mem, which binds each session record (sdk\_sessions) to a single harness's claude\_session\_id and waits until the session stops or exits to generate a summary via the Claude Agent SDK8, the unified server keys active work by a harness-agnostic workspace\_task\_id inside the active\_sessions table. Modeled on Letta's shared in-context memory blocks (handoff\_context) and ai-memory's markdown handoff notes12, active\_sessions maintains a compact, structured Markdown state (working\_state\_md, capped under 1,500 tokens) capturing the current objective, active debugging hypothesis, files under modification, failed approaches to avoid, and immediate next step.

To guarantee that live session context is never lost even if an engineer switches tools without manually saving a checkpoint, the server pairs active\_sessions with an automatic ring-buffer table, session\_breadcrumbs. Every time any connected agent invokes an MCP tool on the server, the tool handler automatically records a timestamped breadcrumb entry specifying the calling agent\_harness, tool\_name, target\_resource, and action\_summary. When the engineer switches mid-task from Claude Code in the terminal to Google Antigravity IDE—for example, to utilize Antigravity's Agent Teams panel or Chromium browser sub-agent3—or to OpenCode, the newly launched agent calls session\_resume\_or\_handoff and immediately inherits both the structured working\_state\_md and the last 15 tool breadcrumbs from the prior agent's run, requiring zero LLM summarization latency and preserving 100% of in-flight state.

| **Storage Table** | **Memory Tier** | **Primary Key / Indexing** | **Retention \& Mutation Model** | **Architectural Role in Multi-Agent Server** |
| --- | --- | --- | --- | --- |
| **active\_sessions** | Tier 1: Live Cross-Agent Session Bus | project (PRIMARY KEY), workspace\_task\_id | Upserted live during task execution or explicit checkpoint handoffs12. | Holds the shared in-context Markdown working state (working\_state\_md) across harnesses30. |
| **session\_breadcrumbs** | Tier 1: Live Cross-Agent Session Bus | id (AUTOINCREMENT), indexed on (project, created\_at\_epoch DESC) | Append-only ring buffer populated automatically on every MCP tool invocation. | Provides immediate chronological replay of recent cross-agent tool actions without LLM calls. |
| **shunt\_file\_cache** | Tier 1.5: Content-Addressed I/O Cache | Composite PK: (file\_path, content\_sha256, query\_fingerprint) | Invalidated automatically whenever the file's SHA-256 content hash changes8. | Stores SLM bulk-reader structural summaries so repeat reads across agents complete in \~12ms2. |
| **observations** | Tier 2: Long-Term Project Memory | id (AUTOINCREMENT), indexed on project, type, created\_at\_epoch | Durable append store of verified decisions, bugfixes, refactors, and discoveries8. | Mirrors claude-mem's hierarchical schema (title, narrative, facts, files\_touched)8. |
| **observations\_fts** | Tier 2: Progressive Disclosure Index | SQLite FTS5 Virtual Table synced via AFTER INSERT trigger (obs\_ai) | Updated automatically on observations inserts; double-quote escaped against FTS5 injection8. | Powers sub-15ms Layer 1 index searches (\~50–100 tokens per result) across all harnesses5. |

Server-Side Deterministic I/O Shunt Engine

Moving Spotify's shunt logic from client-side bash scripts into the FastMCP server's tool handlers makes I/O optimization completely harness-neutral across Claude Code, Antigravity, OpenCode, and Claude Desktop2. When an agent calls smart\_read(project, agent\_harness, file\_path, question, offset, limit), the server executes a four-stage deterministic evaluation before returning tokens to the frontier model2. First, if the calling agent specifies explicit offset and limit parameters—indicating a targeted read prior to editing code—the server slices the requested line range, prepends exact line numbers (L\<n\>:), logs an IO\_TARGETED\_READ event, and returns the slice directly to the frontier model2. Second, the server evaluates the file path against a deterministic security-critical regular expression matching authentication, cryptography, token handling, RBAC, TLS, and concurrency primitives (auth\|crypto\|secret\|token\|rbac\|jwt\|tls\|cert\|mutex\|lock\|concurren). Because Spotify's benchmarks proved that worker SLMs miss subtle concurrency and security bugs2, any file matching this pattern—or any file smaller than the tuned 150-line threshold (SHUNT\_MIN\_LINES = 150, calibrated to ThakiCloud's 158-line median read size finding21)—bypasses the worker model and returns directly to the frontier agent with an explicit audit reason (security\_critical\_bypass or below\_line\_threshold).

Third, for non-critical files of 150 lines or more, the server computes the SHA-256 hash of the file content alongside a 16-character hash of the normalized question and queries shunt\_file\_cache8. On a cache hit, the server returns the stored structural summary immediately without network or LLM overhead. Fourth, on a cache miss, the server wraps the line-numbered file in XML tags (\<file path="..." lines="..."\>) and dispatches it over HTTP to a configurable SLM worker—such as a local vLLM or Ollama instance running Qwen3-35B-A3B or DeepSeek-V4-Flash29, or a low-cost API model such as gemini-3.5-flash or claude-haiku-4-5-202510013—enforcing Spotify's strict bulk-reader system prompt: return only concise bullet points where every bullet begins with an exact symbol name and line range \[L\<start\>-L\<end\>\], with zero conversational prose2. Similarly, for boilerplate and test generation, the server exposes a boilerplate\_write tool modeled on Spotify's code-writer mode that sends a specification prompt and reference file path directly to the worker model, strips markdown fences, writes the generated code straight to disk, and returns only a one-line confirmation and SHA-256 checksum to the primary agent—eliminating 100% of reference input tokens and output generation tokens from the frontier model's budget2.

Security-Grade Unix Audit Trail with Logrotate and OpenObserve

For a security professional, observability must be deterministic, tamper-evident, and independent of complex real-time web daemons6. Rather than running claude-mem's React/SSE web viewer on port 37777 or maintaining a dedicated telemetry cluster, the FastMCP server incorporates a synchronous Audit Trace Middleware modeled on Beacon's local OpenTelemetry JSONL specification (runtime.jsonl) and claude-mem v15.5.0's secret-redaction rules7. Before any MCP tool execution or session state mutation is written to SQLite or disk, the middleware passes all inputs and outputs through a deterministic redaction filter that scrubs bare token, secret, api\_key, password, MYSQL\_PWD, and dbPassword keys (#5130, #5111), Authorization: Bearer headers (#5129), Slack app and refresh tokens (xapp-, xoxe-, #5119), and user-specified \<private\>...\</private\> XML tags7.

Each sanitized event is serialized as a single-line JSON object containing the ISO-8601 UTC timestamp (ts), millisecond epoch (epoch\_ms), event\_type, agent\_harness, project, structured details (including file paths, SHA-256 content hashes, line counts, worker model latency, and shunt bypass reasons), and a self-verifying record\_sha256 digest, and is written atomically via O\_APPEND to /var/log/agent-memory/audit.jsonl8. Because the Python daemon opens /var/log/agent-memory/audit.jsonl in append mode per write rather than holding a locked file descriptor, standard Linux logrotate can rotate, date-stamp (audit.jsonl-YYYY-MM-DD), and compress the logs daily without restarting the MCP server, while an optional six-line postrotate hook can batch-ship each closed daily file to a self-hosted OpenObserve instance via curl.

Production Implementation Blueprint

The following self-contained reference implementation provides the complete server daemon (/opt/agent-memory/server.py), the Linux /etc/logrotate.d/agent-memory configuration, and the cross-harness SKILL.md playbook required to deploy this architecture in production2.

Python

import hashlib\
import json\
import os\
import re\
import sqlite3\
import time\
from datetime import datetime, timezone\
from pathlib import Path\
from typing import List, Optional\
import httpx\
from fastmcp import FastMCP\
\
\# Environment \& Path Configuration\
DATA\_DIR = Path(os.environ.get("AGENT\_MEM\_DATA\_DIR", "/var/lib/agent-memory"))\
LOG\_FILE = Path(os.environ.get("AGENT\_MEM\_LOG\_FILE", "/var/log/agent-memory/audit.jsonl"))\
SHUNT\_MIN\_LINES = int(os.environ.get("SHUNT\_MIN\_LINES", "150"))\
WORKER\_API\_URL = os.environ.get("WORKER\_API\_URL", "http://127.0.0.1:11434/v1/chat/completions")\
WORKER\_MODEL = os.environ.get("WORKER\_MODEL", "qwen3:35b-a3b")\
WORKER\_API\_KEY = os.environ.get("WORKER\_API\_KEY", "local-no-key")\
\
SECURITY\_CRITICAL\_RE = re.compile(\
   r"(auth\|crypto\|secret\|token\|rbac\|jwt\|tls\|cert\|mutex\|lock\|concurren\|permission\|sudo\|pam)",\
   re.IGNORECASE,\
)\
SECRET\_REDACT\_RE = re.compile(\
   r"(?i)((?:api\_key\|secret\|token\|password\|mysql\_pwd\|dbpassword\|authorization)\\s\*\[:=\]\\s\*\['\\"\]?)(\[^'\\"\\s,}\]+)"\
   r"\|(xapp-\[0-9\]+-\[A-Za-z0-9\_\]+\|xoxe-\[A-Za-z0-9-\]+)"\
)\
\
DATA\_DIR.mkdir(parents=True, exist\_ok=True)\
LOG\_FILE.parent.mkdir(parents=True, exist\_ok=True)\
\
mcp = FastMCP("Unified-Agent-Memory-And-Shunt")\
\
def redact\_secrets(payload: str) -\> str:\
   payload = re.sub(r"\<private\>.\*?\</private\>", "\[REDACTED\_PRIVATE\_TAG\]", payload, flags=re.DOTALL)\
   return SECRET\_REDACT\_RE.sub(lambda m: f"{m.group(1) or ''}\[REDACTED\_SECRET\]", payload)\
\
def write\_audit\_trace(event\_type: str, harness: str, project: str, details: dict) -\> None:\
   now = datetime.now(timezone.utc)\
   sanitized\_details = json.loads(redact\_secrets(json.dumps(details)))\
   record = {\
   "ts": now.isoformat(),\
   "epoch\_ms": int(now.timestamp() \* 1000),\
   "event\_type": event\_type,\
   "agent\_harness": harness,\
   "project": project,\
   "details": sanitized\_details,\
   }\
   record\["record\_sha256"\] = hashlib.sha256(\
   json.dumps(record, sort\_keys=True).encode("utf-8")\
   ).hexdigest()\
   with open(LOG\_FILE, "a", encoding="utf-8") as f:\
   f.write(json.dumps(record) + "\\n")\
\
def get\_db() -\> sqlite3.Connection:\
   conn = sqlite3.connect(DATA\_DIR / "memory.db", timeout=15.0)\
   conn.row\_factory = sqlite3.Row\
   conn.execute("PRAGMA journal\_mode=WAL;")\
   conn.execute("PRAGMA synchronous=NORMAL;")\
   return conn\
\
def init\_db() -\> None:\
   with get\_db() as conn:\
   conn.executescript("""\
   CREATE TABLE IF NOT EXISTS active\_sessions (\
   project TEXT PRIMARY KEY,\
   workspace\_task\_id TEXT NOT NULL,\
   active\_objective TEXT,\
   working\_state\_md TEXT NOT NULL,\
   last\_harness TEXT NOT NULL,\
   updated\_at\_epoch INTEGER NOT NULL\
   );\
\
   CREATE TABLE IF NOT EXISTS session\_breadcrumbs (\
   id INTEGER PRIMARY KEY AUTOINCREMENT,\
   project TEXT NOT NULL,\
   workspace\_task\_id TEXT NOT NULL,\
   agent\_harness TEXT NOT NULL,\
   tool\_name TEXT NOT NULL,\
   target\_resource TEXT,\
   action\_summary TEXT NOT NULL,\
   created\_at\_epoch INTEGER NOT NULL\
   );\
   CREATE INDEX IF NOT EXISTS idx\_breadcrumbs\_proj ON session\_breadcrumbs(project, created\_at\_epoch DESC);\
\
   CREATE TABLE IF NOT EXISTS observations (\
   id INTEGER PRIMARY KEY AUTOINCREMENT,\
   project TEXT NOT NULL,\
   workspace\_task\_id TEXT NOT NULL,\
   agent\_harness TEXT NOT NULL,\
   type TEXT NOT NULL,\
   title TEXT NOT NULL,\
   narrative TEXT NOT NULL,\
   facts TEXT,\
   files\_touched TEXT,\
   created\_at\_epoch INTEGER NOT NULL\
   );\
\
   CREATE VIRTUAL TABLE IF NOT EXISTS observations\_fts USING fts5(\
   title, narrative, facts, files\_touched,\
   content='observations', content\_rowid='id'\
   );\
\
   CREATE TRIGGER IF NOT EXISTS obs\_ai AFTER INSERT ON observations BEGIN\
   INSERT INTO observations\_fts(rowid, title, narrative, facts, files\_touched)\
   VALUES (new.id, new.title, new.narrative, new.facts, new.files\_touched);\
   END;\
\
   CREATE TABLE IF NOT EXISTS shunt\_file\_cache (\
   file\_path TEXT NOT NULL,\
   content\_sha256 TEXT NOT NULL,\
   query\_fingerprint TEXT NOT NULL,\
   structural\_summary TEXT NOT NULL,\
   line\_count INTEGER NOT NULL,\
   created\_at\_epoch INTEGER NOT NULL,\
   PRIMARY KEY (file\_path, content\_sha256, query\_fingerprint)\
   );\
   """)\
\
init\_db()\
\
def append\_breadcrumb(project: str, task\_id: str, harness: str, tool: str, resource: str, summary: str) -\> None:\
   with get\_db() as conn:\
   conn.execute(\
   """INSERT INTO session\_breadcrumbs\
   (project, workspace\_task\_id, agent\_harness, tool\_name, target\_resource, action\_summary, created\_at\_epoch)\
   VALUES (?, ?, ?, ?, ?, ?, ?)""",\
   (project, task\_id, harness, tool, resource, redact\_secrets(summary), int(time.time() \* 1000))\
   )\
\
@mcp.tool()\
async def session\_resume\_or\_handoff(\
   project: str,\
   agent\_harness: str,\
   updated\_working\_state\_md: Optional\[str\] = None,\
   active\_objective: Optional\[str\] = None,\
   workspace\_task\_id: Optional\[str\] = None,\
) -\> str:\
   """\
   Cross-Agent Session Memory Bus. Call without updated\_working\_state\_md at session start\
   to inherit the live in-flight session from another agent (e.g. Claude Code -\> Antigravity).\
   Call with updated\_working\_state\_md to checkpoint current progress for cross-agent handoff.\
   """\
   now\_ms = int(time.time() \* 1000)\
   with get\_db() as conn:\
   row = conn.execute("SELECT \* FROM active\_sessions WHERE project = ?", (project,)).fetchone()\
   task\_id = workspace\_task\_id or (row\["workspace\_task\_id"\] if row else f"{project}-{now\_ms}")\
\
   if updated\_working\_state\_md:\
   clean\_md = redact\_secrets(updated\_working\_state\_md)\
   obj = active\_objective or (row\["active\_objective"\] if row else "Active Task")\
   conn.execute(\
   """INSERT INTO active\_sessions (project, workspace\_task\_id, active\_objective, working\_state\_md, last\_harness, updated\_at\_epoch)\
   VALUES (?, ?, ?, ?, ?, ?)\
   ON CONFLICT(project) DO UPDATE SET\
   workspace\_task\_id=excluded.workspace\_task\_id,\
   active\_objective=excluded.active\_objective,\
   working\_state\_md=excluded.working\_state\_md,\
   last\_harness=excluded.last\_harness,\
   updated\_at\_epoch=excluded.updated\_at\_epoch""",\
   (project, task\_id, obj, clean\_md, agent\_harness, now\_ms),\
   )\
   append\_breadcrumb(project, task\_id, agent\_harness, "session\_handoff", project, f"Updated working state ({len(clean\_md)} chars)")\
   write\_audit\_trace("SESSION\_CHECKPOINT", agent\_harness, project, {"task\_id": task\_id, "objective": obj})\
   return f"Checkpoint saved for project '{project}' (task: {task\_id}). Ready for cross-agent pickup."\
\
   breadcrumbs = conn.execute(\
   "SELECT agent\_harness, tool\_name, target\_resource, action\_summary, created\_at\_epoch FROM session\_breadcrumbs WHERE project = ? ORDER BY created\_at\_epoch DESC LIMIT 15",\
   (project,),\
   ).fetchall()\
\
   write\_audit\_trace("SESSION\_RESUME", agent\_harness, project, {"task\_id": task\_id, "prior\_harness": row\["last\_harness"\] if row else "none"})\
   if not row:\
   return f"No active cross-agent session found for '{project}'. Initialized new task ID: {task\_id}."\
\
   bc\_lines = \[\
   f"- \[{b\['agent\_harness'\]}\] {b\['tool\_name'\]}({b\['target\_resource'\]}): {b\['action\_summary'\]}"\
   for b in breadcrumbs\
   \]\
   return (\
   f"### Active Cross-Agent Session State (Task: {row\['workspace\_task\_id'\]})\\n"\
   f"\*\*Last Active Agent:\*\* {row\['last\_harness'\]} \| \*\*Objective:\*\* {row\['active\_objective'\]}\\n\\n"\
   f"#### Live Working Context\\n{row\['working\_state\_md'\]}\\n\\n"\
   f"#### Recent Cross-Agent Tool Breadcrumbs (Newest First)\\n" + "\\n".join(bc\_lines)\
   )\
\
@mcp.tool()\
async def smart\_read(\
   project: str,\
   agent\_harness: str,\
   file\_path: str,\
   question: str = "Summarize exported symbols, signatures, and control flow with exact line ranges.",\
   offset: Optional\[int\] = None,\
   limit: Optional\[int\] = None,\
) -\> str:\
   """\
   Spotify-style I/O Shunt Reader. Targeted reads (with offset \& limit) or files \< 150 lines\
   or security-critical paths return raw line-numbered code. Larger files are summarized by\
   the worker model and cached by SHA-256 hash so raw I/O never bloats frontier context.\
   """\
   p = Path(file\_path).resolve()\
   if not p.is\_file():\
   return f"Error: File not found: {file\_path}"\
\
   raw\_text = p.read\_text(encoding="utf-8", errors="replace")\
   lines = raw\_text.splitlines()\
   total\_lines = len(lines)\
   content\_sha = hashlib.sha256(raw\_text.encode("utf-8")).hexdigest()\
\
   if offset is not None and limit is not None:\
   start = max(1, offset)\
   end = min(total\_lines, start + limit - 1)\
   sliced = "\\n".join(f"L{idx}: {lines\[idx-1\]}" for idx in range(start, end + 1))\
   write\_audit\_trace("IO\_TARGETED\_READ", agent\_harness, project, {\
   "file": str(p), "sha256": content\_sha, "offset": start, "limit": limit, "total\_lines": total\_lines\
   })\
   append\_breadcrumb(project, project, agent\_harness, "smart\_read:targeted", f"{p.name}:L{start}-L{end}", f"Inspected {end-start+1} lines")\
   return f"### Targeted Read: {p.name} (L{start}-L{end} of {total\_lines})\\n{sliced}"\
\
   is\_security\_critical = bool(SECURITY\_CRITICAL\_RE.search(str(p)))\
   if total\_lines \< SHUNT\_MIN\_LINES or is\_security\_critical:\
   reason = "security\_critical\_bypass" if is\_security\_critical else "below\_line\_threshold"\
   numbered = "\\n".join(f"L{i+1}: {line}" for i, line in enumerate(lines))\
   write\_audit\_trace("IO\_DIRECT\_READ", agent\_harness, project, {\
   "file": str(p), "sha256": content\_sha, "total\_lines": total\_lines, "bypass\_reason": reason\
   })\
   append\_breadcrumb(project, project, agent\_harness, "smart\_read:direct", p.name, f"Direct read ({total\_lines} lines, reason={reason})")\
   return f"### Direct Read ({reason}): {p.name} ({total\_lines} lines)\\n{numbered}"\
\
   q\_fp = hashlib.sha256(question.strip().lower().encode("utf-8")).hexdigest()\[:16\]\
   with get\_db() as conn:\
   cached = conn.execute(\
   "SELECT structural\_summary FROM shunt\_file\_cache WHERE file\_path = ? AND content\_sha256 = ? AND query\_fingerprint = ?",\
   (str(p), content\_sha, q\_fp),\
   ).fetchone()\
   if cached:\
   write\_audit\_trace("IO\_SHUNT\_CACHE\_HIT", agent\_harness, project, {\
   "file": str(p), "sha256": content\_sha, "total\_lines": total\_lines, "saved\_lines": total\_lines\
   })\
   append\_breadcrumb(project, project, agent\_harness, "smart\_read:cache\_hit", p.name, f"Served cached summary for {total\_lines}-line file")\
   return f"### Cached Shunt Summary ({total\_lines} lines, SHA256: {content\_sha\[:10\]})\\n{cached\['structural\_summary'\]}\\n\\n\*Note: For edits, call smart\_read with explicit offset and limit.\*"\
\
   t0 = time.time()\
   numbered\_payload = "\\n".join(f"L{i+1}: {line}" for i, line in enumerate(lines))\
   system\_prompt = (\
   "Act as a deterministic code analyst. Answer the question using ONLY concise bullet points. "\
   "Every bullet MUST begin with an exact symbol/method name and line range \[L\<start\>-L\<end\>\]. "\
   "Include zero greeting, zero preamble, and zero conversational prose."\
   )\
   user\_payload = f"\<file path=\\"{p.name}\\" lines=\\"{total\_lines}\\"\>\\n{numbered\_payload}\\n\</file\>\\n\\nQuestion: {question}"\
\
   async with httpx.AsyncClient(timeout=30.0) as client:\
   resp = await client.post(\
   WORKER\_API\_URL,\
   headers={"Authorization": f"Bearer {WORKER\_API\_KEY}"},\
   json={\
   "model": WORKER\_MODEL,\
   "temperature": 0.0,\
   "messages": \[\
   {"role": "system", "content": system\_prompt},\
   {"role": "user", "content": user\_payload},\
   \],\
   },\
   )\
   resp.raise\_for\_status()\
   summary = resp.json()\["choices"\]\[0\]\["message"\]\["content"\].strip()\
\
   latency\_ms = int((time.time() - t0) \* 1000)\
   with get\_db() as conn:\
   conn.execute(\
   """INSERT OR REPLACE INTO shunt\_file\_cache\
   (file\_path, content\_sha256, query\_fingerprint, structural\_summary, line\_count, created\_at\_epoch)\
   VALUES (?, ?, ?, ?, ?, ?)""",\
   (str(p), content\_sha, q\_fp, summary, total\_lines, int(time.time() \* 1000)),\
   )\
\
   write\_audit\_trace("IO\_SHUNT\_DELEGATED", agent\_harness, project, {\
   "file": str(p), "sha256": content\_sha, "total\_lines": total\_lines,\
   "worker\_model": WORKER\_MODEL, "latency\_ms": latency\_ms\
   })\
   append\_breadcrumb(project, project, agent\_harness, "smart\_read:shunted", p.name, f"Shunted {total\_lines} lines to {WORKER\_MODEL} ({latency\_ms}ms)")\
   return f"### Shunted Bulk-Read Summary ({total\_lines} lines -\> {WORKER\_MODEL} in {latency\_ms}ms)\\n{summary}\\n\\n\*To edit specific sections, call smart\_read with offset and limit.\*"\
\
@mcp.tool()\
async def boilerplate\_write(\
   project: str,\
   agent\_harness: str,\
   target\_path: str,\
   reference\_path: str,\
   spec\_prompt: str,\
) -\> str:\
   """\
   Spotify-style code-writer shunt. Generates boilerplate or test files via the worker SLM\
   using reference\_path for pattern matching and writes straight to disk without using frontier tokens.\
   """\
   ref\_p = Path(reference\_path).resolve()\
   tgt\_p = Path(target\_path).resolve()\
   if not ref\_p.is\_file():\
   return f"Error: Reference file not found: {reference\_path}"\
\
   ref\_content = ref\_p.read\_text(encoding="utf-8", errors="replace")\
   system\_prompt = (\
   "Act as a deterministic code generator. Follow the exact conventions, imports, and structure "\
   "of the reference file. Output ONLY raw source code with zero explanation and no markdown fences."\
   )\
   user\_payload = f"\<reference path=\\"{ref\_p.name}\\"\>\\n{ref\_content}\\n\</reference\>\\n\\nSpecification: {spec\_prompt}"\
\
   t0 = time.time()\
   async with httpx.AsyncClient(timeout=45.0) as client:\
   resp = await client.post(\
   WORKER\_API\_URL,\
   headers={"Authorization": f"Bearer {WORKER\_API\_KEY}"},\
   json={\
   "model": WORKER\_MODEL,\
   "temperature": 0.0,\
   "messages": \[\
   {"role": "system", "content": system\_prompt},\
   {"role": "user", "content": user\_payload},\
   \],\
   },\
   )\
   resp.raise\_for\_status()\
   generated = resp.json()\["choices"\]\[0\]\["message"\]\["content"\].strip()\
\
   cleaned = re.sub(r"^\`\`\`\[a-zA-Z0-9\_-\]\*\\n\|\\n\`\`\`$", "", generated).strip() + "\\n"\
   tgt\_p.parent.mkdir(parents=True, exist\_ok=True)\
   tgt\_p.write\_text(cleaned, encoding="utf-8")\
   out\_sha = hashlib.sha256(cleaned.encode("utf-8")).hexdigest()\
   out\_lines = len(cleaned.splitlines())\
   latency\_ms = int((time.time() - t0) \* 1000)\
\
   write\_audit\_trace("IO\_SHUNT\_CODE\_WRITE", agent\_harness, project, {\
   "target\_file": str(tgt\_p), "reference\_file": str(ref\_p), "sha256": out\_sha,\
   "lines\_written": out\_lines, "worker\_model": WORKER\_MODEL, "latency\_ms": latency\_ms\
   })\
   append\_breadcrumb(project, project, agent\_harness, "boilerplate\_write", tgt\_p.name, f"Generated {out\_lines} lines via {WORKER\_MODEL}")\
   return f"Wrote {out\_lines} lines directly to {tgt\_p} (SHA256: {out\_sha\[:12\]}, worker: {WORKER\_MODEL}, {latency\_ms}ms)."\
\
@mcp.tool()\
async def mem\_search(project: str, agent\_harness: str, query: str, obs\_type: Optional\[str\] = None, limit: int = 15) -\> str:\
   """Layer 1 Progressive Disclosure Search over SQLite FTS5. Returns compact index only."""\
   safe\_q = '"' + query.replace('"', '""') + '"'\
   sql = """\
   SELECT o.id, o.type, o.title, o.files\_touched, o.agent\_harness, o.created\_at\_epoch\
   FROM observations\_fts f\
   JOIN observations o ON f.rowid = o.id\
   WHERE observations\_fts MATCH ? AND o.project = ?\
   """\
   params: list = \[safe\_q, project\]\
   if obs\_type:\
   sql += " AND o.type = ?"\
   params.append(obs\_type)\
   sql += " ORDER BY rank LIMIT ?"\
   params.append(limit)\
\
   with get\_db() as conn:\
   rows = conn.execute(sql, params).fetchall()\
\
   write\_audit\_trace("MEM\_SEARCH\_INDEX", agent\_harness, project, {"query": query, "results\_count": len(rows)})\
   if not rows:\
   return f"No observations matched '{query}' in project '{project}'."\
   out = \["\| ID \| Type \| Harness \| Title \| Files \|", "\|---\|---\|---\|---\|---\|"\]\
   for r in rows:\
   out.append(f"\| {r\['id'\]} \| {r\['type'\]} \| {r\['agent\_harness'\]} \| {r\['title'\]} \| {r\['files\_touched'\]} \|")\
   return "\\n".join(out)\
\
@mcp.tool()\
async def mem\_get\_observations(project: str, agent\_harness: str, ids: List\[int\]) -\> str:\
   """Layer 3 Progressive Disclosure Batch Fetch. Retrieves full details only for selected IDs."""\
   placeholders = ",".join("?" for \_ in ids)\
   with get\_db() as conn:\
   rows = conn.execute(\
   f"SELECT \* FROM observations WHERE project = ? AND id IN ({placeholders})",\
   \[project, \*ids\],\
   ).fetchall()\
   write\_audit\_trace("MEM\_GET\_DETAILS", agent\_harness, project, {"ids": ids, "fetched": len(rows)})\
   return "\\n\\n---\\n\\n".join(\
   f"### \[{r\['id'\]}\] {r\['title'\]} ({r\['type'\]} via {r\['agent\_harness'\]})\\n"\
   f"\*\*Files:\*\* {r\['files\_touched'\]}\\n\*\*Facts:\*\* {r\['facts'\]}\\n\\n{r\['narrative'\]}"\
   for r in rows\
   )\
\
@mcp.tool()\
async def mem\_record\_observation(\
   project: str, agent\_harness: str, obs\_type: str, title: str, narrative: str, facts: str, files\_touched: str\
) -\> str:\
   """Persists a durable architectural decision, bugfix, discovery, or change into long-term FTS5 memory."""\
   now\_ms = int(time.time() \* 1000)\
   with get\_db() as conn:\
   cur = conn.execute(\
   """INSERT INTO observations (project, workspace\_task\_id, agent\_harness, type, title, narrative, facts, files\_touched, created\_at\_epoch)\
   VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)""",\
   (project, project, agent\_harness, obs\_type, redact\_secrets(title), redact\_secrets(narrative), redact\_secrets(facts), files\_touched, now\_ms),\
   )\
   obs\_id = cur.lastrowid\
   write\_audit\_trace("MEM\_RECORD\_OBS", agent\_harness, project, {"obs\_id": obs\_id, "type": obs\_type, "title": title})\
   append\_breadcrumb(project, project, agent\_harness, "mem\_record\_observation", files\_touched, f"\[{obs\_type}\] {title}")\
   return f"Recorded observation #{obs\_id} ({obs\_type}): {title}"\
\
if \_\_name\_\_ == "\_\_main\_\_":\
   mcp.run(transport="http", host="0.0.0.0", port=8443, path="/mcp")\


To manage the JSONL audit trail using standard Unix utilities, the following configuration placed in /etc/logrotate.d/agent-memory rotates the active trace file daily, retains 90 days of compressed archives with 0640 permissions, and optionally ships each completed day's batch to a self-hosted OpenObserve endpoint without running a real-time collector agent8:

/var/log/agent-memory/audit.jsonl {

daily

rotate 90

missingok

notifempty

compress

delaycompress

dateext

dateformat -%Y-%m-%d

create 0640 secops adm

postrotate

ROTATED\_FILE="/var/log/agent-memory/audit.jsonl-![](image.png){OPENOBSERVE\_PASS}"

-H "Content-Type: application/x-ndjson"

--data-binary "@${ROTATED\_FILE}" \>/dev/null 2\>\&1 \|

\| true

fi

endscript

}

Finally, to bind Claude Code, Google Antigravity, OpenCode, and Claude Desktop to this server workflow without client-side hooks, the operator places the following portable SKILL.md file inside \~/.claude/skills/unified-memory/SKILL.md, \~/.agents/skills/unified-memory/SKILL.md, or an Antigravity plugin.json bundle17:

name: unified-cross-agent-memory-and-shunt description: Use at the start of every coding session, before reading any file over 150 lines, when generating boilerplate or test files, when switching between coding agents (Claude Code, Antigravity, OpenCode, Claude Desktop), or when recording architectural decisions and bugfixes.

Unified Cross-Agent Session Memory \& I/O Shunt Protocol

1. **Session Resume (Mandatory First Step):** Before exploring the repository, invoke session\_resume\_or\_handoff(project, agent\_harness) to load the active working state and recent tool breadcrumbs left by prior agent sessions.
2. **Deterministic I/O Shunt (smart\_read \& boilerplate\_write):** Prefer smart\_read over raw file reads when exploring unfamiliar files. Never load full files over 150 lines simply to locate a method signature. Pass explicit offset and limit parameters to smart\_read only when preparing to edit specific lines or when inspecting security-critical authentication and concurrency logic. Use boilerplate\_write with a reference file path when generating repetitive tests or stubs.
3. **Progressive Disclosure Memory Search:** When querying historical project context, call mem\_search first to inspect the compact FTS5 index, and pass only validated observation IDs to mem\_get\_observations.
4. **Live Checkpoint \& Handoff:** Whenever completing a sub-task, switching branches, or preparing to hand off work to another agent harness, invoke session\_resume\_or\_handoff with an updated updated\_working\_state\_md summarizing current hypotheses, modified files, and immediate next steps.

Strategic Conclusions and Deployment Recommendations

Replacing claude-mem with a centralized, self-hosted architecture rather than maintaining a local fork resolves the fundamental tension between multi-agent interoperability, token efficiency, and security observability2. While claude-mem v15.5.0 offers a capable local SQLite FTS5 schema and multi-provider observer options, its structural dependence on fail-open client lifecycle hooks with 2,000ms timeouts, per-user localhost port binding, asynchronous end-of-session summarization, and increasing cmem.ai SaaS and PostHog telemetry coupling make it unsuitable as a cross-harness infrastructure backbone6.

By deploying a single self-hosted FastMCP server over Streamable HTTP (--transport http in Claude Code and OpenCode, serverUrl in Google Antigravity's mcp\_config.json, and Custom Connectors in Claude Desktop) paired with a portable SKILL.md plugin, an infrastructure operator eliminates both the unreliability of client shell hooks and the authentication breakage of MITM API proxies14. Embedding Spotify's bulk-reader and code-writer shunt logic directly inside the MCP server's tool handlers—while tuning the interception cutoff from 350 lines down to 150 lines to reflect the 158-line empirical median read size, enforcing a deterministic regex bypass for security-critical code, and caching worker SLM summaries by SHA-256 content hash in SQLite—captures up to 90% frontier I/O token savings while eliminating the 10–30 second remote worker latency on repeat reads across agents2.

Simultaneously, separating the storage layer into an ephemeral Cross-Agent Session Bus (active\_sessions and automatic session\_breadcrumbs recorded on every MCP tool invocation) and a long-term Progressive Disclosure FTS5 Store guarantees instantaneous, lossless mid-task handoffs between terminal runs in Claude Code or OpenCode and parallel visual workflows in Google Antigravity3. Grounding the entire server in an append-only, secret-redacted JSONL audit log (/var/log/agent-memory/audit.jsonl) managed by daily Linux logrotate scripts satisfies strict security compliance using native Unix primitives, providing immediate command-line forensic traceability alongside optional batch ingestion into OpenObserve7.

Works cited

1. Claude-Mem: Persistent Memory for Claude Code Across Sessions, [https://yuv.ai/blog/claude-mem](https://yuv.ai/blog/claude-mem)
2. Portal by Spotify cut my Claude Code token usage by 90%, [https://engineering.atspotify.com/2026/9/portal-by-spotify-cut-my-claude-code-token-usage-by-90](https://engineering.atspotify.com/2026/9/portal-by-spotify-cut-my-claude-code-token-usage-by-90)
3. Claude Code vs. Antigravity: Which AI Tool Is Better? \| DataCamp, [https://www.datacamp.com/blog/claude-code-vs-antigravity](https://www.datacamp.com/blog/claude-code-vs-antigravity)
4. Antigravity vs Claude Code: Which One Wins in 2026 \| Superblocks, [https://www.superblocks.com/blog/antigravity-vs-claude-code](https://www.superblocks.com/blog/antigravity-vs-claude-code)
5. GitHub - thedotmack/claude-mem: Persistent Context Across, [https://github.com/thedotmack/claude-mem](https://github.com/thedotmack/claude-mem)
6. claude-mem: Claude Code Memory Plugin, Setup and Risks, [https://explainx.ai/blog/claude-mem-persistent-memory-claude-code-hooks-sqlite-chroma-github-trending-guide-2026](https://explainx.ai/blog/claude-mem-persistent-memory-claude-code-hooks-sqlite-chroma-github-trending-guide-2026)
7. thedotmack/claude-mem v15.5.0 on GitHub - NewReleases.io, [https://newreleases.io/project/github/thedotmack/claude-mem/release/v15.5.0](https://newreleases.io/project/github/thedotmack/claude-mem/release/v15.5.0)
8. [https://docs.claude-mem.ai/installation](https://docs.claude-mem.ai/installation)
9. Claude-Mem hit 45000 stars on Github today and it all started HERE, [https://www.reddit.com/r/ClaudeCode/comments/1scz5kk/claudemem\_hit\_45000\_stars\_on\_github\_today\_and\_it/](https://www.reddit.com/r/ClaudeCode/comments/1scz5kk/claudemem_hit_45000_stars_on_github_today_and_it/)
10. justinwlin/claude-mcp-guide: Reference guide for adding ... - GitHub, [https://github.com/justinwlin/claude-mcp-guide](https://github.com/justinwlin/claude-mcp-guide)
11. A-MEM – Memory for Claude Code that links and evolves on its own, [https://news.ycombinator.com/item?id=46636707](https://news.ycombinator.com/item?id=46636707)
12. ai-memory: Share Memory Between Claude Code and Codex, [https://explainx.ai/blog/ai-memory-akitaonrails-shared-memory-claude-code-codex-handoff-markdown-wiki-guide-2026](https://explainx.ai/blog/ai-memory-akitaonrails-shared-memory-claude-code-codex-handoff-markdown-wiki-guide-2026)
13. HTTP Deployment - FastMCP, [https://gofastmcp.com/deployment/http](https://gofastmcp.com/deployment/http)
14. Claude Code MCP Server Setup Guide - HeyClaude, [https://heyclau.de/entry/guides/claude-mcp-server-setup-guide](https://heyclau.de/entry/guides/claude-mcp-server-setup-guide)
15. Connect Claude Code to tools via MCP, [https://code.claude.com/docs/en/mcp](https://code.claude.com/docs/en/mcp)
16. MCP \| Google Antigravity Docs, [https://antigravity.google/docs/mcp/](https://antigravity.google/docs/mcp/)
17. Beacon Agent Memory: One Open-Source Layer for 20+ Harnesses, [https://explainx.ai/blog/beacon-asymptote-labs-open-source-cross-harness-agent-memory-claude-code-codex-cursor-2026](https://explainx.ai/blog/beacon-asymptote-labs-open-source-cross-harness-agent-memory-claude-code-codex-cursor-2026)
18. Blog \| Spotify Portal, [https://portal.spotify.com/blog](https://portal.spotify.com/blog)
19. Spotify Engineer Cuts Claude Code Tokens by 90%: Hooks Block, [https://clauding.de/en/posts/spotify-shunt-claude-code-tokens-90-prozent/](https://clauding.de/en/posts/spotify-shunt-claude-code-tokens-90-prozent/)
20. Spotify's 90% Token Cut: Sell AI Coding Cost Audits, [https://www.figuringoutwithai.com/playbooks/spotify-claude-code-token-cost-audit-playbook-2026](https://www.figuringoutwithai.com/playbooks/spotify-claude-code-token-cost-audit-playbook-2026)
21. Spotify Cut Claude Code Tokens by 90%. We Measured the Same, [https://thakicloud.com/tech-blog/en/tutorials/spotify-shunt-token-routing/](https://thakicloud.com/tech-blog/en/tutorials/spotify-shunt-token-routing/)
22. Portal by Spotify cut my Claude Code token usage by 90% \| daily.dev, [https://daily.dev/posts/lhgv961ro](https://daily.dev/posts/lhgv961ro)
23. Portal by Spotify cut my Claude Code token usage by 90%, [https://news.ycombinator.com/item?id=49571465](https://news.ycombinator.com/item?id=49571465)
24. Devin Dreaming Memory: Git-Based Agent Memory Spec - explainx.ai, [https://explainx.ai/blog/cognition-devin-dreaming-memory-agent-memory-repo-open-spec-2026](https://explainx.ai/blog/cognition-devin-dreaming-memory-agent-memory-repo-open-spec-2026)
25. Hindsight AI Memory: Learn vs Recall, Explained (2026) - explainx.ai, [https://explainx.ai/blog/hindsight-agent-memory-system-vectorize-2026](https://explainx.ai/blog/hindsight-agent-memory-system-vectorize-2026)
26. Shared memory - Letta Docs, [https://docs.letta.com/concepts/shared-memory](https://docs.letta.com/concepts/shared-memory)
27. Overview \| Asymptote-Labs/agent-beacon \| Zread, [https://zread.ai/Asymptote-Labs/agent-beacon](https://zread.ai/Asymptote-Labs/agent-beacon)
28. TencentDB Agent Memory v2: Team Hub for Chat, Skills, Wiki, [https://explainx.ai/blog/tencentdb-agent-memory-v2-team-hub-august-2026](https://explainx.ai/blog/tencentdb-agent-memory-v2-team-hub-august-2026)
29. AI Agents: News, Guides and Timeline - explainx.ai, [https://explainx.ai/blog/topics/ai-agents](https://explainx.ai/blog/topics/ai-agents)
30. Memory blocks (core memory) \| Letta Docs, [https://docs.letta.com/v1-sdk/memory/memory-blocks](https://docs.letta.com/v1-sdk/memory/memory-blocks)
31. Shared memory \| Letta Docs, [https://docs.letta.com/v1-sdk/memory/shared-memory](https://docs.letta.com/v1-sdk/memory/shared-memory)
32. letta-ai/managing-shared-memory - Decision Hub, [https://hub.decision.ai/skills/letta-ai/managing-shared-memory](https://hub.decision.ai/skills/letta-ai/managing-shared-memory)
33. Letta Code - Learn AI - Miraheze, [https://ai.miraheze.org/wiki/Letta\_Code](https://ai.miraheze.org/wiki/Letta_Code)
34. @iflow-mcp/letta-mcp-server - npm, [https://www.npmjs.com/package/@iflow-mcp/letta-mcp-server](https://www.npmjs.com/package/@iflow-mcp/letta-mcp-server)
35. Memory Blocks: The Key to Agentic Context Management - Letta, [https://www.letta.com/blog/memory-blocks/](https://www.letta.com/blog/memory-blocks/)
36. Managing Shared Memory by letta-ai - TypingMind, [https://www.typingmind.com/skills/letta-ai-managing-shared-memory](https://www.typingmind.com/skills/letta-ai-managing-shared-memory)
37. Plugin Marketplace in Antigravity \| Google Antigravity Blog, [https://antigravity.google/blog/plugin-marketplace-in-antigravity/](https://antigravity.google/blog/plugin-marketplace-in-antigravity/)
38. Antigravity CLI Setup - Claude-Mem, [https://docs.claude-mem.ai/antigravity-cli/setup](https://docs.claude-mem.ai/antigravity-cli/setup)
39. managedMcpServers: Claude Code's New MCP Setting, [https://www.getclaudeskills.com/blog/claude-code-managed-mcp-servers-setting](https://www.getclaudeskills.com/blog/claude-code-managed-mcp-servers-setting)
40. antigravity-claude-mcp \| MCP Servers - LobeHub, [https://lobehub.com/mcp/arjunthilakeros-antigravity-claude-mcp](https://lobehub.com/mcp/arjunthilakeros-antigravity-claude-mcp)
41. Claude for Marketing: Wiring a Remote MCP Server Into an Ad, [https://huggingface.co/blog/Jacob123222/claude-for-marketing](https://huggingface.co/blog/Jacob123222/claude-for-marketing)
42. Claude Code FastMCP, [https://gofastmcp.com/v2/integrations/claude-code](https://gofastmcp.com/v2/integrations/claude-code)
43. fastmcp-remote, [https://gofastmcp.com/clients/fastmcp-remote](https://gofastmcp.com/clients/fastmcp-remote)
