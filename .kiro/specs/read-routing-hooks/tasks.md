# Implementation Plan

All tasks are required. Each executable task is sized for approximately 1–3 hours.
Order follows the approved Delivery Order: core routing for Claude Code first,
then consent and trust, then Codex and OpenCode, then periphery. Every commit
must pass the project pre-commit gate (fmt, size caps, clippy default and `otel`).

- [ ] 1. Build the decision core
- [x] 1.1 Share the security rule and add the hook audit event types
  - Expose the shunt engine's security-critical path rule as a plain function over the existing pattern and have the engine use it, so the hook and the server apply one rule.
  - Add redirect and pass audit event types with their stable string names.
  - Done when the existing shunt tests still pass, a new test shows the function and the engine agree on security-critical and ordinary paths, and the new event names serialize as specified.
  - _Requirements: 2.3, 5.4_

- [x] 1.2 (P) Implement the read decision with its unit tests
  - Decide a read request in the design's order: exact range, missing or not a regular file, security rule, no worker, not text, below threshold, otherwise redirect.
  - Count lines by streaming with an early stop at the threshold, and treat a NUL byte in the first 8 KiB as not text.
  - Done when a decision-matrix test covers every pass reason and the redirect, and a 10 MiB text file is decided in under 200 ms.
  - _Boundary: decide_
  - _Depends: 1.1_
  - _Requirements: 2.1, 2.2, 2.3, 2.4, 3.3_

- [x] 1.3 (P) Recognise whole-file reads in shell commands
  - Accept only one simple command that reads one whole file (`cat`, `type`, `less`, `more`, `Get-Content` and its aliases without range options), resolving the path against the working directory.
  - Reject pipes, redirects, command separators, subshells, substitutions, globs, several files, partial readers and any `Get-Content` range option.
  - Unwrap shell-invocation arrays such as `bash -lc "<script>"` before recognising.
  - Done when accept and reject tables for Bash and PowerShell syntax pass.
  - _Boundary: shell_
  - _Requirements: 2.6_

- [x] 1.4 (P) Implement the repeat-read window
  - Remember a redirect per harness, session and file for 120 seconds in a small state file in the local data directory, pruning entries older than an hour and writing atomically.
  - Report whether a request repeats a recent redirect, and tolerate a missing, corrupt or concurrently replaced state file by treating it as empty.
  - Done when tests cover a repeat inside and outside the window, pruning, and a corrupt file.
  - _Boundary: state_
  - _Requirements: 2.5_

- [ ] 2. Route Claude Code reads
- [x] 2.1 Map Claude Code hook input and output
  - Parse the pre-tool event for the native read tool and the Bash and PowerShell tools into a read request, treat every other tool as not a read, and treat input as untrusted.
  - Render a redirect as the documented deny decision with the one-line reason, and a pass as no output.
  - Done when golden input and output tests pass for read with and without a range, recognised and unrecognised shell commands, and non-read tools.
  - _Depends: 1.2, 1.3_
  - _Requirements: 2.6, 2.7, 5.5, 6.1_

- [x] 2.2 Add the fail-open hook command
  - Add the hook subcommand for a named harness: bounded stdin, only the threshold and worker presence from the local profile, the repeat window, one audit event per read decision without file content, adapter output, exit 0.
  - Answer pass with exit 0 on any error, oversized or malformed input, or panic.
  - List the command in the help text.
  - Done when tests show garbage, oversized and panicking input pass with exit 0, a large file is redirected once and then passes on repeat, and audit events carry decision, reason, tool, path and line count but no content.
  - _Depends: 1.4, 2.1_
  - _Requirements: 2.4, 2.5, 3.1, 3.2, 3.3, 3.4, 5.2, 5.3, 5.4_

