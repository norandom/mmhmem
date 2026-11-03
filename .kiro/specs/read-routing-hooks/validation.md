# Validation: read-routing-hooks

## Task 2.3: live Claude Code check

Run on 2026-11-03 on Windows 11 with Claude Code 2.1.302 (`claude -p`,
`--model sonnet`, `--output-format stream-json --verbose`). The hook binary
was `D:\Source\mmhmem\target\release\mmhmem.exe`, built from `d47bdef` with
`cargo build --release` and `CARGO_INCREMENTAL=0`.

### Setup

- Scratch root: `D:\Source\mmhmem\tmp\hook-live-53145\` (git-ignored).
  - `project\big.rs`: 600 lines of Rust. `project\short.rs`: 20 lines.
    `project\auth_tokens.rs`: 600 lines, and its name matches the security
    rule.
  - `settings.hook.json` was passed with `--settings`. It holds only
    `hooks.PreToolUse`, using matcher `Read|Bash|PowerShell` and the
    exec-form handler `{"type":"command","command":"<abs exe>","args":["hook","claude"],"timeout":10}`.
  - `settings.broken.json`: the same entry, pointing to a binary that does
    not exist.
  - `settings.slow.json`: `command: "powershell"`,
    `args: ["-NoProfile","-Command","Start-Sleep 30"]`, `timeout: 3`.
- `AGENT_MEM_DATA_DIR=...\hook-live-53145\data` was set for the whole
  `claude` process tree. The debug log shows that the user's global mmhmem
  MCP server also started with this data directory: `mmhmem stdio ready
  (local mode) config=Config { data_dir: "...hook-live-53145\\data" ...`.
  The worker settings in `~/.mmhmem.json` were not changed, and the default
  threshold is 150 lines.
- Each session ran with `--allowedTools
  "Read,Edit,mcp__mmhmem__smart_read,mcp__mmhmem__session_resume_or_handoff"`.
  The last sessions (7a2, 7b2, 8) also used `--include-hook-events --debug
  hooks --debug-file`.
- Raw logs are in `tmp\hook-live-53145\logs\<case>.jsonl`, with
  `<case>.debug.log` where debug was on. The audit log is
  `tmp\hook-live-53145\data\audit.jsonl`. Latency inputs are in
  `tmp\hook-live-53145\inputs\`, their audit log is in `data-latency\`, and
  their timings are in `logs\latency.txt`.
- No user or project settings file was changed. The scratch project has no
  `.claude\` directory.

### Results

| # | Case | Expected | Observed | Evidence | Result |
|---|------|----------|----------|----------|--------|
| 1 | Full `Read` of `big.rs` (600 lines) | Deny with the mmhmem reason | The hook denied the read. Claude Code showed it to the agent as `PreToolUse:Read hook error: mmhmem: ...\big.rs has at least 150 lines. Call smart_read for a summary, or read an exact line range with offset and limit. Repeat this read within 2 minutes to read the whole file anyway.` In 1, the agent chose the repeat route: it read the whole file again, and that read passed. | `case1-full-big.jsonl`: Read(big.rs), then deny, then Read(big.rs), then full content. Audit: `HOOK_REDIRECT reason=large_file lines=150`, then `HOOK_PASS reason=repeat`. The hook stdout in `case8-timing.jsonl` `hook_response` is the documented `{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"deny","permissionDecisionReason":"mmhmem: ..."}}`, exit 0. | PASS |
| 1b | Same read, agent told to follow the first alternative | Deny, then `smart_read` | Deny. The agent called `smart_read` with only `file_path`, and the call failed with `failed to deserialize parameters: missing field 'project'`. The agent fell back to `Grep` and still answered correctly (`compute_step_0`). | `case1b-follow-redirect.jsonl`: Read, deny, smart_read (error), Grep, Grep. Audit: `HOOK_REDIRECT large_file`. | PASS (hook). See concern C2. |
| 1c | Redirect, then `smart_read` with a project, then a ranged read, then Edit | `smart_read` returns a summary | `smart_read(project="hook-live-check", question=...)` returned `### Shunted Summary (worker, 1030 ms) - compute_step_3 [L26-L30]`, with an estimated 6275 input tokens avoided. Read(offset 26, limit 1) passed, and Edit succeeded. | `case6b-redirect-smartread-edit.jsonl`. Audit: `HOOK_REDIRECT large_file`, `IO_SHUNT_DELEGATED`, `HOOK_PASS range`. | PASS |
| 2 | Ranged read: `big.rs`, offset 10, limit 11 | Pass | Passed, and lines 10 to 20 were returned. | `case234-range-short-auth.jsonl`. Audit: `HOOK_PASS reason=range`. | PASS |
| 3 | Full read of `short.rs` (20 lines) | Pass | Passed, and the full file was returned. | Same log. Audit: `HOOK_PASS reason=short`. | PASS |
| 4 | Full read of `auth_tokens.rs` (600 lines, security-critical) | Pass | Passed, and the full file was returned. | Same log. Audit: `HOOK_PASS reason=security_critical`. | PASS |
| 5 | Repeated full read of `big.rs` | Deny, then pass | The first read was denied and the second, identical read passed and returned 600 lines. | `case5-repeat.jsonl`: Glob, Read (deny), Read (full). Audit: `HOOK_REDIRECT large_file` at 17:14:14, then `HOOK_PASS repeat` at 17:14:15. Case 1 shows the same pair. | PASS |
| 6 | Edit after an exact-range read (Req 2.8) | Edit succeeds | Read(big.rs, offset 12, limit 1) passed. Edit (`compute_step_1` to `compute_step_one`) returned `The file ... has been updated successfully.` Line 12 on disk now reads `pub fn compute_step_one(`. | `case6-edit-after-range.jsonl`. Audit: `HOOK_PASS range`. | PASS |
| 6c | Edit after a denied full read, with no successful read | (Control) | The Edit succeeded. | `case6c-edit-without-read.jsonl`: Read (deny), Edit (success). | Info |
| 6d | Edit with no read at all | (Control) | The Edit succeeded. Claude Code 2.1.302 in `-p` mode did not enforce read-before-edit for this file. | `case6d-edit-no-read-control.jsonl`: Edit (success). | Info. See concern C3. |
| 7a | Hook command points to a binary that does not exist | The Read proceeds | The full Read of `big.rs` proceeded and returned all lines. Claude Code logged `Hook command failed to spawn (PreToolUse:Read): ENOENT ...`, and the hook event shows `exit_code=1 outcome=error`. The agent saw no hook message. | `case7a2-broken-binary.jsonl` and `.debug.log`. The first run, `case7a-broken-binary.jsonl` on `short.rs`, also proceeded. | PASS |
| 7b | Hook sleeps 30 s with `timeout: 3` | The Read proceeds after the timeout | The Read proceeded after the timeout. The debug log shows `Hooks: PreToolUse:Read [powershell -NoProfile -Command Start-Sleep 30] timed out after 3000ms (3265ms)`, then `cancelled`, then `Slow PreToolUse hooks: 3273ms for Read`. The hook event shows `outcome=cancelled`. The full file was returned. | `case7b2-slow-hook.jsonl` and `.debug.log`. The first run, `case7b-slow-hook.jsonl` on `short.rs`, also proceeded. | PASS (timeout fails open; this settles the open point in research.md) |
| 8a | Decision time as Claude Code measures it | Under 200 ms (Req 3.3) | The redirect of `big.rs` took 47 ms and the pass of `short.rs` took 35 ms. Both times include process spawn and exit status 0. | `case8-timing.debug.log`: `Hooks: PreToolUse:Read [...mmhmem.exe hook claude] finished with status 0 (47ms)` and `(35ms)`. | PASS |
| 8b | Direct invocations of the release binary with a captured input (5 each) | Under 200 ms | Redirect: 26.8, 15.7, 15.6, 14.5 and 15.7 ms. Short pass: 15.1, 14.0, 14.5, 13.8 and 14.1 ms. Range pass: 13.3, 14.1, 13.5, 13.7 and 14.2 ms. Every run exited 0. Each deny printed the deny JSON, and each pass printed nothing. These times are wall-clock and include process start. | `logs\latency.txt`. Each redirect used a fresh `session_id` so the repeat window did not apply. `data-latency\audit.jsonl` holds 5 `large_file`, 5 `short` and 5 `range` records. | PASS |

