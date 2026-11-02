# Handoff: one-line installers (install.sh / install.ps1) + README quickstart rewrite

Status: **IN PROGRESS, blocked mid-task.** This file is the source of truth
for whoever picks this up — do not assume any other in-flight context.

## Objective (user directives, near-verbatim)

- README must document a **per-platform install WITHOUT checkout, using the
  LATEST RELEASE build**. "releases are irregular; latest is the released
  variant to install. users dont need the code."
- **"this is a job that belongs in install.sh, which gets executed with a
  pipe to bash."** The multi-step fetch dance (`mktemp` + `$d`/`$b` + 6
  curl/irm lines) must NOT appear in the README. One-liners only.
- macOS needs **Intel AND Arm** builds (both are now shipped as release
  assets — done).
- **WSL IS A CI BUILD ENV ONLY — NEVER AN INSTALL TARGET.** No WSL in any
  user-facing install doc or flow. Normal Linux users install on Linux;
  Windows users install `mmhmem.exe`.
- Repo default branch is `main` (NOT master); raw URLs use the `HEAD` ref so
  docs do not depend on the branch name.

## Done and verified (do not redo)

- Release v0.2.2 ships all four assets:
  `mmhmem-linux-x86_64`, `mmhmem-macos-aarch64`, `mmhmem-macos-x86_64`,
  `mmhmem.exe`. The macOS pair was pulled from CI run 41438483764 artifact
  `mmhmem-macos` (Mach-O cputypes verified: 0x0100000C = arm64,
  0x01000007 = x86_64) and attached via `gh release upload`. All four
  `releases/latest/download/<asset>` URLs verified live (206 range probe).
- README commits already on main: `88c450d` (checkout-free quickstart,
  multi-step form) and `2fcb4b3` (WSL removed as install target). The
  multi-step POSIX/Windows blocks from `88c450d` are NOW CONSIDERED WRONG
  SHAPE by the user (the `$d` dance) and get replaced by the one-liner
  blocks below.
- Both installers' engine (`deploy/install_local.sh`, `deploy/install_local.ps1`)
  is UNCHANGED and correct: it resolves companion files from `$RepoRoot` =
  parent of its own dir, so a fetched tree `<tmp>/deploy/{...}` satisfies it
  (validated by a live `-DryRun` against the fetched layout on
  2026-11-02).

## In flight (uncommitted!)

- `install.sh` at the repo root: **WRITTEN but UNTESTED and UNTRACKED.**
  Thin bootstrap around the engine. Behavior: flag parse first (`--help`,
  `--remove`, no args → `set -- --global`), platform detect (Linux: refuse
  WSL via `/proc/version` `grep -qi microsoft`; require x86_64 else die;
  Darwin: `arm64`→`mmhmem-macos-aarch64`, `x86_64`→`mmhmem-macos-x86_64`;
  MINGW/MSYS/CYGWIN → die pointing at the Windows flow), download latest
  release binary to `~/.local/bin/mmhmem` (curl, wget fallback), `chmod +x`,
  print `--version`, fetch engine + `SKILL.md` + 3 recap command templates
  into a `mktemp` tree shaped `$tmp/deploy/{...}`, run
  `sh "$tree/install_local.sh" --binary "$HOME/.local/bin/mmhmem" "$@"`.
  `--remove` skips the binary and companion downloads (removal needs only
  the engine). **Review it, then TEST (below) before committing.**
- `install.ps1`: NOT yet written. Spec below.
- README: still has the old multi-step blocks (lines ~72–118 at HEAD).

## Remaining work, in order

1. **Write `install.ps1` at the repo root** — the Windows mirror:
   - One-liner target:
     `irm https://raw.githubusercontent.com/norandom/mmhmem/HEAD/install.ps1 | iex`
   - Piped (`iex`) passes NO args: defaults = global install for every
     detected harness (engine `-Global`, no `-Agent`, no worker settings).
     For flags the docs tell users to save the script first:
     `irm <url> -OutFile install.ps1`, then `.\install.ps1 -Agent claude`,
     `.\install.ps1 -Remove`, `.\install.ps1 -WorkerModel qwen3:latest`.
   - Params: `[string]$Agent`, `[string]$WorkerModel`, `[string]$WorkerUrl`,
     `[string]$WorkerApiKey`, `[switch]$Remove`, `[switch]$DryRun`
     (DryRun forwards to the engine for testing).
   - Refuse non-Windows and Windows ARM64 (no asset).
   - Binary: download `releases/latest/download/mmhmem.exe` to
     `$env:LOCALAPPDATA\Programs\mmhmem\mmhmem.exe` (matches the engine's
     global copy target, so the engine skips its own copy). try/catch the
     download — a running harness locks the exe; die with "close mmhmem
     harnesses and re-run".
   - Fetch engine + companions into a per-run temp tree shaped
     `<tmp>\deploy\{install_local.ps1, skills\unified-memory\SKILL.md,
     commands\{claude,opencode,codex}\mmhmem-recap.md}`; `-Remove` fetches
     only the engine.
   - **CRITICAL:** run the engine in a CHILD powershell process
     (`& powershell -NoProfile -File <engine> -Global ...`) — the engine
     calls `exit`, which under `irm | iex` would kill the user's whole
     console session. Check `$LASTEXITCODE` after. PS 5.1 compatible;
     `finally`-cleanup of the temp tree.
