# Installation

## Install

**Windows x64** (PowerShell 5.1 or 7):

```powershell
irm https://github.com/norandom/mmhmem/releases/latest/download/install.ps1 | iex
```

**Linux x86_64 and macOS** (Apple Silicon or Intel):

```sh
curl -fsSL https://github.com/norandom/mmhmem/releases/latest/download/install.sh | sh
```

These commands install the latest signed release. You need no checkout, no
Rust toolchain and no Python. The installer takes no options: you make every
choice in `mmhmem setup`.

## What the installer does

Each release ships its own `install.ps1` and `install.sh`. The installer:

1. picks the archive for your computer (Windows x64, Linux x86_64, macOS
   Apple Silicon or Intel) and downloads it from that release;
2. uses the archive only if its SHA-256 matches the one built into the
   installer, and stops with "Nothing was installed or changed" otherwise;
3. unpacks it into a temporary folder and starts `mmhmem setup` from there.
   The temporary folder is always removed.

The installer itself changes nothing outside that temporary folder. Setup
then checks the release signature before it installs the program. It needs
a terminal for its menus; on Linux and macOS it reads your answers from the
terminal even when the script is piped into `sh`.

On Windows the installer never calls `exit`, so your PowerShell window stays
open. Afterwards `$LASTEXITCODE` holds the result of `mmhmem setup` (0 means
success).

On Linux and macOS the installer needs `sh`, `curl` or `wget`, `sha256sum`
or `shasum`, `tar`, `mktemp` and `awk`. These are present on ordinary Linux
and macOS systems.

## Setup screens

`mmhmem setup` takes no options. In a normal terminal it shows a menu; in a
terminal that cannot draw one it asks numbered questions instead. Without
any terminal it uses the standard choices and says so.

1. **Tools**: Claude Code, Codex, OpenCode, Claude Desktop and Antigravity.
   The ones found on your computer are ticked. If Claude Desktop has more
   than one profile, setup asks which one to use.
2. **Features** per tool:
   - shared memory (required): registers mmhmem with the tool;
   - read-routing hook (on by default), see [below](#read-routing-hook);
   - memory guide: a short skill that tells the tool how to use the memory;
   - recap command: shows what you were working on last time;
   - agent instructions: short routing notes for the tool.
3. **Review**: every file setup will change, before anything is written.
   Press `d` to see the details of one change. Each file is backed up first;
   press Enter to apply. Files that hold secrets, such as `~/.claude.json`,
   are listed without their contents.

If setup finds what the old installer (before v0.5.0) set up, it takes that
over first and shows it as its own step. Nothing changes until you confirm.

### Checks

After applying, setup starts mmhmem exactly as each tool's configuration
starts it, lists its tools and calls the read-only `status` tool. It then
asks each tool's own command (`claude mcp get mmhmem`, `codex mcp list`,
`opencode mcp list`, `agy mcp list`) whether it sees mmhmem, and, with the
read-routing hook on, runs a sample read through it. Each tool gets a pass
or a plain reason and what you can do about it.

### Next steps per tool

- **Claude Code**: start a new session.
- **Codex**: start a new session and approve the mmhmem hook in `/hooks`.
  Codex skips hooks you have not approved, also silently in `codex exec`.
- **OpenCode**: start a new session. The read-routing plugin is the file
  `~/.config/opencode/plugins/mmhmem-read-routing.js`; OpenCode loads it
  without an approval step.
- **Claude Desktop**: quit it completely and open it again.
- **Antigravity**: start a new session. Setup configures Antigravity itself:
  mmhmem's server entry in `~/.gemini/config/mcp_config.json` and the
  read-routing hook in `~/.gemini/config/hooks.json`, the files the
  Antigravity CLI (`agy`) and IDE read. Without `agy` on your `PATH`, the
  check reads the server file only and says so.

## Read-routing hook

The read-routing hook sends reads of 300 lines or more (whole files, or line
ranges that large; `SHUNT_MIN_LINES`) to `smart_read`, so the agent gets a
cached summary instead of the full file. Smaller ranges pass. It is on by
default for Claude Code, Codex, OpenCode and Antigravity. Untick it on the
features screen if you do not want it.

