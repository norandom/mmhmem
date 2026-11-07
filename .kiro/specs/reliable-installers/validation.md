# Validation Log — reliable-installers

## 2026-11-07 — Post-release bugfixes (user reports 1–3, fixed before the next release)

All three fixes ride the same plan/review/record/backup machinery as the
rest of the installer; each carries unit tests.

### Report 1: the Antigravity snippet was never shown

- Symptom: the installer said "paste the snippet shown above" and "import
  the snippet that setup saved", but no view ever displayed or saved a
  snippet.
- Root cause: only `Antigravity::check` consumed `Antigravity::snippet()`;
  no plan change, no results line, no file existed.
- Fix: `plan` now writes `antigravity-mcp.json` next to the installed
  binary (idempotent, recorded, removed by `plan_remove`); the results
  screen prints the JSON under a "saved to <path>" line; the check
  screen's pending step names the file.
- Tests: `agents/antigravity_tests.rs` (plan saves, removes, is
  idempotent), `ui/text_tests.rs` (snippet lines), `ui/flow_tests.rs`
  (results next steps).

### Report 2: the worker model was "not getting detected" during the installer (tested on Linux)

- Reproduced on a fresh Linux musl build (0.5.1) driven through a PTY:
  startup detection of `~/.mmhmem.json` and of `WORKER_MODEL` both show
  "Worker model (configured)" correctly, and an empty home shows
  "(optional)". The real gaps: the features screen promised "setup will
  show you how to connect one after the install" but no such step
  existed, and the worker state was a snapshot taken when the menu
  opened.
- Fix: the worker row and the record sync read the live configuration on
  every screen build (a model connected while the menu is open shows
  without a restart); after an install with the Worker model ticked but
  none set up, the results end with the promised how-to: where
  `.mmhmem.json` is, what to put in it, and — when a local Ollama answers
  on `/api/tags` (derived from the worker api URL, 1.5 s timeout,
  fail-soft) — the model names found there.
- Tests: `ui/flow_tests.rs` (the how-to appears when ticked and absent
  when already configured), `worker_guide_tests.rs` (endpoint shape,
  fail-soft).

### Report 3: downloads froze the installer screen

- Root cause: `self_update` ran with progress reporting disabled
  (Cargo.toml: "no progress bar"); the menu blocked its UI thread inside
  the download; the bootstrap scripts silenced curl (`-fsSL`) and wget
  (`-q`), and install.ps1 used a silent `Invoke-WebRequest`.
- Fix: one in-place stderr line — `Downloading vX: 4.0/8.0 MB (50%) ·
  1.3 MB/s · 4s left` — TTY-gated, redrawn at most every 100 ms, with a
  smoothed speed estimate and a final unthrottled draw. The full-screen
  menu suspends its alternate screen around the download (rustup-style)
  so the line draws on the plain screen and the menu returns afterwards;
  the numbered fallback and `mmhmem update` print the line directly; the
  bootstrap scripts use curl's and wget's own meters, and install.ps1
  prefers curl.exe's meter with `Invoke-WebRequest` as the fallback.
- Tests: `download_line_tests.rs` (units, line formats, speed smoothing,
  in-place redraw, final draw, disabled reporter).