### Hook audit records from the isolated session data directory

`tmp\hook-live-53145\data\audit.jsonl` holds 14 hook records (6
`HOOK_REDIRECT`, 8 `HOOK_PASS`) and 1 `IO_SHUNT_DELEGATED` record. Each
record carries `agent_harness: "claude"`, `project: "project"`, and
`details{decision, reason, tool, file_path, lines}`. No record contains file
content.

| ts (UTC) | event | tool | reason | lines | file |
|---|---|---|---|---|---|
| 17:12:41 | HOOK_REDIRECT | Read | large_file | 150 | big.rs |
| 17:12:43 | HOOK_PASS | Read | repeat | null | big.rs |
| 17:13:14 | HOOK_PASS | Read | range | null | big.rs |
| 17:13:17 | HOOK_PASS | Read | short | null | short.rs |
| 17:13:18 | HOOK_PASS | Read | security_critical | null | auth_tokens.rs |
| 17:13:32 | HOOK_REDIRECT | Read | large_file | 150 | big.rs |
| 17:14:14 | HOOK_REDIRECT | Read | large_file | 150 | big.rs |
| 17:14:15 | HOOK_PASS | Read | repeat | null | big.rs |
| 17:14:27 | HOOK_PASS | Read | range | null | big.rs |
| 17:14:46 | HOOK_REDIRECT | Read | large_file | 150 | big.rs |
| 17:14:52 | HOOK_PASS | Read | range | null | big.rs |
| 17:15:05 | HOOK_REDIRECT | Read | large_file | 150 | big.rs |
| 17:17:19 | HOOK_REDIRECT | Read | large_file | 150 | big.rs |
| 17:17:21 | HOOK_PASS | Read | short | null | short.rs |

