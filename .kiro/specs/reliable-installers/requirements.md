# Requirements Document

## Project Description (Input)

The mmhmem installers have never produced a working install. Runs have left
harness config files broken, needed flags that only project insiders know,
and failed with `404: Not Found` because the bootstrap mixes three versions
(its own file list, scripts from GitHub `HEAD`, the `releases/latest`
binary). Success was judged by commands finishing, not by the agents working.

The user requires one clear installer per platform that a person with no
knowledge of the project can find and run, that never breaks their config
files, that installs only published releases (users never install from a
repository checkout), and that ends by proving each agent can actually use
mmhmem. Findings are in `brief.md`.

## Introduction

This specification governs how a user installs, updates and removes mmhmem
for Claude Code, Claude Desktop, Codex and OpenCode on Linux, macOS and
Windows, and how the installer proves the result works.

## Boundary Context

- **In scope:** the user-facing installer on each platform, its questions
  and messages, every file it changes, the functional check at the end,
  update, removal, and the CI tests that prove all of this.
- **Out of scope:** installing from a source checkout (a maintainer
  concern, kept out of user documentation and out of the user installer);
  what hooks and routing instructions decide at run time
  (`read-routing-hooks`, `agent-tool-routing`); the release build
  (`static-linux-release`); server deployment under `deploy/systemd`.
- **Adjacent expectations:** release assets and `SHA256SUMS` keep being
  published by the existing verified release pipeline.

## Requirements

### Requirement 1: One obvious way to install

**Objective:** As a user who has never seen the project, I want one command
for my platform, so that I can install without reading the code or docs.

#### Acceptance Criteria

1. The project shall offer exactly one install command for Linux and macOS and exactly one for Windows, each a single line the user can copy.
2. The README and the release page shall show these two commands first, before any other installation text.
3. The install command shall need no flags, options or environment variables for a normal install, update or removal.
4. The installer shall describe each step in plain words a user without knowledge of mmhmem understands, and shall not use project-internal terms (such as playbook, shunt, routing instructions or worker) without a one-line explanation.
5. When the install finishes, the installer shall tell the user exactly what to do next in each agent, such as restarting it or approving a prompt.

### Requirement 2: Published releases only

**Objective:** As a user, I want the installer to install a tested release,
so that I never get half-finished code.

#### Acceptance Criteria

1. The installer shall install only a published, stable release, and every file it writes shall come from that one release.
2. The installer shall not download anything from a branch such as `HEAD` or `main`, and shall not read files from a source checkout, even when it is run from inside one.
3. The installer shall verify every file it downloads against the `SHA256SUMS` of the same release before using it, and shall show the release tag and the verified SHA-256 of the binary.
4. The installer shall check that `SHA256SUMS` itself was published by the project's release pipeline, using a mechanism chosen in design (for example a signature checked against a public key built into the installer), so that a replaced checksum file is detected.
5. If any download or verification fails, then the installer shall stop before changing anything, keep any previously installed version working, and say in plain words what failed, including expected and actual SHA-256.
6. When showing status, the installer shall re-hash the installed binary and report whether it still matches the install record.
7. If the platform has no release, then the installer shall say so and change nothing.
8. The Windows release shall be published as a `.zip` (or a bare `.exe`) that the Windows installer unpacks with built-in PowerShell commands; the installer shall not depend on `tar`, `.tgz` or any tool that is not part of a default Windows install. Linux and macOS releases shall use `.tar.gz` unpacked with the system `tar`.

### Requirement 3: Config files are never broken

**Objective:** As a user, I want my agents' config files to stay valid, so
that installing mmhmem never breaks an agent I already use.

#### Acceptance Criteria

1. When the installer changes an agent's config file, it shall change only the entries mmhmem owns and keep every other setting, comment and formatting unchanged.
2. Before replacing a config file, the installer shall check that the new content is valid for that agent's file format, and shall write it atomically so an interrupted install leaves either the old or the new file.
3. The installer shall keep a backup of every config file it changes and tell the user where it is.
4. If an existing config file is already invalid, then the installer shall leave it unchanged, name the file and the problem, and continue with the other agents.
5. If the functional check (Requirement 6) fails for an agent after its config was changed, then the installer shall restore that agent's original config files and say that it did.
6. The installer shall not require Python, Node.js or any other runtime beyond the shell the install command runs in.

### Requirement 4: A real menu

**Objective:** As a user, I want a proper interactive menu, so that I can see
my current setup and change it without typing answers or flags.

#### Acceptance Criteria