2. **Rewrite the README quickstart** (lines ~72–118 at HEAD). Replace the
   "The installer needs only itself and four companion files…" paragraph,
   the whole POSIX block (`d=$(mktemp -d)` …) and the whole Windows block
   (`$d = "$env:TEMP\mmhmem-install"` …) with the verbatim blocks below.
   Also change the intro sentence "`releases/latest/download/…` URLs below"
   → "one-liners below". Keep: the asset table, the "`HEAD` … default
   branch" sentence, the "Or build it yourself" section.
3. **Test BEFORE committing** (no unverified commits to main):
   - `install.sh`, in the WSL build VM (running bash for CI/scripts there is
     fine; executing mmhmem there is not — the script must refuse before any
     download): `bash -n install.sh` (syntax); `cat install.sh | bash` must
     die with the WSL refusal; `cat install.sh | bash -s -- --help` must
     print usage. PS 5.1/wsl.exe gotcha: put the test commands in a script
     file in `./tmp` and run
     `wsl -d Debian -e bash /mnt/d/Source/mmhmem/tmp/<testscript>`.
   - `install.ps1`, on the dev host WITHOUT touching the real install: set
     `$env:LOCALAPPDATA` to `D:\Source\mmhmem\tmp\fakehome` in the child
     scope, run `install.ps1 -DryRun -Agent opencode` — the engine dry-run
     must enumerate the opencode registration + playbook + recap template +
     binary (fake path) and write NOTHING. All scratch in `./tmp`
     (git-ignored, cleaned after) — never AppData.
   - After push: verify the raw URLs serve (HTTP 200):
     `raw.githubusercontent.com/norandom/mmhmem/HEAD/install.sh` and
     `.../install.ps1`.
4. **Commit and push** all three together (install.sh, install.ps1,
   README), e.g.
   `feat(install): one-line installers (pipe to bash / iex); README quickstart follows`.
   Confirm CI green (script/docs-only change). The earlier docs commits
   (23e216d steering, 88c450d, 2fcb4b3) should already be green — check
   `gh run list`.
5. **Checkpoint the session bus** (`session_resume_or_handoff` with
   `updated_working_state_md`, project `mmhmem`) so cross-agent pickup sees
   completion, and record a `mem_record_observation` (type `feature`).

## README replacement blocks (verbatim)

POSIX block (replaces the `d=$(mktemp -d)` block):

````markdown
Linux and macOS install with one pipe-to-bash line (root `install.sh`):

**Linux / macOS**

```sh
# one line: latest release binary + installer + playbook — every detected
# harness, user-wide; shunting stays off until you configure a worker model
curl -fsSL https://raw.githubusercontent.com/norandom/mmhmem/HEAD/install.sh | bash

# flags pass straight through to the installer, e.g. one harness only:
curl -fsSL https://raw.githubusercontent.com/norandom/mmhmem/HEAD/install.sh \
  | bash -s -- --global --agent claude

# uninstall (deregisters harnesses; your per-user store is kept)
curl -fsSL https://raw.githubusercontent.com/norandom/mmhmem/HEAD/install.sh \
  | bash -s -- --remove --global
```

`install.sh` detects the platform, downloads the binary to
`~/.local/bin/mmhmem`, fetches `deploy/install_local.sh` with the playbook
and the `/mmhmem-recap` command templates, and runs the installer with your
flags. It refuses on WSL (a CI build environment, not an install target —
use the Windows flow below) and on Linux aarch64 (no release asset yet —
build from source).
````

Windows block (replaces the `$env:TEMP\mmhmem-install` block):

````markdown
**Windows**

```powershell
# one line: latest release binary + installer + playbook — every detected
# harness, user-wide
irm https://raw.githubusercontent.com/norandom/mmhmem/HEAD/install.ps1 | iex

# flags need a saved copy (iex passes none), e.g.:
irm https://raw.githubusercontent.com/norandom/mmhmem/HEAD/install.ps1 -OutFile install.ps1
.\install.ps1 -Agent claude        # one harness
.\install.ps1 -Remove              # uninstall (the store is kept)
```
````

## Hard rules and gotchas (the next actor WILL hit these)

- WSL = CI build env ONLY. Never an install target; never execute mmhmem
  there; no scratch storage there. Bash for build/CI/test scripts is fine.
- ALL dev scratch lives in `./tmp` (git-ignored, cleaned after use) — never
  AppData, never /tmp outside the repo.
- PS 5.1 strips inner double quotes from native args; `wsl.exe` mangles
  embedded double quotes in `bash -c` strings — use script files.
- `gh` on this host: ONE file per `gh release upload` call ("accepts at most
  1 arg(s), received 3"); keep `--jq` expressions single-arg.
- Repo is physically at `D:\Source\mmhmem` (the `C:` path is a junction).
- After any local gate run: `opencode service restart` (reconnects the
  mmhmem MCP child; expect tools=9).
- The engine (`deploy/install_local.*`) stays the registration machinery;
  the root installers are thin bootstraps. Do not duplicate engine logic.

## Queue after this lands (user-set priority)

1. Dogfood codex on LINUX — the whole point of the one-liner:
   `curl -fsSL https://raw.githubusercontent.com/norandom/mmhmem/HEAD/install.sh | bash -s -- --global --agent codex`
   (engine registers the managed `[mcp_servers.mmhmem]` block in
   `~/.codex/config.toml` + the `/mmhmem-recap` prompt in `~/.codex/prompts`)
2. codex on WINDOWS: `.\install.ps1 -Agent codex`
3. claude harness install on Linux; Linux VM dogfood (a real VM, not WSL);
   multiprocess fresh-open flake (resilience + child-stderr capture in
   `wait_for`); adaptive-cache spec (see
   `.kiro/specs/corpus-warmup-prune/handoff.md`).