- [x] 2.3 Verify the Claude Code hook live
  - Run headless Claude Code sessions in a scratch project with the hook configured through a temporary settings file: a full read of a large file, a ranged read, a short file, a security-critical file, a repeated full read, and an edit of the redirected file.
  - Measure the hook's decision time and confirm fail-open behaviour with a broken binary path and with a timeout.
  - Done when the validation report records each outcome, including whether the edit succeeded after an exact-range read; if it did not, stop and return to design.
  - _Depends: 2.2_
  - _Requirements: 2.1, 2.2, 2.3, 2.5, 2.8, 3.1, 3.3, 6.1_

- [ ] 3. Make the hook acceptable to install
- [ ] 3.1 Manage the Claude Code entry on Windows
  - Show, install, remove and report the Claude Code entry in the user settings file with an order-preserving edit, a backup, one entry per repeated install, and every other key and hook kept.
  - Refuse when user or managed settings disable hooks, naming the setting; verify the managed settings location for Windows.
  - Done when Windows install tests show idempotent install, exact removal, preserved neighbours, refusal under policy, and no change to any project settings file.
  - _Boundary: manage_hooks.ps1_
  - _Depends: 2.2_
  - _Requirements: 4.2, 4.3, 4.4, 4.6, 4.7, 4.9, 5.1_

- [ ] 3.2 (P) Manage the Claude Code entry on Linux and macOS
  - Provide the same contract for the Unix installer with Python's order-preserving JSON, the same identity rule and the same policy checks for the Linux and macOS managed settings locations.
  - Done when Unix install tests cover the same cases as 3.1.
  - _Boundary: manage_hooks.py_
  - _Depends: 2.2_
  - _Requirements: 4.2, 4.3, 4.4, 4.6, 4.7, 4.9, 5.1_

- [ ] 3.3 Offer the hook as an explicit opt-in in both installers
  - Add the opt-in flag and an interactive prompt (default no) that shows the exact entry, the file, what the hook sees and decides, and how to remove it; add hook-only removal and include the hook in full removal.
  - Print the harness's own approval step after installing, and say where only advisory guidance applies.
  - Pass the options through the one-line installers and document them in the installation guide.
  - Done when installer tests show that a default install adds no hook, the opt-in adds exactly one, removal leaves settings unchanged otherwise, and dry runs write nothing.
  - _Depends: 3.1, 3.2_
  - _Requirements: 4.1, 4.2, 4.5, 4.6, 6.4, 6.5_

- [ ] 4. Extend coverage to Codex and OpenCode
- [ ] 4.1 Route Codex shell reads
  - Map Codex pre-tool input (shell command string or array) and output, and manage the Codex entry in the user hooks file in both helpers, with feature-flag and managed-policy checks and never writing trust state.
  - Verify with the real Codex CLI that a user hooks file coexists with the existing inline hook state without warnings, that the hook asks for approval in `/hooks`, and that an mmhmem binary update keeps it trusted; use a disposable Codex home for the live test.
  - Done when Codex adapter and installer tests pass and the validation report records the live redirect, approval prompt and trust persistence.
  - _Depends: 2.2, 3.3_
  - _Requirements: 4.4, 4.5, 4.8, 6.2_

- [ ] 4.2 Route OpenCode reads through a v2 plugin
  - First prove with the installed OpenCode 2.2.5 that a v2 plugin can spawn a process and block a call with a message; stop and return to design if it cannot.
  - Then ship the plugin that forwards read and shell events to the hook command with a short time limit, throws only on a deny answer and catches every other failure, and let both helpers install and remove it.
  - Done when the live check records a redirected read, a passed ranged read, and a call that proceeds when the binary is missing.
  - _Depends: 2.2, 3.3_
  - _Requirements: 3.1, 6.3, 6.4_

- [ ] 5. Measure adoption and effect
- [ ] 5.1 Show read activity in `mmhmem stats`
  - Count, per day, project and harness, summary reads, cache hits, exact slices, direct reads and hook redirects from the audit log, read-only, including rotated files.
  - Show them in the table and under stable JSON fields while keeping every savings field unchanged.
  - Done when stats tests cover a day with activity but no savings rows and the JSON contract.
  - _Requirements: 1.1, 1.2, 1.3, 1.4, 1.5_

