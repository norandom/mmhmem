# Validation Notes — shunt-content-tool

## Task 4.2 — Live worker check (2026-11-07)

Run at commit `a5c5e3a` (4.1), `target/debug/mmhmem.exe stdio`, isolated temp store
via `AGENT_MEM_DATA_DIR` (real store untouched), global worker profile, MCP JSON-RPC
`tools/call` `shunt_content` with `agent_harness=opencode`,
`project=shunt-content-4-2-live-check`, `kind=file_read`, 200-line benign synthetic
content (~17.5 KB).

- **Latency**: 803 ms wall end-to-end; wire `latency_ms` 785 for the delegated summarize.
- **Summary shape**: one structural entry — `"- stampsArtifactsAndWritesReport [L1-L200]"`;
  `outcome: summarized`; `worker_model: mistral-small-2603` echoed; `line_count: 200`;
  `content_sha256: 1f31e114517e41da04a2701aa08c50af0add63fed4b6b7568a24af1ee4a70e47`;
  `avoided_input_tokens_est: 4539`.
- **Statistics delta** (before: no rows): +1 row for day 2026-11-07, project
  `shunt-content-4-2-live-check`, harness `opencode` — `summary_reads: 1`, `cache_hits: 0`,
  `worker_prompt_tokens: 4993`, `worker_completion_tokens: 17`,
  `avoided_input_tokens_est: 4539`; `direct_reads`/`slice_reads`/`hook_redirects` all 0.
- **Deterministic gate unchanged**: the run used an isolated temp store and made no repo
  changes; full suite last green at `a5c5e3a` (893 passed / 0 failed / 1 ignored; fmt and
  strict clippy clean, default and `otel` feature sets).
