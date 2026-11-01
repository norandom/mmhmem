# Handoff — session 2026-11-01 (local-mode complete; corpus-warmup-prune in flight)

For the next session picking this work up. Repo state at pause: everything
committed and pushed through `bdf1c63` on `main`; working tree clean except
an untracked `.openidea/` directory (foreign, not ours — leave it alone).
Run `git status --porcelain && git log --oneline -5` first: the user runs
parallel sessions (one landed CI fixes mid-session as `c77c620`).

## Mandates that govern all work here

- **Foreground only, no subagents** (user mandate; this session honored it
  throughout — inline reviews instead of reviewer agents).
- One conventional commit per phase/task, push to `origin/main` after each.
- English artifacts; EARS requirements with numeric AC IDs; Req 13-style
  house caps (cognitive ≤ 15, ≤ 500 LOC/src file, ≤ 80 LOC/fn, ≤ 400
  non-test LOC/task diff, tests ≤ 800 LOC/file).
- TDD: red → green → refactor → verify per task.

## Completed today (all pushed)

1. **local-mode — FULL cycle, implemented and validated.** Init →
   requirements (13 reqs/62 ACs, user-approved) → design (user-approved) →
   tasks (21 incl. a repair: 3.3 stdio integration tests covering Req 2.4,
   which a substring check had falsely marked covered) → implementation
   21/21 via TDD. `mmhmem serve`/`stdio`/`stats` subcommands; stdio serving
   with full tool parity (rmcp `transport-io`, `stdio_pipes`, stderr-only
   diagnostics, EOF-clean exit, malformed-input resilience); local profile
   (zero mandatory env, per-OS user dirs, optional confinement with
   audited denials); multi-process store (busy_timeout, default 5 s,
   `AGENT_MEM_BUSY_TIMEOUT_MS`); read-only stats store opening;
   size-bounded audit rotation (10 MiB/keep 5, serve rotation stays off);
   `mmhmem stats` (+`--json`); Windows/macOS native CI jobs; client-first
   README/deploy docs + per-harness stdio examples; connection-neutral
   SKILL.md (<200 tokens).
   **Suite at pause: 160/160 green** (97 pre-existing, byte-identical —
   git-diff proof recorded; 63 new). Full gate chain (fmt, clippy
   `-D warnings`, loc caps) green locally. CI native jobs verified on the
   GitHub runners via pushes.
2. **Installers** `deploy/install_local.sh` + `deploy/install_local.ps1`:
   scope prompt (global/per-project) or `--global`/`-Global`; harness
   selection (`--agent claude,opencode,desktop,antigravity,all` or an
   interactive multi-select menu); JSON-merge config edits for
   OpenCode/Claude Desktop (python3 in bash, native in ps1); Antigravity
   prints an import snippet; `--remove`/`-Remove` uninstalls (store NEVER
   deleted); `--dry-run`/`-DryRun` (validated in WSL + on the host);
   non-interactive guards so piped invocations fail fast instead of
   hanging; optional worker configuration via
   `--worker-model/--worker-url/--worker-api-key` flags + interactive
   offer + unset notices (INTERIM — see the .mmhmem.json decision below:
   R7.6 replaces the env embedding with config-file writing).
3. **README motto banner** (`assets/mmhmem.jpg`, 82 KB, added under the
   title — image content never viewed; alt text is generic).
4. **corpus-warmup-prune quick spec** (4 phases + inline sanity review
   PASSED): `brief.md`, `requirements.md`, `design.md`, `tasks.md`,
   `spec.json` (phase tasks-generated, all approvals true,
   ready_for_implementation true). Then amended mid-session (commit
   `bdf1c63`): **Req 7 `.mmhmem.json` worker config file** (7 ACs) and
   **Req 8 worker-unconfigured session notice** (2 ACs) added; design
   traceability rows + directory-structure entry added.

## User decisions to honor (2026-11-01, all in this session's record)

1. **Worker model is optional.** Warmup NEEDS it; organic accrued savings
   don't. When unset: the MCP server shall notify at session start (Req 8)
   and the installers already notice (implemented).
2. **`.mmhmem.json` is the config home for the worker** (was: bake env into
   harness registrations — superseded). JSON format, per-project
   `./.mmhmem.json` + global `~/.mmhmem.json`, worker settings only
   (model/api_url/api_key), per-setting precedence env > project > global >
   defaults, serve stays environment-only, key never logged (redaction
   parity — VERIFY the Config Debug impl actually redacts
   `worker_api_key`, not just the token), `.gitignore` ignores
   `.mmhmem.json`, invalid JSON → fail fast naming the file. This amends
   steering `structure.md`'s "Config is environment-only" rule for the
   local profile — the steering file still needs that amendment.