### Contract observations against research.md

- The input shape matched research.md. Read's `file_path` arrived absolute
  (`D:\Source\...\big.rs`), with `offset` and `limit` when they were given.
  Every decision was correct, so the adapter parsed the input. The raw
  stdin was not captured.
- The deny JSON on stdout with exit 0 is honoured, and the agent sees the
  reason. Claude Code adds the prefix `PreToolUse:Read hook error: ` to the
  reason in the tool result, which research.md does not mention. Every
  agent still understood that the call had been blocked on purpose.
- Research.md left PreToolUse timeout behaviour open. It is now settled: on
  timeout the hook is cancelled and the tool call proceeds. A handler that
  fails to spawn also proceeds.
- Research.md expected read-before-edit to be enforced. In 2.1.302 headless
  sessions it was not enforced: an Edit with no prior read succeeded (case
  6d).

### Concerns

- **C1. The agent bypasses the redirect easily.** The reason ends with
  `Repeat this read within 2 minutes to read the whole file anyway.` In case
  1, with a neutral prompt ("If a tool call is blocked, follow the
  instructions in the block message"), the agent repeated the full read at
  once, so the redirect saved nothing. Requirement 2.5 holds (no deadlock),
  but the wording invites the bypass. Design should consider rewording the
  reason to make `smart_read` and the ranged read the default, and the
  repeat a last resort.
- **C2. The redirect does not tell the agent how to call `smart_read`.**
  `smart_read` requires a `project` argument, and the reason does not
  mention it. In case 1b the agent's first `smart_read` call failed with
  `missing field 'project'`. With a project given (case 1c), `smart_read`
  worked.
- **C3. This Claude Code version did not let Requirement 2.8 be tested
  strictly.** The Edit after an exact-range read succeeded (case 6), but so
  did Edits with no successful read (cases 6c and 6d). So the redirect
  cannot make Edit fail here, and there is no need to return to design. The
  open question in research.md, whether a ranged read counts as a read
  before an edit, still has no answer.
- **C4. Pass records carry no line count.** Pass audit records have
  `lines: null`, even for `short` (20 lines). Only redirects record a count,
  and that count is capped at the threshold (150).
- **C5. Shell-read routing was not exercised.** The matcher included
  `Bash|PowerShell`, but no session ran a `cat` or `Get-Content` read, and
  Bash was not in `--allowedTools`. Live shell-read routing is still
  unverified; the unit tests from task 1.3 cover it.
