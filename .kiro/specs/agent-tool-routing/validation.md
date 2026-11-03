# Implementation Validation

Status: **MANUAL_VERIFY_REQUIRED**. Task 5.2 remains incomplete.

## Fresh deterministic evidence

- Full native `cargo test` passed; formatting, strict default and OTel clippy passed.
- `mmhmem --help` reached the usable CLI.
- Routing and checkpoint mutation checks passed. Installer and marketplace
  distribution checks passed on Debian/Alpine in task 5.1; PowerShell 5/7
  cross-helper and full installer suites passed.
- Accounting tests prove cold summary and matching-identity cache hit with one
  worker request total, correct attribution, and no false savings on direct,
  targeted, security, session, or failed-worker paths.

## Live Claude evidence

- Claude Code 2.1.302; configured model alias `fable`; authenticated-local mode.
- Actual installer created `<fixture>/.claude/rules/mmhmem.md`; authentication and
  ambient global instructions were retained, not copied into the fixture.
- Prompt: `Explain transform.rs responsibilities and structure.`
- Both fresh sessions selected: Glob â†’ ToolSearch â†’ mcp__mmhmem__smart_read â†’ Read â†’ Glob.
- Both smart reads successfully inspected the intended fixture without offset/limit.
- Content SHA-256: `462efcdf9f1347b8366bbf23500bdd3df1cbae4b1d64c8973950c2b8581a74a9`.
- Instruction SHA-256: `53eb06880f5341296a6082e9e34d696f457be97467c0755a6a195b3ec3fb89b2`.
- Question fingerprints differed: `b950ae9b7bbf589f`, `e5f3acf7bf473357`.
- Worker request snapshots: 0 â†’ 1 â†’ 2. Savings rows: 0 â†’ 1 â†’ 2.
- Estimated avoided: 0 â†’ 7,437 â†’ 14,874. Worker prompt: 0 â†’ 100 â†’ 200;
  completion: 0 â†’ 10 â†’ 20. Cache hits: 0 â†’ 0 â†’ 0.
- Natural selection: PASS. Live matching-identity repeat: NOT_VERIFIED. Different
  generated questions produced different cache identities; no cache-hit claim.
- Runtime verification found and repaired a CLI bug: the variadic allowed-tools
  option consumed the prompt. A `--` argument terminator now separates it.

## Live OpenCode evidence (corrected after recheck)

- OpenCode v2.2.5, model `vibe-oidc/zai-glm-5-3`, successfully executed both fresh sessions.
- Root cause was our capability parser: actual help prints `--model, -m`, but the parser required the exact whitespace token `--model`. A regression-tested trailing-comma normalization fixes this.
- The earlier external plugin/runtime diagnosis was incorrect for this acceptance failure. Native help succeeds in the repository and disposable directories.
- First session: glob, resume, non-targeted smart_read, Read, Read, compress, checkpoint. Worker requests 1; estimated avoided 7,437.
- Second session: glob, resume, two targeted smart_read calls, compress. Worker requests remained 1, estimates unchanged, cache hits 0.
- Natural summary selection was observed in the first session. The aggregate test remains unsuccessful because the second session selected targeted reads; live matching-identity cache reuse remains NOT_VERIFIED.
- Evidence: `tmp/routing-opencode-recheck.log`. No user service restart was needed.

## Authorized installation on the user's workstation

- Ran the global Windows installer for Claude Code, OpenCode, and Codex with the current tested debug build, then repeated installation.
- Worker connection verified on both runs; existing settings retained and API key masked.
- Installed binary hash matches the tested build. All three installed skills match canonical bytes.
- Codex TOML parses and contains exactly one mmhmem table; Claude/OpenCode each contain one owned routing section after both runs.
- Existing running processes retain their old binary; fresh sessions load the update.

## Requirement coverage

| Criteria | Evidence and remaining limits |
|---|---|
| 1.1, 1.2, 1.3, 1.4, 1.5 | stdio/HTTP metadata tests and unchanged parameter contracts pass |
| 2.1, 2.2 | Installer distribution passes; Claude live selection passes; OpenCode first live summary selection observed |
| 2.3, 2.4, 2.5 | Guidance/checkpoint contracts and deterministic exact/fallback tests pass; no universal compliance claim |
| 3.1, 3.2, 3.3, 3.4, 3.5 | Golden, filesystem, cross-helper, scope, override, bootstrap and distribution suites pass |
| 4.1 | Claude passes; OpenCode natural summary selection observed; strict two-summary run remains incomplete |
| 4.2 | Deterministic matching-identity hit passes; live repeat NOT_VERIFIED |
| 4.3 | Real MCP negative accounting tests pass |
| 4.4 | Sanitized versions, source, prompt, traces and counters recorded above |
| 4.5 | Missing evidence explicitly blocks completion |

Local command logs are under `tmp/routing-final-*.log` and
`tmp/routing-live-{claude,opencode}.log`. They are development artifacts, not
credential or conversation exports. Production installation was subsequently performed twice with explicit user authorization, as recorded above.