It needs a worker model to make summaries. Without one it stays inactive and
setup says so; memory and targeted reads still work. See
[worker settings](configuration.md#worker-settings). Tick **Worker model**
on the features screen and setup shows how to connect one after the
install. Setup never downloads or starts a model.

Where the hook goes:

| Tool | Entry | Approval |
|---|---|---|
| Claude Code | `hooks.PreToolUse` in `~/.claude/settings.json` (or `$CLAUDE_CONFIG_DIR/settings.json`) | none |
| Codex | `PreToolUse` in `~/.codex/hooks.json` (or `$CODEX_HOME/hooks.json`) | approve in `/hooks` |
| OpenCode | `~/.config/opencode/plugins/mmhmem-read-routing.js` | none |
| Antigravity | `mmhmem-read-routing` in `~/.gemini/config/hooks.json`, for the `read_file` tool | none |

Setup never writes a trust or approval record for you. Every other key and
hook in those files stays as it was.

## Update

```sh
mmhmem update
```

It shows the installed and the newest version, asks once, then downloads the
new release, checks its signature and checksum, and keeps your tools and
choices. If the new version needs to change files in your tools, it lists
them and asks first; otherwise it only checks your tools. Start new agent
sessions afterwards.

`mmhmem update` exists since v0.5.1. On v0.5.0, run `mmhmem setup` and
choose **Update to ...**. Running the install command again offers the same
update.

## Change, check or remove

Run `mmhmem setup` and choose:

- **Change agents and features**: add or remove tools and features. The
  review screen shows the changes first.
- **Check that everything works**: runs the checks above again and changes
  nothing.
- **Remove mmhmem**: takes out only what mmhmem added and restores your
  files, then deletes the program and, on Windows, its `PATH` entry. Your
  saved memories are kept unless you tick "Also delete my saved memories".
  Copies of your settings from before the removal stay in the backups
  folder.

If you edited a file that setup manages, setup notices it and shows the
difference before it changes anything.

## Where things are installed

| | Windows | Linux | macOS |
|---|---|---|---|
| Program | `%LOCALAPPDATA%\Programs\mmhmem\mmhmem.exe` | `~/.local/bin/mmhmem` | `~/.local/bin/mmhmem` |
| Install record (`install.json`) | `%LOCALAPPDATA%\mmhmem` | `~/.local/state/mmhmem` (or `$XDG_STATE_HOME/mmhmem`) | `~/Library/Application Support/mmhmem` |
| Backups | `backups\<time>\` next to the record | `backups/<time>/` next to the record | `backups/<time>/` next to the record |
| Memory store | `%APPDATA%\mmhmem` | `~/.local/share/mmhmem` | `~/Library/Application Support/mmhmem` |

The install record lists what setup changed, so a later change, update or
removal touches only that. Backups are readable only by you; `<time>` is the
UTC time of the run.

On Windows, setup adds the program's folder to your user `PATH` as a
reviewed change, and removal takes it out again. Open a new terminal to use
it. On Linux and macOS setup never edits shell start-up files: if
`~/.local/bin` is not on your `PATH`, the results show the line to add.
Agent configurations always use the full path to the program.

## Check where a release came from

Setup already checks the release signature. To check the build provenance
of an archive yourself, download it from the release page and run:

```sh
gh attestation verify <archive> --repo norandom/mmhmem
```

## Platforms

| Platform | Archive target |
|---|---|
| Windows x64 | `x86_64-pc-windows-msvc` |
| Linux x86_64 | `x86_64-unknown-linux-musl` |
| macOS Apple Silicon | `aarch64-apple-darwin` |
| macOS Intel | `x86_64-apple-darwin` |

The Linux x86_64 release is statically linked with musl and needs no
dynamic loader or shared libraries. The same executable is verified on
Debian 12 and Alpine 3.22. Its support floor is Linux 5.10 and baseline
x86-64/SSE2; the release's `verification.json` records the kernel and CPU it
was actually tested on. Linux ARM64 has no release. On Windows, use the
Windows installer; WSL is not an install target.

## Agent connections and plugins

For manual local registrations, use [Claude Code's stdio command](../deploy/examples/claude-code.md),
[OpenCode](../deploy/examples/opencode-local.json), [Antigravity](../deploy/examples/antigravity-mcp_config-local.json)
(for `~/.gemini/config/mcp_config.json`) or the local section of [Claude Desktop](../deploy/examples/claude-desktop.md).
The [playbook guide](../deploy/skills/unified-memory/README.md) lists per-agent paths.

Setup installs the memory guide as a standalone skill. To manage it as a
plugin instead:

```sh
claude plugin marketplace add https://github.com/norandom/mmhmem.git
claude plugin install unified-memory@mmhmem
codex plugin marketplace add norandom/mmhmem
codex plugin add unified-memory@mmhmem
```

The plugin contains the skill, not the program. Untick **Memory guide** in
`mmhmem setup` (Change agents and features) to avoid two copies, then
restart the agent. See the [marketplace guide](../.marketplace/README.md)
for other supported manifests.

Codex gets `$mmhmem-recap` at `~/.agents/skills/mmhmem-recap/SKILL.md`. Use
it to recover the latest checkpoint and recorded activity. Start a new Codex
session if the skill picker has not refreshed.
