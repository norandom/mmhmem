# Requirements Document

## Project Description (Input)

A review on 2026-11-09 of the Antigravity integration in `mmhmem setup`
(Windows 11, Antigravity CLI `agy` 1.5.3 plus the Antigravity IDE) found
that mmhmem never shows up in Antigravity:

- Setup saves a JSON snippet next to the binary and asks the user to paste
  it. The user pasted it into `~/.gemini/antigravity/mcp_config.json`, a
  location Antigravity no longer reads. Current Antigravity (CLI and IDE)
  loads global servers from `~/.gemini/config/mcp_config.json`, and
  `agy mcp list` reports "No MCP servers configured".
- Setup offers no read-routing hook for Antigravity, although Antigravity
  supports lifecycle hooks (a global `hooks.json`, `PreToolUse` with a tool
  name matcher, JSON on stdin and stdout).
- Detection looks for a program named `antigravity` (the CLI is `agy`) and
  otherwise treats the bare `~/.gemini` folder as proof, which the old
  Gemini CLI also creates.
- The functional check parses mmhmem's own snippet, so the install record
  reports "pass" for Antigravity without reading what Antigravity loads.
- The research that justified the design ("Antigravity has no CLI or
  documented config file", `reliable-installers/research.md` section 8) is
  stale: `agy mcp add/remove/list/enable/disable` and `agy plugin ...`
  exist and the config paths are documented.

The owner has an existing `mmhmem setup` install on this Windows machine
that must be brought to the corrected state by re-running setup or update,
not by a fresh install.

## Introduction

This specification corrects how `mmhmem setup` installs, checks, updates and
removes mmhmem for Antigravity (CLI and IDE) on Linux, macOS and Windows, so
that Antigravity is treated like the other agents: setup edits the files
Antigravity reads, installs the read-routing hook, proves the result with the
same functional check, and migrates installs made with the earlier
snippet-based integration.

## Boundary Context

- **In scope:** the Antigravity entry in the setup menu and its features;
  every file setup changes for Antigravity; the read-routing hook for
  Antigravity; the functional check for Antigravity; migration of existing
  installs; removal; the user-facing text and documentation about
  Antigravity; CI tests that prove all of this.
- **Out of scope:** the Antigravity plugin bundle under
  `plugins/unified-memory` (marketplace distribution is a separate route);
  installing the memory guide skill or recap command into Antigravity
  (their Antigravity locations are not confirmed; a later spec may add
  them); the hook eligibility rules themselves (`read-routing-hooks`); any
  other agent's integration.
- **Adjacent expectations:** the safe-write foundation, install record,
  functional check pipeline, update and removal flows of
  `reliable-installers` keep working unchanged and are reused; the hook
  engine of `read-routing-hooks` decides pass or redirect; the project's
  release pipeline publishes the resulting binary.

## Requirements

### Requirement 1: Antigravity is registered where it loads servers

**Objective:** As a user who selects Antigravity in setup, I want mmhmem to
appear in Antigravity's MCP servers after setup finishes, so that I do not
paste anything by hand.

#### Acceptance Criteria

1. When the user selects Antigravity with shared memory, setup shall register mmhmem in the location the current Antigravity CLI and IDE load global MCP servers from, without any manual step.
2. When setup changes that location, it shall change only the mmhmem entry and keep every other server, key, comment and formatting unchanged, with the same backup, validation and atomic-write guarantees as for the other agents.
3. If the location does not exist yet, then setup shall create it with only the mmhmem entry.
4. Setup shall no longer save or show a paste snippet for Antigravity and shall no longer list "import the snippet" as a step the user must do.
5. When setup finishes for Antigravity, the only step left for the user shall be to start a new Antigravity session.

### Requirement 2: Read-routing hook for Antigravity

**Objective:** As a user who turned the read-routing hook on, I want it to
work in Antigravity as it does in Claude Code and Codex, so that large
whole-file reads are routed to `smart_read` there too.

#### Acceptance Criteria

1. Where the read-routing hook is selected for Antigravity, setup shall install a hook that runs before Antigravity's file-read tool executes.
2. The hook shall apply the same eligibility rules as for the other agents: whole-file reads of files at or above the threshold are redirected to `smart_read`, while ranged reads and security-critical paths pass.
3. If the hook's input is unexpected, the worker model is not set up, or any error occurs, then the hook shall let the read proceed.
4. The hook's answer shall always be one Antigravity accepts, and a redirect shall tell the agent which `smart_read` call to make instead.
5. When setup installs the hook, it shall keep every other hook in Antigravity's hook settings unchanged.
6. While the read-routing hook is selected but no worker model is set up, setup shall say that the hook stays inactive, as it does for the other agents.

### Requirement 3: Detection reflects the installed Antigravity

**Objective:** As a user, I want the setup menu to tick Antigravity exactly
when Antigravity is installed, so that I am not offered it by mistake or
missed when I have it.

#### Acceptance Criteria

1. When the Antigravity CLI program is on the user's PATH, or Antigravity's own data folders (CLI, app or IDE) exist, setup shall report Antigravity as installed.
2. If only the Gemini CLI's home folder exists without any Antigravity program or data folder, then setup shall not report Antigravity as installed.
3. Where the Antigravity CLI can report its version, setup shall show that version next to Antigravity.

### Requirement 4: The functional check reads what Antigravity reads

**Objective:** As a user, I want "pass" for Antigravity to mean Antigravity
can start and use mmhmem, so that the result list is trustworthy.

#### Acceptance Criteria

1. When checking Antigravity, setup shall read the mmhmem registration back from the file Antigravity loads, not from setup's own plan, and shall start exactly that command for the handshake.
2. Where the read-routing hook is selected for Antigravity, the check shall run the hook with a sample read in Antigravity's format and shall report a hook that does not answer as a failure.
3. Where the Antigravity CLI is present, the check shall confirm that Antigravity's own server listing names mmhmem; where it is absent, the check shall mark that step as skipped and say why.
4. If the check fails for Antigravity after its files were changed, then setup shall restore the original files and say that it did, as for the other agents.

### Requirement 5: Existing installs are brought forward

**Objective:** As a user with an earlier snippet-based install, I want
re-running setup or update to fix Antigravity for me, so that I do not have
to uninstall and reinstall.

#### Acceptance Criteria

1. When setup or update runs on a machine whose install record has the earlier snippet-based Antigravity install, it shall plan the new registration and the removal of the snippet file it saved, and show both on the review screen.
2. If a mmhmem entry that points at the installed mmhmem program exists in a location Antigravity no longer reads, then setup shall show its removal on the review screen with a one-line explanation and remove only that entry on confirmation, leaving everything else in that file unchanged.
3. When the user removes mmhmem, setup shall take the mmhmem entry out of Antigravity's server settings and the mmhmem hook out of its hook settings, leaving the rest of both files unchanged, and shall tell the user nothing else is left to undo.
4. When an update or re-run finishes, the install record shall list the new managed entries and features for Antigravity and no step from the earlier integration.

### Requirement 6: Guidance tells the truth

**Objective:** As a user reading the README, the installation guide or the
setup screens, I want the Antigravity instructions to match what setup does,
so that I am not told to paste or import anything.

#### Acceptance Criteria

1. The README and the installation guide shall say that setup configures Antigravity itself and that the user only starts a new session.
2. Setup's screens, results and status output shall contain no reference to a snippet, an import or a file to paste for Antigravity.
3. The project's recorded agent facts shall state the confirmed Antigravity CLI commands, config locations and hook contract with their sources and the version they were confirmed against.