- [ ] 5.2 Record the measured effect
  - Requires working sessions by the user with each enabled harness, before and after enabling the hook.
  - Compare adoption counts, redirects followed by `smart_read`, repeat-read pass-throughs and estimated tokens avoided, and apply the NOT_VERIFIED rule.
  - Done when the validation report holds the comparison for every enabled harness.
  - _Depends: 5.1, 2.3_
  - _Requirements: 7.1, 7.2, 7.3_

## Implementation Notes

- Task 1.1 (Sonnet, inline review): `src/services/shunt.rs` is at 489 of 500 lines, so later work must not grow it. `tests/audit_redaction.rs` pins the event-type count (now 20).
- Task 1.2 (Opus, Opus reviewer APPROVED): line count matches the server's `str::lines`; `Redirect.lines` is capped at the threshold and means "at least N" (redirect text must say so). Parent applied the reviewer's suggestion: invalid UTF-8 in the 8 KiB probe now passes as NotText, because `smart_read` reads with `read_to_string`. Open suggestions: a byte cap for huge files with few newlines, and a test pinning early stop.
- Task 1.3 (Opus, Opus reviewer APPROVED): `Get-Content -Raw` is accepted (it reads the whole file). Parent added the reviewer's hardening: any Unicode whitespace other than space and tab means not recognised. Requirement 2.6 corrected: `head`/`tail` are partial readers and pass. For task 2.1: `recognise` has no shell dialect, so a few dialect-specific false positives exist (bash `type x`, Git Bash `cat src\\main.rs`), all recoverable through the repeat window; paths are joined without normalisation.
- Task 1.4 (Sonnet, inline review): `RepeatWindow::new(dir, window_ms)` / `with_default_window(dir)`; keys are SHA-256 of harness, session and path, so the state file holds no plain paths; `remember` creates the directory and returns an error the caller ignores (fail open).
- Task 2.1 (Sonnet, inline review): `adapters::parse(Harness, &[u8]) -> ParsedCall {harness, session_id, cwd, tool, read}` and `render(Harness, &Decision, &Path) -> String`; any integer offset or limit makes a ranged read; a relative path with an empty cwd is not a read. Redirect reason says "at least <n> lines".
- Task 2.2 (Opus, Opus reviewer APPROVED): release-build decisions 11-43 ms, 10 MiB file 16-24 ms. The project `.mmhmem.json` is resolved against the hook process's working directory (existing loader), so the OpenCode plugin (4.2) must spawn the hook with the project as cwd. Audit reasons: `large_file` for redirects, snake_case pass reasons, `lines` null on pass. Parent replaced `eprintln!` with non-panicking stderr writes and clarified Requirement 5.2 to name the repeat-window state file.
- Task 2.3 live findings (Claude Code 2.1.302): all routing and fail-open cases pass; a slow hook is cancelled at its timeout and the read proceeds; Claude prefixes the reason with `PreToolUse:Read hook error:`. The first reason text made the agent repeat the read immediately and omitted `smart_read`'s required `project`; the parent changed the reason to give the full `smart_read` call (project, agent_harness, file_path) first and the repeat only as a last resort, and re-ran the neutral case. Headless 2.1.302 did not enforce read-before-edit, so Requirement 2.8 holds but a ranged read was not strictly tested as the precondition.
- Task 2.3 re-run (after 329f429): when the hook fires (Read named explicitly), agents call `smart_read` correctly on the first try and use a ranged Read for detail (3/3), saving about 5,654 estimated tokens each; with a broad question they may still take the repeat (1/1). With the global mmhmem rule, neutral prompts already route to `smart_read` (4/4), so the hook is a backstop. The summary cache key is file path + content hash + question fingerprint (not project), so a custom `question` misses the cache; the hook's suggested call uses the default question.