1. When started in a terminal, the installer shall open a keyboard-driven menu (arrow keys, space to toggle, enter to confirm, escape to go back, a visible key legend) in Windows Terminal, the Windows PowerShell console, macOS Terminal and common Linux terminals.
2. The menu's first screen shall show the current state from the install record (Requirement 5): installed version, newest release, and per agent whether mmhmem is installed, which features are on, and the last check result.
3. The menu shall offer install or update, change features, check, and remove, and shall offer only the actions that apply to the current state.
4. The menu shall list each detected agent as a checkbox, preselected for a new install, and shall list agents that are not detected as unavailable with the reason.
5. The menu shall list optional features, including the read-routing hook, as checkboxes per agent, each with a one-line plain explanation, switched off by default.
6. Before changing anything, the menu shall show a review screen listing every file and entry it will add, change or remove, and shall change nothing until the user confirms it.
7. When the user cancels or quits at any screen, the installer shall exit without changing anything.
8. If the terminal cannot show the full menu, then the installer shall show the same choices as a numbered list.
9. When run without a terminal from the install command (the bootstrap), the installer shall install the default selection (all detected agents, no optional features), or stop without changes if no agent is detected. When `mmhmem setup` is run without a terminal any other way (for example by an AI agent in a non-interactive shell), it shall change nothing and print the command to run in a terminal. (Amended 2026-11-04 by user decision.)

### Requirement 5: A real install record

**Objective:** As a user, I want the installer to know exactly what it
installed, so that status, update and removal are based on facts, not
guesses.

#### Acceptance Criteria

1. When the installer changes anything, it shall keep one install record per user that lists the installed release, the binary's path and SHA-256, the selected agents and features, and every file and entry it added or changed with its SHA-256 after the change and the path of its backup.
2. The installer shall update the install record atomically together with each change, so that the record always describes what is actually on disk, including after an interrupted run.
3. When showing status, updating or removing, the installer shall compare the record with the files on disk and report any managed file or entry that was changed or deleted outside the installer.
4. If a managed entry was changed outside the installer, then the installer shall show the difference and ask before overwriting or removing it.
5. The install record shall store the result and time of the last functional check per agent.
6. If the install record is missing or unreadable, then the installer shall rebuild what it can from the files on disk, say that it did, and change nothing without confirmation.
7. The install record shall contain no keys or other secrets.

### Requirement 6: The installer proves each agent works

**Objective:** As a user, I want the installer to show that each agent can use
mmhmem, so that "installed" means working, not "commands ran".

#### Acceptance Criteria

1. When an install or update finishes, the installer shall, for each selected agent, start mmhmem exactly as that agent's config starts it and complete an MCP handshake, list mmhmem's tools, and call one tool that has no side effects.
2. Where an agent offers its own command to check MCP servers, the installer shall run it and require that it report mmhmem as connected.
3. The installer shall check that each agent's config file still loads in that agent, using the agent's own command where it has one.
4. Where the read-routing hook was selected, the installer shall run the hook as that agent would, with a sample read, and require the expected answer.
5. The installer shall print one line per agent with the result, and for any failure shall say in plain words what is wrong and what the user can do.
6. The installer shall report steps only the user can do inside an agent, such as approving the hook in Codex `/hooks`, as still to do, not as passed.
7. The user shall be able to run the same check later by running the install command again and choosing status.

### Requirement 7: Safe updates and removal

**Objective:** As a user, I want updating and removing to be safe, so that I
can change my mind at any time.

#### Acceptance Criteria

1. When the installer runs again on an up-to-date install, it shall change nothing and report that everything is current and working.
2. When a newer release exists, the installer shall update the binary and every file it installed, keep the user's choices, and run the functional check again.
3. When the user chooses remove, the installer shall remove only what mmhmem added and leave every other setting in shared config files unchanged.
4. The removal shall keep the user's stored memories unless the user explicitly chooses to delete them.

### Requirement 8: One implementation for every platform

**Objective:** As a maintainer, I want each agent integration written once,
so that the Linux, macOS and Windows installers cannot drift apart.

#### Acceptance Criteria

1. The agent integrations, the menu, the install record and the functional check shall be implemented once and shared by all platforms.
2. The platform install scripts shall contain no agent-specific logic beyond downloading, verifying and starting that shared implementation.
3. Config files shall be read and written with established parsers for their formats, not with text substitution.
4. Each agent integration shall be one module with the same set of operations (detect, plan, apply, check, remove), so that adding or fixing an agent touches one place.
5. Every change shall first be computed as a plan that the review screen shows and that is then applied unchanged, so the user confirms exactly what happens.
6. All installer code shall live in one documented location, and the scripts and helpers it replaces shall be deleted from the repository.

### Requirement 9: Worker key protection

**Objective:** As a user, I want my worker API key protected, so that other
local users cannot read it.

#### Acceptance Criteria

1. When the installer writes a config file containing a key, it shall restrict the file to the current user on every platform from the first byte written.
2. The installer shall never print the key.

### Requirement 10: Proven before release

**Objective:** As a maintainer, I want every installer change proven like a
real user install, so that a broken installer is never released.

#### Acceptance Criteria

1. The CI pipeline shall run the exact user install command on Linux, macOS and Windows against release artifacts built from the same commit.
2. Each CI run shall start from a clean user home with existing agent config files that contain unrelated settings, including comments and unusual formatting.
3. Each CI run shall install, pass the functional check, run again to prove nothing changes, update from the previous release, remove, and confirm that the unrelated settings are byte-identical to the start.
4. Each CI run shall also cover a cancelled install, a failed download, a bad checksum and an already-invalid config file, and confirm that nothing was changed.
5. If any of these fails, then the CI pipeline shall fail and block publishing the release.