3. **Warmup tool**: explicit paths list only, cap 25 per call, honest
   corpus (real delegations, real stats; no synthetic rows).
4. **Prune**: CLI + MCP tool (user picked both), stale-only (missing file
   or changed content sha), dry-run/preview, stats/observations/sessions
   untouched.
5. **Mistral worker config, validated live**: model ID
   `mistral-small-2603` (the docs name mistral-small-4-0-26-03 does NOT
   exist as an API ID), URL `https://api.mistral.ai/v1/chat/completions`,
   and a working user API key (the key stays with the user — NEVER commit
   it anywhere; it goes into `~/.mmhmem.json` at install time).
6. **Real install on this machine is DEFERRED** until `.mmhmem.json`
   support lands (loader + installer writing). Then: global install, all
   detected harnesses, Mistral worker into `~/.mmhmem.json`.

## In flight — exactly where things stand

The corpus-warmup-prune spec amendment (Req 7/8) is PARTIALLY applied.
Remaining spec work, in order:

1. **design.md**: add the `config/mmhmem_file.rs` component block
   (loader: `MmhmemFile { worker_model, worker_api_url, worker_api_key }`
   via serde, `load()` for both paths, `Option`-based per-setting merge in
   `Config::load_local`, `ConfigError::InvalidMmhmemFile(file)` naming the
   file) + a note that installers write the file (R7.6). Traceability rows
   and the directory entry already exist. Reference seam already verified:
   `Config::load_local()`/`from_env_local()` in `src/config/profile.rs`;
   parser helpers reuse `present()`/`optional()`.
2. **tasks.md**: add Major 8 (three sub-tasks: 8.1 loader TDD — precedence
   matrix, absent settings, invalid-JSON fail-fast, redaction test;
   8.2 installers write `.mmhmem.json` instead of env embedding + notice
   wording; 8.3 README section + `.gitignore` entry). Amend task 3's
   detail/requirements to carry Req 8 (the session-start notice). Then
   re-run the AC coverage gate (now 36 ACs: 8+1+6+3+4+5+7+2; the check
   script pattern lives in this session's history — a small .ps1 loop
   over IDs vs tasks.md text).
3. **The interrupted validations** (user said "run both", then paused):
   `/kiro-validate-gap corpus-warmup-prune` → append findings to
   `.kiro/specs/corpus-warmup-prune/research.md` (key finding already in
   hand: `ShuntEngine::smart_read` returns
   `ReadOutcome{Targeted|Direct|Cached|Shunted}` and warmup is pure
   orchestration over it — no engine refactor; error mapping:
   `FileNotFound`→missing, `NotConfigured`→propagate,
   `OutsideRoots`→count denied) and `/kiro-validate-design
   corpus-warmup-prune` → GO/NO-GO verdict. Both were loaded and
   mid-execution when the user paused; their skill instructions and rules
   files must be re-loaded fresh in the next session.
4. Then implement: task 8 first (loader — the real install depends on
   it), then `/kiro-impl corpus-warmup-prune` for the rest (warmup,
   prune, notice, docs, integration suites, final validation).

## Known gotchas pinned this session

- PowerShell harness `$`-stripping: complex one-liners lose variables —
  write a script file to `C:\Users\mariu\AppData\Local\Temp\opencode\` and
  run it via `-File` (an `ac-coverage.ps1` from this session may still be
  there).
- rmcp serve API: `service.serve((reader, writer))` via
  `use rmcp::ServiceExt;` → `RunningService`; `.waiting()` ends on stdin
  EOF; `ServerInitializeError::ConnectionClosed` = parent gone = clean
  exit 0; `stdio_pipes()` panics off-runtime on Linux (construct inside
  `block_on`).
- `tracing` macros with `?field` captures explode the cognitive-complexity
  budget (end_session was 17/15 until the field moved into a pre-formatted
  string).
- PS 5.1: redirected native stderr can throw under
  `$ErrorActionPreference='Stop'` (see `Remove-ClaudeServer` in the ps1
  installer); `ConvertFrom-Json -AsHashtable` does not exist (5.1) —
  PSObject + `Add-Member -Force` is the pattern used in `Merge-Json`.
- House test conventions: tools/call errors are JSON-RPC error objects
  with actionable messages; protocolVersion `"2025-11-25"`; the six-tool
  pins live in this repo's own `tests/local_stdio.rs` (`names == 6`) and
  `tests/multiprocess.rs` — they get updated to eight under this spec
  (Req 5.4), while the protected pre-existing suite does not pin the count
  (verified by grep before the spec was written).
- `src/lib.rs` may show a phantom "M" in `git status` (CRLF
  normalization noise, no content diff).
- The untracked `.openidea/` directory is another session's — never add it.
